//! Embedded interactive terminals bound to a session's working folder (its worktree, or the
//! main folder). Each terminal owns a native PTY (`openpty` on Unix, ConPTY on Windows) and
//! streams raw bytes (base64-encoded) over the `core` event channel so multibyte UTF-8 and ANSI
//! escape sequences arrive intact.
//!
//! Output is gathered for a moment (`FLUSH_AFTER`, at most `CHUNK` bytes) and sent as one event.
//! The window acknowledges what xterm has processed (`terminal_ack`); with `IN_FLIGHT` bytes
//! unacknowledged nothing more is sent, the reader stops reading, and the PTY makes the program
//! wait. So `yes` can't flood the window's event queue or grow memory.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};

use crate::{Answer, App};

/// Output gathered this long after its first byte goes out as one event.
const FLUSH_AFTER: Duration = Duration::from_millis(16);
/// The most one event carries.
const CHUNK: usize = 64 * 1024;
/// Bytes sent that the window hasn't acknowledged yet, at most (then reading stops).
const IN_FLIGHT: usize = 1024 * 1024;
/// After SIGHUP, how long a terminal's processes get before SIGKILL; then how long the reap may
/// take (Windows: how long closing the pseudo console may take to end the shell).
#[cfg(unix)]
const HANGUP_GRACE: Duration = Duration::from_millis(400);
const KILL_GRACE: Duration = Duration::from_millis(1500);

/// Sends one `core` event to the window.
pub(crate) type Emit = Arc<dyn Fn(Value) + Send + Sync>;

/// What the reader, the sender and `close` share about one terminal.
#[derive(Default)]
struct Flow {
    state: Mutex<FlowState>,
    changed: Condvar,
}

#[derive(Default)]
struct FlowState {
    /// Sent, not yet acknowledged by the window.
    unacked: usize,
    /// Closed by the app: nothing more is sent, the reader stops.
    closed: bool,
    /// The shell was reaped (its pid may name another process now).
    reaped: bool,
}

impl Flow {
    fn lock(&self) -> std::sync::MutexGuard<'_, FlowState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn closed(&self) -> bool {
        self.lock().closed
    }

    fn close(&self) {
        self.lock().closed = true;
        self.changed.notify_all();
    }

    /// Waits until the shell was reaped, `limit` at most; whether it was.
    fn wait_reaped(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut state = self.lock();
        while !state.reaped {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }
}

struct Entry {
    directory: PathBuf,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// The shell's pid: on Unix it leads its own session and process group.
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
    flow: Arc<Flow>,
    /// Linux: the record of the shell's session, so what of it a crash leaves goes at the next
    /// start (see `anysola_core::children::adopt_session`); gone with the entry.
    _ours: anysola_core::children::Ours,
}

