//! Sessions in their own git worktree of the main folder: making it ready (checkout, submodules,
//! the main folder's ignored `.env*` files, the user's setup command), the session's diff against
//! its base, applying it onto the main folder as one commit, bringing the base in, discarding,
//! making it again, and the start-up cleanup. The core keeps the state (`session_worktree`,
//! compare-and-set); this module runs git and the setup command and tells the window
//! (`worktree`, `setup`, `setupEnd` events on the `core` channel).
//!
//! The main folder is touched only by an apply, and only once every check passed: the merge is
//! computed with `git merge-tree` first, the files then move with `read-tree -m -u` (which
//! refuses to overwrite anything), and a failing commit puts them back.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anysola_core::api::{Core, Release};
use anysola_core::storage::{Db, SessionRow, SessionWorktreeRow, WorktreeState};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::git::{self, Call, Failure, Op};
use crate::{Answer, App, core_for, plain_error, text};

/// How long the setup command may run.
const SETUP_LIMIT: Duration = Duration::from_secs(20 * 60);
/// Setup output kept per session.
const SETUP_LINES: usize = 2000;
/// Env files copied into a new worktree: at most this many, each at most this large.
const ENV_FILES: usize = 50;
const ENV_SIZE: u64 = 1024 * 1024;
/// How deep the main folder is searched for env files.
const ENV_DEPTH: usize = 4;
/// Folders never searched for env files.
const ENV_SKIP: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "vendor",
];
/// A file larger than this shows in Review without its text.
const DIFF_SIZE: u64 = 1024 * 1024;
/// Files listed in Review at most.
const DIFF_FILES: usize = 1000;
/// The Git that has `merge-tree --write-tree --merge-base`.
const APPLY_GIT: (u32, u32) = (2, 40);
/// Said wherever an older Git is found.
const OLD_GIT: &str = "Update Git to 2.40 or later to apply.";
/// Who the session's internal commits are by when the user has no Git identity (they never
/// reach the main folder: an apply makes the user's own commit).
const FALLBACK_NAME: &str = "Anysola Code";
const FALLBACK_EMAIL: &str = "code@anysola.invalid";

// ---------------------------------------------------------------------------
// The row as the window reads it
// ---------------------------------------------------------------------------

/// `SessionWorktree` in bridge.ts.
pub(crate) fn json(row: &SessionWorktreeRow) -> Value {
    json!({
        "path": row.directory,
        "branch": row.branch,
        "baseBranch": row.base_branch,
        "baseCommit": row.base_commit,
        "state": row.state.as_str(),
        "error": row.error,
        "appliedCommit": row.applied_commit,
        "restored": row.restored,
        "leftover": row.leftover,
        "bringing": row.base_pending.is_some(),
        "applyBranch": row.apply_branch,
    })
}

/// Tells the window the session's worktree changed.
pub(crate) fn emit(handle: &AppHandle, db: &Db, session_id: &str) {
    if let Ok(Some(row)) = db.session_worktree(session_id) {
        let _ = handle.emit(
            "core",
            json!({"type": "worktree", "directory": row.folder, "sessionId": session_id,
                   "worktree": json(&row)}),
        );
    }
}

fn row_of(db: &Db, session_id: &str) -> Result<SessionWorktreeRow, String> {
    db.session_worktree(session_id)
        .map_err(text)?
        .ok_or_else(|| "This session works in the main folder.".to_string())
}

// ---------------------------------------------------------------------------
// Start-up cleanup
// ---------------------------------------------------------------------------

static CLEANED: AtomicBool = AtomicBool::new(false);

/// Settles every session worktree left from the last run (deleted sessions, leftovers, a
/// preparation or an apply cut off), in the background; nothing makes a worktree before it ends.
pub(crate) fn cleanup_at_start(db: Option<Arc<Db>>) {
    let Some(db) = db else {
        CLEANED.store(true, Ordering::SeqCst);
        return;
    };
    std::thread::spawn(move || {
        for cleaned in anysola_core::api::cleanup_worktrees(&db) {
            if let Some(applied) = &cleaned.undo {
                settle_undo(&db, &cleaned.session_id, applied);
            }
            if let Some(notice) = &cleaned.notice {
                anysola_core::log::warn(
                    "a session worktree was settled at start",
                    &[("session", &cleaned.session_id), ("notice", notice)],
                );
            }
        }
        CLEANED.store(true, Ordering::SeqCst);
    });
}

/// Waits (a while at most) for the start-up cleanup, which must not see a worktree being made
/// as one cut off.
pub(crate) async fn cleaned() {
    let start = Instant::now();
    while !CLEANED.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(60) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// Git: version, availability
// ---------------------------------------------------------------------------

/// `git --version` as (major, minor); None when git can't be run.
fn git_version(directory: &Path) -> Option<(u32, u32)> {
    let out = wgit(directory, &["--version"])
        .timeout(Duration::from_secs(10))
        .ok()
        .ok()?;
    parse_version(&out)
}

fn parse_version(out: &str) -> Option<(u32, u32)> {
    let version = out
        .split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??))
}

fn can_apply(directory: &Path) -> bool {
    git_version(directory).is_some_and(|v| v >= APPLY_GIT)
}

/// A git call for the worktree work: on Windows with long paths on (worktrees sit deep under
/// the home folder), as the core's `worktree add` has them.
fn wgit<'a>(directory: &'a Path, args: &'a [&'a str]) -> Call<'a> {
    // File names as they are (`şema.txt`, not `"\305\237ema.txt"`), so lists compare and read.
    let call = git::git(directory, args)
        .env("GIT_CONFIG_KEY_0", "core.quotePath")
        .env("GIT_CONFIG_VALUE_0", "false");
    if cfg!(windows) {
        call.env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_1", "core.longpaths")
            .env("GIT_CONFIG_VALUE_1", "true")
    } else {
        call.env("GIT_CONFIG_COUNT", "1")
    }
}

fn run_git(directory: &Path, args: &[&str]) -> Result<String, Failure> {
    wgit(directory, args).ok()
}

/// A git call's output, `None` on any failure.
fn read_git(directory: &Path, args: &[&str]) -> Option<String> {
    run_git(directory, args).ok()
}

