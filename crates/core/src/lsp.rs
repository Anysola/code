//! Lightweight LSP diagnostics client for `edit` and `write`: starts an installed language
//! server lazily per `(root, server)` when a matching source file is modified, opens the file
//! with its new text (`didOpen` + `didSave`), waits briefly for `textDocument/publishDiagnostics`
//! for that version, closes it again (`didClose`: the server reads every other file from disk,
//! so what a shell command, an undo or the user changed is what it sees), and returns
//! errors/warnings to append to the tool output. Diagnostics that aren't ready in time are not
//! guessed: nothing is returned rather than an earlier version's.
//!
//! Off by default: only `"lsp": true` in the user's own (or managed) config turns it on, and a
//! project document can only turn it off. A language server runs the project's build tooling
//! (`cargo metadata`, build scripts, a workspace's own TypeScript), outside the shell sandbox and
//! without the project approval its hooks or formatters wait for.
//!
//! Only well-known binaries resolved from `PATH` (absolute entries only) are spawned, never
//! project-local executables under `node_modules/.bin` or `.venv/bin`, and never commands from
//! project config; relative `PATH` entries are left out of the server's environment too. If no
//! matching server is installed, or LSP is off, diagnostics silently return empty with zero
//! overhead.
//!
//! Every write goes through one writer task with a timeout, so a server that stops reading never
//! holds up an edit; a server whose output can't be read, or that stops, is dropped and started
//! again on the next edit, unless it keeps failing (then it rests for a while). Servers stop when
//! idle, when their root goes ([`LspManager::shutdown_root`]), when too many run, and on exit:
//! `shutdown`/`exit` first, then their whole process group (Unix).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc, oneshot};
use url::Url;

/// Maximum diagnostics appended to a single tool result.
const MAX_DIAGNOSTICS: usize = 20;
/// Characters of one diagnostic's message kept (TypeScript prints whole types on one line).
const MAX_MESSAGE_CHARS: usize = 400;
/// The largest message a server may send; a bigger `Content-Length` drops the server.
const MAX_FRAME: usize = 64 << 20;
/// Messages waiting for the writer; past that a send waits (up to the write timeout).
const QUEUE: usize = 64;
/// Failures in a row (it died, its output couldn't be read) after which a server rests.
const CRASHES: u32 = 2;

/// Timeouts and limits of an [`LspManager`].
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// For the server's `initialize` response.
    pub init: Duration,
    /// For `publishDiagnostics` after a file edit.
    pub diagnostics: Duration,
    /// After the first diagnostics arrive, in case a follow-up batch lands.
    pub settle: Duration,
    /// For one message to be written to (or queued for) the server.
    pub write: Duration,
    /// Before a server that failed to start, or kept failing, is tried again.
    pub retry: Duration,
    /// Without an edit for this long, a server stops.
    pub idle: Duration,
    /// Servers running at once; starting one more stops the one used longest ago.
    pub servers: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            init: Duration::from_secs(8),
            diagnostics: Duration::from_millis(2500),
            settle: Duration::from_millis(120),
            write: Duration::from_secs(5),
            retry: Duration::from_secs(60),
            idle: Duration::from_secs(10 * 60),
            servers: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error = 1,
    Warning = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number.
    pub col: u32,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
}

/// `text` safe inside the `<diagnostics>` block: what the project wrote (a file name, a string
/// in a message) can't close it or open another tag.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// A message's first line, cut to [`MAX_MESSAGE_CHARS`] characters.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(MAX_MESSAGE_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

