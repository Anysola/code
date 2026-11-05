//! Integration credentials other than the Anysola login (MCP OAuth tokens).

use rusqlite::params;
use serde_json::Value;

use super::{Db, Result, json};
use crate::time::now_ms;

impl Db {
    /// Stores an active credential of `integration_id`; returns its id.
    pub fn insert_credential(
        &self,
        integration_id: &str,
        label: &str,
        value: &Value,
    ) -> Result<String> {
        let id = crate::ids::credential();
        let now = now_ms() as i64;
        self.with(|c| {
            c.execute(
                "INSERT INTO credential (id, integration_id, label, active, value, time_created, time_updated)
                 VALUES (?1, ?2, ?3, 1, ?4, ?5, ?5)",
                params![id, integration_id, label, json(value), now],
            )
            .map(drop)
        })?;
        Ok(id)
    }

    /// Removes every credential of an integration; returns how many there were.
    pub fn remove_integration_credentials(&self, integration_id: &str) -> Result<usize> {
        self.with(|c| {
            c.execute(
                "DELETE FROM credential WHERE integration_id = ?1",
                [integration_id],
            )
        })
    }

    /// The newest active credential of an integration: `(id, value)`.
    pub fn active_credential(&self, integration_id: &str) -> Result<Option<(String, Value)>> {
        let row = self.with(|c| {
            c.query_row(
                "SELECT id, value FROM credential WHERE integration_id = ?1 AND active = 1
                 ORDER BY time_updated DESC LIMIT 1",
                [integration_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })?;
        Ok(row.and_then(|(id, value)| Some((id, serde_json::from_str(&value).ok()?))))
    }

    /// Rewrites a credential in place (a refresh rotates tokens without a new row).
    pub fn update_credential(&self, id: &str, value: &Value) -> Result<()> {
        let now = now_ms() as i64;
        self.with(|c| {
            c.execute(
                "UPDATE credential SET value = ?2, time_updated = ?3 WHERE id = ?1",
                params![id, json(value), now],
            )
            .map(drop)
        })
    }

    /// Removes a credential only while it still holds this refresh token (another process may
    /// have rotated it meanwhile); true when it was removed.
    pub fn remove_credential_holding(&self, id: &str, refresh: &str) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "DELETE FROM credential WHERE id = ?1 AND json_extract(value, '$.refresh') = ?2",
                params![id, refresh],
            )
        })
        .map(|n| n > 0)
    }
}
