//! Sessions that work in their own git worktree of the main folder (one core per main folder;
//! the session's directory is its worktree). The session is *pinned* there: its file tools treat
//! only the worktree as inside, its snapshots, hooks and instructions are the worktree's, and it
//! cannot move out. Creating, preparing, applying and removing go through the row's state, every
//! change a compare-and-set.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Core, CoreError, Shared};
use crate::session::message::Message;
use crate::snapshot::Store;
use crate::storage::{Db, SessionRow, SessionWorktreeRow, WorktreeState};
use crate::worktree::git_sync;

/// Where a session works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The checkout's root: the session's worktree, else the project's root.
    pub root: PathBuf,
    /// The project's (main folder's) root.
    pub main: PathBuf,
    /// The session works in its own worktree and cannot leave it.
    pub pinned: bool,
}

/// How a fork's files were put back to its fork point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkRestore {
    /// Every file the snapshots cover is as it was at the fork point.
    pub exact: bool,
    /// Files left as on the commit the fork started from: over 2 MiB and never captured.
    pub kept: Vec<String>,
    pub fallback: Option<Fallback>,
}

/// How the files were put back when not from the fork point's snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fallback {
    /// The source's commit now with only the session's own files as at the fork point (history
    /// from before 1.1.9): other uncommitted changes of the source are not included.
    Edits,
    /// The fork point's files are gone: the source's files as they are now.
    Now,
}

/// How a worktree is let go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Release {
    Applied(String),
    Discarded,
}

/// Whether a turn may start now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnGate {
    Run,
    /// The worktree is still being made: input waits in the inbox until it is ready.
    Wait,
}

/// Where released sessions' branch tips are kept, so the `head` their steps recorded stays
/// reachable for forks: `refs/anysola/sessions/<session id>/<commit>`.
const HIDDEN: &str = "refs/anysola/sessions";
const MAX_SUFFIX: u32 = 99;

fn worktree_error(text: impl Into<String>) -> CoreError {
    CoreError::Worktree(text.into())
}

fn busy() -> CoreError {
    worktree_error("This session is working. Stop it first.")
}

/// The plain sentence for a state a turn cannot start in.
fn blocked(state: WorktreeState) -> &'static str {
    match state {
        WorktreeState::Missing => {
            "This session's folder is gone. Recreate it, discard it, or fork the session."
        }
        WorktreeState::Failed => {
            "Making this session's worktree failed. Try again, or work in the main folder."
        }
        WorktreeState::Applying => {
            "This session's changes are being applied. Try again in a moment."
        }
        WorktreeState::Applied => "This session's changes were applied; fork it to keep going.",
        WorktreeState::Discarded => "This session's worktree was discarded.",
        WorktreeState::Preparing | WorktreeState::Ready => "",
    }
}