/// Formats diagnostics for appending to an `edit` or `write` tool result. Returns `None` when
/// there are no errors or warnings.
pub fn format_diagnostics(resource: &str, diags: &[Diagnostic]) -> Option<String> {
    if diags.is_empty() {
        return None;
    }
    let mut sorted = diags.to_vec();
    sorted.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.col.cmp(&b.col))
    });
    let total = sorted.len();
    let shown = total.min(MAX_DIAGNOSTICS);
    let mut out = format!("<diagnostics file=\"{}\">\n", escape(resource));
    for d in &sorted[..shown] {
        let level = match d.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
        };
        let msg = escape(&first_line(&d.message));
        out.push_str(&format!("{level} [{}:{}] {msg}\n", d.line, d.col));
    }
    if total > shown {
        out.push_str(&format!("... and {} more\n", total - shown));
    }
    out.push_str("</diagnostics>");
    Some(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ServerKey {
    root: PathBuf,
    id: &'static str,
}

#[derive(Debug, Clone)]
struct ServerSpec {
    id: &'static str,
    language_id: &'static str,
    command: PathBuf,
    args: Vec<String>,
}

/// `which`, absolute results only: a relative `PATH` entry would be looked up from the app's
/// folder and run from another.
fn which_absolute(name: &str) -> Option<PathBuf> {
    crate::which::which(name).filter(|p| p.is_absolute())
}

/// Language server lookup by file extension. Resolves only from system/user `PATH` so untrusted
/// repositories cannot execute arbitrary binaries from `node_modules/.bin` or `.venv/bin`.
fn detect_server(root: &Path, path: &Path) -> Option<ServerSpec> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let spec = |id, language_id, command, args: &[&str]| ServerSpec {
        id,
        language_id,
        command,
        args: args.iter().map(|a| a.to_string()).collect(),
    };
    match ext.as_str() {
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => {
            let lang = match ext.as_str() {
                "tsx" => "typescriptreact",
                "jsx" => "javascriptreact",
                "js" | "mjs" | "cjs" => "javascript",
                _ => "typescript",
            };
            let cmd = which_absolute("typescript-language-server")?;
            Some(spec("typescript", lang, cmd, &["--stdio"]))
        }
        "rs" => {
            if !has_ancestor_file(path, root, "Cargo.toml") {
                return None;
            }
            let cmd = which_absolute("rust-analyzer")?;
            Some(spec("rust", "rust", cmd, &[]))
        }
        "py" | "pyi" => {
            for (bin, args) in [
                ("basedpyright-langserver", &["--stdio"][..]),
                ("pyright-langserver", &["--stdio"][..]),
                ("pylsp", &[][..]),
            ] {
                if let Some(cmd) = which_absolute(bin) {
                    return Some(spec("python", "python", cmd, args));
                }
            }
            None
        }
        "go" => {
            let cmd = which_absolute("gopls")?;
            Some(spec("go", "go", cmd, &[]))
        }
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" => {
            let cmd = which_absolute("clangd")?;
            let lang = if matches!(ext.as_str(), "c" | "h") {
                "c"
            } else {
                "cpp"
            };
            Some(spec("clangd", lang, cmd, &[]))
        }
        _ => None,
    }
}

fn has_ancestor_file(path: &Path, stop_at: &Path, filename: &str) -> bool {
    let mut cur = path.parent();
    while let Some(dir) = cur {
        if dir.join(filename).is_file() {
            return true;
        }
        if dir == stop_at {
            break;
        }
        cur = dir.parent();
    }
    stop_at.join(filename).is_file()
}

fn file_uri(path: &Path) -> Option<String> {
    let canonical = crate::paths::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Url::from_file_path(canonical).ok().map(String::from)
}

/// A path as a stable key: canonical where it exists, lower-cased with `\` on Windows.
fn path_key(path: &Path) -> String {
    let canonical = crate::paths::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = canonical.to_string_lossy();
    if cfg!(windows) {
        text.replace('/', "\\").to_lowercase()
    } else {
        text.into_owned()
    }
}

/// Normalizes a `file://` URI into a stable map key across platforms (handles Windows drive-letter
/// casing and percent-encoded colons like `file:///c%3A/...` from `vscode-uri` servers).
fn uri_key(uri: &str) -> String {
    if let Ok(parsed) = Url::parse(uri)
        && parsed.scheme() == "file"
        && let Ok(path) = parsed.to_file_path()
    {
        return path_key(&path);
    }
    uri.to_string()
}

