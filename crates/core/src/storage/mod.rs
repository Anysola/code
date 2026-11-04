//! `~/.anysola/anysola.db`: bundled SQLite in WAL mode, shared by every `anysola` process.
//! Timestamps are epoch milliseconds. A session driven by one process carries a busy marker that
//! other processes respect.

mod inbox;
mod instruction;
mod permission;
mod revert;
mod schema;
mod transfer;

pub use inbox::InboxRow;
pub use instruction::{InstructionState, Values};
pub use transfer::{ImportError, SessionSummary, sanitize};

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::time::now_ms;

/// A busy marker older than this is considered abandoned.
pub const BUSY_STALE_MS: u64 = 15_000;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cannot open the database: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub id: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub fork_session_id: Option<String>,
    pub fork_boundary: Option<Value>,
    pub slug: String,
    pub directory: String,
    pub title: String,
    pub version: String,
    pub agent: Option<String>,
    /// `{"id","providerID","variant"?}`
    pub model: Option<Value>,
    pub time_created: u64,
    pub time_updated: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRow {
    pub id: String,
    pub seq: i64,
    pub kind: String,
    pub data: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenTotals {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

fn json(value: &Value) -> String {
    crate::json::compact(value)
}

fn parse(text: Option<String>) -> Option<Value> {
    text.and_then(|t| serde_json::from_str(&t).ok())
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if path.exists() {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let mut options = std::fs::OpenOptions::new();
            options.create(true).append(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            options.open(path)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000; \
             PRAGMA cache_size = -64000; PRAGMA foreign_keys = ON;",
        )?;
        migrate(&conn)?;
        for suffix in ["", "-wal", "-shm"] {
            restrict(Path::new(&format!("{}{suffix}", path.display())));
        }
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(f(&mut conn)?)
    }

    pub fn kv_get(&self, key: &str) -> Result<Option<Value>> {
        self.with(|c| {
            c.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
        })
        .map(parse_opt)
    }

    pub fn kv_set(&self, key: &str, value: &Value) -> Result<()> {
        let now = now_ms() as i64;
        self.with(|c| {
            c.execute(
                "INSERT INTO kv (key, value, time_created, time_updated) VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, time_updated = excluded.time_updated",
                params![key, json(value), now],
            )
            .map(drop)
        })
    }

    pub fn kv_remove(&self, key: &str) -> Result<()> {
        self.with(|c| c.execute("DELETE FROM kv WHERE key = ?1", [key]).map(drop))
    }

    /// The access token of the active Anysola device login, if any.
    pub fn anysola_login(&self) -> Result<Option<String>> {
        Ok(self.anysola_credential()?.map(|(_, token)| token))
    }

    /// The credential id and token of the active Anysola device login.
    pub fn anysola_credential(&self) -> Result<Option<(String, String)>> {
        let row = self.with(|c| {
            c.query_row(
                "SELECT id, value FROM credential WHERE integration_id = 'anysola' AND active = 1
                 ORDER BY time_updated DESC LIMIT 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
        })?;
        Ok(row.and_then(|(id, value)| {
            let value: Value = serde_json::from_str(&value).ok()?;
            let token = value
                .get("access")
                .or_else(|| value.get("key"))
                .and_then(Value::as_str)?;
            Some((id, token.to_string()))
        }))
    }

    /// Stores a device login as the active Anysola credential; returns its id.
    pub fn insert_anysola_login(&self, value: &Value) -> Result<String> {
        let id = crate::ids::credential();
        let now = now_ms() as i64;
        self.with(|c| {
            c.execute(
                "INSERT INTO credential (id, integration_id, label, connector_id, method_id, active, value, time_created, time_updated)
                 VALUES (?1, 'anysola', 'Anysola account', NULL, NULL, 1, ?2, ?3, ?3)",
                params![id, json(value), now],
            )
            .map(drop)
        })?;
        Ok(id)
    }

    pub fn remove_credential(&self, id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM credential WHERE id = ?1", [id])
                .map(drop)
        })
    }

    pub fn upsert_project(&self, project: &crate::project::Project) -> Result<()> {
        let now = now_ms() as i64;
        let worktree = project.worktree.display().to_string();
        self.with(|c| {
            c.execute(
                "INSERT INTO project (id, worktree, vcs, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(id) DO UPDATE SET time_updated = excluded.time_updated",
                params![project.id, worktree, project.vcs, now],
            )?;
            // The checkout itself is an unowned row: no worktree strategy created it.
            c.execute(
                "INSERT OR IGNORE INTO worktree (project_id, directory, strategy, time_created) VALUES (?1, ?2, NULL, ?3)",
                params![project.id, worktree, now],
            )
            .map(drop)
        })
    }

    pub fn create_session(&self, row: &SessionRow) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO session (id, project_id, parent_id, fork_session_id, fork_boundary, slug, directory, title,
                    version, agent, model, time_created, time_updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    row.id,
                    row.project_id,
                    row.parent_id,
                    row.fork_session_id,
                    row.fork_boundary.as_ref().map(json),
                    row.slug,
                    row.directory,
                    row.title,
                    row.version,
                    row.agent,
                    row.model.as_ref().map(json),
                    row.time_created as i64,
                    row.time_updated as i64
                ],
            )
            .map(drop)
        })
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT id, project_id, parent_id, fork_session_id, fork_boundary, slug, directory, title, version,
                        agent, model, time_created, time_updated FROM session WHERE id = ?1",
                [id],
                session_row,
            )
            .optional()
        })
    }

    /// The most recently updated root session whose directory is `directory`.
    pub fn latest_root_session(&self, directory: &str) -> Result<Option<SessionRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT id, project_id, parent_id, fork_session_id, fork_boundary, slug, directory, title, version,
                        agent, model, time_created, time_updated FROM session
                 WHERE directory = ?1 AND parent_id IS NULL AND time_archived IS NULL
                 ORDER BY time_updated DESC, id ASC LIMIT 1",
                [directory],
                session_row,
            )
            .optional()
        })
    }

    /// Titles of sessions forked from `source`, to number a new fork.
    pub fn fork_count(&self, source: &str) -> Result<u64> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM session WHERE fork_session_id = ?1",
                [source],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as u64)
        })
    }

    pub fn set_title(&self, id: &str, title: &str) -> Result<()> {
        self.touch("title = ?2", params![id, title, now_ms() as i64])
    }

    pub fn set_directory(&self, id: &str, directory: &str) -> Result<()> {
        self.touch("directory = ?2", params![id, directory, now_ms() as i64])
    }

    /// A project's worktrees, newest first: `(directory, strategy)`.
    pub fn worktrees(&self, project_id: &str) -> Result<Vec<(String, Option<String>)>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT directory, strategy FROM worktree WHERE project_id = ?1 ORDER BY time_created DESC, directory ASC",
            )?;
            let rows = statement.query_map([project_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
    }

    pub fn put_worktree(&self, project_id: &str, directory: &str, strategy: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO worktree (project_id, directory, strategy, time_created) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(project_id, directory) DO UPDATE SET strategy = excluded.strategy",
                params![project_id, directory, strategy, now_ms() as i64],
            )
            .map(drop)
        })
    }

    pub fn delete_worktree(&self, project_id: &str, directory: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM worktree WHERE project_id = ?1 AND directory = ?2",
                params![project_id, directory],
            )
            .map(drop)
        })
    }

    pub fn set_agent(&self, id: &str, agent: &str) -> Result<()> {
        self.touch("agent = ?2", params![id, agent, now_ms() as i64])
    }

    pub fn set_model(&self, id: &str, model: &Value) -> Result<()> {
        self.touch("model = ?2", params![id, json(model), now_ms() as i64])
    }

    pub fn add_tokens(&self, id: &str, tokens: TokenTotals) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE session SET tokens_input = tokens_input + ?2, tokens_output = tokens_output + ?3,
                    tokens_reasoning = tokens_reasoning + ?4, tokens_cache_read = tokens_cache_read + ?5,
                    tokens_cache_write = tokens_cache_write + ?6, time_updated = ?7 WHERE id = ?1",
                params![
                    id,
                    tokens.input as i64,
                    tokens.output as i64,
                    tokens.reasoning as i64,
                    tokens.cache_read as i64,
                    tokens.cache_write as i64,
                    now_ms() as i64
                ],
            )
            .map(drop)
        })
    }

    /// Records the end of an execution: `succeeded`, `failed` or `interrupted`.
    pub fn set_idle(&self, id: &str, outcome: &str) -> Result<()> {
        self.touch(
            "time_idle = ?3, idle_outcome = ?2",
            params![id, outcome, now_ms() as i64],
        )
    }

    fn touch(&self, set: &str, params: &[&dyn rusqlite::ToSql]) -> Result<()> {
        let sql = format!("UPDATE session SET {set}, time_updated = ?3 WHERE id = ?1");
        self.with(|c| c.execute(&sql, params).map(drop))
    }

    /// Appends a message with the session's next sequence number.
    pub fn append_message(
        &self,
        session_id: &str,
        id: &str,
        kind: &str,
        data: &Value,
    ) -> Result<i64> {
        let now = now_ms() as i64;
        self.with(|c| {
            let tx = c.transaction()?;
            let seq: i64 = tx.query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM message WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO message (id, session_id, seq, type, time_created, time_updated, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
                params![id, session_id, seq, kind, now, json(data)],
            )?;
            tx.execute(
                "UPDATE session SET time_updated = ?2 WHERE id = ?1",
                params![session_id, now],
            )?;
            tx.commit()?;
            Ok(seq)
        })
    }

    pub fn update_message(&self, id: &str, data: &Value) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE message SET data = ?2, time_updated = ?3 WHERE id = ?1",
                params![id, json(data), now_ms() as i64],
            )
            .map(drop)
        })
    }

    pub fn messages(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT id, seq, type, data FROM message WHERE session_id = ?1 ORDER BY seq ASC",
            )?;
            let rows = statement.query_map([session_id], |r| {
                Ok(MessageRow {
                    id: r.get(0)?,
                    seq: r.get(1)?,
                    kind: r.get(2)?,
                    data: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or(Value::Null),
                })
            })?;
            rows.collect()
        })
    }

    /// Copies a session's messages into a fork. Copied ids get the source sequence as a suffix,
    /// as 0.5.1 named forked rows.
    pub fn copy_messages(&self, from: &str, to: &str) -> Result<()> {
        let rows = self.messages(from)?;
        for row in rows {
            let id = format!("{}_{}", row.id, row.seq);
            self.append_message(to, &id, &row.kind, &row.data)?;
        }
        Ok(())
    }

    /// Marks a session busy for `owner` unless another live owner holds it.
    pub fn acquire(&self, session_id: &str, owner: &str) -> Result<bool> {
        let now = now_ms() as i64;
        self.with(|c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let current: Option<(Option<String>, Option<i64>)> = tx
                .query_row("SELECT busy_owner, busy_heartbeat FROM session WHERE id = ?1", [session_id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            if let Some((Some(holder), Some(beat))) = current
                && holder != owner
                && now - beat < BUSY_STALE_MS as i64
                && owner_alive(&holder)
            {
                return Ok(false);
            }
            tx.execute(
                "UPDATE session SET busy_owner = ?2, busy_heartbeat = ?3, time_busy = ?3 WHERE id = ?1",
                params![session_id, owner, now],
            )?;
            tx.commit()?;
            Ok(true)
        })
    }

    pub fn heartbeat(&self, session_id: &str, owner: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE session SET busy_heartbeat = ?3 WHERE id = ?1 AND busy_owner = ?2",
                params![session_id, owner, now_ms() as i64],
            )
            .map(drop)
        })
    }

    pub fn release(&self, session_id: &str, owner: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE session SET busy_owner = NULL, busy_heartbeat = NULL WHERE id = ?1 AND busy_owner = ?2",
                params![session_id, owner],
            )
            .map(drop)
        })
    }
}