fn canonical(path: &Path) -> PathBuf {
    crate::paths::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn root_of(db: &Db, session: &SessionRow) -> String {
    match session.parent_id {
        Some(_) => crate::session::gate::root(db, &session.id),
        None => session.id.clone(),
    }
}

/// A branch- and folder-safe form of the session's slug.
fn plain_slug(slug: &str) -> String {
    let plain: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let plain = plain.trim_matches('-');
    if plain.is_empty() {
        "session".into()
    } else {
        plain.chars().take(40).collect()
    }
}

/// Assistant steps' snapshots, with their message's position.
fn steps(
    rows: &[crate::storage::MessageRow],
) -> Vec<(usize, crate::session::message::StepSnapshot)> {
    rows.iter()
        .enumerate()
        .filter_map(|(i, row)| match Message::decode(&row.kind, &row.data) {
            Message::Assistant(data) => data.snapshot.map(|s| (i, s)),
            _ => None,
        })
        .collect()
}

/// Whether the session ran a step after `since` (ms): its own work, not a fork's copied history.
fn ran_since(db: &Db, session_id: &str, since: u64) -> Result<bool, CoreError> {
    Ok(db.messages(session_id)?.iter().any(|row| {
        matches!(Message::decode(&row.kind, &row.data),
            Message::Assistant(data) if data.time.created >= since)
    }))
}

impl Shared {
    /// The worktree row of the session's root, when it works in its own worktree.
    pub(crate) fn worktree_row(&self, session: &SessionRow) -> Option<SessionWorktreeRow> {
        self.db
            .session_worktree(&root_of(&self.db, session))
            .ok()
            .flatten()
    }

    /// Where the session works: its own worktree (pinned), else the project's root.
    pub(crate) fn checkout_of(&self, session: &SessionRow) -> Checkout {
        let main = self.project.worktree.clone();
        match self.worktree_row(session) {
            Some(row) => Checkout {
                root: PathBuf::from(row.directory),
                main,
                pinned: true,
            },
            None => Checkout {
                root: main.clone(),
                main,
                pinned: false,
            },
        }
    }

    /// The snapshot store of the checkout the session works in; `None` outside git, with
    /// `snapshots: false`, or while its folder does not exist.
    pub(crate) fn store_for(&self, session: &SessionRow) -> Option<Arc<Store>> {
        if !self.snapshots_on {
            return None;
        }
        let directory = PathBuf::from(&session.directory);
        if directory == self.directory {
            return self.snapshots.clone();
        }
        let mut stores = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = stores.get(&directory) {
            return Some(store.clone());
        }
        // A miss is not remembered: a worktree being made has no folder yet.
        let store = Arc::new(Store::open(&self.paths.data, &self.project.id, &directory)?);
        // A session that moved here (or a fork) names trees of the main folder's store too.
        if let Some(main) = &self.snapshots {
            store.borrow_objects(main.git_dir());
        }
        stores.insert(directory, store.clone());
        Some(store)
    }

    fn forget_store(&self, directory: &str) {
        self.stores
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(Path::new(directory));
    }

    /// The repository's top folder, canonical.
    fn top(&self) -> PathBuf {
        canonical(&self.project.worktree)
    }
}

/// Whether a turn of the session may start: always outside worktrees; in one, only once it is
/// ready (`Wait` while it is being made).
pub(crate) fn turn_gate(shared: &Shared, session_id: &str) -> Result<TurnGate, CoreError> {
    let root = crate::session::gate::root(&shared.db, session_id);
    let Some(row) = shared.db.session_worktree(&root)? else {
        return Ok(TurnGate::Run);
    };
    match row.state {
        // A folder deleted while the app runs is found here, not only at the next start.
        WorktreeState::Ready if !Path::new(&row.directory).is_dir() => {
            let _ = shared.db.cas_session_worktree(
                &root,
                WorktreeState::Ready,
                WorktreeState::Missing,
                None,
            );
            Err(worktree_error(blocked(WorktreeState::Missing)))
        }
        WorktreeState::Ready => Ok(TurnGate::Run),
        WorktreeState::Preparing => Ok(TurnGate::Wait),
        state => Err(worktree_error(blocked(state))),
    }
}

/// Refuses changing the session's files from outside a turn (rewind, redo, put back) unless its
/// own worktree, if it has one, is ready: not while an apply or a merge of the base reads it.
pub(crate) fn files_gate(shared: &Shared, session_id: &str) -> Result<(), CoreError> {
    let root = crate::session::gate::root(&shared.db, session_id);
    match shared.db.session_worktree(&root)?.map(|r| r.state) {
        None | Some(WorktreeState::Ready) => Ok(()),
        Some(state) => Err(worktree_error(match blocked(state) {
            "" => "This session's worktree is being made. Try again in a moment.",
            text => text,
        })),
    }
}

/// Keeps `commit` reachable under the session's hidden refs (best effort).
fn pin(top: &Path, session_id: &str, commit: &str) {
    let _ = git_sync(
        top,
        &[
            "update-ref",
            &format!("{HIDDEN}/{session_id}/{commit}"),
            commit,
        ],
        &[],
    );
}

/// Drops the session's hidden refs.
fn unpin(top: &Path, session_id: &str) {
    let Ok(refs) = git_sync(
        top,
        &[
            "for-each-ref",
            "--format=%(refname)",
            &format!("{HIDDEN}/{session_id}/"),
        ],
        &[],
    ) else {
        return;
    };
    for name in refs.lines().filter(|l| !l.is_empty()) {
        let _ = git_sync(top, &["update-ref", "-d", name], &[]);
    }
}

fn branch_exists(top: &Path, branch: &str) -> bool {
    git_sync(
        top,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
        &[],
    )
    .is_ok()
}

/// Removes a session's worktree folder and branch (the tip pinned first). Each part that is
/// already gone counts as removed; returns whether both are gone now.
fn remove_checkout(top: &Path, row: &SessionWorktreeRow) -> bool {
    remove_checkout_keeping(top, row, false)
}

/// [`remove_checkout`]; with `keep_moved`, a branch that moved past `base_commit` (work was
/// committed on it) is kept.
fn remove_checkout_keeping(top: &Path, row: &SessionWorktreeRow, keep_moved: bool) -> bool {
    let tip = git_sync(
        top,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", row.branch),
        ],
        &[],
    )
    .ok();
    if let Some(tip) = &tip {
        pin(top, &row.session_id, tip);
    }
    let directory = Path::new(&row.directory);
    if directory.exists() {
        let removed = git_sync(
            top,
            &["worktree", "remove", "--force", "--force", &row.directory],
            &[],
        );
        if removed.is_err() {
            // A folder git no longer knows, or one a program holds open: remove it directly.
            let _ = std::fs::remove_dir_all(directory);
        }
    }
    let _ = git_sync(top, &["worktree", "prune"], &[]);
    let moved = tip.as_deref().is_some_and(|t| t != row.base_commit);
    if keep_moved && moved {
        return !directory.exists();
    }
    if branch_exists(top, &row.branch) {
        let _ = git_sync(top, &["branch", "-D", "--", &row.branch], &[]);
    }
    !directory.exists() && !branch_exists(top, &row.branch)
}

/// What [`make_checkout`] made.
struct Made {
    directory: PathBuf,
    branch: String,
}

/// A free `anysola/<slug>` branch and `<base>/<slug>` folder (suffixed `-2`… when taken).
fn free_names(
    paths: &crate::paths::Paths,
    top: &Path,
    base: &Path,
    slug: &str,
) -> Result<(String, PathBuf), CoreError> {
    for n in 1..=MAX_SUFFIX {
        let name = if n == 1 {
            slug.to_string()
        } else {
            format!("{slug}-{n}")
        };
        let branch = format!("anysola/{name}");
        let folder = crate::worktree::place(paths, base, &name).map_err(worktree_error)?;
        if !branch_exists(top, &branch) && folder.symlink_metadata().is_err() {
            return Ok((branch, folder));
        }
    }
    Err(worktree_error(
        "Too many worktrees with this name; remove some old ones.",
    ))
}

impl Core {
    /// The session's worktree row (its root's, for a subagent session).
    pub fn session_worktree(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionWorktreeRow>, CoreError> {
        let session = self
            .shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        Ok(self.shared.worktree_row(&session))
    }

    /// Where the session works.
    pub fn checkout(&self, session_id: &str) -> Result<Checkout, CoreError> {
        let session = self
            .shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        Ok(self.shared.checkout_of(&session))
    }

    /// Gives a new top-level session its own worktree of the main folder, on a new branch
    /// `anysola/<slug>` from `HEAD` (uncommitted changes of the main folder are not carried). The
    /// row is `preparing` until [`Core::set_worktree_state`] marks it `ready`; prompts wait in
    /// the inbox meanwhile.
    pub async fn isolate(&self, session_id: &str) -> Result<SessionWorktreeRow, CoreError> {
        let shared = &self.shared;
        let session = shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        if session.parent_id.is_some() || session.fork_session_id.is_some() {
            return Err(worktree_error(
                "Only a new top-level session gets its own worktree this way.",
            ));
        }
        if shared.db.session_worktree(session_id)?.is_some() {
            return Err(worktree_error("This session already has its own worktree."));
        }
        if shared
            .db
            .messages(session_id)?
            .iter()
            .any(|m| m.kind == "assistant")
        {
            return Err(worktree_error(
                "This session already worked in the main folder.",
            ));
        }
        let head = self.head_commit()?;
        let base_branch = git_sync(
            &shared.top(),
            &["symbolic-ref", "-q", "--short", "HEAD"],
            &[],
        )
        .ok()
        .filter(|b| !b.is_empty());
        self.make_checkout(&session, &head, base_branch, None)
            .await
            .map(|(row, _)| row)
    }

    /// The main folder's `HEAD` commit; an error outside git or before the first commit.
    fn head_commit(&self) -> Result<String, CoreError> {
        let shared = &self.shared;
        if shared.project.vcs != Some("git") || shared.project.id == "global" {
            return Err(worktree_error(
                "Worktrees need a Git repository with at least one commit.",
            ));
        }
        git_sync(
            &shared.top(),
            &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
            &[],
        )
        .ok()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| worktree_error("Worktrees need a Git repository with at least one commit."))
    }