/// Live embedded terminals for this window.
#[derive(Clone, Default)]
pub(crate) struct Manager {
    next: Arc<AtomicU64>,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

/// A folder as `close_in_directory` compares it: the real path (never `\\?\`), no trailing
/// separator; on Windows `\` and lower case.
fn dir_key(path: &Path) -> String {
    let real = anysola_core::paths::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    key_of(&real.to_string_lossy(), cfg!(windows))
}

fn key_of(path: &str, windows: bool) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if windows {
        trimmed.replace('/', "\\").to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// Whether the folder `key` is `target` or inside it (both from [`key_of`]).
fn inside(key: &str, target: &str) -> bool {
    key == target
        || key
            .strip_prefix(target)
            .is_some_and(|rest| rest.starts_with(['/', '\\']))
}

/// The shell the embedded terminal runs: the user's `shell` setting, else `$SHELL` (fish and nu
/// too: the agent's limits are about driving a shell, not about the user's own terminal), else
/// the platform's.
pub(crate) fn shell_for(configured: Option<&str>) -> String {
    anysola_core::tool::shell::terminal_shell(configured)
}

impl Manager {
    pub(crate) fn open(
        &self,
        emit: Emit,
        directory: &str,
        shell: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(String, String), String> {
        let dir = Path::new(directory);
        if !dir.is_dir() {
            return Err(format!("{directory} is not a folder on this computer."));
        }
        let pty = native_pty_system();
        let size = PtySize {
            rows: rows.clamp(2, 500),
            cols: cols.clamp(10, 500),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty
            .openpty(size)
            .map_err(|e| format!("Couldn’t open a terminal: {e}"))?;

        let shell_name = anysola_core::tool::shell::name_of(shell);
        let mut cmd = CommandBuilder::new(shell);
        if matches!(shell_name.as_str(), "pwsh" | "powershell") {
            cmd.arg("-NoLogo");
        }
        cmd.cwd(dir);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // What the shell starts carries it, so a crash's leftovers can be told apart (Linux).
        let mark = anysola_core::children::Mark::new();
        cmd.env(anysola_core::children::MARKER, mark.value());

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Couldn’t start {shell_name}: {e}"))?;
        // Drop the slave handle in the parent so the reader sees EOF when the shell exits.
        drop(pair.slave);
        let pid = child.process_id();
        // Not a Job on Windows: like any terminal window, closing it ends what is attached to its
        // console, never a program that left it (`code .`, `start`). That the app dying does the
        // same (the console host leaving once its pipes break) is assumed, not tested here.
        let ours = anysola_core::children::adopt_session(pid, mark);

        let killer = child.clone_killer();
        let reader = Reader::new(pair.master.as_ref())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Couldn’t write to terminal: {e}"))?;

        let n = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("term_{n}");
        let flow = Arc::new(Flow::default());
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id.clone(),
                Entry {
                    directory: dir.to_path_buf(),
                    master: Arc::new(Mutex::new(pair.master)),
                    writer: Arc::new(Mutex::new(writer)),
                    killer,
                    pid,
                    flow: flow.clone(),
                    _ours: ours,
                },
            );

        // The reader hands what it reads over a short queue; the sender gathers, waits for the
        // window, sends, and at the end reaps the shell and says so.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(4);
        {
            let flow = flow.clone();
            std::thread::spawn(move || reader.run(&tx, &flow));
        }
        let entries = self.entries.clone();
        let tid = id.clone();
        std::thread::spawn(move || send(&emit, &tid, &rx, &flow, child, &entries));

        Ok((id, shell_name))
    }

    pub(crate) fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let writer = {
            let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            map.get(id)
                .map(|e| e.writer.clone())
                .ok_or_else(|| "This terminal has closed.".to_string())?
        };
        let mut guard = writer.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .write_all(data.as_bytes())
            .and_then(|_| guard.flush())
            .map_err(|e| format!("Couldn’t send input to the terminal: {e}"))
    }

    pub(crate) fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let master = {
            let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            let Some(entry) = map.get(id) else {
                return Ok(());
            };
            entry.master.clone()
        };
        let guard = master.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .resize(PtySize {
                rows: rows.clamp(2, 500),
                cols: cols.clamp(10, 500),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Couldn’t resize the terminal: {e}"))
    }

    /// The window has processed `bytes` more of the terminal's output.
    pub(crate) fn ack(&self, id: &str, bytes: usize) {
        let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = map.get(id) {
            let mut state = entry.flow.lock();
            state.unacked = state.unacked.saturating_sub(bytes);
            drop(state);
            entry.flow.changed.notify_all();
        }
    }

    fn take(&self, pick: impl Fn(&str, &Entry) -> bool) -> Vec<Entry> {
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let ids: Vec<String> = map
            .iter()
            .filter(|(id, e)| pick(id, e))
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter().filter_map(|id| map.remove(&id)).collect()
    }

    pub(crate) fn close(&self, id: &str) {
        stop(self.take(|i, _| i == id));
    }

    /// Closes any terminal whose working folder is inside `directory` (e.g. before removing a
    /// worktree so Windows does not hold a directory handle open), and waits for them a moment.
    pub(crate) fn close_in_directory(&self, directory: &str) {
        let target = dir_key(Path::new(directory));
        stop(self.take(|_, e| inside(&dir_key(&e.directory), &target)));
    }

    pub(crate) fn close_all(&self) {
        stop(self.take(|_, _| true));
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Ends terminals taken out of the list. Unix: SIGHUP to everything the shell's session runs
/// (its jobs have process groups of their own), a moment, then SIGKILL, and a bounded wait for
/// the reap; the reader stops either way. Windows: the shell is terminated and the pseudo
/// console closed, which ends what is attached to it; then a bounded wait for the reap (the
/// reader sees the end once the console is gone), so a worktree removed next isn't held open.
fn stop(mut entries: Vec<Entry>) {
    if entries.is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        for entry in &mut entries {
            if entry.pid.is_none() {
                let _ = entry.killer.kill();
            }
            signal_session(entry, libc::SIGHUP);
            entry.flow.close();
        }
        // Then whatever is left, the shell or a job that ignores SIGHUP.
        let deadline = Instant::now() + HANGUP_GRACE;
        while Instant::now() < deadline && entries.iter().any(|e| !ended(e)) {
            std::thread::sleep(Duration::from_millis(20));
        }
        for entry in entries.iter().filter(|e| !ended(e)) {
            signal_session(entry, libc::SIGKILL);
        }
        let deadline = Instant::now() + KILL_GRACE;
        for entry in &entries {
            if !entry
                .flow
                .wait_reaped(deadline.saturating_duration_since(Instant::now()))
            {
                anysola_core::log::warn(
                    "a terminal's shell didn't end",
                    &[("pid", &format!("{:?}", entry.pid))],
                );
            }
        }
    }
    #[cfg(not(unix))]
    for entry in &mut entries {
        let _ = entry.killer.kill();
        entry.flow.close();
    }
    let flows: Vec<Arc<Flow>> = entries.iter().map(|e| e.flow.clone()).collect();
    // The master goes last (Windows: closing the pseudo console ends what's attached to it).
    entries.clear();
    if cfg!(windows) {
        let deadline = Instant::now() + KILL_GRACE;
        for flow in flows {
            flow.wait_reaped(deadline.saturating_duration_since(Instant::now()));
        }
    }
}

#[cfg(unix)]
fn pid_of(entry: &Entry) -> Option<libc::pid_t> {
    entry.pid.and_then(|p| libc::pid_t::try_from(p).ok())
}

/// The shell was reaped and (Linux) nothing of its session runs.
#[cfg(unix)]
fn ended(entry: &Entry) -> bool {
    if !entry.flow.lock().reaped {
        return false;
    }
    #[cfg(target_os = "linux")]
    if let Some(pid) = pid_of(entry) {
        return session_members(pid).is_empty();
    }
    true
}

/// Sends `signal` to the terminal's processes: while its shell is unreaped, the shell's group
/// and the foreground job's group; on Linux every process of the shell's session (its jobs have
/// groups of their own). A session id stays taken while a process of it runs, so that never
/// names a stranger; after the reap the groups could, so they aren't signalled then.
#[cfg(unix)]
fn signal_session(entry: &Entry, signal: libc::c_int) {
    let Some(pid) = pid_of(entry) else {
        return;
    };
    let state = entry.flow.lock();
    let mut targets = Vec::new();
    if !state.reaped {
        targets.push(-pid);
        let foreground = entry
            .master
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .process_group_leader();
        if let Some(group) = foreground.filter(|g| *g > 1 && *g != pid) {
            targets.push(-group);
        }
    }
    #[cfg(target_os = "linux")]
    targets.extend(session_members(pid));
    for target in targets {
        // SAFETY: kill(2) with a pid (or a negative process group id) and a signal number.
        unsafe {
            libc::kill(target, signal);
        }
    }
    drop(state);
}

/// The processes of session `sid` (a terminal's shell leads one), from `/proc`.
#[cfg(target_os = "linux")]
fn session_members(sid: libc::pid_t) -> Vec<libc::pid_t> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<libc::pid_t>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                // After the command name in parentheses: state, ppid, pgrp, session.
                stat.rsplit_once(')')
                    .and_then(|(_, rest)| rest.split_whitespace().nth(3)?.parse().ok())
                    == Some(sid)
            })
        })
        .collect()
}

