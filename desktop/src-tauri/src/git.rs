//! The session header's git menu: the folder's branch and changes, and the steps from there to
//! a pull request (new branch, commit, pull, push, open the PR). Runs the user's own `git` (and
//! `gh` when installed), so their credentials and hooks apply.
//!
//! Nothing here can wait on a prompt nobody sees: no terminal prompts, SSH in batch mode, and a
//! time limit on every call. Failures come back as a plain sentence saying what to do, with
//! git's own words on the next line.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, core_for, text};

/// Reading the repository.
pub(crate) const LOCAL: Duration = Duration::from_secs(30);
/// A commit runs the repository's hooks, which can take a while.
pub(crate) const COMMIT: Duration = Duration::from_secs(300);
/// Talking to the remote, a credential helper's sign-in included.
pub(crate) const NETWORK: Duration = Duration::from_secs(120);

#[cfg(test)]
pub(crate) static TEST_ENV: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();

#[cfg(test)]
thread_local! {
    /// A global config of the test's own, over `TEST_ENV`'s shared one.
    static TEST_GLOBAL: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// What a finished call printed.
pub(crate) struct Output {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Why a call has no output to show.
#[derive(Debug, PartialEq)]
pub(crate) enum Failure {
    /// The program isn't installed or isn't on PATH.
    Missing,
    /// It ran past its time and was stopped.
    TimedOut(Duration),
    /// It ran past its time and was stopped, and the repository's index is still locked, maybe
    /// by another git that is working in it.
    LockLeft(Duration),
    /// It couldn't be started, or failed: the text says why.
    Failed(String),
}

/// One call to git or gh.
pub(crate) struct Call<'a> {
    program: &'a str,
    directory: &'a Path,
    args: &'a [&'a str],
    input: Option<Vec<u8>>,
    timeout: Duration,
    env: Vec<(&'static str, String)>,
    /// What a time-out says about the repository's index lock.
    lock: Lock,
}

impl<'a> Call<'a> {
    pub(crate) fn new(program: &'a str, directory: &'a Path, args: &'a [&'a str]) -> Self {
        Call {
            program,
            directory,
            args,
            input: None,
            timeout: LOCAL,
            env: Vec::new(),
            lock: if program == "git" {
                Lock::Other
            } else {
                Lock::None
            },
        }
    }

    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn input(mut self, input: Vec<u8>) -> Self {
        self.input = Some(input);
        self
    }

    /// The call takes the index lock and keeps it until it ends (`add`, `commit --only`), so a
    /// lock made while it ran is its own.
    pub(crate) fn holds_index(mut self) -> Self {
        self.lock = Lock::Own;
        self
    }

    pub(crate) fn env(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.env.push((key, value.into()));
        self
    }

    /// Runs it to the end or to its time limit. A helper the program left behind may keep its
    /// pipes open: the output is taken as far as it came, never waited for past the exit.
    pub(crate) fn run(self) -> Result<Output, Failure> {
        let mut command =
            anysola_core::console::command(anysola_core::which::program(self.program));
        command
            .args(self.args)
            .current_dir(self.directory)
            // No prompt on a terminal nobody sees: a push that needs one fails instead.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            // Untranslated messages, which `explain` reads.
            .env("LANGUAGE", "en")
            .stdin(if self.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &self.env {
            command.env(key, value);
        }
        #[cfg(test)]
        if let Some(env) = TEST_ENV.get() {
            command.envs(env.iter().map(|(k, v)| (k, v)));
        }
        #[cfg(test)]
        TEST_GLOBAL.with(|global| {
            if let Some(global) = global.borrow().as_ref() {
                command.env("GIT_CONFIG_GLOBAL", global);
            }
        });
        // Its hooks, ssh and credential helpers go with it when it is stopped.
        crate::tree::own_group(&mut command);
        let started = std::time::SystemTime::now();
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Failure::Missing
            } else {
                Failure::Failed(format!("Couldn’t run {}: {e}", self.program))
            }
        })?;
        if let (Some(input), Some(mut stdin)) = (self.input, child.stdin.take()) {
            std::thread::spawn(move || {
                let _ = stdin.write_all(&input);
            });
        }
        let stdout = drain(child.stdout.take());
        let stderr = drain(child.stderr.take());
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(15));
                }
                Ok(None) => {
                    crate::tree::kill(&mut child);
                    if self.lock != Lock::None && locked(self.directory, self.lock, started) {
                        return Err(Failure::LockLeft(self.timeout));
                    }
                    return Err(Failure::TimedOut(self.timeout));
                }
                Err(e) => return Err(Failure::Failed(e.to_string())),
            }
        };
        Ok(Output {
            status,
            stdout: stdout.take(),
            stderr: stderr.take(),
        })
    }

    /// stdout on success; on failure the program's own message (stderr, else stdout).
    pub(crate) fn ok(self) -> Result<String, Failure> {
        let out = self.run()?;
        if out.status.success() {
            return Ok(out.stdout.trim().to_string());
        }
        let message = match out.stderr.trim() {
            "" => out.stdout.trim(),
            err => err,
        };
        Err(Failure::Failed(message.to_string()))
    }
}

/// A pipe read on a thread of its own, kept as it comes.
struct Drained {
    bytes: Arc<Mutex<Vec<u8>>>,
    ended: mpsc::Receiver<()>,
}

impl Drained {
    /// What came, once the pipe ends or after a short wait for it.
    fn take(self) -> String {
        let _ = self.ended.recv_timeout(Duration::from_secs(2));
        let bytes = self.bytes.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Drained {
    let (sender, ended) = mpsc::channel();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    match pipe {
        Some(mut pipe) => {
            let kept = bytes.clone();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                while let Ok(n) = pipe.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    kept.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend_from_slice(&buffer[..n]);
                }
                let _ = sender.send(());
            });
        }
        None => {
            let _ = sender.send(());
        }
    }
    Drained { bytes, ended }
}

pub(crate) fn git<'a>(directory: &'a Path, args: &'a [&'a str]) -> Call<'a> {
    Call::new("git", directory, args)
}

/// How a call relates to the repository's index lock.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Lock {
    /// Not a git call.
    None,
    /// A git call that may lock the index only briefly: a lock found after it was stopped can
    /// be another git's, held across its hooks.
    Other,
    /// A git call that holds the lock until it ends: had another git held it, this one would
    /// have failed at once instead of running out of time.
    Own,
}

/// After a git call that was stopped: whether the index is still locked. A lock the stopped
/// call held (made while it ran) is removed, since nothing is left to finish it and every later
/// call would fail on it; any other is left to the git that holds it.
fn locked(directory: &Path, lock: Lock, started: std::time::SystemTime) -> bool {
    let mut find =
        git(directory, &["rev-parse", "--git-path", "index.lock"]).timeout(Duration::from_secs(5));
    find.lock = Lock::None;
    let Ok(path) = find.ok() else {
        return false;
    };
    // Relative to the folder, or absolute when the repository's data lives elsewhere.
    let path = directory.join(path);
    let Ok(found) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    // A second of slack for file systems that keep coarse times.
    let since = started
        .checked_sub(Duration::from_secs(1))
        .unwrap_or(started);
    let ours = lock == Lock::Own && found.modified().is_ok_and(|made| made >= since);
    !(ours && std::fs::remove_file(&path).is_ok())
}

/// What the user was doing, for the headline when git's message says nothing more specific.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    Status,
    Branch,
    Commit,
    Push,
    Pull,
    Identity,
    Pr,
}

/// The sentence for a failure, with the program's own words on the next line.
pub(crate) fn explain(op: Op, failure: Failure) -> String {
    let detail = match failure {
        Failure::Missing if op == Op::Pr => {
            return "GitHub CLI isn’t installed, or the app can’t find it.".into();
        }
        Failure::Missing => {
            return "Git isn’t installed, or the app can’t find it. Install Git, then try again."
                .into();
        }
        Failure::TimedOut(limit) => {
            let wait = wait(limit);
            return format!(
                "Git didn’t finish within {wait}, so it was stopped. It may have been waiting \
                 for a sign-in the app can’t show. Try again, or check your Git credentials."
            );
        }
        Failure::LockLeft(limit) => {
            return format!(
                "Git didn’t finish within {}, so it was stopped. Another Git program is still \
                 working in this repository; trying again in a moment usually works.",
                wait(limit)
            );
        }
        Failure::Failed(detail) => detail,
    };
    let headline = headline(op, &detail.to_lowercase());
    let detail = tail(&detail);
    if detail.is_empty() {
        headline.to_string()
    } else {
        format!("{headline}\n{detail}")
    }
}

