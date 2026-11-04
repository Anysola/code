//! Pending input of a session: prompts admitted while it works (or before its next turn) and
//! control items such as a manual compaction. Items become messages when promoted.

use rusqlite::params;
use serde_json::Value;

use super::{Db, Result, json, parse};
use crate::time::now_ms;

#[derive(Debug, Clone, PartialEq)]
pub struct InboxRow {
    pub id: String,
    /// `user`, `synthetic` or `compaction`.
    pub kind: String,
    pub payload: Value,
    /// `steer` or `queue`.
    pub delivery: String,
}

impl Db {
    pub fn inbox_push(
        &self,
        session_id: &str,
        id: &str,
        kind: &str,
        payload: &Value,
        delivery: &str,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO inbox (id, session_id, type, payload, delivery, enqueued_seq, time_created)
                 VALUES (?1, ?2, ?3, ?4, ?5,
                    (SELECT COALESCE(MAX(enqueued_seq), 0) + 1 FROM inbox WHERE session_id = ?2), ?6)",
                params![id, session_id, kind, json(payload), delivery, now_ms() as i64],
            )
            .map(drop)
        })
    }

    /// Pending items in admission order.
    pub fn inbox(&self, session_id: &str) -> Result<Vec<InboxRow>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT id, type, payload, delivery FROM inbox WHERE session_id = ?1 ORDER BY enqueued_seq",
            )?;
            let rows = statement.query_map([session_id], |r| {
                Ok(InboxRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    payload: parse(r.get(2)?).unwrap_or(Value::Null),
                    delivery: r.get(3)?,
                })
            })?;
            rows.collect()
        })
    }

    /// Removes a pending item; false when it was already promoted or cancelled.
    pub fn inbox_remove(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            c.execute("DELETE FROM inbox WHERE id = ?1", [id])
                .map(|n| n > 0)
        })
    }

    pub fn inbox_set_delivery(&self, id: &str, delivery: &str) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE inbox SET delivery = ?2 WHERE id = ?1",
                params![id, delivery],
            )
            .map(|n| n > 0)
        })
    }
}