/// `PATH` with only its absolute entries: an empty or relative one (`.`, `node_modules/.bin`)
/// would make the server's own lookups (`node`, `cargo`, `go`) find the project's files first.
fn absolute_path_entries(path: Option<OsString>) -> OsString {
    let Some(path) = path else {
        return OsString::new();
    };
    let kept: Vec<PathBuf> = std::env::split_paths(&path)
        .filter(|p| p.is_absolute())
        .collect();
    std::env::join_paths(kept).unwrap_or_default()
}

/// Manages lazily spawned language servers for a [`crate::api::Core`].
pub struct LspManager {
    clients: Mutex<HashMap<ServerKey, Arc<LspClient>>>,
    /// Serializes starting a server without blocking `shutdown()` on `clients`.
    starting: AsyncMutex<()>,
    /// Servers that recently failed to start or kept failing, with cooldown.
    broken: Mutex<HashMap<ServerKey, Instant>>,
    /// Failures in a row per server.
    crashes: Mutex<HashMap<ServerKey, u32>>,
    limits: Limits,
}

impl Default for LspManager {
    fn default() -> Self {
        Self::with_limits(Limits::default())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl LspManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// A manager with its own timeouts and limits (tests shorten them).
    pub fn with_limits(limits: Limits) -> Self {
        LspManager {
            clients: Mutex::default(),
            starting: AsyncMutex::new(()),
            broken: Mutex::default(),
            crashes: Mutex::default(),
            limits,
        }
    }

    /// Runs LSP diagnostics for `path` (which now contains `text`) under `root`. Returns empty on
    /// any timeout, missing server, or disabled config.
    pub async fn diagnose(
        &self,
        root: &Path,
        path: &Path,
        text: &str,
        enabled: bool,
    ) -> Vec<Diagnostic> {
        if !enabled {
            return Vec::new();
        }
        let Some(spec) = detect_server(root, path) else {
            return Vec::new();
        };
        self.diagnose_with(root, &spec, path, text).await
    }

    async fn diagnose_with(
        &self,
        root: &Path,
        spec: &ServerSpec,
        path: &Path,
        text: &str,
    ) -> Vec<Diagnostic> {
        let key = ServerKey {
            root: root.to_path_buf(),
            id: spec.id,
        };
        if let Some(at) = lock(&self.broken).get(&key).copied() {
            if at.elapsed() < self.limits.retry {
                return Vec::new();
            }
            lock(&self.broken).remove(&key);
        }
        let Some(client) = self.get_or_start(&key, spec).await else {
            return Vec::new();
        };
        match client.diagnose_file(path, spec.language_id, text).await {
            Ok(diags) => {
                if client.is_dead() && client.crashed() {
                    // It answered, then died (its output went bad): a failure all the same, or a
                    // server that dies after every report would restart for every edit.
                    self.drop_client(&key, &client);
                    self.failed(&key);
                } else {
                    lock(&self.crashes).remove(&key);
                }
                diags
            }
            Err(_) => {
                // It ended meanwhile: on its own, a failure; stopped by us (idle, its root
                // going), not.
                let crashed = client.crashed();
                self.drop_client(&key, &client);
                if crashed {
                    self.failed(&key);
                }
                Vec::new()
            }
        }
    }

    /// One more failure of `key`'s server in a row; too many and it rests.
    fn failed(&self, key: &ServerKey) {
        let mut crashes = lock(&self.crashes);
        let count = crashes.entry(key.clone()).or_insert(0);
        *count += 1;
        if *count >= CRASHES {
            crashes.remove(key);
            lock(&self.broken).insert(key.clone(), Instant::now());
        }
    }

    /// Removes `client` (if it is still `key`'s) and stops it.
    fn drop_client(&self, key: &ServerKey, client: &Arc<LspClient>) {
        let mut clients = lock(&self.clients);
        if clients.get(key).is_some_and(|c| Arc::ptr_eq(c, client)) {
            clients.remove(key);
        }
        drop(clients);
        client.kill();
    }

    async fn get_or_start(&self, key: &ServerKey, spec: &ServerSpec) -> Option<Arc<LspClient>> {
        if let Some(client) = self.running(key) {
            return Some(client);
        }
        let _guard = self.starting.lock().await;
        if let Some(client) = self.running(key) {
            return Some(client);
        }
        if let Some(at) = lock(&self.broken).get(key).copied()
            && at.elapsed() < self.limits.retry
        {
            return None;
        }
        // Too many running: the one used longest ago stops.
        let evicted = {
            let mut clients = lock(&self.clients);
            let mut evicted = Vec::new();
            while clients.len() >= self.limits.servers.max(1) {
                let Some(oldest) = clients
                    .iter()
                    .min_by_key(|(_, c)| c.last_used())
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                evicted.extend(clients.remove(&oldest));
            }
            evicted
        };
        for client in evicted {
            client.stop().await;
        }
        let client = match LspClient::start(spec, &key.root, self.limits).await {
            Ok(client) => client,
            Err(_) => {
                lock(&self.broken).insert(key.clone(), Instant::now());
                return None;
            }
        };
        lock(&self.clients).insert(key.clone(), client.clone());
        Some(client)
    }

    /// `key`'s client if it is still alive. A dead one is removed: a crash counts toward resting
    /// it, an idle stop doesn't.
    fn running(&self, key: &ServerKey) -> Option<Arc<LspClient>> {
        let client = lock(&self.clients).get(key).cloned()?;
        if !client.is_dead() {
            return Some(client);
        }
        self.drop_client(key, &client);
        if client.crashed() {
            self.failed(key);
        }
        None
    }

    /// Stops the servers whose root is `root` or inside it (a worktree about to be removed):
    /// `shutdown`/`exit`, then their process group. Returns how many stopped.
    pub async fn shutdown_root(&self, root: &Path) -> usize {
        let wanted = path_key(root);
        let inside = |key: &str| {
            key == wanted
                || key
                    .strip_prefix(&wanted)
                    .is_some_and(|rest| rest.starts_with(['/', '\\']))
        };
        let stopping: Vec<Arc<LspClient>> = {
            let mut clients = lock(&self.clients);
            let keys: Vec<ServerKey> = clients
                .iter()
                .filter(|(_, c)| inside(&c.root_key))
                .map(|(k, _)| k.clone())
                .collect();
            keys.iter().filter_map(|k| clients.remove(k)).collect()
        };
        let count = stopping.len();
        futures_util::future::join_all(stopping.iter().map(|c| c.stop())).await;
        count
    }

    /// Terminates every running language server process without blocking on async locks (for
    /// process exit): `exit` is queued, then the process groups are killed.
    pub fn shutdown(&self) {
        let drained: Vec<Arc<LspClient>> = lock(&self.clients).drain().map(|(_, c)| c).collect();
        for client in drained {
            client.send_exit();
            client.kill();
        }
    }

    /// How many servers run (tests).
    #[doc(hidden)]
    pub fn running_count(&self) -> usize {
        lock(&self.clients)
            .values()
            .filter(|c| !c.is_dead())
            .count()
    }
}

/// One message for the writer, and what it marks as written: `(document key, version)`.
struct Frame {
    bytes: Vec<u8>,
    mark: Option<(String, i32)>,
}

#[derive(Default)]
struct SharedState {
    pending: HashMap<i64, oneshot::Sender<Value>>,
    /// Keyed by `uri_key(uri)`: `(generation, optional_version, diagnostics)`.
    diagnostics: HashMap<String, (u64, Option<i32>, Vec<Diagnostic>)>,
    /// The last version sent per document (versions keep growing across reopens).
    versions: HashMap<String, i32>,
    /// Documents open on the server: publishes for others are dropped.
    open: HashSet<String>,
    /// Per document, the version the writer last wrote and the diagnostics generation then: a
    /// publish without a version counts only when read after that.
    written: HashMap<String, (i32, u64)>,
}

/// Whether diagnostics for `version` are in: a publish for that version (or later), or one
/// without a version read after that version was written.
fn ready(
    published: Option<&(u64, Option<i32>, Vec<Diagnostic>)>,
    written: Option<&(i32, u64)>,
    version: i32,
) -> bool {
    let Some(&(written_version, written_generation)) = written else {
        return false;
    };
    if written_version < version {
        return false;
    }
    match published {
        Some((_, Some(reported), _)) => *reported >= version,
        Some((generation, None, _)) => *generation > written_generation,
        None => false,
    }
}

struct LspClient {
    tx: mpsc::Sender<Frame>,
    child: Arc<Mutex<Option<Child>>>,
    pid: Option<u32>,
    next_id: AtomicI64,
    state: Arc<Mutex<SharedState>>,
    notify: Arc<Notify>,
    /// The server can't be used any more (its output ended or couldn't be read, a write timed
    /// out, it was stopped).
    dead: Arc<AtomicBool>,
    /// It died on its own (not stopped by us).
    crashed: Arc<AtomicBool>,
    /// Being stopped by us: no new edit goes to it, and its end isn't a crash.
    stopping: Arc<AtomicBool>,
    last_used: Mutex<Instant>,
    /// One edit of a document at a time (it is opened and closed around each).
    documents: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    /// The root as [`path_key`] spells it.
    root_key: String,
    limits: Limits,
}

impl LspClient {
    async fn start(spec: &ServerSpec, root: &Path, limits: Limits) -> Result<Arc<Self>, String> {
        let mut cmd = Command::new(&spec.command);
        cmd.args(&spec.args)
            .env("PATH", absolute_path_entries(std::env::var_os("PATH")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            cmd.current_dir(root);
            // Its own process group, so stopping it stops what it started (cargo check, tsserver).
            cmd.process_group(0);
        }
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
            // npm's `.cmd` shims run a bare `node`, which cmd.exe looks for in the current folder
            // first: never the project's. The root goes to the server as `rootUri`.
            cmd.env("NoDefaultCurrentDirectoryInExePath", "1");
            cmd.current_dir(
                spec.command
                    .parent()
                    .filter(|p| p.is_dir())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(std::env::temp_dir),
            );
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let pid = child.id();
        let mut stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let child = Arc::new(Mutex::new(Some(child)));
        let state = Arc::new(Mutex::new(SharedState::default()));
        let notify = Arc::new(Notify::new());
        let dead = Arc::new(AtomicBool::new(false));
        let crashed = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = mpsc::channel::<Frame>(QUEUE);

        // The writer: the only one writing the server's stdin, each message under a timeout. A
        // server that stops reading is dropped rather than holding up edits.
        {
            let (state, notify, dead, crashed, stopping, child) = (
                state.clone(),
                notify.clone(),
                dead.clone(),
                crashed.clone(),
                stopping.clone(),
                child.clone(),
            );
            tokio::spawn(async move {
                while let Some(frame) = rx.recv().await {
                    let wrote = tokio::time::timeout(limits.write, async {
                        stdin.write_all(&frame.bytes).await?;
                        stdin.flush().await
                    })
                    .await;
                    if !matches!(wrote, Ok(Ok(()))) {
                        if !dead.swap(true, Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
                            crashed.store(true, Ordering::SeqCst);
                        }
                        notify.notify_waiters();
                        kill_tree(&child, pid);
                        break;
                    }
                    if let Some((key, version)) = frame.mark {
                        let mut st = lock(&state);
                        let generation = st.diagnostics.get(&key).map_or(0, |(g, _, _)| *g);
                        st.written.insert(key, (version, generation));
                        drop(st);
                        notify.notify_waiters();
                    }
                }
                // Dropping stdin: the server reads end of input.
            });
        }

        // The reader: parses Content-Length JSON-RPC frames from stdout, delivers responses and
        // diagnostics, and queues `result: null` for server requests (never waiting for the
        // writer). Output it can't read ends the server.
        {
            let (state, notify, dead, crashed, stopping, child, tx) = (
                state.clone(),
                notify.clone(),
                dead.clone(),
                crashed.clone(),
                stopping.clone(),
                child.clone(),
                tx.clone(),
            );
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                while let Ok(Some(msg)) = read_message(&mut reader).await {
                    if let Some(reply) = handle_incoming(&state, &notify, msg)
                        && let Ok(bytes) = frame(&reply)
                    {
                        let _ = tx.try_send(Frame { bytes, mark: None });
                    }
                }
                if !dead.swap(true, Ordering::SeqCst) {
                    if !stopping.load(Ordering::SeqCst) {
                        crashed.store(true, Ordering::SeqCst);
                    }
                    kill_tree(&child, pid);
                }
                lock(&state).pending.clear();
                notify.notify_waiters();
            });
        }