/// A time limit in words.
fn wait(limit: Duration) -> String {
    if limit.as_secs() >= 120 {
        format!("{} minutes", limit.as_secs() / 60)
    } else {
        format!("{} seconds", limit.as_secs())
    }
}

/// git's message, cut to its last lines.
fn tail(detail: &str) -> String {
    let lines: Vec<&str> = detail
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let kept = lines[lines.len().saturating_sub(8)..].join("\n");
    match kept.char_indices().nth(800) {
        Some((at, _)) => format!("{}…", &kept[..at]),
        None => kept,
    }
}

fn headline(op: Op, detail: &str) -> &'static str {
    let has = |needles: &[&str]| needles.iter().any(|n| detail.contains(n));
    if has(&[
        "please tell me who you are",
        "unable to auto-detect email",
        "empty ident name",
        "user.useconfigonly",
    ]) {
        return "Git needs your name and email before it can commit. Add them, then commit again.";
    }
    if has(&[
        "gpg failed to sign",
        "error: gpg",
        "failed to sign the data",
    ]) || (detail.contains("sign") && detail.contains("ssh-keygen"))
    {
        return "Git couldn’t sign the commit. Commit signing is on for this repository: check \
                your signing key, or turn signing off.";
    }
    if has(&["detected dubious ownership"]) {
        return "Git won’t work in this folder because another user account owns it. Mark the \
                folder as safe in your Git settings, or open a folder you own.";
    }
    if has(&["index.lock"]) {
        return "Another Git program is working in this repository. Wait for it to finish, then \
                try again.";
    }
    if has(&["host key verification failed"]) {
        return "This computer doesn’t know the remote’s SSH host yet, so Git can’t connect \
                without asking you. Use the remote’s HTTPS address, or trust the host once in \
                your SSH settings.";
    }
    if has(&[
        "permission denied (publickey",
        "no supported authentication methods",
    ]) {
        return "The remote didn’t accept your SSH key. If the key has a passphrase, add it to \
                your SSH agent, or use the remote’s HTTPS address.";
    }
    if has(&[
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "invalid username or password",
        "invalid credentials",
        "returned error: 401",
        "returned error: 403",
    ]) {
        return "The remote didn’t accept your sign-in. Check that your Git credentials are set \
                up and that you have access to this repository.";
    }
    if has(&[
        "repository not found",
        "does not appear to be a git repository",
    ]) {
        return "The remote repository wasn’t found, or your account can’t see it.";
    }
    if has(&[
        "could not resolve host",
        "unable to access",
        "connection timed out",
        "failed to connect",
        "network is unreachable",
        "connection refused",
        "could not read from remote repository",
    ]) {
        return "Can’t reach the remote. Check your connection, then try again.";
    }
    if has(&[
        "would be overwritten by merge",
        "would be overwritten by checkout",
        "your local changes to the following files would be overwritten",
        "untracked working tree files would be overwritten",
    ]) {
        return "Pulling would overwrite changes you haven’t committed. Commit them first, then \
                pull.";
    }
    if has(&[
        "not possible to fast-forward",
        "diverging branches",
        "have diverged",
    ]) {
        return "Your branch and the remote both have new commits, so Pull can’t just bring \
                yours up to date. Merge or rebase them (the agent can do it), then pull again.";
    }
    if has(&[
        "non-fast-forward",
        "fetch first",
        "updates were rejected",
        "[rejected]",
    ]) {
        return "The remote has commits you don’t have. Pull first.";
    }
    if has(&[
        "you have unmerged paths",
        "unmerged files",
        "fix conflicts",
        "conflict",
    ]) {
        return "Resolve the conflicts first.";
    }
    if has(&[
        "you are not currently on a branch",
        "head detached",
        "detached head",
    ]) {
        return "You’re not on a branch. Create a branch first, then try again.";
    }
    if has(&[
        "does not have any commits yet",
        "ambiguous argument 'head'",
        "bad default revision 'head'",
        "src refspec",
    ]) {
        return "This repository has no commits yet. Make the first commit, then try again.";
    }
    if has(&[
        "has no upstream branch",
        "no tracking information",
        "no upstream configured",
    ]) {
        return match op {
            Op::Pull => {
                "This branch isn’t on the remote yet, so there’s nothing to pull. Push it first."
            }
            _ => "This branch isn’t on the remote yet. Push it first.",
        };
    }
    if has(&["no configured push destination", "no such remote"]) {
        return "This repository isn’t connected to a remote, so there’s nowhere to push.";
    }
    if has(&["nothing to commit", "no changes added to commit"]) {
        return "There’s nothing to commit.";
    }
    if has(&["not a git repository"]) {
        return "This folder isn’t a Git repository.";
    }
    match op {
        Op::Status => "Couldn’t read the repository’s state.",
        Op::Branch => "The branch wasn’t created.",
        Op::Commit => "The commit wasn’t made.",
        Op::Push => "The push didn’t go through.",
        Op::Pull => "The pull didn’t go through.",
        Op::Identity => "Your name and email weren’t saved.",
        Op::Pr => "The pull request wasn’t opened.",
    }
}

pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(text)?
}

/// `## main...origin/main [ahead 1, behind 2]` from `git status -b`.
fn parse_branch(line: &str) -> (String, Option<String>, u64, u64) {
    let line = line.trim_start_matches("## ");
    let (head, counts) = match line.split_once(" [") {
        Some((head, counts)) => (head, counts.trim_end_matches(']')),
        None => (line, ""),
    };
    let (branch, upstream) = match head.split_once("...") {
        Some((branch, upstream)) => (branch.to_string(), Some(upstream.to_string())),
        None => (head.to_string(), None),
    };
    let branch = branch
        .strip_prefix("No commits yet on ")
        .unwrap_or(&branch)
        .to_string();
    let count = |key: &str| {
        counts
            .split(", ")
            .find_map(|part| part.strip_prefix(key))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0)
    };
    (branch, upstream, count("ahead "), count("behind "))
}

/// One changed path of `git status`: relative to the repository's top.
#[derive(Debug, PartialEq)]
pub(crate) struct Entry {
    pub(crate) file: String,
    status: &'static str,
    /// A rename's or copy's old path.
    pub(crate) from: Option<String>,
    /// Both sides changed it: a merge conflict.
    unmerged: bool,
    /// Not in the repository yet (`??`).
    untracked: bool,
}

/// `git status --porcelain=v1 -b -z` as the branch line and its entries.
fn parse_status(raw: &str) -> (Option<String>, Vec<Entry>) {
    let mut parts = raw.split('\0').filter(|p| !p.is_empty());
    let mut branch = None;
    let mut entries = Vec::new();
    while let Some(part) = parts.next() {
        if let Some(line) = part.strip_prefix("## ") {
            branch = Some(line.to_string());
            continue;
        }
        if part.len() < 4 || !part.is_char_boundary(3) {
            continue;
        }
        let (code, file) = part.split_at(3);
        let unmerged = matches!(&code[..2], "UU" | "AA" | "DD" | "AU" | "UA" | "DU" | "UD");
        let code = code.trim();
        // A rename's old path follows as its own entry.
        let from = (code.starts_with('R') || code.starts_with('C'))
            .then(|| parts.next().map(str::to_string))
            .flatten();
        let status = match code {
            "??" => "added",
            c if c.contains('D') && !unmerged => "deleted",
            c if c.contains('A') && !unmerged => "added",
            _ => "modified",
        };
        entries.push(Entry {
            file: file.to_string(),
            status,
            from,
            unmerged,
            untracked: code == "??",
        });
    }
    (branch, entries)
}

pub(crate) fn status_entries(directory: &Path) -> Result<(Option<String>, Vec<Entry>), Failure> {
    let raw = git(
        directory,
        &[
            "status",
            "--porcelain=v1",
            "-b",
            "-z",
            "--untracked-files=all",
        ],
    )
    .ok()?;
    Ok(parse_status(&raw))
}

