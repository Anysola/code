//! Saved "always" permissions (per project, all `allow`) and a session's own rules.

use rusqlite::{OptionalExtension, params};

use serde_json::Value;

use super::{Db, Result, parse};
use crate::permission::{Effect, Rule};
use crate::time::now_ms;

impl Db {
    /// The project's saved rules, oldest first.
    pub fn saved_permissions(&self, project_id: &str) -> Result<Vec<Rule>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT action, resource FROM permission WHERE project_id = ?1 ORDER BY time_created, id",
            )?;
            let rows = statement.query_map([project_id], |r| {
                Ok(Rule {
                    action: r.get(0)?,
                    resource: r.get(1)?,
                    effect: Effect::Allow,
                })
            })?;
            rows.collect()
        })
    }

    /// Saves `resources` as allowed for `action` in the project; already saved ones are kept.
    pub fn save_permissions(
        &self,
        project_id: &str,
        action: &str,
        resources: &[String],
    ) -> Result<()> {
        let now = now_ms() as i64;
        self.with(|c| {
            let transaction = c.transaction()?;
            for resource in resources {
                transaction.execute(
                    "INSERT INTO permission (id, project_id, action, resource, time_created, time_updated)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                     ON CONFLICT(project_id, action, resource) DO NOTHING",
                    params![crate::ids::saved_permission(), project_id, action, resource, now],
                )?;
            }
            transaction.commit()
        })
    }

    /// Rules stored on the session itself (its `permission` column), appended to its agent's.
    pub fn session_permissions(&self, session_id: &str) -> Result<Vec<Rule>> {
        let value = self.with(|c| {
            c.query_row(
                "SELECT permission FROM session WHERE id = ?1",
                [session_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
        })?;
        Ok(parse(value.flatten())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    /// The session's `metadata` column (a JSON object; empty when unset).
    pub fn session_metadata(&self, session_id: &str) -> Result<serde_json::Map<String, Value>> {
        let value = self.with(|c| {
            c.query_row(
                "SELECT metadata FROM session WHERE id = ?1",
                [session_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
        })?;
        Ok(match parse(value.flatten()) {
            Some(Value::Object(map)) => map,
            _ => Default::default(),
        })
    }

    /// Replaces the session's own rules and its metadata together.
    pub fn set_session_access(
        &self,
        session_id: &str,
        rules: &[Rule],
        metadata: &serde_json::Map<String, Value>,
    ) -> Result<()> {
        let rules = serde_json::to_string(rules).expect("rules serialize");
        let metadata = serde_json::to_string(metadata).expect("metadata serializes");
        self.touch(
            "permission = ?2, metadata = ?4",
            params![session_id, rules, crate::time::now_ms() as i64, metadata],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_permissions_are_per_project_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        db.save_permissions("p1", "shell", &["git status *".into(), "ls *".into()])
            .unwrap();
        db.save_permissions("p1", "shell", &["git status *".into()])
            .unwrap();
        db.save_permissions("p2", "read", &["*".into()]).unwrap();
        let saved = db.saved_permissions("p1").unwrap();
        assert_eq!(
            saved,
            [
                Rule::new("shell", "git status *", Effect::Allow),
                Rule::new("shell", "ls *", Effect::Allow)
            ]
        );
        assert!(db.session_permissions("ses_missing").unwrap().is_empty());
    }
}