        let client = Arc::new(LspClient {
            tx,
            child,
            pid,
            next_id: AtomicI64::new(1),
            state,
            notify,
            dead,
            crashed,
            stopping,
            last_used: Mutex::new(Instant::now()),
            documents: Mutex::default(),
            root_key: path_key(root),
            limits,
        });
        idle_watch(Arc::downgrade(&client), limits.idle);

        let started = async {
            let root_uri = file_uri(root).ok_or("invalid root uri")?;
            let init_params = json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {
                    "textDocument": {
                        "synchronization": { "didSave": true },
                        "publishDiagnostics": { "relatedInformation": false, "versionSupport": true }
                    }
                },
                "workspaceFolders": [{
                    "uri": root_uri,
                    "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace")
                }]
            });
            client
                .request("initialize", init_params, limits.init)
                .await?;
            client.notify_server("initialized", json!({})).await
        };
        if let Err(error) = started.await {
            client.kill();
            return Err(error);
        }
        Ok(client)
    }

    /// Can't take edits: it ended, or is being stopped.
    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst) || self.stopping.load(Ordering::SeqCst)
    }

    fn crashed(&self) -> bool {
        self.crashed.load(Ordering::SeqCst)
    }

    fn last_used(&self) -> Instant {
        *lock(&self.last_used)
    }

    fn touch(&self) {
        *lock(&self.last_used) = Instant::now();
    }

    /// Marks the server dead without it counting as a crash.
    fn retire(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.dead.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.state).pending.insert(id, tx);
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(e) = self.send(&msg, None).await {
            lock(&self.state).pending.remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(val)) => Ok(val),
            _ => {
                lock(&self.state).pending.remove(&id);
                Err("timeout".into())
            }
        }
    }

    async fn notify_server(&self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send(&msg, None).await
    }

    /// Queues `msg` for the writer, waiting at most the write timeout for room.
    async fn send(&self, msg: &Value, mark: Option<(String, i32)>) -> Result<(), String> {
        if self.dead.load(Ordering::SeqCst) {
            return Err("the language server stopped".into());
        }
        let bytes = frame(msg)?;
        match tokio::time::timeout(self.limits.write, self.tx.send(Frame { bytes, mark })).await {
            Ok(Ok(())) => Ok(()),
            _ => {
                if !self.dead.swap(true, Ordering::SeqCst) && !self.stopping.load(Ordering::SeqCst)
                {
                    self.crashed.store(true, Ordering::SeqCst);
                }
                self.notify.notify_waiters();
                Err("the language server isn't reading".into())
            }
        }
    }

    async fn diagnose_file(
        &self,
        path: &Path,
        language_id: &str,
        text: &str,
    ) -> Result<Vec<Diagnostic>, String> {
        let uri = file_uri(path).ok_or("invalid file uri")?;
        let key = uri_key(&uri);
        let document = lock(&self.documents)
            .entry(key.clone())
            .or_default()
            .clone();
        let _one_edit = document.lock().await;
        self.touch();
        let version = {
            let mut st = lock(&self.state);
            let version = st.versions.entry(key.clone()).or_insert(0);
            *version += 1;
            let version = *version;
            st.open.insert(key.clone());
            st.diagnostics.remove(&key);
            version
        };
        let open = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": version,
                    "text": text,
                }
            }
        });
        let save = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didSave",
            "params": { "textDocument": { "uri": uri }, "text": text }
        });
        let waited = async {
            self.send(&open, Some((key.clone(), version))).await?;
            self.send(&save, None).await?;
            self.wait_for(&key, version).await
        }
        .await;
        // Closed again whatever happened: the server reads it from disk from now on, as every
        // other file, and open documents don't pile up.
        let close = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        });
        // A server that ended after its report: the report stands; the next edit finds it gone.
        let _ = self.send(&close, None).await;
        {
            let mut st = lock(&self.state);
            st.open.remove(&key);
            st.diagnostics.remove(&key);
            st.written.remove(&key);
        }
        self.touch();
        waited
    }

    /// The diagnostics for `version` once they are in (after a short settle), empty when they
    /// don't arrive in time; `Err` when the server stops meanwhile.
    async fn wait_for(&self, key: &str, version: i32) -> Result<Vec<Diagnostic>, String> {
        let deadline = tokio::time::Instant::now() + self.limits.diagnostics;
        loop {
            // Register the waiter BEFORE checking state to eliminate lost wakeups.
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let is_ready = {
                let st = lock(&self.state);
                ready(st.diagnostics.get(key), st.written.get(key), version)
            };
            // What arrived before the server ended still counts.
            if !is_ready && self.is_dead() {
                return Err("the language server stopped".into());
            }
            if is_ready {
                tokio::time::sleep(self.limits.settle).await;
                let st = lock(&self.state);
                return Ok(st
                    .diagnostics
                    .get(key)
                    .map(|(_, _, d)| d.clone())
                    .unwrap_or_default());
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                // Not in yet: nothing, never an earlier version's.
                return Ok(Vec::new());
            }
            let _ = tokio::time::timeout(deadline - now, notified).await;
        }
    }

    /// `shutdown`, then `exit`, then (after a moment) the process group: for a server stopped
    /// while the app goes on (its root went, it was idle, too many ran).
    async fn stop(&self) {
        let ended = self.dead.load(Ordering::SeqCst);
        self.stopping.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        let graceful = async {
            let _ = self
                .request("shutdown", Value::Null, Duration::from_secs(1))
                .await;
            let _ = self.notify_server("exit", Value::Null).await;
            for _ in 0..10 {
                let exited = lock(&self.child)
                    .as_mut()
                    .is_none_or(|c| c.try_wait().ok().flatten().is_some());
                if exited {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        if !ended {
            graceful.await;
        }
        self.kill();
    }

    /// Queues `exit` without waiting (process exit).
    fn send_exit(&self) {
        if let Ok(bytes) = frame(&json!({"jsonrpc": "2.0", "method": "exit"})) {
            let _ = self.tx.try_send(Frame { bytes, mark: None });
        }
    }

    fn kill(&self) {
        self.retire();
        kill_tree(&self.child, self.pid);
    }
}

/// A client dropped without being stopped (its manager went) takes its server with it.
impl Drop for LspClient {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Kills the server and, on Unix, its whole process group (it runs in its own). On Windows only
/// the server process for now: the process-tree helper (a Job Object) goes here.
fn kill_tree(child: &Arc<Mutex<Option<Child>>>, pid: Option<u32>) {
    // Only while the child is still ours: once taken (killed before), its id may be another's.
    if let Some(mut child) = lock(child).take() {
        #[cfg(unix)]
        if let Some(pid) = pid {
            crate::tool::shell::exec::force_kill(pid);
        }
        #[cfg(not(unix))]
        let _ = pid;
        let _ = child.start_kill();
        // Reaped in the background, so no zombie is left (at process exit there is no runtime to
        // do it, and no need).
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = child.wait().await;
            });
        }
    }
}

