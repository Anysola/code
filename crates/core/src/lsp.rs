//! Lightweight LSP diagnostics client for `edit` and `write`: starts an installed language
//! server lazily per `(root, server)` when a matching source file is modified, sends
//! `didOpen`/`didChange` + `didSave`, waits briefly for `textDocument/publishDiagnostics`, and
//! returns errors/warnings to append to the tool output.
//!
//! Only well-known locally installed language servers are ever spawned (never arbitrary commands
//! from project config). If no matching server is installed, or if config sets `"lsp": false`,
//! diagnostics silently return empty with zero overhead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// Language server lookup by file extension. Only resolves binaries already present locally.
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
            let cmd = local_node_bin(root, "typescript-language-server")
                .or_else(|| crate::which::which("typescript-language-server"))?;
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
                if let Some(cmd) = local_venv_bin(root, bin).or_else(|| crate::which::which(bin)) {
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

fn local_node_bin(root: &Path, name: &str) -> Option<PathBuf> {
    let bin = root.join("node_modules").join(".bin");
    let candidate = if cfg!(windows) {
        bin.join(format!("{name}.cmd"))
    } else {
        bin.join(name)
    };
    candidate.is_file().then_some(candidate)
}

fn local_venv_bin(root: &Path, name: &str) -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        root.join(".venv").join("Scripts")
    } else {
        root.join(".venv").join("bin")
    };
    let candidate = if cfg!(windows) {
        dir.join(format!("{name}.exe"))
    } else {
        dir.join(name)
    };
    candidate.is_file().then_some(candidate)
}

fn file_uri(path: &Path) -> Option<String> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Url::from_file_path(canonical).ok().map(String::from)
}

/// Manages lazily spawned language servers for a [`crate::api::Core`].
#[derive(Default)]
pub struct LspManager {
    clients: AsyncMutex<HashMap<ServerKey, Arc<LspClient>>>,
    /// Servers that failed to start or initialize so we don't stall every subsequent edit.
    broken: Mutex<HashMap<ServerKey, ()>>,
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
        if self
            .broken
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&key)
        {
            return Vec::new();
        }
        let client = match self.get_or_start(&key, &spec).await {
            Some(c) => c,
            None => {
                self.broken
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, ());
                return Vec::new();
            }
        };
        match client.diagnose_file(path, spec.language_id, text).await {
            Ok(diags) => diags,
            Err(_) => {
                // Drop a dead client so a later edit can try fresh if needed.
                self.clients.lock().await.remove(&key);
                Vec::new()
            }
        }
    }

    async fn get_or_start(&self, key: &ServerKey, spec: &ServerSpec) -> Option<Arc<LspClient>> {
        let mut map = self.clients.lock().await;
        if let Some(existing) = map.get(key) {
            return Some(existing.clone());
        }
        let client = LspClient::start(&spec.command, spec.args, &key.root)
            .await
            .ok()?;
        let client = Arc::new(client);
        map.insert(key.clone(), client.clone());
        Some(client)
    }

    /// Terminates every running language server process.
    pub fn shutdown(&self) {
        if let Ok(mut map) = self.clients.try_lock() {
            for (_, client) in map.drain() {
                client.kill();
            }
        }
    }
}

struct SharedState {
    pending: HashMap<i64, oneshot::Sender<Value>>,
    diagnostics: HashMap<String, (u64, Vec<Diagnostic>)>,
    versions: HashMap<String, i32>,
}

struct LspClient {
    stdin: AsyncMutex<ChildStdin>,
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
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;

        let state = Arc::new(Mutex::new(SharedState {
            pending: HashMap::new(),
            diagnostics: HashMap::new(),
            versions: HashMap::new(),
        }));
        let notify = Arc::new(Notify::new());
        let client = Self {
            stdin: AsyncMutex::new(stdin),
            child: Mutex::new(Some(child)),
            next_id: AtomicI64::new(1),
            state: state.clone(),
            notify: notify.clone(),
        };

        // Background reader task: parses Content-Length JSON-RPC frames from stdout.
        let reader_state = state.clone();
        let reader_notify = notify.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(msg)) = read_message(&mut reader).await {
                handle_incoming(&reader_state, &reader_notify, msg);
            }
            // Wake any pending waiters on EOF.
            reader_state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .clear();
            reader_notify.notify_waiters();
        });

        let root_uri = file_uri(root).unwrap_or_else(|| format!("file://{}", root.display()));
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "publishDiagnostics": { "relatedInformation": false }
                }
            },
            "workspaceFolders": [{ "uri": root_uri, "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace") }]
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
        let (prev_gen, version, opened) = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let prev = st.diagnostics.get(&uri).map(|(g, _)| *g).unwrap_or(0);
            let entry = st.versions.entry(uri.clone()).or_insert(0);
            let opened = *entry > 0;
            *entry += 1;
            (prev, *entry, opened)
        };

        if !opened {
            self.notify_server(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_id,
                        "version": version,
                        "text": text,
                    }
                }),
            )
            .await?;
        } else {
            self.notify_server(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": [{ "text": text }],
                }),
            )
            .await?;
        }
        self.notify_server(
            "textDocument/didSave",
            json!({
                "textDocument": { "uri": uri },
                "text": text,
            }),
        )
        .await?;

        let deadline = tokio::time::Instant::now() + DIAG_TIMEOUT;
        loop {
            let current_gen = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .diagnostics
                .get(&uri)
                .map(|(g, _)| *g)
                .unwrap_or(0);
            if current_gen > prev_gen {
                // Brief settle window in case the server publishes a refined batch right after.
                tokio::time::sleep(SETTLE_WAIT).await;
                break;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            let notified = self.notify.notified();
            let _ = tokio::time::timeout(deadline - now, notified).await;
        }

        let diags = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostics
            .get(&uri)
            .map(|(_, d)| d.clone())
            .unwrap_or_default();
        Ok(diags)
    }

    fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.start_kill();
        }
    }
}

fn handle_incoming(state: &Arc<Mutex<SharedState>>, notify: &Arc<Notify>, msg: Value) {
    // Response to one of our requests
    if let Some(id) = msg.get("id").and_then(Value::as_i64)
        && msg.get("method").is_none()
    {
        if let Some(tx) = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .remove(&id)
        {
            let _ = tx.send(msg);
        }
        return;
    }

    // publishDiagnostics notification
    if msg.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        && let Some(params) = msg.get("params")
        && let Some(uri) = params.get("uri").and_then(Value::as_str)
    {
        let parsed = parse_diagnostics(params.get("diagnostics"));
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        let next_gen = st.diagnostics.get(uri).map(|(g, _)| g + 1).unwrap_or(1);
        st.diagnostics.insert(uri.to_string(), (next_gen, parsed));
        drop(st);
        notify.notify_waiters();
    }
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