fn parse_opt(value: Option<String>) -> Option<Value> {
    parse(value)
}

fn session_row(r: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        project_id: r.get(1)?,
        parent_id: r.get(2)?,
        fork_session_id: r.get(3)?,
        fork_boundary: parse(r.get(4)?),
        slug: r.get(5)?,
        directory: r.get(6)?,
        title: r.get(7)?,
        version: r.get(8)?,
        agent: r.get(9)?,
        model: parse(r.get(10)?),
        time_created: r.get::<_, i64>(11)? as u64,
        time_updated: r.get::<_, i64>(12)? as u64,
    })
}

/// Busy owners are `<pid>:<nonce>`; a holder whose process is gone does not block.
fn owner_alive(owner: &str) -> bool {
    let Some(pid) = owner.split(':').next().and_then(|p| p.parse::<i32>().ok()) else {
        return true;
    };
    #[cfg(unix)]
    {
        // Signal 0 checks existence; EPERM still means the process exists.
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: usize =
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    for (index, sql) in schema::MIGRATIONS.iter().enumerate().skip(version) {
        conn.execute_batch(&format!(
            "BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;",
            index + 1
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(id: &str, dir: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            project_id: "global".into(),
            parent_id: None,
            fork_session_id: None,
            fork_boundary: None,
            slug: "s".into(),
            directory: dir.into(),
            title: "t".into(),
            version: "v".into(),
            agent: None,
            model: None,
            time_created: 1,
            time_updated: 1,
        }
    }

    #[test]
    fn sessions_messages_kv() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        db.create_session(&session("ses_1", "/p")).unwrap();
        assert_eq!(
            db.append_message("ses_1", "msg_a", "user", &json!({"text": "hi"}))
                .unwrap(),
            1
        );
        assert_eq!(
            db.append_message("ses_1", "msg_b", "assistant", &json!({"content": []}))
                .unwrap(),
            2
        );
        db.update_message("msg_b", &json!({"content": [1]}))
            .unwrap();
        let rows = db.messages("ses_1").unwrap();
        assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(rows[1].data, json!({"content": [1]}));
        assert_eq!(db.latest_root_session("/p").unwrap().unwrap().id, "ses_1");
        db.kv_set("k", &json!({"total": 3})).unwrap();
        assert_eq!(db.kv_get("k").unwrap(), Some(json!({"total": 3})));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("a.db"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn busy_marker() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        db.create_session(&session("ses_1", "/p")).unwrap();
        let me = format!("{}:a", std::process::id());
        let other = format!("{}:b", std::process::id());
        assert!(db.acquire("ses_1", &me).unwrap());
        assert!(!db.acquire("ses_1", &other).unwrap());
        db.release("ses_1", &me).unwrap();
        assert!(db.acquire("ses_1", &other).unwrap());
        db.release("ses_1", &other).unwrap();
        assert!(db.acquire("ses_1", "999999999:dead").unwrap());
        assert!(
            db.acquire("ses_1", &me).unwrap(),
            "a dead holder does not block"
        );
    }
}