    /// Records the row (`preparing`) and then adds the worktree at `commit`; the session's
    /// directory moves there. A failed add leaves the row `failed` with git's words.
    async fn make_checkout(
        &self,
        session: &SessionRow,
        commit: &str,
        base_branch: Option<String>,
        restored: Option<&ForkRestore>,
    ) -> Result<(SessionWorktreeRow, Made), CoreError> {
        let shared = &self.shared;
        let top = shared.top();
        let folder = canonical(&shared.directory);
        let sub = folder
            .strip_prefix(&top)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let base =
            crate::worktree::session_base(&shared.paths, &shared.directory, &shared.project.id)
                .map_err(worktree_error)?;
        std::fs::create_dir_all(&base).map_err(|e| worktree_error(e.to_string()))?;
        let base = canonical(&base);
        let (branch, directory) =
            free_names(&shared.paths, &top, &base, &plain_slug(&session.slug))?;
        let now = crate::time::now_ms();
        let mut row = SessionWorktreeRow {
            session_id: session.id.clone(),
            project_id: shared.project.id.clone(),
            folder: shared.directory.display().to_string(),
            directory: directory.display().to_string(),
            branch: branch.clone(),
            base_branch,
            base_commit: commit.to_string(),
            snapshot_dir: None,
            state: WorktreeState::Preparing,
            error: None,
            applied_commit: None,
            apply_from: None,
            apply_tree: None,
            restored: restored.and_then(|r| serde_json::to_value(r).ok()),
            leftover: false,
            time_created: now,
            time_updated: now,
        };
        shared.db.insert_session_worktree(&row)?;
        let added = crate::worktree::git(
            &top,
            &[
                "-c".as_ref(),
                "core.longpaths=true".as_ref(),
                "worktree".as_ref(),
                "add".as_ref(),
                "--quiet".as_ref(),
                "-b".as_ref(),
                branch.as_ref(),
                "--".as_ref(),
                directory.as_os_str(),
                commit.as_ref(),
            ],
        )
        .await;
        if let Err(message) = added {
            let error = format!("Git could not make the worktree: {message}");
            shared.db.cas_session_worktree(
                &session.id,
                WorktreeState::Preparing,
                WorktreeState::Failed,
                Some(&error),
            )?;
            return Err(worktree_error(error));
        }
        let working = directory.join(&sub);
        let _ = std::fs::create_dir_all(&working);
        shared
            .db
            .set_directory(&session.id, &working.display().to_string())?;
        shared.db.put_worktree(
            &shared.project.id,
            &row.directory,
            crate::worktree::SESSION_STRATEGY,
        )?;
        if shared.snapshots_on
            && let Some(store) = Store::open(&shared.paths.data, &shared.project.id, &working)
        {
            if let Ok(git_dir) = git_sync(&directory, &["rev-parse", "--absolute-git-dir"], &[]) {
                store.seed_index(&Path::new(&git_dir).join("index"));
            }
            let dir = store.git_dir().display().to_string();
            shared.db.update_session_worktree(
                &session.id,
                WorktreeState::Preparing,
                &[("snapshot_dir", dir.clone().into())],
            )?;
            row.snapshot_dir = Some(dir);
            shared
                .stores
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(working.clone(), Arc::new(store));
        }
        Ok((row, Made { directory, branch }))
    }

    /// Moves the session's worktree from `from` to `to`; false when it was not in `from`.
    /// Reaching `ready` starts the turn for input that waited.
    pub fn set_worktree_state(
        &self,
        session_id: &str,
        from: WorktreeState,
        to: WorktreeState,
    ) -> Result<bool, CoreError> {
        self.change_state(session_id, from, to, None)
    }

    /// Like [`Core::set_worktree_state`], recording why (a failure, a notice).
    pub fn set_worktree_error(
        &self,
        session_id: &str,
        from: WorktreeState,
        to: WorktreeState,
        error: &str,
    ) -> Result<bool, CoreError> {
        self.change_state(session_id, from, to, Some(error))
    }

    /// A state change made under the `turns` lock, so a turn either started before it (and
    /// leaving `ready` is refused while it runs) or sees the new state: a prompt held while
    /// `preparing` is either queued before `ready` (and woken by it) or finds it ready.
    fn change_state(
        &self,
        session_id: &str,
        from: WorktreeState,
        to: WorktreeState,
        error: Option<&str>,
    ) -> Result<bool, CoreError> {
        let idle = from == WorktreeState::Ready && to == WorktreeState::Applying;
        let moved = self.cas_locked(session_id, from, to, error, idle)?;
        if moved && to == WorktreeState::Ready {
            self.wake_waiting(session_id)?;
        }
        Ok(moved)
    }

    /// The compare-and-set under the `turns` lock; with `idle`, refused while a turn or a
    /// subagent of the session runs.
    fn cas_locked(
        &self,
        session_id: &str,
        from: WorktreeState,
        to: WorktreeState,
        error: Option<&str>,
        idle: bool,
    ) -> Result<bool, CoreError> {
        let shared = &self.shared;
        let turns = shared.turns.lock().unwrap_or_else(|e| e.into_inner());
        if idle && (turns.contains_key(session_id) || self.working(session_id)) {
            return Err(busy());
        }
        let moved = shared
            .db
            .cas_session_worktree(session_id, from, to, error)?;
        drop(turns);
        Ok(moved)
    }

