//! One-time import of Anysola Code 0.5.x data on the first start of 1.0: sessions with their full
//! history, projects, credentials, budgets, saved permissions, TUI settings and state, and the
//! snapshot stores. 0.5.x files are only read. The rows and the marker (KV `migration:0.5.x`) are
//! committed in one transaction, so a failed import leaves nothing behind and runs again next start.

mod database;
mod files;
mod rows;
mod snapshots;
mod source;

use anysola_core::paths::Paths;
use anysola_core::storage::Db;
use rusqlite::OptionalExtension;
use serde_json::json;

pub use database::Counts;

pub const MARKER: &str = "migration:0.5.x";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Run Anysola Code 0.5.x once to upgrade its database, then start 1.0 again.")]
    Outdated,
    #[error("the database is damaged ({0})")]
    Corrupt(String),
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Storage(#[from] anysola_core::storage::StorageError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

/// Imports 0.5.x data unless it was imported before or there is none. Returns what was imported.
pub fn run(paths: &Paths) -> Result<Option<Counts>, Error> {
    let source = source::Source::locate(paths);
    let Some(database) = &source.database else {
        return Ok(None);
    };
    let db = Db::open(&paths.database())?;
    if db.kv_get(MARKER)?.is_some() {
        return Ok(None);
    }
    let old = source::Snapshot::take(database)?;
    database::check_schema(&old.conn)?;
    db.immediate(|tx| {
        // Another process may have imported while this one copied the database.
        let done = tx
            .query_row("SELECT 1 FROM kv WHERE key = ?1", [MARKER], |_| Ok(()))
            .optional()?;
        if done.is_some() {
            return Ok(None);
        }
        let counts = database::import(&old.conn, tx)?;
        files::settings(&source, paths)?;
        files::state(&source, paths)?;
        snapshots::attach_all(&source, paths)?;
        let now = anysola_core::time::now_ms() as i64;
        let marker = json!({
            "sessions": counts.sessions,
            "messages": counts.messages,
            "projects": counts.projects,
            "time": now,
        });
        tx.execute(
            "INSERT INTO kv (key, value, time_created, time_updated) VALUES (?1, ?2, ?3, ?3)",
            rusqlite::params![MARKER, anysola_core::json::compact(&marker), now],
        )?;
        Ok(Some(counts))
    })
}
