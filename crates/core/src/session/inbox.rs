//! The inbox: input admitted to a session before it becomes part of the transcript. `steer`
//! items join the running turn at its next step boundary; `queue` items wait until the session is
//! idle and start a turn of their own, one at a time. Control items (a manual compaction) are
//! handled between turns and hold back the steers admitted after them.

use serde_json::{Value, json};

use super::message::{Created, FileAttachment, UserData, encode};
use crate::api::Prompt;
use crate::storage::{Db, InboxRow, Result};
use crate::time::now_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Steer,
    Queue,
}

impl Delivery {
    pub fn as_str(self) -> &'static str {
        match self {
            Delivery::Steer => "steer",
            Delivery::Queue => "queue",
        }
    }
}

/// What a boundary may promote: anything (an idle session) or steers only (a running turn).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Input,
    Steer,
}

fn is_control(row: &InboxRow) -> bool {
    row.kind == "compaction"
}

pub fn user_data(prompt: &Prompt) -> Value {
    let files = prompt
        .files
        .iter()
        .map(|(mime, data, name)| FileAttachment {
            data: data.clone(),
            mime: mime.clone(),
            source: json!({"type": "inline"}),
            name: Some(name.clone()),
        })
        .collect();
    encode(&UserData {
        time: Created { created: now_ms() },
        text: prompt.text.clone(),
        files,
    })
}

/// Admits a prompt; returns the id its message will have.
pub fn enqueue(db: &Db, session_id: &str, prompt: &Prompt, delivery: Delivery) -> Result<String> {
    let id = crate::ids::message();
    db.inbox_push(
        session_id,
        &id,
        "user",
        &user_data(prompt),
        delivery.as_str(),
    )?;
    Ok(id)
}

pub fn enqueue_compaction(db: &Db, session_id: &str) -> Result<String> {
    let id = crate::ids::message();
    let payload = json!({"time": {"created": now_ms()}});
    db.inbox_push(
        session_id,
        &id,
        "compaction",
        &payload,
        Delivery::Steer.as_str(),
    )?;
    Ok(id)
}

/// The item a boundary would act on next.
pub fn next(db: &Db, session_id: &str, scope: Scope) -> Result<Option<InboxRow>> {
    let pending = db.inbox(session_id)?;
    if let Some(steer) = pending.iter().find(|r| r.delivery == "steer") {
        return Ok(Some(steer.clone()));
    }
    Ok(match scope {
        Scope::Input => pending.into_iter().find(|r| r.delivery == "queue"),
        Scope::Steer => None,
    })
}

fn publish(db: &Db, session_id: &str, rows: &[InboxRow]) -> Result<Vec<UserText>> {
    let mut promoted = Vec::new();
    for row in rows {
        // A concurrent cancel wins; the item is then gone.
        if db.inbox_remove(&row.id)? {
            db.append_message(session_id, &row.id, &row.kind, &row.payload)?;
            promoted.push(UserText(
                row.payload["text"].as_str().unwrap_or_default().to_string(),
            ));
        }
    }
    Ok(promoted)
}

/// The text of a promoted prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserText(pub String);

/// Moves pending input into the transcript: every steer up to the first control item, or with an
/// idle scope and no steers, the oldest queued item and the steers that arrived meanwhile. `None`
/// when a control item comes first.
pub fn promote(db: &Db, session_id: &str, scope: Scope) -> Result<Option<Vec<UserText>>> {
    let steers = |db: &Db| -> Result<Vec<InboxRow>> {
        Ok(db
            .inbox(session_id)?
            .into_iter()
            .filter(|r| r.delivery == "steer")
            .collect())
    };
    let before_control = |rows: Vec<InboxRow>| -> Vec<InboxRow> {
        rows.into_iter().take_while(|r| !is_control(r)).collect()
    };
    let pending = steers(db)?;
    if !pending.is_empty() || scope == Scope::Steer {
        if pending.first().is_some_and(is_control) {
            return Ok(None);
        }
        return publish(db, session_id, &before_control(pending)).map(Some);
    }
    let Some(queued) = db
        .inbox(session_id)?
        .into_iter()
        .find(|r| r.delivery == "queue")
    else {
        return Ok(Some(Vec::new()));
    };
    if is_control(&queued) {
        return Ok(None);
    }
    let mut promoted = publish(db, session_id, &[queued])?;
    promoted.extend(publish(db, session_id, &before_control(steers(db)?))?);
    Ok(Some(promoted))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        db.create_session(&crate::storage::SessionRow {
            id: "ses_1".into(),
            project_id: "global".into(),
            parent_id: None,
            fork_session_id: None,
            fork_boundary: None,
            slug: "s".into(),
            directory: "/".into(),
            title: "t".into(),
            version: "v".into(),
            agent: None,
            model: None,
            time_created: 1,
            time_updated: 1,
        })
        .unwrap();
        (dir, db)
    }

    fn prompt(text: &str) -> Prompt {
        Prompt {
            text: text.into(),
            files: vec![],
        }
    }

    fn texts(promoted: Option<Vec<UserText>>) -> Vec<String> {
        promoted.unwrap().into_iter().map(|t| t.0).collect()
    }

    #[test]
    fn steers_join_queued_wait() {
        let (_dir, db) = db();
        enqueue(&db, "ses_1", &prompt("q1"), Delivery::Queue).unwrap();
        enqueue(&db, "ses_1", &prompt("s1"), Delivery::Steer).unwrap();
        let q2 = enqueue(&db, "ses_1", &prompt("q2"), Delivery::Queue).unwrap();
        assert_eq!(
            texts(promote(&db, "ses_1", Scope::Steer).unwrap()),
            vec!["s1"]
        );
        assert!(texts(promote(&db, "ses_1", Scope::Steer).unwrap()).is_empty());
        assert_eq!(
            texts(promote(&db, "ses_1", Scope::Input).unwrap()),
            vec!["q1"]
        );
        assert!(db.inbox_set_delivery(&q2, "steer").unwrap());
        assert_eq!(
            texts(promote(&db, "ses_1", Scope::Steer).unwrap()),
            vec!["q2"]
        );
        let rows = db.messages("ses_1").unwrap();
        assert_eq!(
            rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
            ["user"; 3]
        );
        assert_eq!(rows[2].id, q2, "the message keeps the inbox id");
    }

    #[test]
    fn control_items_hold_back_later_steers() {
        let (_dir, db) = db();
        enqueue(&db, "ses_1", &prompt("a"), Delivery::Steer).unwrap();
        enqueue_compaction(&db, "ses_1").unwrap();
        enqueue(&db, "ses_1", &prompt("b"), Delivery::Steer).unwrap();
        assert_eq!(
            texts(promote(&db, "ses_1", Scope::Steer).unwrap()),
            vec!["a"]
        );
        assert_eq!(promote(&db, "ses_1", Scope::Steer).unwrap(), None);
        assert_eq!(
            next(&db, "ses_1", Scope::Steer).unwrap().unwrap().kind,
            "compaction"
        );
    }

    #[test]
    fn cancelled_items_are_not_promoted() {
        let (_dir, db) = db();
        let id = enqueue(&db, "ses_1", &prompt("a"), Delivery::Queue).unwrap();
        assert!(db.inbox_remove(&id).unwrap());
        assert!(!db.inbox_remove(&id).unwrap());
        assert!(texts(promote(&db, "ses_1", Scope::Input).unwrap()).is_empty());
    }
}