    /// A subagent of the session (or the session as one) runs in this process.
    fn working(&self, session_id: &str) -> bool {
        crate::session::subagent::is_running(&self.shared, session_id)
            || crate::session::subagent::has_running_child(&self.shared, session_id)
    }

    /// After the base branch was brought into the session's branch: the session's changes are
    /// measured from `commit` (the base's tip) from now on. Only while the row is in `state`.
    pub fn set_worktree_base(
        &self,
        session_id: &str,
        state: WorktreeState,
        commit: &str,
    ) -> Result<bool, CoreError> {
        Ok(self.shared.db.update_session_worktree(
            session_id,
            state,
            &[("base_commit", commit.to_string().into())],
        )?)
    }

    /// Records where an apply starts (the main folder's `HEAD`) and the tree it puts there, while
    /// `applying`, so an apply cut off by a crash can be told finished or not.
    pub fn mark_apply(&self, session_id: &str, from: &str, tree: &str) -> Result<bool, CoreError> {
        Ok(self.shared.db.update_session_worktree(
            session_id,
            WorktreeState::Applying,
            &[
                ("apply_from", from.to_string().into()),
                ("apply_tree", tree.to_string().into()),
            ],
        )?)
    }

    fn wake_waiting(&self, session_id: &str) -> Result<(), CoreError> {
        let waiting = crate::session::inbox::next(
            &self.shared.db,
            session_id,
            crate::session::inbox::Scope::Input,
        )?
        .is_some();
        if waiting {
            let _entered = self.shared.runtime.enter();
            super::wake(&self.shared, session_id);
        }
        Ok(())
    }

    /// Refuses while a turn or a subagent of the session runs.
    fn ensure_idle(&self, session_id: &str) -> Result<(), CoreError> {
        if self.is_busy(session_id) || self.working(session_id) {
            return Err(busy());
        }
        Ok(())
    }

    /// Stops the background commands of the session and its subagents (they may run in the
    /// worktree about to go, or write it while an apply reads it).
    pub fn stop_background(&self, session_id: &str) {
        let tree = self
            .shared
            .db
            .session_tree(session_id)
            .unwrap_or_else(|_| vec![session_id.to_string()]);
        for id in tree {
            for shell in self.background_shells(&id) {
                self.kill_shell(&shell.id);
            }
        }
    }

