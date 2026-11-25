//! Lightweight LSP diagnostics client for `edit` and `write`: starts an installed language
//! server lazily per `(root, server)` when a matching source file is modified, sends
//! `didOpen`/`didChange` + `didSave`, waits briefly for `textDocument/publishDiagnostics`, and
//! returns errors/warnings to append to the tool output.
//!
//! Off by default: only `"lsp": true` in the user's own (or managed) config turns it on, and a
//! project document can only turn it off. A language server runs the project's build tooling
//! (`cargo metadata`, build scripts, a workspace's own TypeScript), outside the shell sandbox and
//! without the project approval its hooks or formatters wait for.
//!
//! Only well-known binaries resolved from `PATH` are spawned (never project-local executables
//! under `node_modules/.bin` or `.venv/bin`, and never commands from project config). If no
//! matching server is installed, or LSP is off, diagnostics silently return empty with zero
//! overhead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex as AsyncMutex, Notify, oneshot};
use url::Url;

/// Maximum time to wait for the server's `initialize` response.
const INIT_TIMEOUT: Duration = Duration::from_secs(8);
/// Maximum time to wait for `publishDiagnostics` after a file edit.
const DIAG_TIMEOUT: Duration = Duration::from_millis(2500);
/// Brief settle window after the first diagnostics arrive in case a follow-up batch lands.
const SETTLE_WAIT: Duration = Duration::from_millis(120);
/// Cooldown before retrying a language server that failed to start or initialize.
const RETRY_COOLDOWN: Duration = Duration::from_secs(60);
/// Maximum diagnostics appended to a single tool result.
const MAX_DIAGNOSTICS: usize = 20;

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
    let mut out = format!("<diagnostics file=\"{resource}\">\n");
    for d in &sorted[..shown] {
        let level = match d.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
        };
        let msg = d.message.lines().next().unwrap_or("").trim();
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

struct ServerSpec {
    id: &'static str,
    language_id: &'static str,
    command: PathBuf,
    args: &'static [&'static str],
}

/// Language server lookup by file extension. Resolves only from system/user `PATH` so untrusted
/// repositories cannot execute arbitrary binaries from `node_modules/.bin` or `.venv/bin`.
fn detect_server(root: &Path, path: &Path) -> Option<ServerSpec> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => {
            let lang = match ext.as_str() {
                "tsx" => "typescriptreact",
                "jsx" => "javascriptreact",
                "js" | "mjs" | "cjs" => "javascript",
                _ => "typescript",
            };
            let cmd = crate::which::which("typescript-language-server")?;
            Some(ServerSpec {
                id: "typescript",
                language_id: lang,
                command: cmd,
                args: &["--stdio"],
            })
        }
        "rs" => {
            if !has_ancestor_file(path, root, "Cargo.toml") {
                return None;
            }
            let cmd = crate::which::which("rust-analyzer")?;
            Some(ServerSpec {
                id: "rust",
                language_id: "rust",
                command: cmd,
                args: &[],
            })
        }
        "py" | "pyi" => {
            for (bin, args) in [
                ("basedpyright-langserver", &["--stdio"][..]),
                ("pyright-langserver", &["--stdio"][..]),
                ("pylsp", &[][..]),
            ] {
                if let Some(cmd) = crate::which::which(bin) {
                    return Some(ServerSpec {
                        id: "python",
                        language_id: "python",
                        command: cmd,
                        args,
                    });
                }
            }
            None
        }
        "go" => {
            let cmd = crate::which::which("gopls")?;
            Some(ServerSpec {
                id: "go",
                language_id: "go",
                command: cmd,
                args: &[],
            })
        }
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" => {
            let cmd = crate::which::which("clangd")?;
            let lang = if matches!(ext.as_str(), "c" | "h") {
                "c"
            } else {
                "cpp"
            };
            Some(ServerSpec {
                id: "clangd",
                language_id: lang,
                command: cmd,
                args: &[],
            })
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

/// Normalizes a `file://` URI into a stable map key across platforms (handles Windows drive-letter
/// casing and percent-encoded colons like `file:///c%3A/...` from `vscode-uri` servers).
fn uri_key(uri: &str) -> String {
    if let Ok(parsed) = Url::parse(uri)
        && parsed.scheme() == "file"
        && let Ok(path) = parsed.to_file_path()
    {
        let canonical = crate::paths::canonicalize(&path).unwrap_or(path);
        let text = canonical.to_string_lossy();
        return if cfg!(windows) {
            text.replace('/', "\\").to_lowercase()
        } else {
            text.into_owned()
        };
    }
    uri.to_string()
}

