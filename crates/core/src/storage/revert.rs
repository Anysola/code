//! A session's staged revert and the history truncation that commits it.

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::{Db, Result, json, parse};
use crate::time::now_ms;

impl Db {
    /// `{messageID, snapshot?, files}` while a revert is staged.
    pub fn revert(&self, session_id: &str) -> Result<Option<Value>> {
        self.with(|c| {
            c.query_row(
                "SELECT revert FROM session WHERE id = ?1",
                [session_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
        })
        .map(|v| parse(v.flatten()))
    }

    pub fn set_revert(&self, session_id: &str, revert: Option<&Value>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE session SET revert = ?2, time_updated = ?3 WHERE id = ?1",
                params![session_id, revert.map(json), now_ms() as i64],
            )
            .map(drop)
        })
    }

    pub fn message_seq(&self, session_id: &str, message_id: &str) -> Result<Option<i64>> {
        self.with(|c| {
            c.query_row(
                "SELECT seq FROM message WHERE session_id = ?1 AND id = ?2",
                [session_id, message_id],
                |r| r.get(0),
            )
            .optional()
        })
    }

    /// Removes the messages from `seq` on.
    pub fn truncate_messages(&self, session_id: &str, seq: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM message WHERE session_id = ?1 AND seq >= ?2",
                params![session_id, seq],
            )
            .map(drop)
        })
    }
}
