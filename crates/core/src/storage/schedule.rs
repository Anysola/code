//! The `schedule` table and the `schedule:leader` lease.

use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};

use super::{Db, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleRow {
    pub id: String,
    pub session_id: Option<String>,
    pub name: Option<String>,
    pub action: Value,
    pub check: Option<Value>,
    pub when: Value,
    pub tz: String,
    /// `active`, `paused` or `done`.
    pub status: String,
    pub next_due_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub remaining_runs: Option<i64>,
    pub run_count: i64,
    pub checks: i64,
    pub failures: i64,
    pub last_fired_at: Option<i64>,
    pub last_message_id: Option<String>,
    pub last_outcome: Option<String>,
    /// The loop leaves a due row alone until then.
    pub retry_at: Option<i64>,
    /// Check state kept across checks (`Null` when there is none).
    pub state: Value,
    pub time_created: i64,
    pub time_updated: i64,
}

/// Which rows a save may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// Only a row that is still active: a pause that landed meanwhile wins.
    Active,
    /// Any row; its status changes only when the save sets one other than `active`.
    Any,
}

const COLUMNS: &str = "id, session_id, name, action, \"check\", \"when\", tz, status, next_due_at, ends_at, \
    remaining_runs, run_count, checks, failures, last_fired_at, last_message_id, last_outcome, retry_at, \
    state, time_created, time_updated";

fn text_json(value: Option<String>) -> Value {
    value
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(Value::Null)
}

fn stored(value: &Value) -> Option<String> {
    (!value.is_null()).then(|| value.to_string())
}

fn read(r: &Row) -> rusqlite::Result<ScheduleRow> {
    Ok(ScheduleRow {
        id: r.get(0)?,
        session_id: r.get(1)?,
        name: r.get(2)?,
        action: text_json(r.get(3)?),
        check: Some(text_json(r.get(4)?)).filter(|v| !v.is_null()),
        when: text_json(r.get(5)?),
        tz: r.get(6)?,
        status: r.get(7)?,
        next_due_at: r.get(8)?,
        ends_at: r.get(9)?,
        remaining_runs: r.get(10)?,
        run_count: r.get(11)?,
        checks: r.get(12)?,
        failures: r.get(13)?,
        last_fired_at: r.get(14)?,
        last_message_id: r.get(15)?,
        last_outcome: r.get(16)?,
        retry_at: r.get(17)?,
        state: text_json(r.get(18)?),
        time_created: r.get(19)?,
        time_updated: r.get(20)?,
    })
}