/// The remote a branch goes to: its own, the push default, `origin`, or the only one there is.
fn remote_for(directory: &Path, branch: Option<&str>) -> Option<String> {
    let config = |key: &str| {
        git(directory, &["config", "--get", key])
            .ok()
            .ok()
            .filter(|v| !v.is_empty() && v != ".")
    };
    if let Some(remote) = branch.and_then(|b| config(&format!("branch.{b}.remote"))) {
        return Some(remote);
    }
    if let Some(remote) = config("remote.pushDefault") {
        return Some(remote);
    }
    let remotes = git(directory, &["remote"]).ok().ok()?;
    pick_remote(remotes.lines().map(str::trim).filter(|r| !r.is_empty()))
}

fn pick_remote<'a>(remotes: impl Iterator<Item = &'a str>) -> Option<String> {
    let remotes: Vec<&str> = remotes.collect();
    if remotes.contains(&"origin") {
        return Some("origin".into());
    }
    match remotes.as_slice() {
        [only] => Some(only.to_string()),
        _ => None,
    }
}

/// `user.name` and `user.email` are both set.
pub(crate) fn has_identity(directory: &Path) -> bool {
    let Ok(found) = git(
        directory,
        &["config", "--get-regexp", r"^user\.(name|email)$"],
    )
    .ok() else {
        return false;
    };
    let set = |key: &str| {
        found.lines().any(|line| {
            line.split_once(' ')
                .is_some_and(|(k, v)| k.eq_ignore_ascii_case(key) && !v.trim().is_empty())
        })
    };
    set("user.name") && set("user.email")
}

fn gh_installed(directory: &Path) -> bool {
    Call::new("gh", directory, &["--version"])
        .timeout(Duration::from_secs(10))
        .ok()
        .is_ok()
}

/// Conflict markers are still in the text: `<<<<<<<` and `>>>>>>>` lines.
pub(crate) fn has_markers(text: &str) -> bool {
    let mut lines = text.lines();
    lines.any(|l| l.starts_with("<<<<<<<")) && lines.any(|l| l.starts_with(">>>>>>>"))
}

/// A conflicted file the user hasn't fixed yet. Once its markers are gone (or it was deleted)
/// it counts as resolved, and committing stages it: there's no `git add` to run in the app.
fn still_conflicted(top: &Path, entry: &Entry) -> bool {
    entry.unmerged
        && std::fs::read(top.join(&entry.file))
            .is_ok_and(|bytes| has_markers(&String::from_utf8_lossy(&bytes)))
}

/// A merge (or cherry-pick, or revert) is waiting for its commit.
fn finishing(directory: &Path) -> bool {
    ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"]
        .iter()
        .any(|head| {
            git(directory, &["rev-parse", "-q", "--verify", head])
                .ok()
                .is_ok()
        })
}

fn status(directory: &Path) -> Result<Value, String> {
    match git(directory, &["rev-parse", "--is-inside-work-tree"]).ok() {
        Err(Failure::Missing) => return Ok(json!({"repo": false, "git": "missing"})),
        Ok(inside) if inside == "true" => {}
        _ => return Ok(json!({"repo": false})),
    }
    let (line, entries) = status_entries(directory).map_err(|f| explain(Op::Status, f))?;
    let (branch, upstream, ahead, behind) = parse_branch(line.as_deref().unwrap_or(""));
    let detached = branch.starts_with("HEAD");
    let remote = remote_for(directory, (!detached).then_some(branch.as_str()));
    // Paths in the answer are relative to the repository's top, which a session's folder may sit below.
    let top = top(directory, Op::Status)?;
    let (unmerged, files): (Vec<_>, Vec<_>) =
        entries.into_iter().partition(|e| still_conflicted(&top, e));
    // A resolved conflict shows as the change it now is.
    let files: Vec<Value> = files
        .iter()
        .map(|e| {
            let status = match e.status {
                _ if e.unmerged && !top.join(&e.file).exists() => "deleted",
                status => status,
            };
            json!({"file": e.file, "status": status})
        })
        .collect();
    Ok(json!({
        "repo": true,
        "root": top.to_string_lossy(),
        "branch": branch,
        "detached": detached,
        "upstream": upstream,
        "ahead": ahead,
        "behind": behind,
        "files": files,
        "unmerged": unmerged.iter().map(|e| e.file.clone()).collect::<Vec<_>>(),
        "identity": has_identity(directory),
        "remote": remote,
        "gh": gh_installed(directory),
    }))
}

/// The folder's branch, how far it is from its upstream, its uncommitted files and conflicts,
/// whether a commit can be made and whether a pull request can be opened with `gh`.
/// `{"repo": false}` outside git, with `"git": "missing"` when git can't be run at all.
#[tauri::command]
pub async fn git_status(directory: String) -> Answer {
    blocking(move || status(Path::new(&directory))).await
}

/// A new branch from here, taking the uncommitted changes along.
#[tauri::command]
pub async fn git_branch(directory: String, name: String) -> Answer {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("A branch needs a name.".into());
    }
    blocking(move || {
        let directory = Path::new(&directory);
        git(directory, &["check-ref-format", "--branch", &name])
            .ok()
            .map_err(|_| format!("“{name}” isn’t a valid branch name."))?;
        git(directory, &["switch", "-c", &name])
            .ok()
            .map_err(|f| explain(Op::Branch, f))?;
        Ok(Value::Null)
    })
    .await
}

fn top(directory: &Path, op: Op) -> Result<std::path::PathBuf, String> {
    let top = git(directory, &["rev-parse", "--show-toplevel"])
        .ok()
        .map_err(|f| explain(op, f))?;
    Ok(std::path::PathBuf::from(top))
}

/// The paths to stage for a commit of `chosen`: each, and the old path of a chosen rename.
fn with_origins(chosen: &[String], entries: &[Entry]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for file in chosen {
        let file = file.trim();
        if file.is_empty() || paths.iter().any(|p| p == file) {
            continue;
        }
        paths.push(file.to_string());
        if let Some(from) = entries
            .iter()
            .find(|e| e.file == file)
            .and_then(|e| e.from.as_ref())
            && !paths.contains(from)
        {
            paths.push(from.clone());
        }
    }
    paths
}

/// A NUL-separated pathspec list (`--pathspec-file-nul`), each path taken as it is.
fn literal<'p>(paths: impl IntoIterator<Item = &'p String>) -> Vec<u8> {
    paths
        .into_iter()
        .map(|p| format!(":(literal){p}"))
        .collect::<Vec<_>>()
        .join("\0")
        .into_bytes()
}

fn conflicts(count: usize) -> String {
    match count {
        1 => "Resolve the conflicts in 1 file first.".into(),
        n => format!("Resolve the conflicts in {n} files first."),
    }
}

