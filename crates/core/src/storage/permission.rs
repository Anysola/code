//! Saved "always" permissions (per project, all `allow`) and a session's own rules.

use rusqlite::{OptionalExtension, params};

use serde_json::Value;

use super::{Db, Result, parse};
use crate::permission::{Effect, Rule};
use crate::time::now_ms;

/// A saved "always allow" answer with the project it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedPermission {
    pub project_id: String,
    /// The project's folder; `None` for sessions outside any project and unknown projects.
    pub worktree: Option<String>,
    pub action: String,
    pub resource: String,
}

impl Db {
    /// Every project's saved rules, by project folder and then oldest first.
    pub fn all_saved_permissions(&self) -> Result<Vec<SavedPermission>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT permission.project_id, project.worktree, permission.action, permission.resource
                 FROM permission LEFT JOIN project ON project.id = permission.project_id
                 ORDER BY project.worktree, permission.project_id, permission.time_created, permission.id",
            )?;
            let rows = statement.query_map([], |r| {
                let project_id: String = r.get(0)?;
                let worktree: Option<String> = r.get(1)?;
                Ok(SavedPermission {
                    worktree: worktree.filter(|_| project_id != "global"),
                    project_id,
                    action: r.get(2)?,
                    resource: r.get(3)?,
                })
            })?;
            rows.collect()
        })
    }

    /// Forgets one saved rule; false when it was not saved.
    pub fn forget_permission(
        &self,
        project_id: &str,
        action: &str,
        resource: &str,
    ) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "DELETE FROM permission WHERE project_id = ?1 AND action = ?2 AND resource = ?3",
                params![project_id, action, resource],
            )
            .map(|n| n > 0)
        })
    }

    /// Forgets every saved rule of the project; returns how many there were.
    pub fn forget_permissions(&self, project_id: &str) -> Result<usize> {
        self.with(|c| c.execute("DELETE FROM permission WHERE project_id = ?1", [project_id]))
    }

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
                    project: false,
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

    #[test]
    fn saved_permissions_are_listed_and_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        db.save_permissions("p1", "shell", &["ls *".into(), "git *".into()])
            .unwrap();
        db.save_permissions("p2", "read", &["*".into()]).unwrap();
        let all = db.all_saved_permissions().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            all.iter()
                .map(|p| (p.project_id.as_str(), p.resource.as_str()))
                .collect::<Vec<_>>(),
            [("p1", "ls *"), ("p1", "git *"), ("p2", "*")]
        );
        assert_eq!(all[0].worktree, None, "an unknown project has no folder");
        assert!(db.forget_permission("p1", "shell", "ls *").unwrap());
        assert!(!db.forget_permission("p1", "shell", "ls *").unwrap());
        assert_eq!(
            db.saved_permissions("p1").unwrap(),
            [Rule::new("shell", "git *", Effect::Allow)]
        );
        assert_eq!(db.forget_permissions("p2").unwrap(), 1);
        assert_eq!(db.all_saved_permissions().unwrap().len(), 1);
    }
}
