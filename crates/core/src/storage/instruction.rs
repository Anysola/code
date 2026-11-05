//! Per-session instruction state: the frozen baseline (`initial_values`) a session's requests start
//! with, the values the model has seen since (`current_values`), and the content-addressed blobs
//! their hashes point at. Sequences are message sequences.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::{Db, Result, json, parse};

/// Source key → value hash.
pub type Values = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq)]
pub struct InstructionState {
    pub epoch_start: i64,
    pub through_seq: i64,
    pub initial: Values,
    pub current: Values,
}

fn values(text: String) -> Values {
    serde_json::from_str(&text).unwrap_or_default()
}

fn encode(values: &Values) -> String {
    serde_json::to_string(values).unwrap_or_default()
}

impl Db {
    pub fn instruction_state(&self, session_id: &str) -> Result<Option<InstructionState>> {
        self.with(|c| {
            c.query_row(
                "SELECT epoch_start, through_seq, initial_values, current_values FROM instruction_state
                 WHERE session_id = ?1",
                [session_id],
                |r| {
                    Ok(InstructionState {
                        epoch_start: r.get(0)?,
                        through_seq: r.get(1)?,
                        initial: values(r.get(2)?),
                        current: values(r.get(3)?),
                    })
                },
            )
            .optional()
        })
    }

    /// Stores the baseline (`epoch_start = through_seq = seq`) unless the session has one.
    pub fn instruction_initialize(
        &self,
        session_id: &str,
        seq: i64,
        values: &Values,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT OR IGNORE INTO instruction_state
                    (session_id, epoch_start, through_seq, initial_values, current_values)
                 VALUES (?1, ?2, ?2, ?3, ?3)",
                params![session_id, seq, encode(values)],
            )
            .map(drop)
        })
    }

    pub fn instruction_advance(&self, session_id: &str, seq: i64, current: &Values) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE instruction_state SET through_seq = ?2, current_values = ?3 WHERE session_id = ?1",
                params![session_id, seq, encode(current)],
            )
            .map(drop)
        })
    }

    /// After a compaction the current values become the baseline of a new epoch.
    pub fn instruction_new_epoch(&self, session_id: &str, seq: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE instruction_state SET epoch_start = ?2, through_seq = ?2, initial_values = current_values
                 WHERE session_id = ?1",
                params![session_id, seq],
            )
            .map(drop)
        })
    }

    /// Forgets the state when history from `boundary` on is removed and it saw any of it.
    pub fn instruction_rewind(&self, session_id: &str, boundary: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM instruction_state WHERE session_id = ?1 AND through_seq >= ?2",
                params![session_id, boundary],
            )
            .map(drop)
        })
    }

    pub fn instruction_blobs_put(&self, blobs: &BTreeMap<String, Value>) -> Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            for (hash, value) in blobs {
                tx.execute(
                    "INSERT OR IGNORE INTO instruction_blob (hash, value) VALUES (?1, ?2)",
                    params![hash, json(value)],
                )?;
            }
            tx.commit()
        })
    }

    pub fn instruction_blob(&self, hash: &str) -> Result<Option<Value>> {
        self.with(|c| {
            c.query_row(
                "SELECT value FROM instruction_blob WHERE hash = ?1",
                [hash],
                |r| r.get::<_, String>(0),
            )
            .optional()
        })
        .map(parse)
    }

    /// The session's API entries ordered by key; a removed entry has no value.
    pub fn instruction_entries(&self, session_id: &str) -> Result<Vec<(String, Option<Value>)>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT key, value, removed FROM instruction_entry WHERE session_id = ?1 ORDER BY key",
            )?;
            let rows = statement.query_map([session_id], |r| {
                let removed: bool = r.get(2)?;
                let value: String = r.get(1)?;
                Ok((r.get(0)?, if removed { None } else { parse(Some(value)) }))
            })?;
            rows.collect()
        })
    }

    /// Sets an entry, or marks it removed when `value` is `None`.
    pub fn instruction_entry_set(
        &self,
        session_id: &str,
        key: &str,
        value: Option<&Value>,
    ) -> Result<()> {
        self.with(|c| {
            match value {
                Some(value) => c.execute(
                    "INSERT INTO instruction_entry (session_id, key, value, removed) VALUES (?1, ?2, ?3, 0)
                     ON CONFLICT (session_id, key) DO UPDATE SET value = excluded.value, removed = 0",
                    params![session_id, key, json(value)],
                ),
                None => c.execute(
                    "UPDATE instruction_entry SET value = 'null', removed = 1 WHERE session_id = ?1 AND key = ?2",
                    params![session_id, key],
                ),
            }
            .map(drop)
        })
    }

    /// A fork starts with its source's entries.
    pub fn instruction_entries_copy(&self, from: &str, to: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO instruction_entry (session_id, key, value, removed)
                 SELECT ?2, key, value, removed FROM instruction_entry WHERE session_id = ?1",
                params![from, to],
            )
            .map(drop)
        })
    }

    pub fn delete_message(&self, id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM message WHERE id = ?1", [id])
                .map(drop)
        })
    }

    /// The highest message sequence of the session (0 when empty).
    pub fn last_seq(&self, session_id: &str) -> Result<i64> {
        self.with(|c| {
            c.query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM message WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
        })
    }
}
