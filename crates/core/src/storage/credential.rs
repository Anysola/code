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
}