    /// After a successful apply (`applying`) the session goes on: the old branch tip is pinned,
    /// the worktree and its branch move to the applied commit, which becomes the base; `ready`.
    pub fn continue_after_apply(
        &self,
        session_id: &str,
        commit: &str,
    ) -> Result<SessionWorktreeRow, CoreError> {
        let shared = &self.shared;
        let row = shared
            .db
            .session_worktree(session_id)?
            .ok_or_else(|| worktree_error("This session has no worktree."))?;
        if row.state != WorktreeState::Applying {
            return Err(worktree_error("This session is not being applied."));
        }
        let top = shared.top();
        let commit = git_sync(
            &top,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{commit}^{{commit}}"),
            ],
            &[],
        )
        .map_err(|_| worktree_error(format!("No commit {commit}")))?;
        if let Ok(tip) = git_sync(Path::new(&row.directory), &["rev-parse", "HEAD"], &[]) {
            pin(&top, session_id, &tip);
        }
        let reset = git_sync(
            Path::new(&row.directory),
            &["reset", "--hard", "--quiet", &commit],
            &[],
        );
        let moved = match &reset {
            Ok(_) => shared.db.update_session_worktree(
                session_id,
                WorktreeState::Applying,
                &[
                    ("state", WorktreeState::Ready.as_str().to_string().into()),
                    ("base_commit", commit.clone().into()),
                    ("applied_commit", commit.clone().into()),
                    ("apply_from", rusqlite::types::Value::Null),
                    ("apply_tree", rusqlite::types::Value::Null),
                    ("error", rusqlite::types::Value::Null),
                ],
            )?,
            Err(message) => shared.db.update_session_worktree(
                session_id,
                WorktreeState::Applying,
                &[
                    ("state", WorktreeState::Ready.as_str().to_string().into()),
                    ("applied_commit", commit.clone().into()),
                    (
                        "error",
                        format!(
                            "Applied as {commit}, but the worktree could not follow: {message}"
                        )
                        .into(),
                    ),
                ],
            )?,
        };
        if !moved {
            return Err(worktree_error("This session's apply changed meanwhile."));
        }
        self.wake_waiting(session_id)?;
        shared
            .db
            .session_worktree(session_id)?
            .ok_or_else(|| worktree_error("This session has no worktree."))
    }

    /// Lets the worktree go: background commands stop, the branch tip is pinned, the folder and
    /// the branch are removed. Applied: from `applying`; discarded: from any state but
    /// `applying`. A part that cannot be removed yet leaves `leftover` for cleanup.
    pub async fn release_worktree(
        &self,
        session_id: &str,
        release: Release,
    ) -> Result<SessionWorktreeRow, CoreError> {
        let shared = &self.shared;
        self.ensure_idle(session_id)?;
        let row = shared
            .db
            .session_worktree(session_id)?
            .ok_or_else(|| worktree_error("This session has no worktree."))?;
        let (to, set) = match &release {
            Release::Applied(commit) => {
                if row.state != WorktreeState::Applying {
                    return Err(worktree_error("This session is not being applied."));
                }
                (
                    WorktreeState::Applied,
                    vec![(
                        "applied_commit",
                        rusqlite::types::Value::from(commit.clone()),
                    )],
                )
            }
            Release::Discarded => {
                if row.state.released() || row.state == WorktreeState::Applying {
                    return Err(worktree_error(blocked(row.state)));
                }
                (WorktreeState::Discarded, Vec::new())
            }
        };
        let mut set = set;
        set.push(("state", to.as_str().to_string().into()));
        // Under the `turns` lock: no turn starts between the idle check and the change.
        let changed = {
            let turns = shared.turns.lock().unwrap_or_else(|e| e.into_inner());
            if turns.contains_key(session_id) || self.working(session_id) {
                return Err(busy());
            }
            shared
                .db
                .update_session_worktree(session_id, row.state, &set)?
        };
        if !changed {
            return Err(worktree_error("This session's worktree changed meanwhile."));
        }
        self.stop_background(session_id);
        let top = shared.top();
        let row = SessionWorktreeRow { state: to, ..row };
        let gone = tokio::task::spawn_blocking({
            let (top, row) = (top.clone(), row.clone());
            move || remove_checkout(&top, &row)
        })
        .await
        .unwrap_or(false);
        let _ = shared
            .db
            .delete_worktree(&shared.project.id, &row.directory);
        if let Some(session) = shared.db.session(session_id)? {
            shared.forget_store(&session.directory);
        }
        if !gone {
            shared
                .db
                .update_session_worktree(session_id, to, &[("leftover", 1i64.into())])?;
        }
        shared
            .db
            .session_worktree(session_id)?
            .ok_or_else(|| worktree_error("This session has no worktree."))
    }

    /// Makes a `missing` or `failed` session's worktree again on its branch (or anew at its base
    /// when the branch is gone), with the files of the session's last snapshot put back.
    /// `preparing` again until the front end marks it ready.
    pub async fn recreate_worktree(
        &self,
        session_id: &str,
    ) -> Result<SessionWorktreeRow, CoreError> {
        let shared = &self.shared;
        self.ensure_idle(session_id)?;
        let row = shared
            .db
            .session_worktree(session_id)?
            .ok_or_else(|| worktree_error("This session has no worktree."))?;
        if !matches!(row.state, WorktreeState::Missing | WorktreeState::Failed) {
            return Err(worktree_error("This session's worktree is there."));
        }
        if !shared
            .db
            .cas_session_worktree(session_id, row.state, WorktreeState::Preparing, None)?
        {
            return Err(worktree_error("This session's worktree changed meanwhile."));
        }
        let fail = |message: String| -> CoreError {
            let _ = shared.db.cas_session_worktree(
                session_id,
                WorktreeState::Preparing,
                WorktreeState::Failed,
                Some(&message),
            );
            worktree_error(message)
        };
        let top = shared.top();
        let directory = PathBuf::from(&row.directory);
        let _ = git_sync(&top, &["worktree", "prune"], &[]);
        if directory.symlink_metadata().is_ok() {
            let _ = git_sync(
                &top,
                &["worktree", "remove", "--force", "--force", &row.directory],
                &[],
            );
            let _ = std::fs::remove_dir_all(&directory);
        }
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "-c".as_ref(),
            "core.longpaths=true".as_ref(),
            "worktree".as_ref(),
            "add".as_ref(),
            "--quiet".as_ref(),
        ];
        let existing = branch_exists(&top, &row.branch);
        if !existing {
            args.push("-b".as_ref());
            args.push(row.branch.as_ref());
        }
        args.push("--".as_ref());
        args.push(directory.as_os_str());
        args.push(if existing {
            row.branch.as_ref()
        } else {
            row.base_commit.as_ref()
        });
        crate::worktree::git(&top, &args)
            .await
            .map_err(|m| fail(format!("Git could not make the worktree: {m}")))?;
        shared.db.put_worktree(
            &shared.project.id,
            &row.directory,
            crate::worktree::SESSION_STRATEGY,
        )?;
        let session = shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        let _ = std::fs::create_dir_all(&session.directory);
        shared.forget_store(&session.directory);
        // The uncommitted work up to the last step comes back from its snapshot, but only when
        // that step ran on the commit the branch is at now. After an apply or a merge of the base
        // the branch moved on and holds that work already; laying the older files over it would
        // undo the main folder's commits that came in.
        let last = steps(&shared.db.messages(session_id)?)
            .into_iter()
            .rev()
            .find_map(|(_, s)| Some((s.end.or(s.start)?, s.head)));
        let tip = git_sync(
            &directory,
            &["rev-parse", "--verify", "--quiet", "HEAD"],
            &[],
        )
        .ok();
        if let (Some((tree, Some(head))), Some(store)) = (last, shared.store_for(&session))
            && tip.as_deref() == Some(head.as_str())
            && store.has(&format!("{tree}^{{tree}}"))
        {
            store
                .overlay(&tree, &directory)
                .map_err(|m| fail(format!("Could not put the files back: {m}")))?;
        }
        shared
            .db
            .session_worktree(session_id)?
            .ok_or_else(|| worktree_error("This session has no worktree."))
    }

    /// "Work in the main folder instead": only while the worktree is being made or failed and
    /// the session has not run in it. The worktree and branch go; the session's directory is the
    /// main folder again, and input that waited starts.
    pub async fn unisolate(&self, session_id: &str) -> Result<SessionRow, CoreError> {
        let shared = &self.shared;
        self.ensure_idle(session_id)?;
        let row = shared
            .db
            .session_worktree(session_id)?
            .ok_or_else(|| worktree_error("This session has no worktree."))?;
        if !matches!(row.state, WorktreeState::Preparing | WorktreeState::Failed) {
            return Err(worktree_error(
                "This session already works in its worktree; apply or discard it instead.",
            ));
        }
        if ran_since(&shared.db, session_id, row.time_created)? {
            return Err(worktree_error(
                "This session already worked in its worktree; apply or discard it instead.",
            ));
        }
        // Out of reach of a preparation that finishes meanwhile.
        if !shared
            .db
            .cas_session_worktree(session_id, row.state, WorktreeState::Discarded, None)?
        {
            return Err(worktree_error("This session's worktree changed meanwhile."));
        }
        self.stop_background(session_id);
        let top = shared.top();
        let gone = tokio::task::spawn_blocking({
            let (top, row) = (top.clone(), row.clone());
            move || remove_checkout_keeping(&top, &row, true)
        })
        .await
        .unwrap_or(false);
        let _ = shared
            .db
            .delete_worktree(&shared.project.id, &row.directory);
        if let Some(session) = shared.db.session(session_id)? {
            shared.forget_store(&session.directory);
        }
        shared
            .db
            .set_directory(session_id, &shared.directory.display().to_string())?;
        // The session is the main folder's again, so its row goes either way (a kept row would
        // pin it to the worktree). A folder that could not be removed yet (a program holds it)
        // is left for the user; nothing of the session's work was in it.
        if !gone {
            crate::log::warn(
                "a worktree folder could not be removed",
                &[("directory", &row.directory)],
            );
        }
        shared.db.delete_session_worktree(session_id)?;
        self.wake_waiting(session_id)?;
        shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)
    }

    /// Forks the session into a worktree of its own with the files put back as they were at the
    /// fork point (`before`: the message the fork stops before; `None`: all of it, files as now).
    /// The fork's worktree is `preparing`; the result says how exactly the files came back.
    pub async fn fork_into_worktree(
        &self,
        session_id: &str,
        before: Option<&str>,
    ) -> Result<(SessionRow, ForkRestore), CoreError> {
        let shared = &self.shared;
        let source = shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        let now_head = self.head_commit()?;
        let source_row = shared.worktree_row(&source);
        // The source's store: its live one, or a removed worktree's kept shadow store.
        let source_alive = Path::new(&source.directory).is_dir();
        let store: Option<Arc<Store>> = if source_alive {
            shared.store_for(&source)
        } else {
            source_row.as_ref().and_then(|row| {
                let dir = row.snapshot_dir.as_ref()?;
                Store::at(
                    PathBuf::from(dir),
                    PathBuf::from(&row.directory),
                    Path::new(&source.directory),
                )
                .map(Arc::new)
            })
        };
        let rows = shared.db.messages(&source.id)?;
        let cut = before
            .and_then(|b| rows.iter().position(|m| m.id == b))
            .unwrap_or(rows.len());
        let all = steps(&rows);
        // Where the source's checkout is now.
        let source_head = || -> String {
            if source_alive
                && let Ok(head) = git_sync(
                    Path::new(&source.directory),
                    &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
                    &[],
                )
                && !head.is_empty()
            {
                return head;
            }
            source_row
                .as_ref()
                .map(|r| r.base_commit.clone())
                .unwrap_or_else(|| now_head.clone())
        };
        let base_branch = match &source_row {
            Some(row) => row.base_branch.clone(),
            None => git_sync(
                &shared.top(),
                &["symbolic-ref", "-q", "--short", "HEAD"],
                &[],
            )
            .ok()
            .filter(|b| !b.is_empty()),
        };

        /// What goes into the fork's worktree.
        enum Plan {
            /// A tree from `git_dir`'s objects (scope) over `commit`.
            Tree {
                commit: String,
                git_dir: PathBuf,
                scope: String,
                tree: String,
                fallback: Option<Fallback>,
                /// Then each file back as its tree has it (the "edits" path).
                undo: Vec<(String, String)>,
            },
            /// Only the commit: the session changed nothing (no fallback), or nothing of its
            /// files is left to put back (`Now`).
            Commit(String, Option<Fallback>),
            /// The commit, then only the session's own files as the fork point's tree has them
            /// (history from before heads were recorded): no other uncommitted change of the
            /// source comes along, and no commit made since is undone.
            Files(String, Vec<(String, String)>),
        }
        let scope_of = || {
            let top = shared.top();
            let rel = canonical(Path::new(&source.directory))
                .strip_prefix(
                    source_row
                        .as_ref()
                        .map_or(top.clone(), |r| PathBuf::from(&r.directory)),
                )
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if rel.is_empty() { ".".to_string() } else { rel }
        };
        // The source's files as they are now, from its store or (without one) its own index.
        let current = |fallback: Option<Fallback>, undo: Vec<(String, String)>| -> Option<Plan> {
            if !source_alive {
                return None;
            }
            if let Some(store) = &store
                && let Some(tree) = store.capture()
            {
                return Some(Plan::Tree {
                    commit: store.head().unwrap_or_else(source_head),
                    git_dir: store.git_dir().to_path_buf(),
                    scope: store.scope().to_string(),
                    tree,
                    fallback,
                    undo,
                });
            }
            let top = PathBuf::from(
                git_sync(
                    Path::new(&source.directory),
                    &["rev-parse", "--show-toplevel"],
                    &[],
                )
                .ok()?,
            );
            let (git_dir, tree) = crate::worktree::current_tree(&top)?;
            Some(Plan::Tree {
                commit: source_head(),
                git_dir,
                scope: scope_of(),
                tree,
                fallback,
                undo,
            })
        };
        // The source's last recorded files, for a source whose folder is gone.
        let last_recorded = |fallback: Option<Fallback>| -> Plan {
            let last = all.iter().rev().find_map(|(_, s)| {
                let tree = s.end.clone().or(s.start.clone())?;
                Some((tree, s.head.clone()))
            });
            match (last, &store) {
                (Some((tree, head)), Some(store)) if store.has(&format!("{tree}^{{tree}}")) => {
                    Plan::Tree {
                        commit: head
                            .filter(|h| store.has(&format!("{h}^{{commit}}")))
                            .unwrap_or_else(source_head),
                        git_dir: store.git_dir().to_path_buf(),
                        scope: store.scope().to_string(),
                        tree,
                        fallback,
                        undo: Vec::new(),
                    }
                }
                // Nothing recorded: the session changed nothing.
                (None, _) => Plan::Commit(source_head(), None),
                _ => Plan::Commit(source_head(), Some(Fallback::Now)),
            }
        };
        let now_or_last = |fallback: Option<Fallback>| {
            current(fallback, Vec::new()).unwrap_or_else(|| last_recorded(fallback))
        };

        let plan = if before.is_none() {
            now_or_last(None)
        } else if store.is_none() {
            // Nothing recorded the fork point (`snapshots: false`).
            now_or_last(Some(Fallback::Now))
        } else {
            // The first step at or after the fork point (its start: the user's own edits between
            // turns included), else the last one before it (its end).
            let point = all
                .iter()
                .find(|(i, s)| *i >= cut && s.start.is_some())
                .map(|(_, s)| (s.start.clone(), s.head.clone()))
                .or_else(|| {
                    all.iter()
                        .rev()
                        .find(|(i, _)| *i < cut)
                        .map(|(_, s)| (s.end.clone().or(s.start.clone()), s.head.clone()))
                });
            match point {
                None => Plan::Commit(
                    match &source_row {
                        Some(row) => row.base_commit.clone(),
                        None => source_head(),
                    },
                    None,
                ),
                Some((tree, head)) => {
                    let readable = |spec: String| store.as_ref().is_some_and(|s| s.has(&spec));
                    match (tree, head) {
                        (Some(tree), Some(head))
                            if readable(format!("{tree}^{{tree}}"))
                                && readable(format!("{head}^{{commit}}")) =>
                        {
                            let store = store.as_ref().expect("readable");
                            Plan::Tree {
                                commit: head,
                                git_dir: store.git_dir().to_path_buf(),
                                scope: store.scope().to_string(),
                                tree,
                                fallback: None,
                                undo: Vec::new(),
                            }
                        }
                        (Some(tree), None)
                            if store.is_some() && readable(format!("{tree}^{{tree}}")) =>
                        {
                            // History from before heads were recorded: the source's commit now,
                            // with each file the session changed as the fork point had it.
                            let mut files: Vec<(String, String)> = Vec::new();
                            for (_, step) in &all {
                                for file in step.files.iter().flatten() {
                                    if !files.iter().any(|(f, _)| f == file) {
                                        files.push((file.clone(), tree.clone()));
                                    }
                                }
                            }
                            Plan::Files(source_head(), files)
                        }
                        _ => now_or_last(Some(Fallback::Now)),
                    }
                }
            }
        };

        let fork = self.fork_before(&source, before)?;
        let commit = match &plan {
            Plan::Tree { commit, .. } | Plan::Commit(commit, _) | Plan::Files(commit, _) => {
                commit.clone()
            }
        };
        let (_, made) = self
            .make_checkout(&fork, &commit, base_branch, None)
            .await?;
        let fail = |message: String| -> CoreError {
            let _ = shared.db.cas_session_worktree(
                &fork.id,
                WorktreeState::Preparing,
                WorktreeState::Failed,
                Some(&message),
            );
            worktree_error(message)
        };
        let restore = match plan {
            Plan::Commit(_, fallback) => ForkRestore {
                exact: fallback.is_none(),
                kept: Vec::new(),
                fallback,
            },
            Plan::Files(_, files) => {
                if let Some(store) = &store {
                    store
                        .restore_into(&files, &made.directory)
                        .map_err(|m| fail(format!("Could not put the files back: {m}")))?;
                }
                ForkRestore {
                    exact: false,
                    kept: Vec::new(),
                    fallback: Some(Fallback::Edits),
                }
            }
            Plan::Tree {
                git_dir,
                scope,
                tree,
                fallback,
                undo,
                ..
            } => {
                let kept = crate::snapshot::overlay(&git_dir, &scope, &tree, &made.directory)
                    .map_err(|m| fail(format!("Could not put the files back: {m}")))?;
                if !undo.is_empty()
                    && let Some(store) = &store
                {
                    store
                        .restore_into(&undo, &made.directory)
                        .map_err(|m| fail(format!("Could not put the files back: {m}")))?;
                }
                ForkRestore {
                    exact: fallback.is_none() && kept.is_empty(),
                    kept,
                    fallback,
                }
            }
        };
        let _ = made.branch;
        // The copied history's snapshots stay readable from the fork's own store.
        let fork_row = shared
            .db
            .session(&fork.id)?
            .ok_or(CoreError::SessionNotFound)?;
        if let (Some(source_store), Some(fork_store)) = (&store, shared.store_for(&fork_row)) {
            fork_store.borrow_objects(source_store.git_dir());
        }
        shared.db.update_session_worktree(
            &fork.id,
            WorktreeState::Preparing,
            &[(
                "restored",
                serde_json::to_string(&restore).unwrap_or_default().into(),
            )],
        )?;
        Ok((fork_row, restore))
    }
}