/// Manages lazily spawned language servers for a [`crate::api::Core`].
#[derive(Default)]
pub struct LspManager {
    clients: Mutex<HashMap<ServerKey, Arc<LspClient>>>,
    /// Serializes starting a server without blocking `shutdown()` on `clients`.
    starting: AsyncMutex<()>,
    /// Servers that recently failed to start or initialize, with cooldown.
    broken: Mutex<HashMap<ServerKey, Instant>>,
}

impl LspManager {
    pub fn new() -> Self {
        Self::default()
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
        let key = ServerKey {
            root: root.to_path_buf(),
            id: spec.id,
        };
        if let Some(at) = self
            .broken
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .copied()
            && at.elapsed() < RETRY_COOLDOWN
        {
            return Vec::new();
        }
        let client = match self.get_or_start(&key, &spec).await {
            Some(c) => c,
            None => {
                self.broken
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, Instant::now());
                return Vec::new();
            }
        };
        match client.diagnose_file(path, spec.language_id, text).await {
            Ok(diags) => diags,
            Err(_) => {
                if let Some(dead) = self
                    .clients
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&key)
                {
                    dead.kill();
                }
                Vec::new()
            }
        }
    }

    async fn get_or_start(&self, key: &ServerKey, spec: &ServerSpec) -> Option<Arc<LspClient>> {
        if let Some(existing) = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
        {
            return Some(existing);
        }
        let _guard = self.starting.lock().await;
        if let Some(existing) = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
        {
            return Some(existing);
        }
        let client = LspClient::start(&spec.command, spec.args, &key.root)
            .await
            .ok()?;
        let client = Arc::new(client);
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone(), client.clone());
        Some(client)
    }

    /// Terminates every running language server process without blocking on async locks.
    pub fn shutdown(&self) {
        let drained: Vec<Arc<LspClient>> = self
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, c)| c)
            .collect();
        for client in drained {
            client.kill();
        }
    }
}

struct SharedState {
    pending: HashMap<i64, oneshot::Sender<Value>>,
    /// Keyed by `uri_key(uri)`: `(generation, optional_version, diagnostics)`.
    diagnostics: HashMap<String, (u64, Option<i32>, Vec<Diagnostic>)>,
    versions: HashMap<String, i32>,
}

struct LspClient {
    stdin: Arc<AsyncMutex<ChildStdin>>,
    child: Mutex<Option<Child>>,
    next_id: AtomicI64,
    state: Arc<Mutex<SharedState>>,
    notify: Arc<Notify>,
}

impl LspClient {
    async fn start(command: &Path, args: &[&str], root: &Path) -> Result<Self, String> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let stdin = Arc::new(AsyncMutex::new(child.stdin.take().ok_or("no stdin")?));
        let stdout = child.stdout.take().ok_or("no stdout")?;

        let state = Arc::new(Mutex::new(SharedState {
            pending: HashMap::new(),
            diagnostics: HashMap::new(),
            versions: HashMap::new(),
        }));
        let notify = Arc::new(Notify::new());
        let client = Self {
            stdin: stdin.clone(),
            child: Mutex::new(Some(child)),
            next_id: AtomicI64::new(1),
            state: state.clone(),
            notify: notify.clone(),
        };