/// Stops the client once it has had no edit for `idle`.
fn idle_watch(client: Weak<LspClient>, idle: Duration) {
    tokio::spawn(async move {
        loop {
            let Some(strong) = client.upgrade() else {
                return;
            };
            if strong.is_dead() {
                return;
            }
            let quiet = strong.last_used().elapsed();
            if quiet >= idle {
                strong.stop().await;
                return;
            }
            drop(strong);
            tokio::time::sleep(idle - quiet).await;
        }
    });
}

/// Processes an incoming JSON-RPC message. Returns an optional JSON-RPC response when the server
/// sent a request (`id` + `method`) that expects a reply.
fn handle_incoming(
    state: &Arc<Mutex<SharedState>>,
    notify: &Arc<Notify>,
    msg: Value,
) -> Option<Value> {
    let has_method = msg.get("method").and_then(Value::as_str);
    let id_val = msg.get("id");

    // Server-initiated request (both `id` and `method` present): reply with `result: null`
    // (or `[]` for `workspace/configuration`).
    if let (Some(id), Some(method)) = (id_val, has_method) {
        let result = if method == "workspace/configuration" {
            let n = msg
                .get("params")
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; n])
        } else {
            Value::Null
        };
        return Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }));
    }

    // Response to one of our requests
    if let Some(id) = id_val.and_then(Value::as_i64)
        && has_method.is_none()
    {
        if let Some(tx) = lock(state).pending.remove(&id) {
            let _ = tx.send(msg);
        }
        return None;
    }

    // publishDiagnostics notification, for a document open now.
    if has_method == Some("textDocument/publishDiagnostics")
        && let Some(params) = msg.get("params")
        && let Some(uri) = params.get("uri").and_then(Value::as_str)
    {
        let key = uri_key(uri);
        let version = params
            .get("version")
            .and_then(Value::as_i64)
            .map(|v| v as i32);
        let parsed = parse_diagnostics(params.get("diagnostics"));
        let mut st = lock(state);
        if !st.open.contains(&key) {
            return None;
        }
        let next_gen = st.diagnostics.get(&key).map(|(g, _, _)| g + 1).unwrap_or(1);
        st.diagnostics.insert(key, (next_gen, version, parsed));
        drop(st);
        notify.notify_waiters();
    }
    None
}

fn parse_diagnostics(value: Option<&Value>) -> Vec<Diagnostic> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let severity = match item.get("severity").and_then(Value::as_u64).unwrap_or(1) {
                1 => Severity::Error,
                2 => Severity::Warning,
                _ => return None,
            };
            let message = item.get("message")?.as_str()?.to_string();
            let start = item.get("range")?.get("start")?;
            let line = start.get("line").and_then(Value::as_u64).unwrap_or(0) as u32 + 1;
            let col = start.get("character").and_then(Value::as_u64).unwrap_or(0) as u32 + 1;
            let source = item
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(Diagnostic {
                line,
                col,
                severity,
                message,
                source,
            })
        })
        .collect()
}

/// `msg` with its `Content-Length` header.
fn frame(msg: &Value) -> Result<Vec<u8>, String> {
    let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    Ok(out)
}

async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<Value>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse::<usize>().ok();
        }
    }
    let len = content_length.ok_or("missing Content-Length")?;
    if len > MAX_FRAME {
        return Err(format!("a {len}-byte message is too large"));
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| e.to_string())?;
    let val = serde_json::from_slice(&buf).map_err(|e| e.to_string())?;
    Ok(Some(val))
}

#[cfg(test)]
mod tests;