/// The message an apply's stash of main-folder edits carries, so a crash can't lose track of it.
pub fn apply_stash_message(session_id: &str) -> String {
    format!("Anysola Code: set aside to apply {session_id}")
}

/// The `stash@{n}` holding the session's apply stash, if one is left.
fn find_stash(top: &Path, session_id: &str) -> Option<String> {
    let wanted = apply_stash_message(session_id);
    git_sync(top, &["stash", "list", "--format=%gd%x00%s"], &[])
        .ok()?
        .lines()
        .find_map(|line| {
            let (name, subject) = line.split_once('\0')?;
            subject.ends_with(&wanted).then(|| name.to_string())
        })
}

/// Puts an apply's stash back after its commit landed; on a conflict the main folder's touched
/// files stay as committed and the stash is kept. The sentence to show when it is kept.
fn pop_apply_stash(top: &Path, session_id: &str) -> Option<String> {
    let name = find_stash(top, session_id)?;
    let files: Vec<String> = git_sync(
        top,
        &["stash", "show", "--name-only", "--include-untracked", &name],
        &[],
    )
    .unwrap_or_default()
    .lines()
    .filter(|l| !l.is_empty())
    .map(str::to_string)
    .collect();
    if git_sync(top, &["stash", "pop", "--quiet", &name], &[]).is_ok() {
        return None;
    }
    for file in &files {
        let spec = format!(":(literal){file}");
        let _ = git_sync(top, &["reset", "-q", "--", &spec], &[]);
        let _ = git_sync(top, &["checkout", "HEAD", "--", &spec], &[]);
    }
    let name = find_stash(top, session_id).unwrap_or(name);
    Some(format!(
        "Your main-folder edits set aside for an apply are kept in the stash: {name} in git stash list."
    ))
}