        // Background reader task: parses Content-Length JSON-RPC frames from stdout, delivers
        // responses/diagnostics, and replies `result: null` to server-initiated requests so
        // servers like pyright, gopls, and rust-analyzer never stall waiting on client replies.
        let reader_state = state.clone();
        let reader_notify = notify.clone();
        let reply_stdin = stdin;
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(msg)) = read_message(&mut reader).await {
                if let Some(reply) = handle_incoming(&reader_state, &reader_notify, msg) {
                    let mut guard = reply_stdin.lock().await;
                    let _ = write_message(&mut *guard, &reply).await;
                }
            }
            reader_state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .clear();
            reader_notify.notify_waiters();
        });

        let root_uri = file_uri(root).ok_or("invalid root uri")?;
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "publishDiagnostics": { "relatedInformation": false }
                }
            },
            "workspaceFolders": [{
                "uri": root_uri,
                "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace")
            }]
        });
        client
            .request("initialize", init_params, INIT_TIMEOUT)
            .await?;
        client.notify_server("initialized", json!({})).await?;
        Ok(client)
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .insert(id, tx);
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(e) = self.send(&msg).await {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(val)) => Ok(val),
            _ => {
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .pending
                    .remove(&id);
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
        self.send(&msg).await
    }

    async fn send(&self, msg: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().await;
        write_message(&mut *stdin, msg).await
    }

    async fn diagnose_file(
        &self,
        path: &Path,
        language_id: &str,
        text: &str,
    ) -> Result<Vec<Diagnostic>, String> {
        let uri = file_uri(path).ok_or("invalid file uri")?;
        let key = uri_key(&uri);

        // Hold `stdin` across version bump + didOpen/didChange + didSave so concurrent edits
        // never send out-of-order document versions.
        let (prev_gen, version) = {
            let mut stdin = self.stdin.lock().await;
            let (prev, ver, opened) = {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let prev = st.diagnostics.get(&key).map(|(g, _, _)| *g).unwrap_or(0);
                let entry = st.versions.entry(key.clone()).or_insert(0);
                let opened = *entry > 0;
                *entry += 1;
                (prev, *entry, opened)
            };
            let change = if !opened {
                json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didOpen",
                    "params": {
                        "textDocument": {
                            "uri": uri,
                            "languageId": language_id,
                            "version": ver,
                            "text": text,
                        }
                    }
                })
            } else {
                json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didChange",
                    "params": {
                        "textDocument": { "uri": uri, "version": ver },
                        "contentChanges": [{ "text": text }],
                    }
                })
            };
            write_message(&mut *stdin, &change).await?;
            let save = json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didSave",
                "params": {
                    "textDocument": { "uri": uri },
                    "text": text,
                }
            });
            write_message(&mut *stdin, &save).await?;
            (prev, ver)
        };

        let deadline = tokio::time::Instant::now() + DIAG_TIMEOUT;
        loop {
            // Register the waiter BEFORE checking state to eliminate lost wakeups.
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let ready = {
                let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                st.diagnostics.get(&key).is_some_and(|(g, v, _)| {
                    *g > prev_gen && v.is_none_or(|reported| reported >= version)
                })
            };
            if ready {
                tokio::time::sleep(SETTLE_WAIT).await;
                break;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            let _ = tokio::time::timeout(deadline - now, notified).await;
        }

        let diags = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
            .get(&key)
            .map(|(_, _, d)| d.clone())
            .unwrap_or_default();
        Ok(diags)
    }

    fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.start_kill();
        }
    }
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
        if let Some(tx) = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .remove(&id)
        {
            let _ = tx.send(msg);
        }
        return None;
    }

    // publishDiagnostics notification
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
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
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

async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, msg: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer
        .write_all(header.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    writer.write_all(&body).await.map_err(|e| e.to_string())?;
    writer.flush().await.map_err(|e| e.to_string())
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
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| e.to_string())?;
    let val = serde_json::from_slice(&buf).map_err(|e| e.to_string())?;
    Ok(Some(val))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn framing_roundtrips_json_rpc_messages() {
        let mut wire = Vec::new();
        let msg =
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"ok": true}});
        write_message(&mut wire, &msg).await.unwrap();
        let mut reader = BufReader::new(&wire[..]);
        let decoded = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(decoded, msg);
        assert!(read_message(&mut reader).await.unwrap().is_none());
    }

    #[test]
    fn replies_to_server_requests() {
        let state = Arc::new(Mutex::new(SharedState {
            pending: HashMap::new(),
            diagnostics: HashMap::new(),
            versions: HashMap::new(),
        }));
        let notify = Arc::new(Notify::new());
        let req = json!({
            "jsonrpc": "2.0",
            "id": "cfg-1",
            "method": "workspace/configuration",
            "params": {"items": [{}, {}]}
        });
        let reply = handle_incoming(&state, &notify, req).unwrap();
        assert_eq!(reply["id"], "cfg-1");
        assert_eq!(reply["result"], json!([null, null]));

        let progress = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "window/workDoneProgress/create",
            "params": {"token": "1"}
        });
        let reply = handle_incoming(&state, &notify, progress).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"], Value::Null);
    }

    #[test]
    fn parses_and_formats_diagnostics_errors_first() {
        let raw = json!([
            {
                "range": {"start": {"line": 9, "character": 2}, "end": {"line": 9, "character": 5}},
                "severity": 2,
                "message": "Unused variable"
            },
            {
                "range": {"start": {"line": 3, "character": 0}, "end": {"line": 3, "character": 4}},
                "severity": 1,
                "message": "Type mismatch\nwith detail"
            },
            {
                "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 1}},
                "severity": 3,
                "message": "Info ignored"
            }
        ]);
        let diags = parse_diagnostics(Some(&raw));
        assert_eq!(diags.len(), 2);
        let formatted = format_diagnostics("src/app.ts", &diags).unwrap();
        assert_eq!(
            formatted,
            "<diagnostics file=\"src/app.ts\">\nERROR [4:1] Type mismatch\nWARNING [10:3] Unused variable\n</diagnostics>"
        );
        assert!(format_diagnostics("src/app.ts", &[]).is_none());
    }
}