/// Commits `files` (paths from `git_status`; deletions and new files included), or every
/// change when there's no list. Hooks run as usual.
fn commit(directory: &Path, message: &str, files: Option<Vec<String>>) -> Result<String, String> {
    let top = top(directory, Op::Commit)?;
    let (_, entries) = status_entries(&top).map_err(|f| explain(Op::Commit, f))?;
    // `git add` would mark a conflicted file resolved and commit its markers.
    let unmerged = entries.iter().filter(|e| still_conflicted(&top, e)).count();
    if unmerged > 0 {
        return Err(conflicts(unmerged));
    }
    // Git finishes a merge with every change to tracked files; untracked ones are the user's
    // to add or not.
    let merge = files.is_some() && finishing(&top);
    if let Some(chosen) = &files
        && merge
        && entries
            .iter()
            .any(|e| !e.untracked && !chosen.contains(&e.file))
    {
        return Err(
            "A merge is being finished: commit every changed file to finish it, not just some."
                .into(),
        );
    }
    match files {
        None => {
            git(&top, &["add", "-A"])
                .ok()
                .map_err(|f| explain(Op::Commit, f))?;
            git(&top, &["commit", "-q", "-m", message])
                .timeout(COMMIT)
                .ok()
                .map_err(|f| explain(Op::Commit, f))?;
        }
        Some(files) => {
            let paths = with_origins(&files, &entries);
            if paths.is_empty() {
                return Err("Choose at least one file to commit.".into());
            }
            // Lists go on stdin, so none is too long for a command line, each path marked to be
            // taken literally (not through the environment, which the hooks would inherit).
            // Mid-merge git takes no partial commit: the chosen files are staged, deletions
            // included, and the index committed. Otherwise only what is on disk is added (new
            // files must be, to be committed); `--only` takes deletions from the working tree.
            let staged: Vec<&String> = paths
                .iter()
                .filter(|p| merge || top.join(p).symlink_metadata().is_ok())
                .collect();
            if !staged.is_empty() {
                git(
                    &top,
                    &["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"],
                )
                .input(literal(staged))
                .holds_index()
                .ok()
                .map_err(|f| explain(Op::Commit, f))?;
            }
            let commit: &[&str] = if merge {
                &["commit", "-q", "-m", message]
            } else {
                &[
                    "commit",
                    "-q",
                    "-m",
                    message,
                    "--only",
                    "--pathspec-from-file=-",
                    "--pathspec-file-nul",
                ]
            };
            let mut call = git(&top, commit).timeout(COMMIT);
            if !merge {
                call = call.input(literal(paths.iter())).holds_index();
            }
            call.ok().map_err(|f| explain(Op::Commit, f))?;
        }
    }
    git(&top, &["rev-parse", "--short", "HEAD"])
        .ok()
        .map_err(|f| explain(Op::Commit, f))
}

/// Commits the chosen files, or every change in the folder when `files` is null.
#[tauri::command]
pub async fn git_commit(directory: String, message: String, files: Option<Vec<String>>) -> Answer {
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("A commit needs a message.".into());
    }
    blocking(move || {
        let head = commit(Path::new(&directory), &message, files)?;
        Ok(json!({"commit": head}))
    })
    .await
}

/// The ssh command a network call uses: whatever the user set, in batch mode, so a host key or
/// passphrase prompt fails at once instead of waiting on a terminal that isn't there.
fn ssh_command(directory: &Path) -> Option<String> {
    // `GIT_SSH` names a program, not a command line: nothing can be added to it.
    if std::env::var_os("GIT_SSH").is_some_and(|v| !v.is_empty()) {
        return None;
    }
    let set = std::env::var("GIT_SSH_COMMAND")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            git(directory, &["config", "--get", "core.sshCommand"])
                .ok()
                .ok()
                .filter(|v| !v.is_empty())
        });
    batch(set.as_deref())
}

/// `None` leaves the command as the user has it: PuTTY's plink takes no `-o`.
fn batch(command: Option<&str>) -> Option<String> {
    match command {
        Some(command) if command.to_lowercase().contains("plink") => None,
        Some(command) if command.contains("BatchMode") => Some(command.to_string()),
        Some(command) => Some(format!("{command} -o BatchMode=yes")),
        None => Some("ssh -o BatchMode=yes".into()),
    }
}

fn network<'a>(directory: &'a Path, args: &'a [&'a str]) -> Call<'a> {
    let call = git(directory, args).timeout(NETWORK);
    match ssh_command(directory) {
        Some(ssh) => call.env("GIT_SSH_COMMAND", ssh),
        None => call,
    }
}

fn current_branch(directory: &Path, op: Op) -> Result<String, String> {
    let branch = git(directory, &["branch", "--show-current"])
        .ok()
        .map_err(|f| explain(op, f))?;
    if branch.is_empty() {
        return Err("You’re not on a branch. Create a branch first, then try again.".into());
    }
    Ok(branch)
}

fn has_upstream(directory: &Path) -> bool {
    git(
        directory,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok()
    .is_ok()
}

fn push(directory: &Path) -> Result<(), String> {
    let branch = current_branch(directory, Op::Push)?;
    if has_upstream(directory) {
        network(directory, &["push", "-q"])
            .ok()
            .map_err(|f| explain(Op::Push, f))?;
        return Ok(());
    }
    let remote = remote_for(directory, Some(&branch)).ok_or(
        "This repository isn’t connected to a remote (or has several and none is named origin), \
         so there’s nowhere to push.",
    )?;
    network(directory, &["push", "-q", "-u", &remote, "HEAD"])
        .ok()
        .map_err(|f| explain(Op::Push, f))?;
    Ok(())
}

/// Pushes the branch, setting its upstream the first time.
#[tauri::command]
pub async fn git_push(directory: String) -> Answer {
    blocking(move || push(Path::new(&directory)).map(|()| Value::Null)).await
}

fn pull(directory: &Path) -> Result<(), String> {
    current_branch(directory, Op::Pull)?;
    if !has_upstream(directory) {
        return Err(
            "This branch isn’t on the remote yet, so there’s nothing to pull. Push it first."
                .into(),
        );
    }
    network(directory, &["pull", "--ff-only", "--no-rebase", "-q"])
        .ok()
        .map_err(|f| explain(Op::Pull, f))?;
    Ok(())
}

/// Brings the branch up to its upstream when that is a fast-forward, or says why it isn't.
#[tauri::command]
pub async fn git_pull(directory: String) -> Answer {
    blocking(move || pull(Path::new(&directory)).map(|()| Value::Null)).await
}

fn identity(home: &Path, name: &str, email: &str) -> Result<(), String> {
    let (name, email) = (name.trim(), email.trim());
    if name.is_empty() || name.contains(['\n', '\r', '<', '>']) {
        return Err("Enter your name.".into());
    }
    let valid = email
        .split_once('@')
        .is_some_and(|(user, host)| !user.is_empty() && !host.is_empty())
        && !email.contains(char::is_whitespace)
        && !email.contains(['<', '>']);
    if !valid {
        return Err("Enter an email address, such as you@example.com.".into());
    }
    for (key, value) in [("user.name", name), ("user.email", email)] {
        git(home, &["config", "--global", key, value])
            .ok()
            .map_err(|f| explain(Op::Identity, f))?;
    }
    Ok(())
}

/// Sets the name and email commits are made with, for every repository of this user.
#[tauri::command]
pub async fn git_identity(app: State<'_, App>, name: String, email: String) -> Answer {
    let home = app.paths.home.clone();
    blocking(move || identity(&home, &name, &email).map(|()| Value::Null)).await
}

/// Who serves a repository, for the address of its "new pull request" page.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Host {
    /// GitHub, and hosts that take its compare address (GitHub Enterprise, Gitea, Forgejo).
    GitHub,
    GitLab,
    Bitbucket,
    Azure,
}

fn host_of(base: &str) -> Host {
    let host = base
        .split_once("://")
        .map_or(base, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if host.contains("gitlab") {
        Host::GitLab
    } else if host == "bitbucket.org" || host.ends_with(".bitbucket.org") {
        Host::Bitbucket
    } else if host == "dev.azure.com" || host.ends_with(".visualstudio.com") {
        Host::Azure
    } else {
        Host::GitHub
    }
}

/// Strips `user@` and, for ssh, `:port` from an authority.
fn host_part(authority: &str, keep_port: bool) -> &str {
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if keep_port || host.starts_with('[') {
        return host;
    }
    host.split_once(':').map_or(host, |(h, _)| h)
}

/// The repository's web address from a remote URL: `git@host:o/r.git`, `ssh://user@host:2222/o/r`,
/// `https://token@host/o/r.git`, `http://host:8080/o/r`, Azure's ssh and https forms.
fn web_url(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/');
    let remote = remote.strip_suffix(".git").unwrap_or(remote);
    let (scheme, host, path) = if let Some((scheme, rest)) = remote.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        match scheme.to_lowercase().as_str() {
            "https" | "http" => (scheme.to_lowercase(), host_part(authority, true), path),
            "ssh" | "git+ssh" | "ssh+git" | "git" => {
                ("https".into(), host_part(authority, false), path)
            }
            _ => return None,
        }
    } else {
        // scp-like `[user@]host:path`; never a Windows drive (`C:\repo`) or a local path.
        let (authority, path) = remote.split_once(':')?;
        if authority.contains(['/', '\\']) || host_part(authority, false).len() < 2 {
            return None;
        }
        ("https".into(), host_part(authority, false), path)
    };
    let path = path.trim_start_matches('/');
    if host.is_empty() || path.is_empty() {
        return None;
    }
    let lower = host.to_lowercase();
    // Azure's ssh remotes: `ssh.dev.azure.com:v3/org/project/repo`.
    if (lower == "ssh.dev.azure.com" || lower == "vs-ssh.visualstudio.com")
        && let Some(rest) = path.strip_prefix("v3/")
    {
        let parts: Vec<&str> = rest.splitn(3, '/').collect();
        if let [org, project, repo] = parts.as_slice() {
            return Some(if lower == "ssh.dev.azure.com" {
                format!("https://dev.azure.com/{org}/{project}/_git/{repo}")
            } else {
                format!("https://{org}.visualstudio.com/{project}/_git/{repo}")
            });
        }
        return None;
    }
    Some(format!("{scheme}://{host}/{path}"))
}