/// A deleted session's worktree: its folder, branch and hidden refs, and its rows.
fn drop_deleted(db: &Db, top: &Path, row: &SessionWorktreeRow) {
    remove_checkout(top, row);
    unpin(top, &row.session_id);
    let _ = db.delete_worktree(&row.project_id, &row.directory);
    let _ = db.delete_session_worktree(&row.session_id);
}

/// After a session was deleted: its worktree, branch and hidden refs go now instead of at the
/// next start. Nothing happens while the session still exists. False when something is left
/// for the start-up cleanup (the main folder is gone, or the folder is held).
pub fn forget_deleted(db: &Db, session_id: &str) -> bool {
    let Ok(Some(row)) = db.session_worktree(session_id) else {
        return true;
    };
    if db.session_exists(session_id).unwrap_or(true) {
        return false;
    }
    let Ok(top) = git_sync(
        Path::new(&row.folder),
        &["rev-parse", "--show-toplevel"],
        &[],
    ) else {
        return false;
    };
    let top = canonical(Path::new(&top));
    drop_deleted(db, &top, &row);
    !Path::new(&row.directory).exists()
}

/// What [`cleanup`] did with one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub session_id: String,
    pub folder: String,
    /// The row's state afterwards; `None` when the row is gone (its session was deleted).
    pub state: Option<WorktreeState>,
    /// A sentence for the user, when there is one.
    pub notice: Option<String>,
}

