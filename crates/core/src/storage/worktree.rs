//! `session_worktree`: sessions that work in their own git worktree. Every state change is a
//! compare-and-set, so a double click or a racing cleanup cannot apply or remove twice.

use rusqlite::{OptionalExtension, Row, params};

use super::{Db, Result};
use crate::time::now_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeState {
    /// Being made (checkout, env files, setup); the first message waits for it.
    Preparing,
    Ready,
    /// The folder is gone (deleted outside the app).
    Missing,
    /// Making it failed or was cut off; `error` says why.
    Failed,
    Applying,
    Applied,
    Discarded,
}

impl WorktreeState {
    pub fn as_str(self) -> &'static str {
        match self {
            WorktreeState::Preparing => "preparing",
            WorktreeState::Ready => "ready",
            WorktreeState::Missing => "missing",
            WorktreeState::Failed => "failed",
            WorktreeState::Applying => "applying",
            WorktreeState::Applied => "applied",
            WorktreeState::Discarded => "discarded",
        }
    }

    pub fn parse(text: &str) -> Option<WorktreeState> {
        Some(match text {
            "preparing" => WorktreeState::Preparing,
            "ready" => WorktreeState::Ready,
            "missing" => WorktreeState::Missing,
            "failed" => WorktreeState::Failed,
            "applying" => WorktreeState::Applying,
            "applied" => WorktreeState::Applied,
            "discarded" => WorktreeState::Discarded,
            _ => return None,
        })
    }

    /// Removed for good: the folder and branch are gone (or going).
    pub fn released(self) -> bool {
        matches!(self, WorktreeState::Applied | WorktreeState::Discarded)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionWorktreeRow {
    pub session_id: String,
    pub project_id: String,
    /// The main folder as the window opened it (the core's key).
    pub folder: String,
    /// The canonical worktree top.
    pub directory: String,
    /// `anysola/<slug>`.
    pub branch: String,
    /// The branch the worktree was made from; `None` when the main folder was on a detached HEAD.
    pub base_branch: Option<String>,
    /// What the session's changes are measured from.
    pub base_commit: String,
    /// The worktree's shadow snapshot store, kept for forks after removal.
    pub snapshot_dir: Option<String>,
    pub state: WorktreeState,
    pub error: Option<String>,
    pub applied_commit: Option<String>,
    /// The main folder's HEAD when an apply started (crash recovery).
    pub apply_from: Option<String>,
    /// The tree the apply puts on the main folder (crash recovery).
    pub apply_tree: Option<String>,
    /// The fork restore report, JSON.
    pub restored: Option<serde_json::Value>,
    /// The folder or branch could not be removed yet.
    pub leftover: bool,
    pub time_created: u64,
    pub time_updated: u64,
}

const COLUMNS: &str = "session_id, project_id, folder, directory, branch, base_branch, base_commit,
    snapshot_dir, state, error, applied_commit, apply_from, apply_tree, restored, leftover,
    time_created, time_updated";

fn row(r: &Row) -> rusqlite::Result<SessionWorktreeRow> {
    let state: String = r.get(8)?;
    Ok(SessionWorktreeRow {
        session_id: r.get(0)?,
        project_id: r.get(1)?,
        folder: r.get(2)?,
        directory: r.get(3)?,
        branch: r.get(4)?,
        base_branch: r.get(5)?,
        base_commit: r.get(6)?,
        snapshot_dir: r.get(7)?,
        // An unknown state (a newer version wrote it) reads as failed: nothing acts on it.
        state: WorktreeState::parse(&state).unwrap_or(WorktreeState::Failed),
        error: r.get(9)?,
        applied_commit: r.get(10)?,
        apply_from: r.get(11)?,
        apply_tree: r.get(12)?,
        restored: super::parse(r.get(13)?),
        leftover: r.get::<_, i64>(14)? != 0,
        time_created: r.get::<_, i64>(15)? as u64,
        time_updated: r.get::<_, i64>(16)? as u64,
    })
}

impl Db {
    pub fn insert_session_worktree(&self, w: &SessionWorktreeRow) -> Result<()> {
        self.with(|c| {
            c.execute(
                &format!(
                    "INSERT INTO session_worktree ({COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
                ),
                params![
                    w.session_id,
                    w.project_id,
                    w.folder,
                    w.directory,
                    w.branch,
                    w.base_branch,
                    w.base_commit,
                    w.snapshot_dir,
                    w.state.as_str(),
                    w.error,
                    w.applied_commit,
                    w.apply_from,
                    w.apply_tree,
                    w.restored.as_ref().map(super::json),
                    w.leftover as i64,
                    w.time_created as i64,
                    w.time_updated as i64,
                ],
            )
            .map(drop)
        })
    }

    pub fn session_worktree(&self, session_id: &str) -> Result<Option<SessionWorktreeRow>> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT {COLUMNS} FROM session_worktree WHERE session_id = ?1"),
                [session_id],
                row,
            )
            .optional()
        })
    }

    /// Every row, oldest first (for the session list and cleanup).
    pub fn session_worktrees(&self) -> Result<Vec<SessionWorktreeRow>> {
        self.with(|c| {
            let mut statement = c.prepare(&format!(
                "SELECT {COLUMNS} FROM session_worktree ORDER BY time_created ASC, session_id ASC"
            ))?;
            let rows = statement.query_map([], row)?;
            rows.collect()
        })
    }

    /// Moves the row from `from` to `to`; false when it was not in `from` (someone else moved it).
    pub fn cas_session_worktree(
        &self,
        session_id: &str,
        from: WorktreeState,
        to: WorktreeState,
        error: Option<&str>,
    ) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE session_worktree SET state = ?3, error = ?4, time_updated = ?5
                 WHERE session_id = ?1 AND state = ?2",
                params![
                    session_id,
                    from.as_str(),
                    to.as_str(),
                    error,
                    now_ms() as i64
                ],
            )
            .map(|n| n == 1)
        })
    }

    /// Sets the named columns of a row while it is in `state`; false otherwise.
    pub(crate) fn update_session_worktree(
        &self,
        session_id: &str,
        state: WorktreeState,
        set: &[(&str, rusqlite::types::Value)],
    ) -> Result<bool> {
        const ALLOWED: &[&str] = &[
            "directory",
            "branch",
            "base_branch",
            "base_commit",
            "snapshot_dir",
            "state",
            "error",
            "applied_commit",
            "apply_from",
            "apply_tree",
            "restored",
            "leftover",
        ];
        let mut sql = String::from("UPDATE session_worktree SET time_updated = ?1");
        let mut values: Vec<rusqlite::types::Value> = vec![(now_ms() as i64).into()];
        for (column, value) in set {
            assert!(ALLOWED.contains(column), "unknown column {column}");
            values.push(value.clone());
            sql.push_str(&format!(", {column} = ?{}", values.len()));
        }
        values.push(session_id.to_string().into());
        sql.push_str(&format!(" WHERE session_id = ?{}", values.len()));
        values.push(state.as_str().to_string().into());
        sql.push_str(&format!(" AND state = ?{}", values.len()));
        self.with(|c| {
            c.execute(&sql, rusqlite::params_from_iter(values))
                .map(|n| n == 1)
        })
    }

    pub fn delete_session_worktree(&self, session_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM session_worktree WHERE session_id = ?1",
                [session_id],
            )
            .map(drop)
        })
    }

    /// Whether a session row exists (cleanup: rows outlive deleted sessions).
    pub(crate) fn session_exists(&self, session_id: &str) -> Result<bool> {
        self.with(|c| {
            c.query_row("SELECT 1 FROM session WHERE id = ?1", [session_id], |_| {
                Ok(())
            })
            .optional()
            .map(|r| r.is_some())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> SessionWorktreeRow {
        SessionWorktreeRow {
            session_id: id.into(),
            project_id: "p".into(),
            folder: "/f".into(),
            directory: "/w".into(),
            branch: "anysola/x".into(),
            base_branch: Some("main".into()),
            base_commit: "abc".into(),
            snapshot_dir: None,
            state: WorktreeState::Preparing,
            error: None,
            applied_commit: None,
            apply_from: None,
            apply_tree: None,
            restored: None,
            leftover: false,
            time_created: 1,
            time_updated: 1,
        }
    }

    #[test]
    fn rows_change_state_only_from_the_state_they_are_in() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("db")).unwrap();
        db.insert_session_worktree(&sample("s1")).unwrap();
        let ready = WorktreeState::Ready;
        assert!(
            !db.cas_session_worktree("s1", ready, WorktreeState::Applying, None)
                .unwrap()
        );
        assert!(
            db.cas_session_worktree("s1", WorktreeState::Preparing, ready, None)
                .unwrap()
        );
        // A second identical move finds it already moved.
        assert!(
            !db.cas_session_worktree("s1", WorktreeState::Preparing, ready, None)
                .unwrap()
        );
        assert!(
            db.update_session_worktree(
                "s1",
                ready,
                &[
                    ("base_commit", "def".to_string().into()),
                    ("leftover", 1i64.into())
                ]
            )
            .unwrap()
        );
        let row = db.session_worktree("s1").unwrap().unwrap();
        assert_eq!(
            (row.state, row.base_commit.as_str(), row.leftover),
            (ready, "def", true)
        );
        assert!(
            !db.update_session_worktree("s1", WorktreeState::Failed, &[("leftover", 0i64.into())])
                .unwrap()
        );
        assert_eq!(db.session_worktrees().unwrap().len(), 1);
        db.delete_session_worktree("s1").unwrap();
        assert!(db.session_worktree("s1").unwrap().is_none());
    }
}