impl Db {
    pub fn schedule_insert(&self, row: &ScheduleRow) -> Result<()> {
        self.with(|c| {
            c.execute(
                &format!(
                    "INSERT INTO schedule ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
                     ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)"
                ),
                params![
                    row.id,
                    row.session_id,
                    row.name,
                    row.action.to_string(),
                    row.check.as_ref().map(Value::to_string),
                    row.when.to_string(),
                    row.tz,
                    row.status,
                    row.next_due_at,
                    row.ends_at,
                    row.remaining_runs,
                    row.run_count,
                    row.checks,
                    row.failures,
                    row.last_fired_at,
                    row.last_message_id,
                    row.last_outcome,
                    row.retry_at,
                    stored(&row.state),
                    row.time_created,
                    row.time_updated,
                ],
            )
            .map(drop)
        })
    }

    /// Writes the row's changing columns back; false when the guard left it alone.
    pub fn schedule_save(&self, row: &ScheduleRow, guard: Guard) -> Result<bool> {
        let condition = match guard {
            Guard::Active => " AND status = 'active'",
            Guard::Any => "",
        };
        self.with(|c| {
            c.execute(
                &format!(
                    "UPDATE schedule SET status = CASE WHEN ?2 = 'active' THEN status ELSE ?2 END, \
                     next_due_at = ?3, ends_at = ?4, remaining_runs = ?5, run_count = ?6, checks = ?7, \
                     failures = ?8, last_fired_at = ?9, last_message_id = ?10, last_outcome = ?11, \
                     retry_at = ?12, state = ?13, time_updated = ?14 WHERE id = ?1{condition}"
                ),
                params![
                    row.id,
                    row.status,
                    row.next_due_at,
                    row.ends_at,
                    row.remaining_runs,
                    row.run_count,
                    row.checks,
                    row.failures,
                    row.last_fired_at,
                    row.last_message_id,
                    row.last_outcome,
                    row.retry_at,
                    stored(&row.state),
                    row.time_updated,
                ],
            )
            .map(|n| n > 0)
        })
    }

    /// Sets the status outright (pause and resume), from `from` only.
    pub fn schedule_set_status(&self, row: &ScheduleRow, from: &str) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE schedule SET status = ?2, next_due_at = ?3, failures = ?4, retry_at = ?5, \
                 time_updated = ?6 WHERE id = ?1 AND status = ?7",
                params![
                    row.id,
                    row.status,
                    row.next_due_at,
                    row.failures,
                    row.retry_at,
                    row.time_updated,
                    from
                ],
            )
            .map(|n| n > 0)
        })
    }

    pub fn schedule(&self, id: &str) -> Result<Option<ScheduleRow>> {
        self.with(|c| {
            c.query_row(
                &format!("SELECT {COLUMNS} FROM schedule WHERE id = ?1"),
                [id],
                read,
            )
            .optional()
        })
    }

    /// Schedules oldest first, of one session or all.
    pub fn schedules(&self, session_id: Option<&str>) -> Result<Vec<ScheduleRow>> {
        self.with(|c| {
            let mut statement = c.prepare(&format!(
                "SELECT {COLUMNS} FROM schedule WHERE ?1 IS NULL OR session_id = ?1 \
                 ORDER BY time_created, id"
            ))?;
            statement.query_map([session_id], read)?.collect()
        })
    }

    pub fn schedule_delete(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            c.execute("DELETE FROM schedule WHERE id = ?1", [id])
                .map(|n| n > 0)
        })
    }

    /// Active rows due by `now` whose retry time has come, earliest first.
    pub fn schedules_due(&self, now: i64) -> Result<Vec<ScheduleRow>> {
        self.with(|c| {
            let mut statement = c.prepare(&format!(
                "SELECT {COLUMNS} FROM schedule WHERE status = 'active' AND next_due_at <= ?1 \
                 AND (retry_at IS NULL OR retry_at <= ?1) ORDER BY next_due_at"
            ))?;
            statement.query_map([now], read)?.collect()
        })
    }

    /// The earliest due time after `from` among active rows.
    pub fn schedule_next_due(&self, from: i64) -> Result<Option<i64>> {
        self.with(|c| {
            c.query_row(
                "SELECT MIN(next_due_at) FROM schedule WHERE status = 'active' AND next_due_at > ?1",
                [from],
                |r| r.get(0),
            )
        })
    }

    pub fn schedules_active(&self) -> Result<bool> {
        self.with(|c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM schedule WHERE status = 'active')",
                [],
                |r| r.get(0),
            )
        })
    }

    /// Sessions whose last scheduled prompt still waits in their inbox.
    pub fn schedule_unclaimed(&self) -> Result<Vec<String>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT DISTINCT schedule.session_id FROM schedule \
                 JOIN inbox ON inbox.id = schedule.last_message_id WHERE schedule.session_id IS NOT NULL",
            )?;
            statement.query_map([], |r| r.get(0))?.collect()
        })
    }

    pub fn message_exists(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM message WHERE id = ?1)",
                [id],
                |r| r.get(0),
            )
        })
    }

    pub fn inbox_has(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM inbox WHERE id = ?1)",
                [id],
                |r| r.get(0),
            )
        })
    }

    /// Takes or renews the `key` lease `{owner, until}`; false while another owner holds it.
    pub fn lease_claim(&self, key: &str, owner: &str, now: i64, ttl: i64) -> Result<bool> {
        let value = json!({"owner": owner, "until": now + ttl}).to_string();
        self.with(|c| {
            c.query_row(
                "INSERT INTO kv (key, value, time_created, time_updated) VALUES (?1, ?2, ?3, ?3) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, time_updated = excluded.time_updated \
                 WHERE json_extract(kv.value, '$.owner') = ?4 OR json_extract(kv.value, '$.until') <= ?3 \
                 RETURNING key",
                params![key, value, now, owner],
                |_| Ok(()),
            )
            .optional()
            .map(|claimed| claimed.is_some())
        })
    }

    pub fn lease_release(&self, key: &str, owner: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM kv WHERE key = ?1 AND json_extract(value, '$.owner') = ?2",
                params![key, owner],
            )
            .map(drop)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owner_holds_the_lease_until_it_lapses_or_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        let key = "schedule:leader";
        assert!(db.lease_claim(key, "a", 1_000, 60_000).unwrap());
        assert!(!db.lease_claim(key, "b", 2_000, 60_000).unwrap());
        // Renewing moves the deadline.
        assert!(db.lease_claim(key, "a", 30_000, 60_000).unwrap());
        assert!(!db.lease_claim(key, "b", 61_000, 60_000).unwrap());
        assert!(db.lease_claim(key, "b", 90_000, 60_000).unwrap());
        assert_eq!(
            db.kv_get(key).unwrap(),
            Some(json!({"owner": "b", "until": 150_000}))
        );
        // Only the holder releases it.
        db.lease_release(key, "a").unwrap();
        assert!(!db.lease_claim(key, "a", 91_000, 60_000).unwrap());
        db.lease_release(key, "b").unwrap();
        assert!(db.lease_claim(key, "a", 92_000, 60_000).unwrap());
    }
}