fn top_of(directory: &Path) -> Option<PathBuf> {
    read_git(directory, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

/// Why new sessions of the folder can't get a worktree, or `None` when they can.
fn unavailable(directory: &Path, paths: &anysola_core::paths::Paths) -> Option<&'static str> {
    if !directory.is_dir() {
        return Some("not-git");
    }
    match wgit(directory, &["rev-parse", "--is-inside-work-tree"]).ok() {
        Err(Failure::Missing) => return Some("git-missing"),
        Ok(inside) if inside == "true" => {}
        _ => return Some("not-git"),
    }
    // The app's own folders (sessions without a project, the worktrees themselves) never.
    let real = anysola_core::config::real_path(directory);
    if real.starts_with(anysola_core::config::real_path(&paths.data)) {
        return Some("not-git");
    }
    if read_git(
        directory,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )
    .is_none_or(|h| h.is_empty())
    {
        return Some("no-commits");
    }
    None
}

/// Whether a new session (or a fork) of the folder gets its own worktree.
pub(crate) async fn available(app: &App, directory: &str) -> bool {
    let (directory, paths) = (PathBuf::from(directory), app.paths.clone());
    tauri::async_runtime::spawn_blocking(move || unavailable(&directory, &paths).is_none())
        .await
        .unwrap_or(false)
}

/// The folder's setup command (`Prefs.worktreeSetup`, by the window's folder key).
fn setup_for(prefs: &crate::prefs::Prefs, folder: &str) -> Option<String> {
    let wanted = crate::key(folder);
    prefs
        .worktree_setup
        .iter()
        .find(|(k, _)| crate::key(k) == wanted)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[tauri::command]
pub async fn worktree_info(app: State<'_, App>, directory: String) -> Answer {
    let setup = setup_for(&app.prefs(), &directory);
    let paths = app.paths.clone();
    git::blocking(move || {
        let folder = Path::new(&directory);
        if let Some(reason) = unavailable(folder, &paths) {
            return Ok(json!({"available": false, "reason": reason}));
        }
        let branch =
            read_git(folder, &["symbolic-ref", "-q", "--short", "HEAD"]).filter(|b| !b.is_empty());
        let uncommitted = read_git(folder, &["status", "--porcelain", "-z"])
            .map(|s| status_count(&s))
            .unwrap_or(0);
        Ok(json!({
            "available": true,
            "branch": branch,
            "uncommitted": uncommitted,
            "setup": setup,
            "canApply": can_apply(folder),
        }))
    })
    .await
}

/// Entries of `status --porcelain -z` (a rename's second path is not an entry).
fn status_count(raw: &str) -> usize {
    let mut count = 0;
    let mut parts = raw.split('\0').filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        count += 1;
        if entry.starts_with('R') || entry.starts_with('C') {
            parts.next();
        }
    }
    count
}

// ---------------------------------------------------------------------------
// Setup output and the setup command
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Run {
    command: Option<String>,
    state: &'static str,
    code: Option<i32>,
    lines: VecDeque<(&'static str, String)>,
    started: u64,
    ended: Option<u64>,
}

impl Run {
    fn new(command: Option<String>) -> Run {
        Run {
            command,
            state: "running",
            code: None,
            lines: VecDeque::new(),
            started: now_ms(),
            ended: None,
        }
    }

    fn json(&self) -> Value {
        json!({
            "command": self.command,
            "state": self.state,
            "code": self.code,
            "lines": self.lines.iter().map(|(stream, text)| json!({"stream": stream, "text": text}))
                .collect::<Vec<_>>(),
            "started": self.started,
            "ended": self.ended,
        })
    }
}

/// One session's preparation and setup runs.
#[derive(Default)]
struct Slot {
    run: Mutex<Option<Run>>,
    /// The setup command running now (preparation or "Run again").
    active: AtomicBool,
    /// Stop the running setup command.
    stop: AtomicBool,
    /// While preparing: don't run (or wait for) the setup command ("Start now").
    skip: AtomicBool,
    /// `git worktree add` of a new session runs: nothing may remove the worktree meanwhile.
    isolating: AtomicBool,
    /// The worktree is going (discarded, deleted, back to the main folder): the preparation
    /// stops at its next step or file, so nothing makes the folder again.
    cancel: AtomicBool,
    /// The last setup command and what it started, also once it ended (`npm run dev &`): kept
    /// until the worktree goes, the command runs again or the app closes, then stopped. A crash
    /// takes them too (see `anysola_core::children`).
    tree: Mutex<Option<anysola_core::children::Tree>>,
}

impl Slot {
    /// Stops what the last setup command left running.
    fn end_tree(&self) {
        let tree = self.tree.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(tree);
    }
}

fn slots() -> &'static Mutex<HashMap<String, Arc<Slot>>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, Arc<Slot>>>> = OnceLock::new();
    SLOTS.get_or_init(Mutex::default)
}

fn slot(session_id: &str) -> Arc<Slot> {
    slots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(session_id.to_string())
        .or_default()
        .clone()
}

fn existing_slot(session_id: &str) -> Option<Arc<Slot>> {
    slots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
}

/// The session's setup command runs now.
pub(crate) fn setup_running(session_id: &str) -> bool {
    existing_slot(session_id).is_some_and(|s| s.active.load(Ordering::SeqCst))
}

/// Stops the session's preparation where it is: no further step or file.
fn cancel_prepare(session_id: &str) {
    if let Some(slot) = existing_slot(session_id) {
        slot.cancel.store(true, Ordering::SeqCst);
    }
    stop_setup(session_id);
}

/// Stops the session's setup command, if one runs, and waits a moment for it to go; what an
/// earlier, finished one left running goes too. Whether a command was running.
fn stop_setup(session_id: &str) -> bool {
    let Some(slot) = existing_slot(session_id) else {
        return false;
    };
    let was_active = {
        let _guard = slot.tree.lock().unwrap_or_else(|e| e.into_inner());
        slot.skip.store(true, Ordering::SeqCst);
        slot.stop.store(true, Ordering::SeqCst);
        slot.active.load(Ordering::SeqCst)
    };
    if !was_active {
        slot.end_tree();
        return false;
    }
    let start = Instant::now();
    while slot.active.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    slot.end_tree();
    true
}

/// Waits until a new session's `git worktree add` is over, so going back to the main folder,
/// discarding or deleting never removes a worktree git is still writing (and the core doesn't
/// move the session into it afterwards).
async fn isolate_done(session_id: &str) {
    let Some(slot) = existing_slot(session_id) else {
        return;
    };
    while slot.isolating.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Every setup command stops, and what finished ones left running (the app is closing). Waits
/// for the running ones as `stop_setup` does, 5 s at most in all.
pub(crate) fn stop_all_setups() {
    let all: Vec<Arc<Slot>> = slots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect();
    for slot in &all {
        let _guard = slot.tree.lock().unwrap_or_else(|e| e.into_inner());
        slot.cancel.store(true, Ordering::SeqCst);
        slot.skip.store(true, Ordering::SeqCst);
        slot.stop.store(true, Ordering::SeqCst);
    }
    let start = Instant::now();
    while all.iter().any(|s| s.active.load(Ordering::SeqCst))
        && start.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    for slot in &all {
        slot.end_tree();
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Where a session's setup lines go: its slot and the window.
#[derive(Clone)]
struct Out {
    handle: Option<AppHandle>,
    folder: String,
    session_id: String,
    slot: Arc<Slot>,
}

impl Out {
    fn new(handle: &AppHandle, folder: &str, session_id: &str) -> Out {
        Out {
            handle: Some(handle.clone()),
            folder: folder.to_string(),
            session_id: session_id.to_string(),
            slot: slot(session_id),
        }
    }

    fn begin(&self, command: Option<String>) {
        *self.slot.run.lock().unwrap_or_else(|e| e.into_inner()) = Some(Run::new(command));
    }

    fn line(&self, stream: &'static str, text: &str) {
        let text = text.trim_end_matches(['\r', '\n']).to_string();
        {
            let mut run = self.slot.run.lock().unwrap_or_else(|e| e.into_inner());
            let run = run.get_or_insert_with(|| Run::new(None));
            run.lines.push_back((stream, text.clone()));
            while run.lines.len() > SETUP_LINES {
                run.lines.pop_front();
            }
        }
        if let Some(handle) = &self.handle {
            let _ = handle.emit(
                "core",
                json!({"type": "setup", "directory": self.folder, "sessionId": self.session_id,
                       "stream": stream, "text": text}),
            );
        }
    }

    fn info(&self, text: &str) {
        self.line("info", text);
    }

    fn end(&self, state: &'static str, code: Option<i32>) {
        let run = {
            let mut run = self.slot.run.lock().unwrap_or_else(|e| e.into_inner());
            let run = run.get_or_insert_with(|| Run::new(None));
            run.state = state;
            run.code = code;
            run.ended = Some(now_ms());
            run.clone()
        };
        if let Some(handle) = &self.handle {
            let _ = handle.emit(
                "core",
                json!({"type": "setupEnd", "directory": self.folder, "sessionId": self.session_id,
                       "run": run.json()}),
            );
        }
    }
}

/// Runs the setup command in the user's shell in `cwd`, line by line into `out`, until it ends,
/// is stopped or runs past [`SETUP_LIMIT`]. Returns the run's state and exit code.
/// Atomically claims `slot` to run a setup command and clears `stop` under `slot.tree`'s lock,
/// so a `stop_setup` or `stop_all_setups` can never race between `active` and `stop`.
fn claim_setup(slot: &Slot) -> bool {
    let _guard = slot.tree.lock().unwrap_or_else(|e| e.into_inner());
    if slot.cancel.load(Ordering::SeqCst) || slot.active.load(Ordering::SeqCst) {
        return false;
    }
    slot.stop.store(false, Ordering::SeqCst);
    slot.active.store(true, Ordering::SeqCst);
    true
}

#[cfg(test)]
fn run_setup(
    out: &Out,
    shell: Option<&str>,
    command: &str,
    cwd: &Path,
    worktree: &str,
    limit: Duration,
) -> (&'static str, Option<i32>) {
    if !claim_setup(&out.slot) {
        return ("stopped", None);
    }
    run_setup_claimed(out, shell, command, cwd, worktree, limit)
}

fn run_setup_claimed(
    out: &Out,
    shell: Option<&str>,
    command: &str,
    cwd: &Path,
    worktree: &str,
    limit: Duration,
) -> (&'static str, Option<i32>) {
    struct ActiveGuard<'a>(&'a AtomicBool);
    impl Drop for ActiveGuard<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let _active = ActiveGuard(&out.slot.active);
    run_setup_inner(out, shell, command, cwd, worktree, limit)
}

fn run_setup_inner(
    out: &Out,
    shell: Option<&str>,
    command: &str,
    cwd: &Path,
    worktree: &str,
    limit: Duration,
) -> (&'static str, Option<i32>) {
    // What the last run left running goes before this one starts (its dev server, its port).
    out.slot.end_tree();
    if out.slot.stop.load(Ordering::SeqCst) || out.slot.cancel.load(Ordering::SeqCst) {
        return ("stopped", None);
    }
    let (program, args) = anysola_core::tool::shell::invocation(shell, command);
    let mut child = anysola_core::console::command(&program);
    child
        .args(&args)
        .current_dir(cwd)
        .env("ANYSOLA_WORKTREE", worktree)
        .env("ANYSOLA_MAIN_FOLDER", &out.folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Everything it starts goes with it when it is stopped.
    crate::tree::own_group(&mut child);
    let mark = anysola_core::children::mark(&mut child);
    let prep = anysola_core::children::prepare_tree();
    // This thread waits for the command below, so it outlives it (Linux: a crash ends the
    // command at once; what it started goes at the next start).
    anysola_core::children::die_with_spawning_thread(&mut child);
    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(e) => {
            out.line("err", &format!("The setup command couldn’t start: {e}"));
            return ("failed", None);
        }
    };
    let tree = prep.attach(&child, mark);
    {
        let mut guard = out.slot.tree.lock().unwrap_or_else(|e| e.into_inner());
        if out.slot.stop.load(Ordering::SeqCst) || out.slot.cancel.load(Ordering::SeqCst) {
            drop(guard);
            crate::tree::kill(&mut child);
            drop(tree);
            return ("stopped", None);
        }
        *guard = Some(tree);
    }
    let (sender, lines) = std::sync::mpsc::channel::<(&'static str, String)>();
    let readers: Vec<_> = [
        (
            "out",
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
        (
            "err",
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
    ]
    .into_iter()
    .filter_map(|(stream, pipe)| pipe.map(|p| (stream, p)))
    .map(|(stream, pipe)| {
        let sender = sender.clone();
        std::thread::spawn(move || read_lines(pipe, stream, sender))
    })
    .collect();
    drop(sender);
    let deadline = Instant::now() + limit;
    let ended = loop {
        while let Ok((stream, line)) = lines.recv_timeout(Duration::from_millis(30)) {
            out.line(stream, &line);
        }
        if out.slot.stop.load(Ordering::SeqCst) {
            crate::tree::kill(&mut child);
            out.slot.end_tree();
            break ("stopped", None);
        }
        if Instant::now() >= deadline {
            crate::tree::kill(&mut child);
            out.slot.end_tree();
            break ("timeout", None);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code();
                break (if status.success() { "done" } else { "failed" }, code);
            }
            Ok(None) => {}
            Err(_) => {
                crate::tree::kill(&mut child);
                out.slot.end_tree();
                break ("failed", None);
            }
        }
    };
    // What came after the last read; a helper that keeps the pipes open is not waited for.
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        match lines.recv_timeout(Duration::from_millis(50)) {
            Ok((stream, line)) => out.line(stream, &line),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
    }
    drop(readers);
    ended
}

fn read_lines(
    mut pipe: Box<dyn Read + Send>,
    stream: &'static str,
    sender: std::sync::mpsc::Sender<(&'static str, String)>,
) {
    let mut pending: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    while let Ok(n) = pipe.read(&mut buffer) {
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..n]);
        while let Some(at) = pending.iter().position(|b| *b == b'\n' || *b == b'\r') {
            let line: Vec<u8> = pending.drain(..=at).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            // A carriage return redraws a progress line: only lines with text are kept.
            if !text.trim().is_empty() && sender.send((stream, text)).is_err() {
                return;
            }
        }
    }
    if !pending.is_empty() {
        let _ = sender.send((stream, String::from_utf8_lossy(&pending).into_owned()));
    }
}

#[tauri::command]
pub async fn setup_output(directory: String, session_id: String) -> Answer {
    let _ = directory;
    Ok(existing_slot(&session_id)
        .and_then(|slot| slot.run.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .map_or(Value::Null, |run| run.json()))
}

/// Runs the folder's setup command again in a ready worktree.
#[tauri::command]
pub async fn setup_run(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let db = app.db()?.clone();
    let row = row_of(&db, &session_id)?;
    if row.state != WorktreeState::Ready {
        return Err("The setup command runs once the worktree is ready.".into());
    }
    let command = setup_for(&app.prefs(), &row.folder)
        .ok_or("This folder has no setup command. Add one in Settings.")?;
    let session = core
        .session(&session_id)
        .map_err(text)?
        .ok_or("This session no longer exists.")?;
    let out = Out::new(&handle, &row.folder, &session_id);
    if !claim_setup(&out.slot) {
        return Err("The setup command is running.".into());
    }
    out.begin(Some(command.clone()));
    let shell = core.loaded().config.shell().map(str::to_string);
    tauri::async_runtime::spawn_blocking(move || {
        out.info(&format!("Running {command}"));
        let (state, code) = run_setup_claimed(
            &out,
            shell.as_deref(),
            &command,
            Path::new(&session.directory),
            &row.directory,
            SETUP_LIMIT,
        );
        setup_said(&out, state, code);
        out.end(state, code);
    });
    Ok(Value::Null)
}

/// The line that closes a setup run.
fn setup_said(out: &Out, state: &str, code: Option<i32>) {
    match (state, code) {
        ("done", _) => out.info("Setup finished."),
        ("failed", Some(code)) => out.line(
            "err",
            &format!("Setup failed (exit {code}). The session goes on."),
        ),
        ("failed", None) => out.line("err", "Setup failed. The session goes on."),
        ("stopped", _) => out.info("Setup stopped."),
        ("timeout", _) => out.line(
            "err",
            "Setup ran for 20 minutes and was stopped. The session goes on.",
        ),
        _ => {}
    }
}

/// Stops the setup command; while preparing also skips it, so the session starts now.
#[tauri::command]
pub async fn setup_stop(app: State<'_, App>, directory: String, session_id: String) -> Answer {
    let _ = directory;
    let preparing = app
        .db()?
        .session_worktree(&session_id)
        .map_err(text)?
        .is_some_and(|r| r.state == WorktreeState::Preparing);
    let slot = slot(&session_id);
    let id = session_id.clone();
    let stopped = git::blocking(move || Ok(stop_setup(&id))).await?;
    if preparing {
        slot.skip.store(true, Ordering::SeqCst);
        return Ok(json!(true));
    }
    Ok(json!(stopped))
}

// ---------------------------------------------------------------------------
// Preparation
// ---------------------------------------------------------------------------

/// How a preparation starts.
pub(crate) enum Made {
    /// A new session: the worktree is made here (`Core::isolate`).
    New,
    /// The core made it already (a fork, "Try again").
    Already,
}

/// Starts making a new session's worktree in the background and returns once its row exists
/// (`preparing`), so a prompt sent now waits for it. False when making it failed at once (the
/// session then works in the main folder).
pub(crate) async fn start_new(handle: &AppHandle, core: &Core, db: &Arc<Db>, id: &str) -> bool {
    cleaned().await;
    let task = tauri::async_runtime::spawn(prepare(
        handle.clone(),
        core.clone(),
        db.clone(),
        id.to_string(),
        Made::New,
    ));
    let start = Instant::now();
    loop {
        if db.session_worktree(id).ok().flatten().is_some() {
            return true;
        }
        if start.elapsed() > Duration::from_secs(30) {
            return false;
        }
        if task.inner().is_finished() {
            return db.session_worktree(id).ok().flatten().is_some();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Starts the rest of a preparation the core began (fork, recreate) in the background.
pub(crate) fn start_made(handle: &AppHandle, core: &Core, db: &Arc<Db>, id: &str) {
    tauri::async_runtime::spawn(prepare(
        handle.clone(),
        core.clone(),
        db.clone(),
        id.to_string(),
        Made::Already,
    ));
}

fn still_preparing(db: &Db, id: &str) -> bool {
    db.session_worktree(id)
        .ok()
        .flatten()
        .is_some_and(|r| r.state == WorktreeState::Preparing)
}

fn seconds(since: Instant) -> String {
    format!("{:.1} s", since.elapsed().as_secs_f64())
}

/// Makes the worktree ready: checkout (for a new session), submodules, env files, setup; then
/// `ready`, which starts a message that waited. Every step is told in the setup output.
async fn prepare(handle: AppHandle, core: Core, db: Arc<Db>, id: String, made: Made) {
    let folder = core.directory().display().to_string();
    let out = Out::new(&handle, &folder, &id);
    out.slot.skip.store(false, Ordering::SeqCst);
    out.slot.stop.store(false, Ordering::SeqCst);
    out.slot.cancel.store(false, Ordering::SeqCst);
    let state = handle.state::<App>();
    let command = setup_for(&state.prefs(), &folder);
    out.begin(command.clone());
    let started = Instant::now();
    if let Made::New = made {
        out.info("Making the worktree");
        out.slot.isolating.store(true, Ordering::SeqCst);
        let made = core.isolate(&id).await;
        out.slot.isolating.store(false, Ordering::SeqCst);
        emit(&handle, &db, &id);
        match made {
            Ok(row) => out.info(&format!(
                "Checked out {} from {} in {}",
                row.branch,
                row.base_branch.as_deref().unwrap_or("the current commit"),
                seconds(started)
            )),
            Err(error) => {
                out.line("err", &plain_error(error));
                out.end("failed", None);
                return;
            }
        }
    } else {
        emit(&handle, &db, &id);
        if let Ok(row) = row_of(&db, &id) {
            out.info(&format!("Checked out {}", row.branch));
        }
    }
    let Ok(row) = row_of(&db, &id) else {
        out.end("failed", None);
        return;
    };
    let shell = core.loaded().config.shell().map(str::to_string);
    let session_dir = core
        .session(&id)
        .ok()
        .flatten()
        .map(|s| PathBuf::from(s.directory))
        .unwrap_or_else(|| PathBuf::from(&row.directory));
    let steps = {
        let (out, row, folder) = (out.clone(), row.clone(), folder.clone());
        let db = db.clone();
        let id = id.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let worktree = Path::new(&row.directory);
            let go = || still_preparing(&db, &id) && !out.slot.cancel.load(Ordering::SeqCst);
            if go() && worktree.join(".gitmodules").is_file() && verify_worktree(&row).is_ok() {
                submodules(&out, worktree);
            }
            if go() {
                copy_env_files(&out, Path::new(&folder), worktree, &go);
            }
            if go() {
                copy_local_files(&out, Path::new(&folder), worktree, &go);
            }
        })
    };
    let _ = steps.await;
    let mut ran: Option<(&'static str, Option<i32>)> = None;
    if let Some(command) = command.as_deref()
        && still_preparing(&db, &id)
        && !out.slot.skip.load(Ordering::SeqCst)
        && claim_setup(&out.slot)
    {
        if out.slot.skip.load(Ordering::SeqCst) {
            out.slot.active.store(false, Ordering::SeqCst);
            out.info("Setup skipped.");
        } else {
            out.info(&format!("Running {command}"));
            let (out2, command, worktree) =
                (out.clone(), command.to_string(), row.directory.clone());
            let result = tauri::async_runtime::spawn_blocking(move || {
                if out2.slot.skip.load(Ordering::SeqCst) {
                    out2.slot.active.store(false, Ordering::SeqCst);
                    return ("stopped", None);
                }
                run_setup_claimed(
                    &out2,
                    shell.as_deref(),
                    &command,
                    &session_dir,
                    &worktree,
                    SETUP_LIMIT,
                )
            })
            .await
            .unwrap_or(("failed", None));
            out.slot.active.store(false, Ordering::SeqCst);
            setup_said(&out, result.0, result.1);
            ran = Some(result);
        }
    } else if command.is_some() && out.slot.skip.load(Ordering::SeqCst) {
        out.info("Setup skipped.");
    }
    if out.slot.cancel.load(Ordering::SeqCst) {
        // Let go meanwhile: anything this preparation put back after the folder was removed goes.
        let gone = db
            .session_worktree(&id)
            .ok()
            .flatten()
            .is_none_or(|r| r.state.released());
        let worktree = PathBuf::from(&row.directory);
        let ours = anysola_core::config::real_path(&worktree).starts_with(
            anysola_core::config::real_path(&state.paths.data.join("worktrees")),
        );
        if gone && ours && worktree.exists() && std::fs::remove_dir_all(&worktree).is_err() {
            anysola_core::log::warn(
                "a cancelled worktree folder could not be removed",
                &[("directory", &row.directory)],
            );
        }
        out.end("stopped", None);
        return;
    }
    let ready = core
        .set_worktree_state(&id, WorktreeState::Preparing, WorktreeState::Ready)
        .unwrap_or(false);
    if ready {
        out.info(&format!("Ready in {}", seconds(started)));
    }
    emit(&handle, &db, &id);
    let (state, code) = ran.unwrap_or(("done", None));
    out.end(state, code);
}

/// `git submodule update --init --recursive`; a failure is told, not fatal.
fn submodules(out: &Out, worktree: &Path) {
    out.info("Checking out submodules");
    let started = Instant::now();
    match wgit(worktree, &["submodule", "update", "--init", "--recursive"])
        .timeout(git::NETWORK * 5)
        .ok()
    {
        Ok(_) => out.info(&format!("Submodules checked out in {}", seconds(started))),
        Err(failure) => {
            let why = match failure {
                Failure::Failed(message) => message,
                other => git::explain(Op::Pull, other),
            };
            out.line("err", &format!("Submodules couldn’t be checked out: {why}"));
        }
    }
}

/// Copies the main folder's ignored `.env*` files to the same place in the worktree, where
/// none is there yet.
fn copy_env_files(out: &Out, folder: &Path, worktree: &Path, go: &dyn Fn() -> bool) {
    let Some(top) = top_of(folder) else {
        return;
    };
    let copied = copy_env(&top, worktree, go);
    match copied.len() {
        0 => {}
        1 => out.info(&format!("Copied {}", copied[0])),
        n => out.info(&format!("Copied {n} env files: {}", copied.join(", "))),
    }
}

/// Local files under these folders that Git doesn't track (hook scripts, settings) come along.
const LOCAL_FOLDERS: [&str; 2] = [".claude", ".anysola"];
/// At most this many local files, each at most this large.
const LOCAL_FILES: usize = 200;
const LOCAL_SIZE: u64 = 1024 * 1024;

/// Copies the main folder's untracked and ignored files under `.claude/` and `.anysola/` into
/// the worktree where it has none, so hooks whose scripts exist only locally run there too.
fn copy_local_files(out: &Out, folder: &Path, worktree: &Path, go: &dyn Fn() -> bool) {
    let Some(top) = top_of(folder) else {
        return;
    };
    let copied = copy_local(&top, worktree, go);
    match copied.len() {
        0 => {}
        1..=4 => out.info(&format!(
            "Copied local hook and config files: {}",
            copied.join(", ")
        )),
        n => out.info(&format!("Copied {n} local hook and config files")),
    }
}

/// The copy itself; returns the copied files' paths (relative, `/`-separated). Only files the
/// main folder's Git ignores: an untracked one that isn't ignored would read as the session's
/// own addition and be applied. A `.claude` or `.anysola` that is a link (or junction) is
/// skipped: it may lead to the home folder's credentials.
fn copy_local(top: &Path, worktree: &Path, go: &dyn Fn() -> bool) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for folder in LOCAL_FOLDERS {
        let root = top.join(folder);
        let plain_folder = std::fs::symlink_metadata(&root)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink() && !reparse_point(&m));
        if plain_folder {
            find_local(top, &root, &mut found);
        }
    }
    if found.is_empty() {
        return Vec::new();
    }
    // Ignored by the main folder's rules and by the worktree's own (its committed .gitignore can
    // differ from the main folder's edited one): a copy never shows as the session's change.
    let ignored_in = |dir: &Path| -> std::collections::HashSet<String> {
        wgit(dir, &["check-ignore", "--no-index", "--stdin", "-z"])
            .input(found.join("\0").into_bytes())
            .run()
            .map(|o| {
                o.stdout
                    .split('\0')
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    // Tracked files aren't candidates (`--no-index` would call a tracked file ignored too).
    let tracked: std::collections::HashSet<String> =
        wgit(top, &["ls-files", "-z", "--", ".claude", ".anysola"])
            .run()
            .map(|o| {
                o.stdout
                    .split('\0')
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    let in_main = ignored_in(top);
    let in_worktree = ignored_in(worktree);
    let ignored: std::collections::HashSet<&str> = in_main
        .iter()
        .filter(|p| in_worktree.contains(*p) && !tracked.contains(*p))
        .map(String::as_str)
        .collect();
    let mut copied = Vec::new();
    for relative in found {
        if copied.len() >= LOCAL_FILES || !go() || !worktree.is_dir() {
            break;
        }
        if !ignored.contains(relative.as_str()) {
            continue;
        }
        let from = top.join(&relative);
        let to = worktree.join(&relative);
        let fits =
            std::fs::symlink_metadata(&from).is_ok_and(|m| m.is_file() && m.len() <= LOCAL_SIZE);
        if !fits || to.symlink_metadata().is_ok() {
            continue;
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::copy(&from, &to).is_ok() {
            copied.push(relative);
        }
    }
    copied
}

/// A Windows reparse point (a junction or a link `symlink_metadata` doesn't call one).
fn reparse_point(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Files under `dir` (any depth, `node_modules` skipped, links not followed), relative to `top`.
fn find_local(top: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // A bound on the walk; only ignored files count toward the copy's own limit.
        if found.len() >= 20_000 {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if entry.file_name() != "node_modules" {
                find_local(top, &entry.path(), found);
            }
        } else if kind.is_file()
            && let Ok(relative) = entry.path().strip_prefix(top)
        {
            found.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// The copy itself; returns the copied files' paths (relative, `/`-separated).
fn copy_env(top: &Path, worktree: &Path, go: &dyn Fn() -> bool) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    find_env(top, top, 0, &mut found);
    if found.is_empty() {
        return Vec::new();
    }
    // Only the ones git ignores: a tracked `.env.example` is in the worktree already.
    let input = found.join("\0").into_bytes();
    let Ok(output) = wgit(top, &["check-ignore", "--stdin", "-z"])
        .input(input)
        .run()
    else {
        return Vec::new();
    };
    let ignored: Vec<&str> = output
        .stdout
        .split('\0')
        .filter(|p| !p.is_empty())
        .collect();
    let mut copied = Vec::new();
    for relative in found {
        // Checked before each file: a worktree let go meanwhile is not made again here.
        if copied.len() >= ENV_FILES || !go() || !worktree.is_dir() {
            break;
        }
        if !ignored.contains(&relative.as_str()) {
            continue;
        }
        let from = top.join(&relative);
        let to = worktree.join(&relative);
        let fits = std::fs::metadata(&from).is_ok_and(|m| m.is_file() && m.len() <= ENV_SIZE);
        if !fits || to.symlink_metadata().is_ok() {
            continue;
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::copy(&from, &to).is_ok() {
            copied.push(relative);
        }
    }
    copied
}

fn find_env(top: &Path, dir: &Path, depth: usize, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if found.len() >= ENV_FILES * 4 {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // Never followed through a link.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth < ENV_DEPTH && !ENV_SKIP.contains(&name.as_str()) {
                find_env(top, &entry.path(), depth + 1, found);
            }
        } else if kind.is_file()
            && name.starts_with(".env")
            && let Ok(relative) = entry.path().strip_prefix(top)
        {
            found.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

// ---------------------------------------------------------------------------
// The session's diff
// ---------------------------------------------------------------------------

/// Checks, without running git there, that the worktree's `.git` still leads to its own
/// administrative folder in the main repository: git run in the worktree reads its config and
/// hooks from wherever `.git` points, and that file is in a folder the agent writes.
fn verify_worktree(row: &SessionWorktreeRow) -> Result<(), String> {
    let top =
        top_of(Path::new(&row.folder)).ok_or("The main folder isn’t a Git repository anymore.")?;
    anysola_core::worktree::verify_checkout(&top, Path::new(&row.directory))
}

/// The worktree's files as a tree (untracked non-ignored files included), without touching its
/// index: a copy of it takes the changes.
fn session_tree(worktree: &Path) -> Result<String, String> {
    let index = read_git(worktree, &["rev-parse", "--git-path", "index"])
        .map(|p| worktree.join(p))
        .ok_or("Couldn’t read the worktree.")?;
    let temp = std::env::temp_dir().join(format!(
        "anysola-index-{}-{}",
        std::process::id(),
        anysola_core::ids::part()
    ));
    if index.is_file() {
        std::fs::copy(&index, &temp).map_err(text)?;
        // The copy keeps the index's time: git tells a file changed within the same second as
        // the index was written ("racily clean") by comparing with it, and a fresh time would
        // hide such a change.
        if let Ok(modified) = std::fs::metadata(&index).and_then(|m| m.modified()) {
            let _ = std::fs::File::options()
                .write(true)
                .open(&temp)
                .and_then(|f| f.set_modified(modified));
        }
    }
    let temp_name = temp.display().to_string();
    let result = (|| {
        wgit(worktree, &["add", "-A", "--", "."])
            .env("GIT_INDEX_FILE", temp_name.clone())
            .timeout(git::COMMIT)
            .ok()
            .map_err(|f| git::explain(Op::Status, f))?;
        wgit(worktree, &["write-tree"])
            .env("GIT_INDEX_FILE", temp_name.clone())
            .ok()
            .map_err(|f| git::explain(Op::Status, f))
    })();
    let _ = std::fs::remove_file(&temp);
    result
}

/// What the session's changes are measured from: during a merge of the base, the base's tip;
/// else where the session's branch and the base branch meet; else the recorded base.
fn diff_base(worktree: &Path, row: &SessionWorktreeRow) -> String {
    if let Some(merging) = read_git(worktree, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]) {
        return merging;
    }
    // The recorded base not on the branch (a merge of it not committed, a worktree made again):
    // where the two meet, never the base itself, whose own changes would read as undone.
    if run_git(
        worktree,
        &["merge-base", "--is-ancestor", &row.base_commit, "HEAD"],
    )
    .is_err()
    {
        return read_git(worktree, &["merge-base", "HEAD", &row.base_commit])
            .unwrap_or_else(|| row.base_commit.clone());
    }
    if let Some(base) = &row.base_branch
        && let Some(meet) = read_git(
            worktree,
            &["merge-base", "HEAD", &format!("refs/heads/{base}")],
        )
    {
        return meet;
    }
    row.base_commit.clone()
}

/// `(status, path, old oid, new oid, new mode)` from `diff-tree -r -z --raw`.
type RawChange = (char, String, String, String, String);

fn raw_changes(directory: &Path, from: &str, to: &str) -> Result<Vec<RawChange>, String> {
    let raw = run_git(
        directory,
        &["diff-tree", "-r", "-z", "--no-renames", "--raw", from, to],
    )
    .map_err(|f| git::explain(Op::Status, f))?;
    let mut out = Vec::new();
    let mut parts = raw.split('\0');
    while let Some(meta) = parts.next() {
        let meta = meta.trim_start_matches('\n');
        if meta.is_empty() {
            continue;
        }
        let Some(path) = parts.next() else {
            break;
        };
        let fields: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        if fields.len() < 5 {
            continue;
        }
        let status = fields[4].chars().next().unwrap_or('M');
        out.push((
            status,
            path.to_string(),
            fields[2].to_string(),
            fields[3].to_string(),
            fields[1].to_string(),
        ));
    }
    Ok(out)
}

fn blob_text(directory: &Path, oid: &str, mode: &str) -> String {
    if mode == "160000" {
        return format!("Subproject commit {oid}\n");
    }
    let size = read_git(directory, &["cat-file", "-s", oid])
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    if size > DIFF_SIZE {
        return format!("(A large file, {} KB: not shown.)\n", size / 1024);
    }
    wgit(directory, &["cat-file", "blob", oid])
        .run()
        .map(|o| o.stdout)
        .unwrap_or_default()
}

/// The session's changes as `FileDiff`s: the worktree's files against its base.
fn worktree_changes(row: &SessionWorktreeRow) -> Result<Vec<Value>, String> {
    let worktree = Path::new(&row.directory);
    if !worktree.is_dir() {
        return Ok(Vec::new());
    }
    verify_worktree(row)?;
    let tree = session_tree(worktree)?;
    let base = diff_base(worktree, row);
    let zero = |oid: &str| oid.chars().all(|c| c == '0');
    let mut out = Vec::new();
    for (status, path, old, new, mode) in raw_changes(worktree, &base, &tree)?
        .into_iter()
        .take(DIFF_FILES)
    {
        let before = (!zero(&old) && status != 'A').then(|| blob_text(worktree, &old, &mode));
        let after = (!zero(&new) && status != 'D').then(|| blob_text(worktree, &new, &mode));
        if let Some(diff) =
            anysola_core::api::file_diff_value(&path, before.as_deref(), after.as_deref())
        {
            out.push(diff);
        }
    }
    Ok(out)
}

/// Review for a worktree session (None: the session works in the main folder).
pub(crate) async fn changes(
    handle: &AppHandle,
    db: &Arc<Db>,
    session_id: &str,
) -> Result<Option<Value>, String> {
    let Some(row) = db.session_worktree(session_id).map_err(text)? else {
        return Ok(None);
    };
    if row.state.released() || noticed_missing(handle, db, &row) {
        return Ok(Some(json!([])));
    }
    git::blocking(move || worktree_changes(&row).map(|list| Some(json!(list)))).await
}

/// A ready worktree whose folder was deleted while the app runs becomes `missing` now (the
/// window shows Recreate); true when that is the case.
fn noticed_missing(handle: &AppHandle, db: &Db, row: &SessionWorktreeRow) -> bool {
    if row.state == WorktreeState::Missing {
        // Found gone elsewhere (a turn's start): the window may not know yet.
        emit(handle, db, &row.session_id);
        return true;
    }
    if row.state != WorktreeState::Ready || Path::new(&row.directory).is_dir() {
        return false;
    }
    // Looked at again a moment later: a busy disk or a network drive can miss once.
    std::thread::sleep(Duration::from_millis(30));
    if Path::new(&row.directory).is_dir() {
        return false;
    }
    let _ = db.cas_session_worktree(
        &row.session_id,
        WorktreeState::Ready,
        WorktreeState::Missing,
        None,
    );
    emit(handle, db, &row.session_id);
    true
}

/// "Put back" for a worktree session: the file as on its base (removed if the session added it).
pub(crate) async fn restore(
    core: &Core,
    db: &Arc<Db>,
    session_id: &str,
    file: &str,
) -> Result<bool, String> {
    let Some(row) = db.session_worktree(session_id).map_err(text)? else {
        return Ok(false);
    };
    if row.state != WorktreeState::Ready {
        return Err("This session’s worktree isn’t ready.".into());
    }
    if core.is_busy(session_id) || setup_running(session_id) {
        return Err("Stop the session before putting files back.".into());
    }
    let file = file.to_string();
    git::blocking(move || put_back_file(&row, &file).map(|_| true)).await
}

/// The file as on the session's base, or gone when the session added it.
fn put_back_file(row: &SessionWorktreeRow, file: &str) -> Result<(), String> {
    verify_worktree(row)?;
    let worktree = Path::new(&row.directory);
    let relative = Path::new(&file);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("{file} isn’t in this session’s worktree."));
    }
    let base = diff_base(worktree, row);
    let spec = format!(":(literal){file}");
    let on_base = run_git(worktree, &["cat-file", "-e", &format!("{base}:{file}")]).is_ok();
    if on_base {
        run_git(worktree, &["checkout", &base, "--", &spec])
            .map_err(|f| git::explain(Op::Status, f))?;
    } else {
        let _ = run_git(
            worktree,
            &["rm", "--cached", "-q", "--ignore-unmatch", "--", &spec],
        );
        let path = worktree.join(relative);
        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(text)?;
        } else if path.symlink_metadata().is_ok() {
            std::fs::remove_file(&path).map_err(text)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// Where an apply would go, worked out without touching the main folder.
struct Plan {
    top: PathBuf,
    /// Files the commit adds that the main folder ignores.
    ignored: Vec<String>,
    onto: String,
    /// The main folder's `HEAD`.
    head: String,
    /// The tree the commit would have.
    result: String,
    files: usize,
    add: u64,
    del: u64,
    behind: u64,
    /// Main-folder files with edits the apply would touch.
    dirty: Vec<String>,
}

/// A refusal as the window's `ApplyCheck`.
fn refused(reason: &str, extra: Value) -> Value {
    let mut value = json!({"ok": false, "reason": reason});
    if let (Some(value), Value::Object(extra)) = (value.as_object_mut(), extra) {
        value.extend(extra);
    }
    value
}

fn git_refusal(message: impl Into<String>) -> Value {
    refused("git", json!({"message": message.into()}))
}

fn identity_env<'a>(call: Call<'a>, directory: &Path) -> Call<'a> {
    if git::has_identity(directory) {
        return call;
    }
    call.env("GIT_AUTHOR_NAME", FALLBACK_NAME)
        .env("GIT_AUTHOR_EMAIL", FALLBACK_EMAIL)
        .env("GIT_COMMITTER_NAME", FALLBACK_NAME)
        .env("GIT_COMMITTER_EMAIL", FALLBACK_EMAIL)
}

/// Conflict markers: a `<<<<<<<` line and a `>>>>>>>` line after it (alone or followed by a
/// space and a name, as git writes them).
fn has_conflict_markers(text: &str) -> bool {
    let marker = |line: &str, mark: &str| {
        line.strip_prefix(mark)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    };
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    lines.any(|l| marker(l, "<<<<<<<")) && lines.any(|l| marker(l, ">>>>>>>"))
}

/// Files the apply would carry with conflict markers in them (from bringing the base in, or any
/// other way), and files still unmerged in the worktree. Never applied: markers would land on
/// the main folder's branch.
fn unresolved(worktree: &Path, tree: &str, base: &str) -> Vec<String> {
    let unmerged: Vec<String> = read_git(worktree, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    // A conflicted file counts as resolved once its markers are gone (as in the git menu).
    let mut files: Vec<String> = Vec::new();
    for (status, path, _, new, mode) in raw_changes(worktree, base, tree).unwrap_or_default() {
        if status == 'D' || mode == "160000" {
            continue;
        }
        if has_conflict_markers(&blob_text(worktree, &new, &mode)) {
            files.push(path);
        }
    }
    for path in unmerged {
        let markers = std::fs::read(worktree.join(&path))
            .is_ok_and(|b| has_conflict_markers(&String::from_utf8_lossy(&b)));
        if markers && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

/// A merge is open in the worktree, or unmerged entries are left.
fn merge_open(worktree: &Path) -> bool {
    read_git(worktree, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_some()
        || read_git(worktree, &["diff", "--name-only", "--diff-filter=U"])
            .is_some_and(|out| !out.trim().is_empty())
}

/// The main folder's operation in progress, if any.
fn operation(top: &Path) -> Option<&'static str> {
    let has = |what: &str| read_git(top, &["rev-parse", "-q", "--verify", what]).is_some();
    let path = |what: &str| {
        read_git(top, &["rev-parse", "--git-path", what]).is_some_and(|p| top.join(p).exists())
    };
    if path("rebase-merge") || path("rebase-apply") {
        return Some("rebase");
    }
    if has("MERGE_HEAD") {
        return Some("merge");
    }
    if has("CHERRY_PICK_HEAD") {
        return Some("cherry-pick");
    }
    if has("REVERT_HEAD") {
        return Some("revert");
    }
    None
}

/// Works out the apply: `Ok(plan)` when it can go (dirty files listed in the plan), else the
/// refusal as `ApplyCheck`.
fn plan(row: &SessionWorktreeRow, markers: bool) -> Result<Plan, Value> {
    let worktree = Path::new(&row.directory);
    let folder = Path::new(&row.folder);
    let top = top_of(folder)
        .ok_or_else(|| git_refusal("The main folder isn’t a Git repository anymore."))?;
    if !can_apply(&top) {
        return Err(git_refusal(OLD_GIT));
    }
    if !worktree.is_dir() {
        return Err(git_refusal("This session’s folder is gone."));
    }
    verify_worktree(row).map_err(git_refusal)?;
    let tree = session_tree(worktree).map_err(git_refusal)?;
    let base = diff_base(worktree, row);
    let open = unresolved(worktree, &tree, &base);
    if !open.is_empty() && !markers {
        let onto =
            read_git(&top, &["symbolic-ref", "-q", "--short", "HEAD"]).filter(|b| !b.is_empty());
        // "Apply anyway" only outside a merge (its markers are its conflicts).
        let anyway = !merge_open(worktree);
        return Err(refused(
            "unresolved",
            json!({"files": open, "onto": onto, "anyway": anyway}),
        ));
    }
    let Some(onto) =
        read_git(&top, &["symbolic-ref", "-q", "--short", "HEAD"]).filter(|b| !b.is_empty())
    else {
        return Err(refused("detached", json!({})));
    };
    if let Some(what) = operation(&top) {
        return Err(refused("operation", json!({"what": what})));
    }
    let head = read_git(&top, &["rev-parse", "HEAD"])
        .ok_or_else(|| git_refusal("Couldn’t read the main folder’s commit."))?;
    let base_tree = read_git(&top, &["rev-parse", &format!("{base}^{{tree}}")]).unwrap_or_default();
    if base_tree == tree {
        return Err(refused("empty", json!({})));
    }
    let staged = wgit(&top, &["diff", "--cached", "--quiet"]).run();
    if staged.is_ok_and(|o| !o.status.success()) {
        return Err(refused("staged", json!({"onto": onto})));
    }
    // The session's files as a commit of their own, on nothing: only the merge reads it.
    let worktree_head = read_git(worktree, &["rev-parse", "HEAD"]).unwrap_or_else(|| base.clone());
    let session_commit = identity_env(
        wgit(
            worktree,
            &["commit-tree", &tree, "-p", &worktree_head, "-m", "session"],
        ),
        worktree,
    )
    .ok()
    .map_err(|f| git_refusal(git::explain(Op::Commit, f)))?;
    let merged = wgit(
        &top,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            &format!("--merge-base={base}"),
            &head,
            &session_commit,
        ],
    )
    .run()
    .map_err(|f| git_refusal(git::explain(Op::Status, f)))?;
    let mut lines = merged.stdout.lines().filter(|l| !l.is_empty());
    let result = lines.next().unwrap_or_default().to_string();
    match merged.status.code() {
        Some(0) => {}
        Some(1) => {
            let files: Vec<String> = lines.map(str::to_string).collect();
            return Err(refused("conflict", json!({"onto": onto, "files": files})));
        }
        _ => {
            return Err(git_refusal(git::explain(
                Op::Status,
                Failure::Failed(merged.stderr),
            )));
        }
    }
    let head_tree = read_git(&top, &["rev-parse", "HEAD^{tree}"]).unwrap_or_default();
    if result == head_tree {
        return Err(refused("empty", json!({})));
    }
    let numstat = read_git(
        &top,
        &[
            "diff-tree",
            "-r",
            "--no-renames",
            "--numstat",
            "-z",
            &head_tree,
            &result,
        ],
    )
    .unwrap_or_default();
    let (mut files, mut add, mut del, mut touched) = (0, 0u64, 0u64, Vec::new());
    for (a, d, path) in numstat_z(&numstat) {
        files += 1;
        add += a;
        del += d;
        touched.push(path);
    }
    let behind = read_git(&top, &["rev-list", "--count", &format!("{base}..{head}")])
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let (_, entries) =
        git::status_entries(&top).map_err(|f| git_refusal(git::explain(Op::Status, f)))?;
    let mut dirty: Vec<String> = entries
        .iter()
        .filter(|e| {
            touched.contains(&e.file) || e.from.as_ref().is_some_and(|f| touched.contains(f))
        })
        .map(|e| e.file.clone())
        .collect();
    // A touched path that is in the main folder but not in its commit, ignored ones included
    // (`.env`): moving the files there would overwrite it, and putting them back delete it.
    // Each leading folder of a touched path too: an ignored file `cache` where the session has
    // `cache/data` is in the way just the same.
    let mut candidates: Vec<String> = Vec::new();
    for path in &touched {
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if !dirty.contains(&prefix) && !candidates.contains(&prefix) {
                candidates.push(prefix.clone());
            }
        }
    }
    let unknown: Vec<&String> = candidates.iter().collect();
    let committed = in_commit(&top, &head, &unknown);
    for path in unknown {
        let here = top.join(path).symlink_metadata();
        // A touched path that is there at all is in the way; of a leading folder, only
        // something that isn't a real folder (a file, a link).
        let in_the_way = match here {
            Ok(_) if touched.contains(path) => true,
            Ok(meta) => !meta.is_dir(),
            Err(_) => false,
        };
        if !committed.contains(path) && in_the_way && !dirty.contains(path) {
            dirty.push(path.clone());
        }
    }
    // Files the commit would add that the main folder's .gitignore ignores (a force-added .env):
    // said, not refused.
    let added: Vec<&String> = touched.iter().filter(|p| !committed.contains(*p)).collect();
    let ignored = ignored_paths(&top, &added);
    Ok(Plan {
        top,
        ignored,
        onto,
        head,
        result,
        files,
        add,
        del,
        behind,
        dirty,
    })
}

/// `diff-tree --numstat -z --no-renames` read: added and removed lines (0 for a binary file) and
/// the path as it is (`-z`: a quote, a tab, a newline or a name beyond ASCII isn't C-quoted).
fn numstat_z(out: &str) -> Vec<(u64, u64, String)> {
    out.split('\0')
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let mut parts = record.splitn(3, '\t');
            let (a, d, path) = (parts.next()?, parts.next()?, parts.next()?);
            Some((
                a.parse().unwrap_or(0),
                d.parse().unwrap_or(0),
                path.to_string(),
            ))
        })
        .collect()
}

/// Which of `paths` the main folder's ignore rules match.
fn ignored_paths(top: &Path, paths: &[&String]) -> Vec<String> {
    if paths.is_empty() {
        return Vec::new();
    }
    let input = paths
        .iter()
        .map(|p| p.as_str())
        .collect::<Vec<_>>()
        .join("\0")
        .into_bytes();
    let Ok(output) = wgit(top, &["check-ignore", "--no-index", "--stdin", "-z"])
        .input(input)
        .run()
    else {
        return Vec::new();
    };
    output
        .stdout
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Which of `paths` the commit `rev` has (one `cat-file --batch-check -Z` for all, so names with
/// newlines never shift subsequent results).
fn in_commit(top: &Path, rev: &str, paths: &[&String]) -> std::collections::HashSet<String> {
    let mut found = std::collections::HashSet::new();
    if paths.is_empty() {
        return found;
    }
    let input: String = paths.iter().map(|p| format!("{rev}:{p}\0")).collect();
    let Ok(output) = wgit(top, &["cat-file", "--batch-check", "-Z"])
        .input(input.into_bytes())
        .run()
    else {
        return found;
    };
    for (path, entry) in paths.iter().zip(output.stdout.split('\0')) {
        if !entry.is_empty() && !entry.ends_with(" missing") {
            found.insert((*path).clone());
        }
    }
    found
}

/// The check before anything else is asked of the user.
fn check(core: &Core, row: &SessionWorktreeRow) -> Value {
    match row.state {
        WorktreeState::Ready => {}
        WorktreeState::Applying => return refused("busy", json!({})),
        _ => return git_refusal("This session’s worktree isn’t ready."),
    }
    if core.is_busy(&row.session_id) || setup_running(&row.session_id) {
        return refused("busy", json!({}));
    }
    match plan(row, false) {
        Err(refusal) => refusal,
        Ok(plan) if !plan.dirty.is_empty() => {
            refused("dirty", json!({"onto": plan.onto, "files": plan.dirty}))
        }
        Ok(plan) => json!({
            "ok": true, "onto": plan.onto, "files": plan.files, "add": plan.add, "del": plan.del,
            "behind": plan.behind, "ignored": plan.ignored,
        }),
    }
}

#[tauri::command]
pub async fn worktree_apply_check(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let db = app.db()?.clone();
    let row = row_of(&db, &session_id)?;
    if noticed_missing(&handle, &db, &row) {
        return Ok(git_refusal("This session’s folder is gone."));
    }
    git::blocking(move || Ok(check(&core, &row))).await
}

/// Tells the user which background commands were stopped for `what` (one line).
fn stopped_notice(handle: &AppHandle, stopped: &[String], what: &str) {
    if stopped.is_empty() {
        return;
    }
    let names: Vec<String> = stopped
        .iter()
        .map(|c| match c.char_indices().nth(40) {
            Some((at, _)) => format!("{}…", &c[..at]),
            None => c.clone(),
        })
        .collect();
    let message = match names.len() {
        1 => format!("Stopped a background command for {what}: {}", names[0]),
        n => format!(
            "Stopped {n} background commands for {what}: {}",
            names.join(", ")
        ),
    };
    let _ = handle.emit("core", json!({"type": "notice", "message": message}));
}

/// One commit on the main folder's branch with the session's changes; the session then goes on
/// from it. (Tauri hands the window's arguments one by one.)
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn worktree_apply(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    message: String,
    onto: String,
    stash: bool,
    markers: Option<bool>,
) -> Answer {
    let markers = markers.unwrap_or(false);
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("A commit needs a message.".into());
    }
    let core = core_for(&handle, &app, &directory).await?;
    let db = app.db()?.clone();
    if setup_running(&session_id) {
        return Err("The setup command is running. Stop it first.".into());
    }
    // Nothing runs in the session from here until the apply ends.
    if !core
        .set_worktree_state(&session_id, WorktreeState::Ready, WorktreeState::Applying)
        .map_err(plain_error)?
    {
        return Err("This session’s worktree isn’t ready.".into());
    }
    // Background commands could write the worktree while its files are read and moved.
    stopped_notice(&handle, &core.stop_background(&session_id), "the apply");
    emit(&handle, &db, &session_id);
    let applied = {
        let (core, db, id) = (core.clone(), db.clone(), session_id.clone());
        git::blocking(move || {
            let row = row_of(&db, &id)?;
            let start = |from: &str, tree: &str| core.mark_apply(&id, from, tree).unwrap_or(false);
            let stashed = |oid: &str| {
                let _ = core.set_apply_stash(&id, oid);
            };
            let marks = Marks {
                start: &start,
                stashed: &stashed,
            };
            Ok(apply(&marks, &row, &message, &onto, stash, markers))
        })
        .await
    };
    let applied = applied.and_then(|r| r);
    match applied {
        Ok((commit, stash_left)) => {
            let result = core.continue_after_apply(&session_id, &commit);
            emit(&handle, &db, &session_id);
            result.map_err(plain_error)?;
            Ok(json!({"commit": commit, "stashLeft": stash_left}))
        }
        Err(error) => {
            // A merge of the base this apply committed before failing: its tip is the base now.
            if let Ok(row) = row_of(&db, &session_id)
                && let Some(pending) = &row.base_pending
                && run_git(
                    Path::new(&row.directory),
                    &["merge-base", "--is-ancestor", pending, "HEAD"],
                )
                .is_ok()
            {
                let _ = core.conclude_base(&session_id, WorktreeState::Applying);
            }
            let _ =
                core.set_worktree_state(&session_id, WorktreeState::Applying, WorktreeState::Ready);
            emit(&handle, &db, &session_id);
            Err(error)
        }
    }
}

/// Undoes the last commit this session applied onto the main folder, as long as the main folder's
/// `HEAD` is the branch the apply went to, still at that exact commit, none of its files have
/// uncommitted or staged edits, and nothing (ignored files included) is where a file it removed
/// comes back. The worktree's own files are kept intact and its base moves back to the undone
/// commit's parent. A commit a remote branch has is undone only once `pushed` says the user
/// confirmed: until then the answer names those branches (`{"pushed": [...]}`) and nothing moves.
#[tauri::command]
pub async fn worktree_undo_apply(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    pushed: Option<bool>,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let db = app.db()?.clone();
    if setup_running(&session_id) {
        return Err("The setup command is running. Stop it first.".into());
    }
    if !core.begin_undo(&session_id).map_err(plain_error)? {
        let row = row_of(&db, &session_id)?;
        if row.state != WorktreeState::Ready {
            return Err("This session’s worktree isn’t ready.".into());
        }
        return Err("There’s no applied commit to undo.".into());
    }
    emit(&handle, &db, &session_id);
    let allow_pushed = pushed.unwrap_or(false);
    let undone = {
        let (core, db, id) = (core.clone(), db.clone(), session_id.clone());
        git::blocking(move || {
            let row = row_of(&db, &id)?;
            let mark = |applied: &str| core.mark_undo(&id, applied).unwrap_or(false);
            Ok(undo_apply_checked(&row, allow_pushed, &mark))
        })
        .await
    };
    let back_to_ready = |db: &Db| {
        let _ = core.set_worktree_state(&session_id, WorktreeState::Applying, WorktreeState::Ready);
        emit(&handle, db, &session_id);
    };
    match undone {
        Ok(Ok(UndoOutcome::Pushed(remotes))) => {
            // Move back to `ready` before clearing `undo_from` so a crash in between is never
            // mistaken for a cut-off apply.
            back_to_ready(&db);
            anysola_core::api::clear_undo_mark(&db, &session_id);
            Ok(json!({"pushed": remotes}))
        }
        Ok(Ok(UndoOutcome::Done(undone))) => match core.undo_apply(&session_id, &undone.parent) {
            Ok(_) => {
                emit(&handle, &db, &session_id);
                Ok(json!({"commit": undone.parent}))
            }
            Err(err) => {
                // The main folder goes back to the applied commit, as the row still says.
                let restored = git::blocking(move || Ok(redo_apply(&undone))).await;
                if matches!(restored, Ok(true)) {
                    back_to_ready(&db);
                    anysola_core::api::clear_undo_mark(&db, &session_id);
                    Err(plain_error(err))
                } else {
                    // Left marked and applying: the next start settles it from the main folder.
                    emit(&handle, &db, &session_id);
                    Err(format!("{} {STUCK}", plain_error(err)))
                }
            }
        },
        Ok(Err(failed)) if failed.settled => {
            back_to_ready(&db);
            anysola_core::api::clear_undo_mark(&db, &session_id);
            Err(failed.message)
        }
        Ok(Err(failed)) => {
            emit(&handle, &db, &session_id);
            Err(format!("{} {STUCK}", failed.message))
        }
        // The git work itself failed to run: leave applying + undo_from for the next start to settle.
        Err(error) => {
            emit(&handle, &db, &session_id);
            Err(format!("{error} {STUCK}"))
        }
    }
}

/// Said when an undo stopped halfway and couldn't put the main folder back.
const STUCK: &str =
    "The main folder may be halfway between the two commits; restart Anysola Code to settle it.";

/// The remote branches that have the session's applied commit (`git branch -r --contains`).
fn pushed_to(row: &SessionWorktreeRow) -> Vec<String> {
    let Some(applied) = row.applied_commit.as_deref().filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    let Some(top) = top_of(Path::new(&row.folder)) else {
        return Vec::new();
    };
    read_git(
        &top,
        &[
            "branch",
            "-r",
            "--contains",
            applied,
            // `origin/HEAD` points at another remote branch, listed on its own: left out.
            "--format=%(if)%(symref)%(then)%(else)%(refname:short)%(end)",
        ],
    )
    .unwrap_or_default()
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty())
    .map(str::to_string)
    .collect()
}

/// An undo done on the main folder: its branch is at `parent`, files and index too.
#[derive(Debug)]
struct Undone {
    top: PathBuf,
    branch: String,
    applied: String,
    parent: String,
}

/// An undo that didn't go through: `settled` when the main folder is as it was before it (the
/// mark can go), false when it may be halfway (the mark stays for the next start).
#[derive(Debug)]
struct Failed {
    message: String,
    settled: bool,
}

impl From<String> for Failed {
    fn from(message: String) -> Failed {
        Failed {
            message,
            settled: true,
        }
    }
}

/// Puts the main folder back on the applied commit after an undo the core couldn't record:
/// the branch, then the index and files. False when either step failed.
fn redo_apply(undone: &Undone) -> bool {
    run_git(
        &undone.top,
        &[
            "update-ref",
            "-m",
            "anysola: undo apply failed",
            &undone.branch,
            &undone.applied,
            &undone.parent,
        ],
    )
    .is_ok()
        && run_git(
            &undone.top,
            &["read-tree", "-m", "-u", &undone.parent, &undone.applied],
        )
        .is_ok()
}

/// Short for `refs/heads/<name>`.
fn short_branch(full: &str) -> &str {
    full.strip_prefix("refs/heads/").unwrap_or(full)
}

#[derive(Debug)]
enum UndoOutcome {
    Done(Undone),
    Pushed(Vec<String>),
}

/// The undo on the main folder (for callers that already confirmed pushed commits, and tests).
#[cfg(test)]
fn undo_apply(row: &SessionWorktreeRow, mark: &dyn Fn(&str) -> bool) -> Result<Undone, Failed> {
    match undo_apply_checked(row, true, mark)? {
        UndoOutcome::Done(undone) => Ok(undone),
        UndoOutcome::Pushed(_) => unreachable!("allow_pushed was true"),
    }
}

/// The undo on the main folder. Every refusal comes before the pushed-remote check and `mark`
/// (the row's undo mark, written before anything is touched); after it, a failure puts the main
/// folder back or says it may not have (`Failed::settled`).
fn undo_apply_checked(
    row: &SessionWorktreeRow,
    allow_pushed: bool,
    mark: &dyn Fn(&str) -> bool,
) -> Result<UndoOutcome, Failed> {
    let applied = row
        .applied_commit
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "There’s no applied commit to undo.".to_string())?;
    verify_worktree(row)?;
    let worktree = Path::new(&row.directory);
    if row.base_pending.is_some() || merge_open(worktree) {
        return Err("Finish bringing main in before undoing the last apply."
            .to_string()
            .into());
    }
    let folder = Path::new(&row.folder);
    let top = top_of(folder)
        .ok_or_else(|| "The main folder isn’t a Git repository anymore.".to_string())?;
    if let Some(what) = operation(&top) {
        return Err(format!("Finish the {what} in the main folder first.").into());
    }
    // On the branch the apply went to (when the row knows it; before 1.4.3, any branch): a
    // detached HEAD, or another branch at the same commit, would be rewound while the apply's
    // branch keeps the commit.
    let current = read_git(&top, &["symbolic-ref", "-q", "HEAD"]).filter(|b| !b.is_empty());
    let branch = match (&row.apply_branch, current) {
        (Some(to), Some(current)) if *to == current => current,
        (Some(to), Some(current)) => {
            return Err(format!(
                "The main folder is on {}, not {}, the branch the commit was applied to. Check out {} first.",
                short_branch(&current),
                short_branch(to),
                short_branch(to)
            )
            .into());
        }
        (Some(to), None) => {
            return Err(format!(
                "The main folder isn’t on a branch (detached HEAD). Check out {}, the branch the commit was applied to, first.",
                short_branch(to)
            )
            .into());
        }
        (None, Some(current)) => current,
        (None, None) => {
            return Err("The main folder isn’t on a branch (detached HEAD). Check out the branch the commit was applied to first.".to_string().into());
        }
    };
    let head = read_git(&top, &["rev-parse", "HEAD"])
        .ok_or_else(|| "Couldn’t read the main folder’s commit.".to_string())?;
    if head != applied {
        let short = &applied[..applied.len().min(7)];
        let moved_on = run_git(&top, &["merge-base", "--is-ancestor", applied, &head]).is_ok();
        return Err(if moved_on {
            format!(
                "The main folder has moved past the applied commit. To take it back, revert it with Git: git revert {short}"
            )
        } else {
            format!(
                "The main folder’s branch isn’t at the applied commit ({short}) anymore: it was reset or rewritten, so there’s nothing here to undo."
            )
        }
        .into());
    }
    let parent = read_git(
        &top,
        &["rev-parse", "--verify", "--quiet", &format!("{applied}^")],
    )
    .ok_or_else(|| "That commit has no parent to return to.".to_string())?;
    let staged = wgit(&top, &["diff", "--cached", "--quiet"])
        .run()
        .map_err(|f| git::explain(Op::Status, f))?;
    if !staged.status.success() {
        return Err(
            "The main folder has staged changes. Commit or unstage them first."
                .to_string()
                .into(),
        );
    }
    in_the_way_of_undo(&top, applied, &parent)?;
    // Files only touched (same content, new timestamp) aren't edits.
    let _ = run_git(&top, &["update-index", "-q", "--refresh"]);
    // Only ask for confirmation when the undo would otherwise proceed.
    if !allow_pushed {
        let remotes = pushed_to(row);
        if !remotes.is_empty() {
            return Ok(UndoOutcome::Pushed(remotes));
        }
    }
    // From here on the main folder changes: a crash is told apart from an apply cut off.
    if !mark(applied) {
        return Err("This session’s apply changed meanwhile.".to_string().into());
    }
    if let Err(failure) = run_git(&top, &["read-tree", "-m", "-u", applied, &parent]) {
        return Err(match failure {
            Failure::Failed(m) if m.contains("not uptodate") || m.contains("would be overwritten") => {
                match blocking_file(&m) {
                    Some(file) => format!("The main folder has uncommitted edits in “{file}”, a file from that commit. Save or stash them first."),
                    None => "The main folder has uncommitted edits in files from that commit. Save or stash them first.".into(),
                }
            }
            Failure::Failed(m) => m,
            other => git::explain(Op::Commit, other),
        }
        .into());
    }
    if let Err(failure) = run_git(
        &top,
        &[
            "update-ref",
            "-m",
            "anysola: undo apply",
            &branch,
            &parent,
            applied,
        ],
    ) {
        let back = run_git(&top, &["read-tree", "-m", "-u", &parent, applied]).is_ok();
        return Err(Failed {
            message: git::explain(Op::Commit, failure),
            settled: back,
        });
    }
    Ok(UndoOutcome::Done(Undone {
        top,
        branch,
        applied: applied.to_string(),
        parent,
    }))
}

/// At start, for an undo cut off with the main folder's branch still on the applied commit
/// ([`anysola_core::api::Cleaned::undo`]): its index and files go back to that commit when the
/// cut came after they moved to the parent; nothing is touched otherwise. The mark goes once
/// the main folder is settled (kept, to try again, when putting it back failed).
fn settle_undo(db: &Db, session_id: &str, applied: &str) {
    let Ok(Some(row)) = db.session_worktree(session_id) else {
        return;
    };
    let Some(top) = top_of(Path::new(&row.folder)) else {
        return;
    };
    let on_branch = match (
        &row.apply_branch,
        read_git(&top, &["symbolic-ref", "-q", "HEAD"]),
    ) {
        (Some(to), Some(cur)) => cur == *to,
        (None, Some(_)) => true,
        _ => false,
    };
    if !on_branch {
        anysola_core::api::clear_undo_mark(db, session_id);
        return;
    }
    let head = read_git(&top, &["rev-parse", "HEAD"]);
    let parent = read_git(
        &top,
        &["rev-parse", "--verify", "--quiet", &format!("{applied}^")],
    );
    if let (Some(head), Some(parent)) = (head, parent)
        && head == applied
        // The index is the parent's: `read-tree` ran, `update-ref` didn't.
        && run_git(&top, &["diff-index", "--cached", "--quiet", &parent, "--"]).is_ok()
        && run_git(&top, &["diff-index", "--cached", "--quiet", applied, "--"]).is_err()
    {
        let _ = run_git(&top, &["update-index", "-q", "--refresh"]);
        if let Err(failure) = run_git(&top, &["read-tree", "-m", "-u", &parent, applied]) {
            anysola_core::log::warn(
                "an undo cut off couldn't be put back",
                &[("session", session_id), ("error", &format!("{failure:?}"))],
            );
            return;
        }
    }
    anysola_core::api::clear_undo_mark(db, session_id);
}

/// Refuses, naming it, a file the undo would write over that Git doesn't guard: a path the
/// applied commit removed (it comes back) that is in the main folder again, ignored files
/// included, or something that isn't a real folder where one of its leading folders goes. As
/// the apply's own check.
fn in_the_way_of_undo(top: &Path, applied: &str, parent: &str) -> Result<(), String> {
    // From the commit to its parent, `A`dded: in the parent, not in the commit.
    let removed = run_git(
        top,
        &[
            "diff-tree",
            "-r",
            "--no-renames",
            "--name-only",
            "-z",
            "--diff-filter=A",
            applied,
            parent,
        ],
    )
    .map_err(|f| git::explain(Op::Status, f))?;
    // `-z`: names as they are (a quote, a tab or a newline in one isn't C-quoted).
    let back: Vec<String> = removed
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    // Where the commit put a folder in a removed file's place, the undo deletes that folder:
    // Git guards its untracked files, not its ignored ones.
    for path in &back {
        if !top.join(path).symlink_metadata().is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let inside = run_git(
            top,
            &[
                "ls-files",
                "--others",
                "-z",
                "--",
                &format!(":(literal){path}/"),
            ],
        )
        .map_err(|f| git::explain(Op::Status, f))?;
        if let Some(file) = inside.split('\0').find(|p| !p.is_empty()) {
            return Err(format!(
                "“{file}” in the main folder would be deleted by undoing the apply (that commit made “{path}” a folder). Move it away first."
            ));
        }
    }
    let mut candidates: Vec<String> = Vec::new();
    for path in &back {
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if !candidates.contains(&prefix) {
                candidates.push(prefix.clone());
            }
        }
    }
    let unknown: Vec<&String> = candidates.iter().collect();
    let committed = in_commit(top, applied, &unknown);
    let blocking = unknown.into_iter().find(|path| {
        let in_the_way = match top.join(path).symlink_metadata() {
            Ok(_) if back.contains(path) => true,
            Ok(meta) => !meta.is_dir(),
            Err(_) => false,
        };
        in_the_way && !committed.contains(*path)
    });
    match blocking {
        Some(file) => Err(format!(
            "“{file}” in the main folder would be overwritten by undoing the apply (that commit removed it). Move it away first."
        )),
        None => Ok(()),
    }
}

/// The file a failed `read-tree` names: `Entry 'a.txt' not uptodate`, or the first one listed
/// after `would be overwritten by merge:`.
fn blocking_file(message: &str) -> Option<String> {
    if let Some(rest) = message.split("Entry '").nth(1)
        && let Some(end) = rest.find("' ")
    {
        return Some(rest[..end].to_string());
    }
    let (_, list) = message.split_once("would be overwritten by merge:")?;
    list.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// The apply itself, while `applying`. `Ok((commit, stash left))`; on `Err` the main folder is
/// as it was (or, after a hook changed files, as the hook left it: the error says so).
/// What the core keeps of an apply while it runs: where it starts and the tree it puts there
/// (false: the apply changed meanwhile), and the stash it set edits aside in.
struct Marks<'a> {
    start: &'a dyn Fn(&str, &str) -> bool,
    stashed: &'a dyn Fn(&str),
}

fn apply(
    marks: &Marks<'_>,
    row: &SessionWorktreeRow,
    message: &str,
    onto: &str,
    stash: bool,
    markers: bool,
) -> Result<(String, Option<String>), String> {
    let mark = marks.start;
    let row = row.clone();
    verify_worktree(&row)?;
    let worktree = Path::new(&row.directory);
    // Markers are applied as they are only outside a merge: one that is open must be finished.
    if markers && merge_open(worktree) {
        return Err(
            "Finish bringing main in first: files with conflict markers can’t be applied while its merge is open."
                .into(),
        );
    }
    // A merge of the base that is resolved now: finished on the session's branch first.
    if read_git(worktree, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_some() {
        let tree = session_tree(worktree)?;
        let base = diff_base(worktree, &row);
        let open = unresolved(worktree, &tree, &base);
        if !open.is_empty() && !markers {
            return Err(format!(
                "Resolve the conflicts in {} first.",
                open.join(", ")
            ));
        }
        run_git(worktree, &["add", "-A", "--", "."]).map_err(|f| git::explain(Op::Commit, f))?;
        identity_env(
            wgit(
                worktree,
                &["commit", "--no-verify", "-q", "-m", "Bring the base in"],
            ),
            worktree,
        )
        .timeout(git::COMMIT)
        .ok()
        .map_err(|f| git::explain(Op::Commit, f))?;
    }
    let row = SessionWorktreeRow {
        state: WorktreeState::Ready,
        ..row
    };
    let plan = match plan(&row, markers) {
        Ok(plan) => plan,
        Err(refusal) => return Err(refusal_sentence(&refusal)),
    };
    if plan.onto != onto {
        return Err(format!(
            "The main folder is on {} now. Check again before applying.",
            plan.onto
        ));
    }
    if !plan.dirty.is_empty() && !stash {
        return Err(format!(
            "The main folder has edits in {}. Set them aside to apply.",
            plan.dirty.join(", ")
        ));
    }
    let top = &plan.top;
    let mut stashed: Option<String> = None;
    if !plan.dirty.is_empty() {
        // `--all`: ignored files in the way (an `.env` the session also has) are set aside too.
        let message = anysola_core::api::apply_stash_message(&row.session_id);
        let mut args = vec!["stash", "push", "--all", "-m", message.as_str(), "--"];
        let specs: Vec<String> = plan
            .dirty
            .iter()
            .map(|f| format!(":(literal){f}"))
            .collect();
        args.extend(specs.iter().map(String::as_str));
        run_git(top, &args).map_err(|f| git::explain(Op::Commit, f))?;
        stashed = read_git(top, &["rev-parse", "-q", "--verify", "refs/stash"]);
        if let Some(oid) = &stashed {
            (marks.stashed)(oid);
        }
    }
    // Back only when it can't conflict; otherwise it stays in the stash, named for the user.
    // The user's files are never reset to make room for it.
    let put_back = |stashed: &Option<String>| -> Option<String> {
        let stashed = stashed.as_ref()?;
        anysola_core::api::pop_stash_if_clean(top, stashed).err()
    };
    if !mark(&plan.head, &plan.result) {
        put_back(&stashed);
        return Err("This session’s apply changed meanwhile. Try again.".into());
    }
    let head_now = read_git(top, &["rev-parse", "HEAD"]);
    if head_now.as_deref() != Some(plan.head.as_str()) {
        put_back(&stashed);
        return Err("The main folder changed while applying. Try again.".into());
    }
    if let Err(failure) = run_git(top, &["read-tree", "-m", "-u", &plan.head, &plan.result]) {
        put_back(&stashed);
        return Err(format!(
            "The main folder changed while applying. Try again.\n{}",
            match failure {
                Failure::Failed(m) => m,
                other => git::explain(Op::Commit, other),
            }
        ));
    }
    let committed = wgit(top, &["commit", "-q", "-F", "-"])
        .input(message.as_bytes().to_vec())
        .timeout(git::COMMIT)
        .holds_index()
        .ok();
    if let Err(failure) = committed {
        let why = git::explain(Op::Commit, failure);
        let back = run_git(top, &["read-tree", "-m", "-u", &plan.result, &plan.head]);
        put_back(&stashed);
        return Err(match back {
            Ok(_) => why,
            Err(_) => format!(
                "The commit hook failed and changed files in the main folder; the session’s \
                 changes are there uncommitted. Commit them yourself or put them back with Git.\n{why}"
            ),
        });
    }
    let commit = read_git(top, &["rev-parse", "HEAD"]).ok_or("Couldn’t read the new commit.")?;
    let left = put_back(&stashed);
    Ok((commit, left))
}

/// A refusal found again at apply time, as a sentence.
fn refusal_sentence(refusal: &Value) -> String {
    let files = |v: &Value| {
        v["files"]
            .as_array()
            .map(|f| {
                f.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    match refusal["reason"].as_str().unwrap_or("") {
        "empty" => "There’s nothing to apply.".into(),
        "busy" => "This session is working. Stop it first.".into(),
        "unresolved" => format!("Resolve the conflicts in {} first.", files(refusal)),
        "conflict" => format!(
            "The changes conflict with {} in {}. Bring it in first.",
            refusal["onto"].as_str().unwrap_or("the main folder"),
            files(refusal)
        ),
        "dirty" => format!(
            "The main folder has edits in {}. Set them aside to apply.",
            files(refusal)
        ),
        "staged" => "The main folder has staged changes; commit or unstage them first.".into(),
        "detached" => "Switch the main folder to a branch to apply.".into(),
        "operation" => format!(
            "Finish the {} in the main folder first.",
            refusal["what"].as_str().unwrap_or("operation")
        ),
        _ => refusal["message"]
            .as_str()
            .unwrap_or("The changes couldn’t be applied.")
            .to_string(),
    }
}

// ---------------------------------------------------------------------------
// Bring the base in, discard, recreate, back to the main folder
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn worktree_bring_base(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let db = app.db()?.clone();
    if setup_running(&session_id) {
        return Err("The setup command is running. Stop it first.".into());
    }
    // Held like an apply: no turn writes the worktree meanwhile.
    if !core
        .set_worktree_state(&session_id, WorktreeState::Ready, WorktreeState::Applying)
        .map_err(plain_error)?
    {
        return Err("This session’s worktree isn’t ready.".into());
    }
    stopped_notice(
        &handle,
        &core.stop_background(&session_id),
        "bringing main in",
    );
    let result = {
        let (core, db, id) = (core.clone(), db.clone(), session_id.clone());
        git::blocking(move || {
            let (conflicts, tip) = bring_base(&row_of(&db, &id)?)?;
            if conflicts.is_empty() {
                // The merge commit exists: the changes are measured from the base's tip now.
                let _ = core.set_worktree_base(&id, WorktreeState::Applying, &tip);
                let _ = core.set_worktree_pending(&id, WorktreeState::Applying, None);
            } else {
                // Not until the merge is committed (by the apply, once resolved).
                let _ = core.set_worktree_pending(&id, WorktreeState::Applying, Some(&tip));
            }
            Ok(conflicts)
        })
        .await
    };
    let _ = core.set_worktree_state(&session_id, WorktreeState::Applying, WorktreeState::Ready);
    emit(&handle, &db, &session_id);
    let conflicts = result?;
    Ok(json!({"conflicts": conflicts}))
}

/// Merges the base branch's tip into the session's branch (pending work committed first).
/// `(conflicts, the tip brought in)`.
fn bring_base(row: &SessionWorktreeRow) -> Result<(Vec<String>, String), String> {
    verify_worktree(row)?;
    let worktree = Path::new(&row.directory);
    let top =
        top_of(Path::new(&row.folder)).ok_or("The main folder isn’t a Git repository anymore.")?;
    let onto = read_git(&top, &["symbolic-ref", "-q", "--short", "HEAD"])
        .filter(|b| !b.is_empty())
        .or(row.base_branch.clone())
        .ok_or("Switch the main folder to a branch first.")?;
    if read_git(worktree, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_some() {
        return Err("A merge is open in this session’s worktree. Resolve it, then apply.".into());
    }
    let pending = read_git(worktree, &["status", "--porcelain"]).unwrap_or_default();
    if !pending.trim().is_empty() {
        run_git(worktree, &["add", "-A", "--", "."]).map_err(|f| git::explain(Op::Commit, f))?;
        identity_env(
            wgit(
                worktree,
                &[
                    "commit",
                    "-q",
                    "--no-verify",
                    "-m",
                    "Session work before bringing the base in",
                ],
            ),
            worktree,
        )
        .timeout(git::COMMIT)
        .ok()
        .map_err(|f| git::explain(Op::Commit, f))?;
    }
    let tip = read_git(&top, &["rev-parse", &format!("refs/heads/{onto}")])
        .ok_or_else(|| format!("The branch {onto} is gone."))?;
    let merged = identity_env(
        wgit(worktree, &["merge", "--no-edit", "--no-verify", &tip]),
        worktree,
    )
    .env("GIT_MERGE_AUTOEDIT", "no")
    .timeout(git::COMMIT)
    .run()
    .map_err(|f| git::explain(Op::Pull, f))?;
    let conflicts: Vec<String> = read_git(worktree, &["diff", "--name-only", "--diff-filter=U"])
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if !merged.status.success() && conflicts.is_empty() {
        return Err(git::explain(
            Op::Pull,
            Failure::Failed(format!("{}{}", merged.stdout, merged.stderr)),
        ));
    }
    Ok((conflicts, tip))
}

#[tauri::command]
pub async fn worktree_discard(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let db = app.db()?.clone();
    // Refuse before stopping setup commands, terminals or language servers.
    if let Ok(Some(w)) = db.session_worktree(&session_id) {
        releasable(&core, &w)?;
    }
    let id = session_id.clone();
    git::blocking(move || {
        cancel_prepare(&id);
        Ok(())
    })
    .await?;
    isolate_done(&session_id).await;
    if let Ok(Some(w)) = db.session_worktree(&session_id)
        && releasable(&core, &w).is_ok()
    {
        let_go(&app.terminals, Some(&core), &w.directory).await;
    }
    let result = core.release_worktree(&session_id, Release::Discarded).await;
    emit(&handle, &db, &session_id);
    result.map_err(plain_error)?;
    Ok(Value::Null)
}

#[tauri::command]
pub async fn worktree_recreate(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    cleaned().await;
    let core = core_for(&handle, &app, &directory).await?;
    let db = app.db()?.clone();
    let before = row_of(&db, &session_id).ok();
    let row = core.recreate_worktree(&session_id).await;
    emit(&handle, &db, &session_id);
    let row = row.map_err(plain_error)?;
    // Made again by git, or committed after all (the base moved to it): nothing was dropped.
    let dropped = before.is_some_and(|b| {
        b.base_pending.is_some() && row.base_pending.is_none() && row.base_commit == b.base_commit
    });
    if dropped {
        // The bring-in that was open is dropped with the folder: say so.
        let _ = handle.emit(
            "core",
            json!({"type": "notice", "message": "Bringing main in wasn’t finished when the folder went, so it was dropped. Bring main in again before applying."}),
        );
    }
    // A folder that came back is used as it is: nothing to prepare.
    if row.state == WorktreeState::Preparing {
        start_made(&handle, &core, &db, &session_id);
    }
    Ok(json(&row))
}

#[tauri::command]
pub async fn worktree_to_folder(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    core.can_unisolate(&session_id).map_err(plain_error)?;
    let id = session_id.clone();
    git::blocking(move || {
        cancel_prepare(&id);
        Ok(())
    })
    .await?;
    isolate_done(&session_id).await;
    if let Ok(db) = app.db()
        && let Ok(Some(w)) = db.session_worktree(&session_id)
        && core.can_unisolate(&session_id).is_ok()
    {
        let_go(&app.terminals, Some(&core), &w.directory).await;
    }
    let row = core.unisolate(&session_id).await.map_err(plain_error)?;
    crate::changed(&handle, &session_id);
    Ok(crate::session_json(&row, None))
}

// ---------------------------------------------------------------------------
// Deleting a session
// ---------------------------------------------------------------------------

/// Files the session changed that aren't on the main folder: its worktree against its base, or
/// (folder gone) its branch against the base. 0 for a released worktree or none.
fn unapplied(row: &SessionWorktreeRow) -> Option<usize> {
    if row.state.released() {
        return Some(0);
    }
    let worktree = Path::new(&row.directory);
    if worktree.is_dir() {
        verify_worktree(row).ok()?;
        let base = diff_base(worktree, row);
        let tree = session_tree(worktree).ok()?;
        return raw_changes(worktree, &base, &tree).ok().map(|c| c.len());
    }
    // The folder is gone: what its branch holds beyond the base.
    let top = top_of(Path::new(&row.folder))?;
    let branch = format!("refs/heads/{}", row.branch);
    if read_git(&top, &["rev-parse", "--verify", "--quiet", &branch]).is_none() {
        return Some(0);
    }
    read_git(&top, &["diff", "--name-only", &row.base_commit, &branch])
        .map(|out| out.lines().filter(|l| !l.is_empty()).count())
}

/// How many changed files deleting the session would lose (the window confirms first).
#[tauri::command]
pub async fn worktree_unapplied(
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let _ = directory;
    let Some(row) = app.db()?.session_worktree(&session_id).map_err(text)? else {
        return Ok(json!(0));
    };
    // null: couldn't tell (the window confirms anyway).
    git::blocking(move || Ok(json!(unapplied(&row)))).await
}

/// Every root session whose worktree holds changes not applied, for "Delete all"'s confirm.
#[tauri::command]
pub async fn worktrees_with_work(app: State<'_, App>) -> Answer {
    let db = app.db()?.clone();
    git::blocking(move || {
        let mut out = Vec::new();
        for row in db.session_worktrees().map_err(text)? {
            let Some(session) = db.session(&row.session_id).map_err(text)? else {
                continue;
            };
            if session.parent_id.is_some() {
                continue;
            }
            let files = unapplied(&row);
            if files != Some(0) {
                out.push(json!({"sessionId": row.session_id, "title": session.title,
                                "directory": row.folder, "files": files}));
            }
        }
        Ok(Value::Array(out))
    })
    .await
}

/// For a rewind to before `message_id`: whether an apply or a merge of the base happened after
/// the first step it undoes, so files go back to before changes already on the main folder. An
/// agent's own commits on the session's branch don't count.
pub(crate) fn crosses_apply(core: &Core, session_id: &str, message_id: &str) -> bool {
    use anysola_core::session::message::Message;
    let Ok(Some(row)) = core.session_worktree(session_id) else {
        return false;
    };
    let worktree = Path::new(&row.directory);
    let Some(base) = &row.base_branch else {
        return false;
    };
    if verify_worktree(&row).is_err() {
        return false;
    }
    let Ok(messages) = core.messages(session_id) else {
        return false;
    };
    let Some(at) = messages.iter().position(|(id, _)| id == message_id) else {
        return false;
    };
    let Some(head) = messages[at..].iter().find_map(|(_, m)| match m {
        Message::Assistant(data) => data.snapshot.as_ref()?.head.clone(),
        _ => None,
    }) else {
        return false;
    };
    // A commit since that step that the base branch has (an apply lands there; a merge brings
    // its commits in).
    let since: std::collections::HashSet<String> =
        read_git(worktree, &["rev-list", &format!("{head}..HEAD")])
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
    if since.is_empty() {
        return false;
    }
    read_git(
        worktree,
        &["rev-list", &format!("{head}..refs/heads/{base}")],
    )
    .unwrap_or_default()
    .lines()
    .any(|c| since.contains(c))
}

/// Whether releasing the worktree would go through: what `release_worktree` refuses (a turn or
/// a subagent running, an apply under way, already released), checked first.
fn releasable(core: &Core, row: &SessionWorktreeRow) -> Result<(), String> {
    core.ensure_idle(&row.session_id).map_err(plain_error)?;
    if row.state.released() || row.state == WorktreeState::Applying {
        return Err("This session’s worktree can’t be let go right now.".into());
    }
    Ok(())
}

/// Before a worktree folder is removed: the embedded terminals in it end (waited for, so
/// Windows lets the folder go) and the language servers running for it stop.
async fn let_go(terminals: &crate::terminal::Manager, core: Option<&Core>, directory: &str) {
    let (manager, folder) = (terminals.clone(), directory.to_string());
    let _ = tauri::async_runtime::spawn_blocking(move || manager.close_in_directory(&folder)).await;
    if let Some(core) = core {
        core.shutdown_lsp_root(Path::new(directory)).await;
    }
}

/// Lets a session's worktree go before the session is deleted: with its core running, as a
/// discard (its background commands stop); refused while an apply runs. The terminals and
/// language servers holding its folder end once nothing will refuse.
pub(crate) async fn before_delete(
    core: Option<&Core>,
    db: &Db,
    terminals: &crate::terminal::Manager,
    session_id: &str,
) -> Result<(), String> {
    let Some(row) = db.session_worktree(session_id).map_err(text)? else {
        return Ok(());
    };
    if row.state == WorktreeState::Applying {
        return Err("This session’s changes are being applied. Try again in a moment.".into());
    }
    if let Some(core) = core
        && !row.state.released()
    {
        releasable(core, &row)?;
    }
    let id = session_id.to_string();
    let _ = git::blocking(move || {
        cancel_prepare(&id);
        Ok(())
    })
    .await;
    isolate_done(session_id).await;
    match core {
        Some(core) if !row.state.released() => {
            let fresh = db
                .session_worktree(session_id)
                .map_err(text)?
                .unwrap_or(row);
            releasable(core, &fresh)?;
            let_go(terminals, Some(core), &fresh.directory).await;
            core.release_worktree(session_id, Release::Discarded)
                .await
                .map_err(plain_error)?;
        }
        // The folder goes with the session's rows (`after_delete`).
        _ => let_go(terminals, core, &row.directory).await,
    }
    Ok(())
}

/// After the session was deleted: its worktree, branch and rows go now.
pub(crate) async fn after_delete(db: &Arc<Db>, session_id: &str) {
    let (db, id) = (db.clone(), session_id.to_string());
    slots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    let _ = tauri::async_runtime::spawn_blocking(move || {
        anysola_core::api::forget_deleted_worktree(&db, &id)
    })
    .await;
}

/// The folder a session's core runs in: its main folder for a worktree session.
pub(crate) fn folder_of(db: &Db, row_directory: &str, session_id: &str) -> String {
    db.session_worktree(session_id)
        .ok()
        .flatten()
        .map(|r| r.folder)
        .unwrap_or_else(|| row_directory.to_string())
}

/// `session_json` for a row, its worktree looked up.
pub(crate) fn session_value(db: &Db, row: &SessionRow) -> Value {
    let worktree = db.session_worktree(&row.id).ok().flatten();
    crate::session_json(row, worktree.as_ref())
}

#[cfg(test)]
mod tests;