/// Start-up cleanup of sessions' worktrees, before any core runs a turn: rows of deleted
/// sessions go with their folder, branch and hidden refs; leftovers are removed again; a
/// preparation cut off is `failed`; a ready worktree whose folder is gone is `missing`; an apply
/// cut off is finished when the main folder shows it was, else the main folder is left alone.
/// Only rows Anysola Code wrote are touched; worktrees the agent made are never cleaned.
pub fn cleanup(db: &Db) -> Vec<Cleaned> {
    let mut out = Vec::new();
    for row in db.session_worktrees().unwrap_or_default() {
        let folder = PathBuf::from(&row.folder);
        let Ok(top) = git_sync(&folder, &["rev-parse", "--show-toplevel"], &[]) else {
            // The main folder is gone or no longer a repository: nothing to act on.
            continue;
        };
        let top = canonical(Path::new(&top));
        let _ = git_sync(&top, &["worktree", "prune"], &[]);
        let directory = Path::new(&row.directory);
        let mut cleaned = Cleaned {
            session_id: row.session_id.clone(),
            folder: row.folder.clone(),
            state: Some(row.state),
            notice: None,
        };
        let set = |to: WorktreeState, extra: &[(&str, rusqlite::types::Value)]| {
            let mut all: Vec<(&str, rusqlite::types::Value)> =
                vec![("state", to.as_str().to_string().into())];
            all.extend(extra.iter().cloned());
            db.update_session_worktree(&row.session_id, row.state, &all)
                .unwrap_or(false)
        };
        if !db.session_exists(&row.session_id).unwrap_or(true) {
            drop_deleted(db, &top, &row);
            cleaned.state = None;
            out.push(cleaned);
            continue;
        }
        match row.state {
            WorktreeState::Applied | WorktreeState::Discarded => {
                if row.leftover && remove_checkout(&top, &row) {
                    let _ = db.update_session_worktree(
                        &row.session_id,
                        row.state,
                        &[("leftover", 0i64.into())],
                    );
                    let _ = db.delete_worktree(&row.project_id, &row.directory);
                } else {
                    continue;
                }
            }
            WorktreeState::Preparing => {
                // The partial folder goes. Its branch too, unless work was committed on it (a
                // worktree being made again after it went missing): "Try again" uses it then. The
                // hidden refs stay: the session's recorded steps may name them.
                remove_checkout_keeping(&top, &row, true);
                let notice = "Making the worktree was cut off.";
                if set(
                    WorktreeState::Failed,
                    &[("error", notice.to_string().into())],
                ) {
                    cleaned.state = Some(WorktreeState::Failed);
                    cleaned.notice = Some(notice.into());
                }
            }
            WorktreeState::Ready if !directory.is_dir() => {
                if set(WorktreeState::Missing, &[]) {
                    cleaned.state = Some(WorktreeState::Missing);
                    cleaned.notice = Some("This session's folder is gone.".into());
                }
            }
            WorktreeState::Applying => {
                let head = git_sync(&top, &["rev-parse", "--verify", "--quiet", "HEAD"], &[]).ok();
                let parent =
                    git_sync(&top, &["rev-parse", "--verify", "--quiet", "HEAD^"], &[]).ok();
                let tree = git_sync(
                    &top,
                    &["rev-parse", "--verify", "--quiet", "HEAD^{tree}"],
                    &[],
                )
                .ok();
                let finished = row.apply_from.is_some()
                    && parent == row.apply_from
                    && tree.is_some()
                    && tree == row.apply_tree;
                if finished && let Some(head) = head {
                    // The commit landed: the session goes on from it.
                    if let Ok(tip) = git_sync(directory, &["rev-parse", "HEAD"], &[]) {
                        pin(&top, &row.session_id, &tip);
                    }
                    let reset = git_sync(directory, &["reset", "--hard", "--quiet", &head], &[]);
                    let mut extra: Vec<(&str, rusqlite::types::Value)> = vec![
                        ("applied_commit", head.clone().into()),
                        ("apply_from", rusqlite::types::Value::Null),
                        ("apply_tree", rusqlite::types::Value::Null),
                    ];
                    if reset.is_ok() {
                        extra.push(("base_commit", head.into()));
                    }
                    let kept = pop_apply_stash(&top, &row.session_id);
                    if let Some(kept) = &kept {
                        extra.push(("error", kept.clone().into()));
                    }
                    if set(WorktreeState::Ready, &extra) {
                        cleaned.state = Some(WorktreeState::Ready);
                        cleaned.notice = kept;
                    }
                } else {
                    let notice = match find_stash(&top, &row.session_id) {
                        Some(name) => format!(
                            "An apply was cut off. Check the main folder's changes before applying again; your edits set aside for it are in the stash: {name} in git stash list."
                        ),
                        None => "An apply was cut off. Check the main folder's changes before applying again.".to_string(),
                    };
                    let notice = notice.as_str();
                    if set(
                        WorktreeState::Ready,
                        &[
                            ("error", notice.to_string().into()),
                            ("apply_from", rusqlite::types::Value::Null),
                            ("apply_tree", rusqlite::types::Value::Null),
                        ],
                    ) {
                        cleaned.state = Some(WorktreeState::Ready);
                        cleaned.notice = Some(notice.into());
                    }
                }
            }
            WorktreeState::Ready | WorktreeState::Missing | WorktreeState::Failed => continue,
        }
        out.push(cleaned);
    }
    out
}
