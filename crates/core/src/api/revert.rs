//! Undo and redo: staging a revert restores the files the steps after a message changed (to the
//! tree before the first step that changed each), clearing it puts them back, and prompting while
//! one is staged commits it by dropping that history.

use serde_json::{Value, json};

use super::{Core, CoreError};
use crate::session::message::Message;
use crate::tool::edit::diff::{FileDiff, Status};

impl Core {
    /// Reverts the session to before `message_id`. Repeating it with another message keeps the
    /// files' original state from the first revert.
    pub fn revert(&self, session_id: &str, message_id: &str) -> Result<Value, CoreError> {
        let db = &self.shared.db;
        let boundary = db
            .message_seq(session_id, message_id)?
            .ok_or_else(|| CoreError::MessageNotFound(message_id.to_string()))?;
        let staged = db.revert(session_id)?;
        let store = self.shared.snapshots.as_deref();
        let original = staged
            .as_ref()
            .and_then(|r| r["snapshot"].as_str().map(str::to_string))
            .or_else(|| store.and_then(|s| s.capture()));
        let mut plan: Vec<(String, String)> = Vec::new();
        for row in db.messages(session_id)? {
            if row.seq <= boundary {
                continue;
            }
            let Message::Assistant(data) = Message::decode(&row.kind, &row.data) else {
                continue;
            };
            let Some(snapshot) = data.snapshot else {
                continue;
            };
            let Some(start) = snapshot.start else {
                continue;
            };
            for file in snapshot.files {
                if !plan.iter().any(|(f, _)| *f == file) {
                    plan.push((file, start.clone()));
                }
            }
        }
        let mut restore: Vec<(String, String)> = Vec::new();
        if let Some(original) = &original {
            for file in staged_files(staged.as_ref()) {
                restore.push((file, original.clone()));
            }
        }
        for (file, tree) in &plan {
            restore.retain(|(f, _)| f != file);
            restore.push((file.clone(), tree.clone()));
        }
        if !restore.is_empty() {
            let store = store.ok_or(CoreError::Snapshot("Snapshots are disabled".into()))?;
            store.restore(&restore).map_err(CoreError::Snapshot)?;
        }
        let files: Vec<Value> = match (store, &original) {
            (Some(store), Some(original)) => {
                let now = store.capture().unwrap_or_else(|| original.clone());
                plan.iter()
                    .filter_map(|(file, _)| file_diff(store, original, &now, file))
                    .collect()
            }
            _ => Vec::new(),
        };
        let mut revert = json!({"messageID": message_id});
        if let Some(original) = original {
            revert["snapshot"] = Value::String(original);
        }
        revert["files"] = Value::Array(files);
        db.set_revert(session_id, Some(&revert))?;
        Ok(revert)
    }

    /// Redo: puts back the files the staged revert changed and forgets it.
    pub fn unrevert(&self, session_id: &str) -> Result<(), CoreError> {
        let db = &self.shared.db;
        let Some(staged) = db.revert(session_id)? else {
            return Ok(());
        };
        if let (Some(original), Some(store)) = (
            staged["snapshot"].as_str(),
            self.shared.snapshots.as_deref(),
        ) {
            let files: Vec<(String, String)> = staged_files(Some(&staged))
                .into_iter()
                .map(|f| (f, original.to_string()))
                .collect();
            store.restore(&files).map_err(CoreError::Snapshot)?;
        }
        Ok(db.set_revert(session_id, None)?)
    }

    /// A prompt sent while a revert is staged makes it final: the reverted messages go, and the
    /// instruction state forgets what it saw of them.
    pub(crate) fn commit_revert(&self, session_id: &str) -> Result<(), CoreError> {
        let db = &self.shared.db;
        let Some(staged) = db.revert(session_id)? else {
            return Ok(());
        };
        let message = staged["messageID"].as_str().unwrap_or_default();
        if let Some(seq) = db.message_seq(session_id, message)? {
            db.truncate_messages(session_id, seq)?;
            db.instruction_rewind(session_id, seq)?;
        }
        Ok(db.set_revert(session_id, None)?)
    }
}

fn staged_files(staged: Option<&Value>) -> Vec<String> {
    staged
        .and_then(|r| r["files"].as_array())
        .map(|files| {
            files
                .iter()
                .filter_map(|f| f["file"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn file_diff(store: &crate::snapshot::Store, from: &str, to: &str, file: &str) -> Option<Value> {
    let before = store.read(from, file);
    let after = store.read(to, file);
    let text = |bytes: &Option<Vec<u8>>| {
        bytes
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    };
    let status = match (&before, &after) {
        (None, None) => return None,
        (None, Some(_)) => Status::Added,
        (Some(_), None) => Status::Deleted,
        (Some(_), Some(_)) => Status::Modified,
    };
    Some(FileDiff::new(file, &text(&before), &text(&after), status).to_value())
}