/// `encodeURIComponent`.
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A body longer than this is left out of the address: browsers and hosts cut long ones.
const BODY_IN_URL: usize = 2000;

/// The host's "new pull request" page for `branch`, filled in where the host takes it.
fn compare_url(base: &str, branch: &str, title: &str, body: &str) -> String {
    let title = title.trim();
    let body = body.trim();
    let body = (body.chars().count() <= BODY_IN_URL).then_some(body);
    let mut query = Vec::new();
    let url = match host_of(base) {
        Host::GitHub => {
            let path: Vec<String> = branch.split('/').map(encode).collect();
            query.push("expand=1".to_string());
            if !title.is_empty() {
                query.push(format!("title={}", encode(title)));
            }
            if let Some(body) = body.filter(|b| !b.is_empty()) {
                query.push(format!("body={}", encode(body)));
            }
            format!("{base}/compare/{}", path.join("/"))
        }
        Host::GitLab => {
            query.push(format!("merge_request[source_branch]={}", encode(branch)));
            if !title.is_empty() {
                query.push(format!("merge_request[title]={}", encode(title)));
            }
            if let Some(body) = body.filter(|b| !b.is_empty()) {
                query.push(format!("merge_request[description]={}", encode(body)));
            }
            format!("{base}/-/merge_requests/new")
        }
        Host::Bitbucket => {
            query.push(format!("source={}", encode(branch)));
            format!("{base}/pull-requests/new")
        }
        Host::Azure => {
            query.push(format!("sourceRef={}", encode(branch)));
            format!("{base}/pullrequestcreate")
        }
    };
    format!("{url}?{}", query.join("&"))
}

/// gh's "a pull request for branch … already exists:" message ends with that pull request.
fn existing_pr(error: &str) -> Option<String> {
    if !error.to_lowercase().contains("already exists") {
        return None;
    }
    error
        .split_whitespace()
        .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        .map(str::to_string)
}

/// gh can't open it here, but the host's page can: signed out of gh, or a host gh doesn't serve.
fn gh_cannot_serve(error: &str) -> bool {
    let error = error.to_lowercase();
    [
        "gh auth login",
        "not logged in",
        "authentication",
        "http 401",
        "bad credentials",
        "none of the git remotes",
        "not a github",
        "could not determine",
        "no git remotes",
        "unknown host",
    ]
    .iter()
    .any(|n| error.contains(n))
}

fn pr_error(error: &str) -> String {
    let lower = error.to_lowercase();
    let headline = if lower.contains("must first push") || lower.contains("push the current branch")
    {
        "Push the branch first, then open the pull request."
    } else if lower.contains("no commits between") {
        "This branch has no commits its base branch doesn’t have, so there’s nothing to open a \
         pull request for."
    } else {
        "The pull request wasn’t opened."
    };
    match tail(error).as_str() {
        "" => headline.into(),
        detail => format!("{headline}\n{detail}"),
    }
}

/// The pull request's address and whether it was created here, or the page to open it on.
fn open_pr(directory: &Path, title: &str, body: &str) -> Result<(String, bool), String> {
    let mut gh_error = None;
    if gh_installed(directory) {
        match Call::new(
            "gh",
            directory,
            &["pr", "create", "--title", title.trim(), "--body", body],
        )
        .timeout(NETWORK)
        .ok()
        {
            Ok(out) => {
                let url = out
                    .lines()
                    .rev()
                    .find(|l| l.trim_start().starts_with("http"))
                    .unwrap_or_else(|| out.lines().last().unwrap_or_default())
                    .trim()
                    .to_string();
                return Ok((url, true));
            }
            Err(Failure::Failed(error)) => {
                if let Some(url) = existing_pr(&error) {
                    return Ok((url, false));
                }
                if !gh_cannot_serve(&error) {
                    return Err(pr_error(&error));
                }
                gh_error = Some(error);
            }
            Err(failure) => return Err(explain(Op::Pr, failure)),
        }
    }
    let branch = current_branch(directory, Op::Pr)?;
    let remote = remote_for(directory, Some(&branch));
    let compare = remote
        .and_then(|remote| git(directory, &["remote", "get-url", &remote]).ok().ok())
        .and_then(|url| web_url(&url))
        .map(|base| compare_url(&base, &branch, title, body));
    match (compare, gh_error) {
        (Some(url), _) => Ok((url, false)),
        (None, Some(error)) => Err(pr_error(&error)),
        (None, None) => Err(
            "Can’t tell this repository’s web address, so the pull request page can’t open.".into(),
        ),
    }
}

/// Opens a pull request for the pushed branch: with `gh` when installed (its URL comes back),
/// else the host's page for a new one in the browser (as for one that already exists).
#[tauri::command]
pub async fn git_pr(handle: AppHandle, directory: String, title: String, body: String) -> Answer {
    let (url, created) = blocking(move || open_pr(Path::new(&directory), &title, &body)).await?;
    if !created {
        use tauri_plugin_opener::OpenerExt;
        handle.opener().open_url(&url, None::<&str>).map_err(text)?;
    }
    Ok(json!({"url": url, "created": created}))
}

/// A commit message for the session's changes, from the session itself (a one-off answer that
/// adds nothing to the session).
#[tauri::command]
pub async fn suggest_commit_message(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let answer = core
        .aside(
            &session_id,
            "Write a git commit message for the changes made in this session: a subject line \
             under 72 characters in the imperative mood, then, only if it helps, a blank line \
             and a short body. Reply with the message only, no code fences.",
            &anysola_core::api::CancellationToken::new(),
        )
        .await
        .map_err(|e| e.message)?;
    Ok(json!(answer.trim().trim_matches('`').trim()))
}