/// Reads the PTY. On Unix it waits with `poll` so it notices a close even while a process that
/// left the shell's session still holds the terminal open; elsewhere it reads until EOF (closing
/// the pseudo console gives one).
struct Reader {
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(not(unix))]
    file: Box<dyn Read + Send>,
}

impl Reader {
    #[cfg(unix)]
    fn new(master: &(dyn MasterPty + Send)) -> Result<Reader, String> {
        use std::os::fd::FromRawFd;
        let fd = master
            .as_raw_fd()
            .ok_or("Couldn’t read terminal output: no descriptor.")?;
        // SAFETY: dup(2) of a descriptor the master owns; the copy is ours alone.
        let own = unsafe { libc::dup(fd) };
        if own < 0 {
            return Err(format!(
                "Couldn’t read terminal output: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: `own` is a fresh descriptor nothing else closes.
        let file = unsafe { std::fs::File::from_raw_fd(own) };
        Ok(Reader { file })
    }

    #[cfg(not(unix))]
    fn new(master: &(dyn MasterPty + Send)) -> Result<Reader, String> {
        let file = master
            .try_clone_reader()
            .map_err(|e| format!("Couldn’t read terminal output: {e}"))?;
        Ok(Reader { file })
    }

    /// Whether there is something to read (or the end), waiting 50 ms at most.
    #[cfg(unix)]
    fn ready(&self) -> bool {
        use std::os::fd::AsRawFd;
        let mut fds = libc::pollfd {
            fd: self.file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll(2) on one descriptor we own.
        let n = unsafe { libc::poll(&mut fds, 1, 50) };
        n > 0
    }

    fn run(mut self, tx: &SyncSender<Vec<u8>>, flow: &Flow) {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            // A close stops the Unix reader. On Windows it reads on to the end, discarding, so
            // closing the pseudo console never waits on an output pipe nobody empties.
            if cfg!(unix) && flow.closed() {
                return;
            }
            #[cfg(unix)]
            if !self.ready() {
                continue;
            }
            let mut data = match self.file.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf[..n].to_vec(),
            };
            if flow.closed() {
                continue;
            }
            // A full queue (the window is behind) holds the reader here: the PTY fills and
            // makes the program wait. A close lets it go.
            loop {
                match tx.try_send(data) {
                    Ok(()) => break,
                    Err(TrySendError::Disconnected(_)) => return,
                    Err(TrySendError::Full(back)) => {
                        if flow.closed() {
                            break;
                        }
                        data = back;
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }
    }
}

/// Gathers the reader's output, sends it when the window has room, then reaps the shell and
/// reports its exit.
fn send(
    emit: &Emit,
    id: &str,
    rx: &Receiver<Vec<u8>>,
    flow: &Flow,
    mut child: Box<dyn Child + Send + Sync>,
    entries: &Mutex<HashMap<String, Entry>>,
) {
    let mut pending: Vec<u8> = Vec::new();
    let mut since: Option<Instant> = None;
    let mut ended = false;
    while !ended {
        let wait = since.map_or(Duration::from_millis(250), |t| {
            FLUSH_AFTER.saturating_sub(t.elapsed())
        });
        match rx.recv_timeout(wait) {
            Ok(data) => {
                since.get_or_insert_with(Instant::now);
                pending.extend_from_slice(&data);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => ended = true,
        }
        let due = since.is_some_and(|t| t.elapsed() >= FLUSH_AFTER);
        while !pending.is_empty() && (due || ended || pending.len() >= CHUNK) {
            let n = pending.len().min(CHUNK);
            if !room(flow, n) {
                // Closed: nothing more goes out.
                pending.clear();
                break;
            }
            let chunk: Vec<u8> = pending.drain(..n).collect();
            emit(json!({"type": "terminalData", "id": id, "data": BASE64.encode(&chunk)}));
        }
        if pending.is_empty() {
            since = None;
        }
    }
    let code = child.wait().ok().map(|s| s.exit_code());
    flow.lock().reaped = true;
    flow.changed.notify_all();
    entries.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    emit(json!({"type": "terminalExit", "id": id, "code": code}));
}

/// Waits until `n` more bytes may be sent and counts them; false once the terminal is closed.
fn room(flow: &Flow, n: usize) -> bool {
    let mut state = flow.lock();
    loop {
        if state.closed {
            return false;
        }
        if state.unacked == 0 || state.unacked + n <= IN_FLIGHT {
            state.unacked += n;
            return true;
        }
        state = flow
            .changed
            .wait_timeout(state, Duration::from_millis(250))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

/// The user's own `shell` setting from the global config file (a project can't choose it).
fn configured_shell(app: &App) -> Option<String> {
    let (_, data) = crate::config::read(&app.paths).ok()?;
    data.get("shell")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[tauri::command]
pub async fn terminal_open(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    cols: u16,
    rows: u16,
) -> Answer {
    let manager = app.terminals.clone();
    let configured = configured_shell(&app);
    let emit: Emit = Arc::new(move |event| {
        let _ = handle.emit("core", event);
    });
    let (id, shell) = tokio::task::spawn_blocking(move || {
        let shell = shell_for(configured.as_deref());
        manager.open(emit, &directory, &shell, cols, rows)
    })
    .await
    .map_err(|e| format!("Terminal start failed: {e}"))??;
    Ok(json!({"id": id, "shell": shell}))
}

#[tauri::command]
pub async fn terminal_write(app: State<'_, App>, id: String, data: String) -> Result<(), String> {
    let manager = app.terminals.clone();
    tokio::task::spawn_blocking(move || manager.write(&id, &data))
        .await
        .map_err(|e| format!("Terminal write failed: {e}"))?
}

#[tauri::command]
pub async fn terminal_resize(
    app: State<'_, App>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let manager = app.terminals.clone();
    tokio::task::spawn_blocking(move || manager.resize(&id, cols, rows))
        .await
        .map_err(|e| format!("Terminal resize failed: {e}"))?
}

/// The window processed `bytes` of the terminal's output (xterm's write callback).
#[tauri::command]
pub fn terminal_ack(app: State<'_, App>, id: String, bytes: u64) {
    app.terminals
        .ack(&id, usize::try_from(bytes).unwrap_or(usize::MAX));
}

#[tauri::command]
pub async fn terminal_close(app: State<'_, App>, id: String) -> Result<(), String> {
    let manager = app.terminals.clone();
    let _ = tokio::task::spawn_blocking(move || manager.close(&id)).await;
    Ok(())
}

/// Closes every terminal: the window calls it as it loads, so a reload leaves no shell nobody
/// can reach.
#[tauri::command]
pub async fn terminal_close_all(app: State<'_, App>) -> Result<(), String> {
    let manager = app.terminals.clone();
    let _ = tokio::task::spawn_blocking(move || manager.close_all()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_resize_handle_missing_id_cleanly() {
        let mgr = Manager::default();
        assert!(mgr.write("missing", "hi").is_err());
        assert!(mgr.resize("missing", 80, 24).is_ok());
        mgr.ack("missing", 10);
        mgr.close("missing");
        mgr.close_in_directory("/no/such/path");
        mgr.close_all();
    }

    #[test]
    fn a_folder_and_whats_inside_it_match_never_a_sibling() {
        let unix = |p: &str| key_of(p, false);
        assert!(inside(&unix("/w/a"), &unix("/w/a/")));
        assert!(inside(&unix("/w/a/src"), &unix("/w/a")));
        assert!(!inside(&unix("/w/ab"), &unix("/w/a")));
        assert!(!inside(&unix("/w"), &unix("/w/a")));
        assert!(!inside(&unix("/W/a"), &unix("/w/a")), "case counts on Unix");
        let win = |p: &str| key_of(p, true);
        assert!(inside(&win(r"C:\Work\App\src"), &win("c:/work/app")));
        assert!(inside(&win("C:/Work/App/"), &win(r"c:\work\app")));
        assert!(!inside(&win(r"C:\Work\Apple"), &win(r"C:\Work\App")));
    }

    #[test]
    fn open_refuses_what_isnt_a_folder() {
        let file = std::env::temp_dir().join(format!(
            "anysola-term-file-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::write(&file, "x").unwrap();
        let mgr = Manager::default();
        let emit: Emit = Arc::new(|_| {});
        for path in [file.clone(), file.with_extension("gone")] {
            let error = mgr
                .open(emit.clone(), &path.display().to_string(), "/bin/sh", 80, 24)
                .unwrap_err();
            assert!(error.contains("is not a folder"), "{error}");
        }
        assert_eq!(mgr.count(), 0);
        std::fs::remove_file(file).unwrap();
    }

    /// A real shell in a PTY, its events collected.
    #[cfg(unix)]
    mod live {
        use super::*;

        pub(super) struct Seen {
            pub events: Mutex<Vec<Value>>,
        }

        impl Seen {
            pub fn bytes(&self, id: &str) -> usize {
                self.events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| e["type"] == "terminalData" && e["id"] == id)
                    .map(|e| BASE64.decode(e["data"].as_str().unwrap()).unwrap().len())
                    .sum()
            }

            pub fn text(&self, id: &str) -> String {
                let mut out = Vec::new();
                for e in self.events.lock().unwrap().iter() {
                    if e["type"] == "terminalData" && e["id"] == id {
                        out.extend(BASE64.decode(e["data"].as_str().unwrap()).unwrap());
                    }
                }
                String::from_utf8_lossy(&out).into_owned()
            }

            pub fn exited(&self, id: &str) -> bool {
                self.events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e["type"] == "terminalExit" && e["id"] == id)
            }
        }

        pub(super) fn folder(name: &str) -> PathBuf {
            // Under the target folder, not /tmp: the shell's rc files may look around.
            let dir = std::env::temp_dir().join(format!(
                "anysola-term-{name}-{}-{}",
                std::process::id(),
                anysola_core::ids::part()
            ));
            std::fs::create_dir_all(dir.join("sub")).unwrap();
            dir
        }

        pub(super) fn open(mgr: &Manager, dir: &Path) -> (String, Arc<Seen>) {
            let seen = Arc::new(Seen {
                events: Mutex::new(Vec::new()),
            });
            let sink = seen.clone();
            let emit: Emit = Arc::new(move |e| sink.events.lock().unwrap().push(e));
            let (id, _) = mgr
                .open(emit, &dir.display().to_string(), "/bin/sh", 120, 30)
                .unwrap();
            (id, seen)
        }

        pub(super) fn until(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
            let start = Instant::now();
            while !ok() {
                assert!(start.elapsed() < limit, "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        pub(super) fn gone(pid: i32) -> bool {
            crate::tree::gone(pid)
        }
    }

    /// Output past what the window acknowledged stays in the PTY: the program waits, nothing
    /// piles up; acknowledging lets it go on.
    #[cfg(unix)]
    #[test]
    fn output_waits_for_the_window() {
        use live::*;
        let dir = folder("flow");
        let mgr = Manager::default();
        let (id, seen) = open(&mgr, &dir);
        mgr.write(&id, "yes\n").unwrap();
        until("yes's output", Duration::from_secs(10), || {
            seen.bytes(&id) > CHUNK
        });
        std::thread::sleep(Duration::from_millis(600));
        let held = seen.bytes(&id);
        assert!(
            held <= IN_FLIGHT + CHUNK,
            "{held} bytes sent with none acknowledged"
        );
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(seen.bytes(&id), held, "nothing more without an ack");
        mgr.ack(&id, held);
        until("more output", Duration::from_secs(10), || {
            seen.bytes(&id) > held
        });
        // Events are gathered, not one per read.
        let events = seen.events.lock().unwrap().len();
        assert!(events * 4096 < seen.bytes(&id) * 2, "{events} events");
        mgr.close(&id);
        until("the exit", Duration::from_secs(5), || seen.exited(&id));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Closing ends what the shell started, its jobs included (each has a process group of its
    /// own), even one that ignores SIGHUP, and the reader stops though a process that left the
    /// session still holds the terminal.
    #[cfg(unix)]
    #[test]
    fn closing_ends_the_shells_jobs_and_its_reader() {
        use live::*;
        let dir = folder("close");
        let mgr = Manager::default();
        let (id, seen) = open(&mgr, &dir);
        let job = dir.join("job");
        let away = dir.join("away");
        mgr.write(
            &id,
            &format!(
                "set -m; (trap '' HUP; sleep 300) & echo $! > '{}'; setsid sh -c 'echo $$ > \"{}\"; exec sleep 300' &  echo ready-$((1+1))\n",
                job.display(),
                away.display()
            ),
        )
        .unwrap();
        until("the jobs", Duration::from_secs(10), || {
            seen.text(&id).contains("ready-2")
        });
        mgr.ack(&id, usize::MAX);
        until("the pids", Duration::from_secs(5), || {
            [&job, &away]
                .iter()
                .all(|p| std::fs::read_to_string(p).is_ok_and(|t| t.ends_with('\n')))
        });
        let read =
            |p: &Path| -> i32 { std::fs::read_to_string(p).unwrap().trim().parse().unwrap() };
        let (job, away) = (read(&job), read(&away));
        let start = Instant::now();
        mgr.close_in_directory(&dir.display().to_string());
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "close waits a bounded time"
        );
        assert_eq!(mgr.count(), 0);
        until("the job to end", Duration::from_secs(5), || gone(job));
        until("the reader to stop", Duration::from_secs(5), || {
            seen.exited(&id)
        });
        // SAFETY: the test's own escaped process.
        unsafe {
            libc::kill(away, libc::SIGKILL);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Deleting a worktree closes terminals opened in it or below it, not one beside it.
    #[cfg(unix)]
    #[test]
    fn closing_a_folder_closes_its_terminals_only() {
        use live::*;
        let dir = folder("folder");
        let other = folder("other");
        let mgr = Manager::default();
        let (inner, a) = open(&mgr, &dir.join("sub"));
        let (beside, b) = open(&mgr, &other);
        mgr.close_in_directory(&format!("{}/", dir.display()));
        until("the inner exit", Duration::from_secs(5), || {
            a.exited(&inner)
        });
        assert!(!b.exited(&beside));
        assert_eq!(mgr.count(), 1);
        mgr.close_all();
        until("the other exit", Duration::from_secs(5), || {
            b.exited(&beside)
        });
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(other).unwrap();
    }
}