/// Opens the folder in the system's file manager.
#[tauri::command]
pub async fn reveal(handle: AppHandle, directory: String) -> Answer {
    use tauri_plugin_opener::OpenerExt;
    if !Path::new(&directory).is_dir() {
        return Err(format!("{directory} is no longer on this computer."));
    }
    handle
        .opener()
        .open_path(&directory, None::<&str>)
        .map_err(text)?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_lines() {
        assert_eq!(
            parse_branch("## main...origin/main [ahead 2, behind 1]"),
            ("main".into(), Some("origin/main".into()), 2, 1)
        );
        assert_eq!(parse_branch("## feature"), ("feature".into(), None, 0, 0));
        assert_eq!(
            parse_branch("## No commits yet on main"),
            ("main".into(), None, 0, 0)
        );
    }

    #[test]
    fn status_entries_parse() {
        let raw = "## main\0 M src/a.rs\0?? new.txt\0R  b.rs\0old.rs\0 D gone.rs\0UU both.rs\0AA added.rs\0DU x.rs\0";
        let (branch, entries) = parse_status(raw);
        assert_eq!(branch.as_deref(), Some("main"));
        let brief: Vec<_> = entries
            .iter()
            .map(|e| (e.file.as_str(), e.status, e.from.as_deref(), e.unmerged))
            .collect();
        assert_eq!(
            brief,
            [
                ("src/a.rs", "modified", None, false),
                ("new.txt", "added", None, false),
                ("b.rs", "modified", Some("old.rs"), false),
                ("gone.rs", "deleted", None, false),
                ("both.rs", "modified", None, true),
                ("added.rs", "modified", None, true),
                ("x.rs", "modified", None, true),
            ]
        );
    }

    #[test]
    fn renames_bring_their_old_path() {
        let (_, entries) = parse_status("R  new.rs\0old.rs\0 M a.rs\0");
        assert_eq!(
            with_origins(
                &["new.rs".into(), "a.rs".into(), "a.rs".into(), " ".into()],
                &entries
            ),
            ["new.rs", "old.rs", "a.rs"]
        );
        assert_eq!(conflicts(1), "Resolve the conflicts in 1 file first.");
        assert_eq!(conflicts(3), "Resolve the conflicts in 3 files first.");
    }

    #[test]
    fn remotes_are_picked() {
        let pick = |list: &[&str]| pick_remote(list.iter().copied());
        assert_eq!(
            pick(&["fork", "origin", "upstream"]).as_deref(),
            Some("origin")
        );
        assert_eq!(pick(&["github"]).as_deref(), Some("github"));
        assert_eq!(pick(&["a", "b"]), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn ssh_runs_in_batch_mode() {
        assert_eq!(batch(None).as_deref(), Some("ssh -o BatchMode=yes"));
        assert_eq!(
            batch(Some("\"C:/Program Files/OpenSSH/ssh.exe\" -i key")).as_deref(),
            Some("\"C:/Program Files/OpenSSH/ssh.exe\" -i key -o BatchMode=yes")
        );
        assert_eq!(
            batch(Some("ssh -o BatchMode=no")).as_deref(),
            Some("ssh -o BatchMode=no")
        );
        assert_eq!(
            batch(Some("\"C:\\Program Files\\PuTTY\\plink.exe\" -batch")),
            None
        );
        assert_eq!(batch(Some("TortoisePlink.exe")), None);
    }

    #[test]
    fn failures_read_as_sentences() {
        let e = |op, s: &str| explain(op, Failure::Failed(s.into()));
        let first = |s: String| s.lines().next().unwrap_or_default().to_string();
        assert_eq!(
            first(e(
                Op::Commit,
                "Author identity unknown\n\n*** Please tell me who you are."
            )),
            "Git needs your name and email before it can commit. Add them, then commit again."
        );
        assert_eq!(
            first(e(
                Op::Push,
                " ! [rejected]        main -> main (fetch first)\nerror: failed to push some refs"
            )),
            "The remote has commits you don’t have. Pull first."
        );
        assert!(
            first(e(
                Op::Push,
                "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
            ))
            .starts_with("The remote didn’t accept your sign-in.")
        );
        assert!(
            first(e(
                Op::Push,
                "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."
            ))
            .starts_with("The remote didn’t accept your SSH key.")
        );
        assert!(
            first(e(
                Op::Pull,
                "Host key verification failed.\nfatal: Could not read from remote repository."
            ))
            .starts_with("This computer doesn’t know the remote’s SSH host")
        );
        assert!(
            first(e(
                Op::Push,
                "fatal: unable to access 'https://h/': Could not resolve host: h"
            ))
            .starts_with("Can’t reach the remote.")
        );
        assert!(
            first(e(
                Op::Pull,
                "fatal: Not possible to fast-forward, aborting."
            ))
            .starts_with("Your branch and the remote both have new commits")
        );
        assert!(
            first(e(
                Op::Pull,
                "error: Your local changes to the following files would be overwritten by merge:\n\ta.rs"
            ))
            .starts_with("Pulling would overwrite changes")
        );
        assert!(
            first(e(
                Op::Pull,
                "There is no tracking information for the current branch."
            ))
            .starts_with("This branch isn’t on the remote yet, so there’s nothing to pull.")
        );
        assert!(
            first(e(
                Op::Commit,
                "error: gpg failed to sign the data\nfatal: failed to write commit object"
            ))
            .starts_with("Git couldn’t sign the commit.")
        );
        assert!(
            first(e(Op::Push, "error: src refspec HEAD does not match any"))
                .starts_with("This repository has no commits yet.")
        );
        assert!(
            first(e(Op::Push, "fatal: You are not currently on a branch."))
                .starts_with("You’re not on a branch.")
        );
        // Unknown failures keep a headline for what was being done, and git's words below it.
        assert_eq!(
            e(Op::Commit, "pre-commit hook: lint failed"),
            "The commit wasn’t made.\npre-commit hook: lint failed"
        );
        assert_eq!(e(Op::Push, ""), "The push didn’t go through.");
        assert!(explain(Op::Pull, Failure::TimedOut(NETWORK)).contains("2 minutes"));
        assert!(explain(Op::Status, Failure::Missing).starts_with("Git isn’t installed"));
        let long = (0..20)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(tail(&long).lines().count(), 8);
        assert!(tail(&long).ends_with("line 19"));
    }

    #[test]
    fn web_urls() {
        let cases = [
            (
                "git@github.com:Anysola/code.git",
                "https://github.com/Anysola/code",
            ),
            (
                "https://github.com/Anysola/code.git",
                "https://github.com/Anysola/code",
            ),
            (
                "https://github.com/Anysola/code/",
                "https://github.com/Anysola/code",
            ),
            (
                "https://x-access-token:abc@github.com/Anysola/code",
                "https://github.com/Anysola/code",
            ),
            (
                "ssh://git@host.example:2222/o/r.git",
                "https://host.example/o/r",
            ),
            ("ssh://deploy@host.example/o/r", "https://host.example/o/r"),
            (
                "forge@git.example.org:team/r.git",
                "https://git.example.org/team/r",
            ),
            (
                "http://gitea.local:3000/o/r.git",
                "http://gitea.local:3000/o/r",
            ),
            ("git://host.example/o/r.git", "https://host.example/o/r"),
            (
                "git@ssh.dev.azure.com:v3/org/proj/repo",
                "https://dev.azure.com/org/proj/_git/repo",
            ),
            (
                "https://org@dev.azure.com/org/proj/_git/repo",
                "https://dev.azure.com/org/proj/_git/repo",
            ),
            (
                "org@vs-ssh.visualstudio.com:v3/org/proj/repo",
                "https://org.visualstudio.com/proj/_git/repo",
            ),
            (
                "git@gitlab.com:group/sub/r.git",
                "https://gitlab.com/group/sub/r",
            ),
        ];
        for (remote, web) in cases {
            assert_eq!(web_url(remote).as_deref(), Some(web), "{remote}");
        }
        for local in [
            "/srv/repo.git",
            r"C:\repos\r.git",
            "C:/repos/r",
            "../r",
            "file:///srv/r",
        ] {
            assert_eq!(web_url(local), None, "{local}");
        }
    }

    #[test]
    fn compare_pages_per_host() {
        assert_eq!(
            compare_url(
                "https://github.com/o/r",
                "fix/yellow price",
                "Fix it",
                "Body & more"
            ),
            "https://github.com/o/r/compare/fix/yellow%20price?expand=1&title=Fix%20it&body=Body%20%26%20more"
        );
        assert_eq!(
            compare_url("https://github.com/o/r", "b", "", ""),
            "https://github.com/o/r/compare/b?expand=1"
        );
        assert_eq!(
            compare_url("https://gitlab.com/g/r", "feat/x", "T", "D"),
            "https://gitlab.com/g/r/-/merge_requests/new?merge_request[source_branch]=feat%2Fx&merge_request[title]=T&merge_request[description]=D"
        );
        assert_eq!(
            compare_url("https://bitbucket.org/o/r", "feat/x", "T", "D"),
            "https://bitbucket.org/o/r/pull-requests/new?source=feat%2Fx"
        );
        assert_eq!(
            compare_url("https://dev.azure.com/org/p/_git/r", "feat/x", "T", "D"),
            "https://dev.azure.com/org/p/_git/r/pullrequestcreate?sourceRef=feat%2Fx"
        );
        let long = "x".repeat(BODY_IN_URL + 1);
        assert!(!compare_url("https://github.com/o/r", "b", "T", &long).contains("body="));
        assert_eq!(host_of("https://gitlab.example.com/g/r"), Host::GitLab);
        assert_eq!(host_of("https://git.corp/o/r"), Host::GitHub);
    }

    #[test]
    fn gh_answers() {
        let exists = "a pull request for branch \"fix\" into branch \"main\" already exists:\nhttps://github.com/o/r/pull/12";
        assert_eq!(
            existing_pr(exists).as_deref(),
            Some("https://github.com/o/r/pull/12")
        );
        assert_eq!(existing_pr("some other error https://x"), None);
        assert!(gh_cannot_serve(
            "To get started with GitHub CLI, please run:  gh auth login"
        ));
        assert!(gh_cannot_serve(
            "none of the git remotes configured for this repository point to a known GitHub host"
        ));
        assert!(!gh_cannot_serve(
            "pull request create failed: GraphQL: No commits between main and fix"
        ));
        assert!(
            pr_error("pull request create failed: GraphQL: No commits between main and fix")
                .starts_with("This branch has no commits its base branch doesn’t have")
        );
        assert!(
            pr_error("aborted: you must first push the current branch to a remote")
                .starts_with("Push the branch first")
        );
    }

    /// Repositories in a temporary folder, with a config of their own: nothing reads or writes
    /// the user's `~/.gitconfig`.
    struct Sandbox(std::path::PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Sandbox {
            let root = std::env::temp_dir().join(format!(
                "anysola-git-{name}-{}-{}",
                std::process::id(),
                anysola_core::ids::part()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let shared = std::env::temp_dir().join(format!("anysola-git-{}", std::process::id()));
            std::fs::create_dir_all(&shared).unwrap();
            let global = shared.join("gitconfig");
            if !global.exists() {
                std::fs::write(
                    &global,
                    "[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n[commit]\n\tgpgsign = false\n",
                )
                .unwrap();
            }
            TEST_ENV.get_or_init(|| {
                vec![
                    ("GIT_CONFIG_GLOBAL".into(), global.display().to_string()),
                    ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                    ("GIT_AUTHOR_DATE".into(), "2026-01-01T00:00:00Z".into()),
                ]
            });
            Sandbox(root)
        }

        fn repo(&self, name: &str) -> std::path::PathBuf {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            self.git(&dir, &["init", "-q"]);
            dir
        }

        fn git(&self, dir: &Path, args: &[&str]) -> String {
            git(dir, args)
                .ok()
                .unwrap_or_else(|f| panic!("git {args:?}: {f:?}"))
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn commits_only_the_chosen_files() {
        let sandbox = Sandbox::new("commit");
        let repo = sandbox.repo("r");
        std::fs::write(repo.join("keep.txt"), "1").unwrap();
        std::fs::write(repo.join("gone.txt"), "1").unwrap();
        sandbox.git(&repo, &["add", "-A"]);
        sandbox.git(&repo, &["commit", "-q", "-m", "first"]);
        std::fs::write(repo.join("keep.txt"), "2").unwrap();
        std::fs::remove_file(repo.join("gone.txt")).unwrap();
        std::fs::create_dir_all(repo.join("sub dir")).unwrap();
        std::fs::write(repo.join("sub dir/new file.txt"), "n").unwrap();
        std::fs::write(repo.join("-odd.txt"), "n").unwrap();
        let chosen = vec![
            "gone.txt".to_string(),
            "sub dir/new file.txt".to_string(),
            "-odd.txt".to_string(),
        ];
        commit(&repo.join("sub dir"), "second", Some(chosen)).unwrap();
        let shown = sandbox.git(
            &repo,
            &["show", "--no-renames", "--name-status", "--format=", "HEAD"],
        );
        let mut lines: Vec<_> = shown.lines().collect();
        lines.sort_unstable();
        assert_eq!(
            lines,
            ["A\t-odd.txt", "A\tsub dir/new file.txt", "D\tgone.txt"]
        );
        let status = status(&repo).unwrap();
        assert_eq!(
            status["files"],
            json!([{"file": "keep.txt", "status": "modified"}])
        );
        assert_eq!(status["identity"], true);
        assert_eq!(status["unmerged"], json!([]));
        assert_eq!(
            commit(&repo, "none", Some(vec![])).unwrap_err(),
            "Choose at least one file to commit."
        );
        // A rename and a removal staged by git itself.
        sandbox.git(&repo, &["mv", "keep.txt", "kept.txt"]);
        sandbox.git(&repo, &["rm", "-q", "--", "-odd.txt"]);
        commit(
            &repo,
            "moved",
            Some(vec!["kept.txt".into(), "-odd.txt".into()]),
        )
        .unwrap();
        let shown = sandbox.git(
            &repo,
            &["show", "--no-renames", "--name-status", "--format=", "HEAD"],
        );
        let mut lines: Vec<_> = shown.lines().collect();
        lines.sort_unstable();
        assert_eq!(lines, ["A\tkept.txt", "D\t-odd.txt", "D\tkeep.txt"]);
        std::fs::write(repo.join("last.txt"), "l").unwrap();
        commit(&repo, "the rest", None).unwrap();
        assert_eq!(status_entries(&repo).unwrap().1, []);
        assert!(
            commit(&repo, "empty", None)
                .unwrap_err()
                .starts_with("There’s nothing to commit.")
        );
    }

    #[test]
    fn conflicts_block_a_commit() {
        let sandbox = Sandbox::new("conflict");
        let repo = sandbox.repo("r");
        std::fs::write(repo.join("a.txt"), "base\n").unwrap();
        std::fs::write(repo.join("c.txt"), "base\n").unwrap();
        sandbox.git(&repo, &["add", "-A"]);
        sandbox.git(&repo, &["commit", "-q", "-m", "base"]);
        sandbox.git(&repo, &["switch", "-q", "-c", "other"]);
        std::fs::write(repo.join("a.txt"), "other\n").unwrap();
        sandbox.git(&repo, &["commit", "-q", "-am", "other"]);
        sandbox.git(&repo, &["switch", "-q", "main"]);
        std::fs::write(repo.join("a.txt"), "main\n").unwrap();
        sandbox.git(&repo, &["commit", "-q", "-am", "main"]);
        assert!(git(&repo, &["merge", "-q", "other"]).ok().is_err());
        let state = status(&repo).unwrap();
        assert_eq!(state["unmerged"], json!(["a.txt"]));
        assert_eq!(state["files"], json!([]));
        assert_eq!(
            commit(&repo, "merge", None).unwrap_err(),
            "Resolve the conflicts in 1 file first."
        );
        // Fixed in an editor, never staged: resolved, and the merge can be finished.
        std::fs::write(repo.join("a.txt"), "both\n").unwrap();
        std::fs::write(repo.join("b.txt"), "new\n").unwrap();
        std::fs::write(repo.join("c.txt"), "edited\n").unwrap();
        let state = status(&repo).unwrap();
        assert_eq!(state["unmerged"], json!([]));
        assert_eq!(
            state["files"],
            json!([
                {"file": "a.txt", "status": "modified"},
                {"file": "c.txt", "status": "modified"},
                {"file": "b.txt", "status": "added"}
            ])
        );
        // Every tracked change is needed; an untracked file isn't.
        assert!(
            commit(&repo, "merge", Some(vec!["a.txt".into(), "b.txt".into()]))
                .unwrap_err()
                .starts_with("A merge is being finished")
        );
        commit(&repo, "merge", Some(vec!["a.txt".into(), "c.txt".into()])).unwrap();
        let parents = sandbox.git(&repo, &["rev-list", "--parents", "-n", "1", "HEAD"]);
        assert_eq!(parents.split_whitespace().count(), 3, "a merge commit");
        let left: Vec<_> = status_entries(&repo)
            .unwrap()
            .1
            .into_iter()
            .map(|e| (e.file, e.untracked))
            .collect();
        assert_eq!(left, [("b.txt".to_string(), true)]);
        assert_eq!(
            sandbox.git(&repo, &["show", "HEAD:c.txt"]),
            "edited",
            "the chosen change is in the merge"
        );
        assert!(has_markers(
            "a\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> other\n"
        ));
        assert!(!has_markers(">>>>>>> before\n<<<<<<< after\n"));
    }

    #[test]
    fn pull_and_push_with_a_remote() {
        let sandbox = Sandbox::new("remote");
        let remote = sandbox.0.join("remote.git");
        std::fs::create_dir_all(&remote).unwrap();
        sandbox.git(&remote, &["init", "-q", "--bare"]);
        let a = sandbox.repo("a");
        sandbox.git(
            &a,
            &["remote", "add", "origin", &remote.display().to_string()],
        );
        std::fs::write(a.join("f.txt"), "1").unwrap();
        commit(&a, "one", None).unwrap();
        // No upstream yet: nothing to pull, and the first push sets it.
        assert!(
            pull(&a)
                .unwrap_err()
                .starts_with("This branch isn’t on the remote yet")
        );
        push(&a).unwrap();
        assert_eq!(status(&a).unwrap()["upstream"], "origin/main");
        assert_eq!(status(&a).unwrap()["remote"], "origin");

        let b = sandbox.0.join("b");
        sandbox.git(
            &sandbox.0,
            &["clone", "-q", &remote.display().to_string(), "b"],
        );
        std::fs::write(b.join("f.txt"), "2").unwrap();
        commit(&b, "two", None).unwrap();
        push(&b).unwrap();

        // `a` is behind: its push is refused, a fast-forward pull brings it up.
        std::fs::write(a.join("g.txt"), "x").unwrap();
        sandbox.git(&a, &["fetch", "-q"]);
        assert_eq!(status(&a).unwrap()["behind"], 1);
        pull(&a).unwrap();
        assert_eq!(std::fs::read_to_string(a.join("f.txt")).unwrap(), "2");
        commit(&a, "three", Some(vec!["g.txt".into()])).unwrap();
        std::fs::write(b.join("h.txt"), "y").unwrap();
        commit(&b, "four", None).unwrap();
        push(&b).unwrap();
        let refused = push(&a).unwrap_err();
        assert!(
            refused.starts_with("The remote has commits you don’t have. Pull first."),
            "{refused}"
        );
        let diverged = pull(&a).unwrap_err();
        assert!(
            diverged.starts_with("Your branch and the remote both have new commits"),
            "{diverged}"
        );
    }

    #[test]
    fn outside_a_repository() {
        let sandbox = Sandbox::new("plain");
        assert_eq!(status(&sandbox.0).unwrap(), json!({"repo": false}));
        let missing = Call::new("anysola-no-such-program", &sandbox.0, &[]).ok();
        assert_eq!(missing.unwrap_err(), Failure::Missing);
    }

    #[test]
    fn identity_is_checked_and_saved() {
        let sandbox = Sandbox::new("identity");
        // A global config of this test's own, with no name or email in it yet.
        let global = sandbox.0.join("gitconfig");
        std::fs::write(&global, "[init]\n\tdefaultBranch = main\n").unwrap();
        TEST_GLOBAL.with(|g| *g.borrow_mut() = Some(global.clone()));
        let repo = sandbox.repo("r");
        assert!(!has_identity(&repo));
        assert_eq!(
            identity(&sandbox.0, " ", "a@b").unwrap_err(),
            "Enter your name."
        );
        assert!(identity(&sandbox.0, "A", "nope").is_err());
        assert!(identity(&sandbox.0, "A", "a b@c").is_err());
        assert!(!has_identity(&repo), "nothing saved for a refused entry");
        identity(&sandbox.0, " Ada Lovelace ", "ada@example.org").unwrap();
        assert!(has_identity(&repo));
        assert_eq!(
            sandbox.git(&repo, &["config", "--global", "user.name"]),
            "Ada Lovelace"
        );
        assert_eq!(
            sandbox.git(&repo, &["config", "--global", "user.email"]),
            "ada@example.org"
        );
        TEST_GLOBAL.with(|g| *g.borrow_mut() = None);
    }

    #[test]
    fn chosen_paths_are_literal_and_hooks_see_plain_pathspecs() {
        let sandbox = Sandbox::new("literal");
        let repo = sandbox.repo("r");
        std::fs::write(repo.join("base.txt"), "1").unwrap();
        commit(&repo, "base", None).unwrap();
        // `lit[ab].rs` as a glob would match `lita.rs` too.
        std::fs::write(repo.join("lit[ab].rs"), "x").unwrap();
        std::fs::write(repo.join("lita.rs"), "y").unwrap();
        #[cfg(unix)]
        let seen = {
            use std::os::unix::fs::PermissionsExt;
            let seen = sandbox.0.join("hook-saw");
            let hook = repo.join(".git/hooks/pre-commit");
            std::fs::write(
                &hook,
                format!(
                    "#!/bin/sh\ngit diff --cached --name-only -- '*.rs' > '{}'\n",
                    seen.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            seen
        };
        commit(&repo, "one", Some(vec!["lit[ab].rs".into()])).unwrap();
        let shown = sandbox.git(&repo, &["show", "--name-only", "--format=", "HEAD"]);
        assert_eq!(shown, "lit[ab].rs");
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_to_string(seen).unwrap().trim(),
            "lit[ab].rs",
            "a hook's own pathspec is a glob as usual"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_program_is_stopped() {
        let dir = std::env::temp_dir();
        let started = Instant::now();
        let out = Call::new("sleep", &dir, &["5"])
            .timeout(Duration::from_millis(200))
            .run();
        assert!(matches!(out, Err(Failure::TimedOut(_))));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn a_stopped_commit_leaves_no_lock_and_no_hook_running() {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new("stopped");
        let repo = sandbox.repo("r");
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        commit(&repo, "base", None).unwrap();
        std::fs::write(repo.join("a.txt"), "2").unwrap();
        // A hook that hangs, with a helper of its own, while git holds the index for `--only`.
        let pidfile = sandbox.0.join("helper");
        let hook = repo.join(".git/hooks/pre-commit");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\nsleep 60 &\necho $! > '{}'\nwait\n",
                pidfile.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let list = literal([&"a.txt".to_string()]);
        let out = git(
            &repo,
            &[
                "commit",
                "-q",
                "-m",
                "hangs",
                "--only",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
        )
        .input(list)
        .holds_index()
        .timeout(Duration::from_secs(1))
        .run();
        assert!(matches!(out, Err(Failure::TimedOut(_))), "{:?}", out.err());
        assert!(!repo.join(".git/index.lock").exists());
        let helper: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let started = Instant::now();
        while !crate::tree::gone(helper) {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the hook's helper runs on"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // The repository works again at once.
        std::fs::remove_file(&hook).unwrap();
        commit(&repo, "two", Some(vec!["a.txt".into()])).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_lock_another_git_holds_is_left_to_it() {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new("held");
        let repo = sandbox.repo("r");
        std::fs::write(repo.join("a.txt"), "1").unwrap();
        commit(&repo, "base", None).unwrap();
        // A plain commit lets go of the index before its hooks run: a lock there meanwhile is
        // another git's, even one made while it ran.
        let hook = repo.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nsleep 60\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(repo.join("a.txt"), "2").unwrap();
        git(&repo, &["add", "a.txt"]).ok().unwrap();
        let other = std::thread::spawn({
            let lock = repo.join(".git/index.lock");
            move || {
                std::thread::sleep(Duration::from_millis(300));
                std::fs::write(lock, "").unwrap();
            }
        });
        let out = git(&repo, &["commit", "-q", "-m", "hangs"])
            .timeout(Duration::from_secs(1))
            .run();
        other.join().unwrap();
        assert!(matches!(out, Err(Failure::LockLeft(_))), "{:?}", out.err());
        assert!(repo.join(".git/index.lock").exists());
        std::fs::remove_file(&hook).unwrap();
        // Older than a call that holds the index: not that call's.
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        assert!(locked(&repo, Lock::Own, later));
        assert!(repo.join(".git/index.lock").exists());
        assert_eq!(
            explain(Op::Commit, Failure::LockLeft(COMMIT))
                .lines()
                .next(),
            Some(
                "Git didn’t finish within 5 minutes, so it was stopped. Another Git program is \
                 still working in this repository; trying again in a moment usually works."
            )
        );
        // Made while a call that holds the index ran: removed.
        let before = std::time::SystemTime::now() - Duration::from_secs(5);
        assert!(locked(&repo, Lock::Other, before));
        assert!(!locked(&repo, Lock::Own, before));
        assert!(!repo.join(".git/index.lock").exists());
        assert!(!locked(&repo, Lock::Own, before), "no lock at all");
    }
}
