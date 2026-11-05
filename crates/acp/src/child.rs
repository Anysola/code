//! What subagents and compactions add to a prompt turn's updates (0.5.1's `acp/child.ts`,
//! `acp/compaction.ts`): a child session's progress goes to clients that advertise
//! `_meta["opencode/child-session-updates"]` as `opencode/session/child_update` notifications and
//! to the others as the root's own updates; compactions are standard `compaction_update`s for
//! clients with `session.compaction`, else a `session_info_update` marker.

use anysola_core::gateway::error::SessionError;
use serde_json::{Value, json};

use crate::agent::Capabilities;
use crate::rpc::Connection;

pub const UPDATE_METHOD: &str = "opencode/session/child_update";
const MARKER_META: &str = "opencode/compaction";

#[derive(Debug, Clone, PartialEq)]
pub struct Child {
    pub id: String,
    pub parent_id: String,
    pub depth: u32,
    pub title: String,
}

impl Child {
    fn meta(&self) -> Value {
        let mut child = json!({"id": self.id, "parentID": self.parent_id, "depth": self.depth});
        if !self.title.is_empty() {
            child["title"] = json!(self.title);
        }
        json!({"opencode/child-session": child})
    }

    /// The update as the root shows it: the child named in `_meta`, tool calls kept apart from
    /// the root's own.
    pub fn project(&self, mut update: Value) -> Value {
        let mut meta = update["_meta"].as_object().cloned().unwrap_or_default();
        meta.extend(self.meta().as_object().cloned().unwrap_or_default());
        update["_meta"] = Value::Object(meta);
        if matches!(
            update["sessionUpdate"].as_str(),
            Some("tool_call" | "tool_call_update")
        ) {
            if let Some(id) = update["toolCallId"].as_str() {
                update["toolCallId"] = json!(format!("{}:{id}", self.id));
            }
            if let Some(title) = update["title"].as_str().filter(|_| !self.title.is_empty()) {
                update["title"] = json!(format!("{}: {title}", self.title));
            }
        }
        update
    }

    fn envelope(&self, root: &str, event: Value) -> Value {
        let mut out = json!({
            "rootSessionId": root,
            "childSessionId": self.id,
            "parentSessionId": self.parent_id,
            "depth": self.depth,
        });
        if !self.title.is_empty() {
            out["title"] = json!(self.title);
        }
        if let (Some(out), Value::Object(event)) = (out.as_object_mut(), event) {
            out.extend(event);
        }
        out
    }
}

/// Sends updates of one prompt turn of `root`.
pub struct Router<'a> {
    pub connection: &'a Connection,
    pub root: &'a str,
    pub capabilities: Capabilities,
}

impl Router<'_> {
    pub fn send(&self, child: Option<&Child>, update: Value) {
        let Some(child) = child else {
            self.connection.update(self.root, update);
            return;
        };
        let projected = child.project(update);
        if self.capabilities.child_updates {
            self.connection.notify(
                UPDATE_METHOD,
                child.envelope(self.root, json!({"type": "update", "update": projected})),
            );
        } else {
            self.connection.update(self.root, projected);
        }
    }

    /// `created`, `running`, `completed`, `failed` or `interrupted`; only clients that take child
    /// updates hear it.
    pub fn status(&self, child: &Child, status: &str, error: Option<&SessionError>) {
        if !self.capabilities.child_updates {
            return;
        }
        let mut event = json!({"type": "status", "status": status});
        if let Some(error) = error {
            event["error"] = json!({"type": error.kind, "message": error.message});
        }
        self.connection
            .notify(UPDATE_METHOD, child.envelope(self.root, event));
    }

    /// A child's standard update would read as the root's own without child updates.
    pub fn standard_compaction(&self, child: Option<&Child>) -> bool {
        self.capabilities.compaction && (child.is_none() || self.capabilities.child_updates)
    }
}

/// One compaction's state as the client hears it.
pub enum Compaction<'a> {
    Started,
    Completed(&'a str),
    Failed(&'a SessionError),
}

pub fn compaction_update(
    message_id: &str,
    reason: &str,
    compaction: Compaction<'_>,
    standard: bool,
) -> Value {
    if !standard {
        let mut marker = json!({"messageId": message_id, "reason": reason});
        marker["status"] = json!(match compaction {
            Compaction::Started => "started",
            Compaction::Completed(_) => "completed",
            Compaction::Failed(_) => "failed",
        });
        if let Compaction::Failed(error) = compaction {
            marker["error"] = serde_json::to_value(error).unwrap_or(Value::Null);
        }
        return json!({"sessionUpdate": "session_info_update", "_meta": {MARKER_META: marker}});
    }
    let base = json!({"sessionUpdate": "compaction_update", "compactionId": message_id});
    let fields = match compaction {
        Compaction::Started => json!({"status": "in_progress"}),
        Compaction::Completed(summary) => {
            let summary = (!summary.is_empty()).then(|| json!([{"type": "text", "text": summary}]));
            json!({"status": "completed", "summary": summary})
        }
        Compaction::Failed(error)
            if matches!(error.kind.as_str(), "aborted" | "compaction.interrupted") =>
        {
            json!({"status": "cancelled", "summary": null})
        }
        Compaction::Failed(error) => {
            // Core reports a defect as `compaction.failed` with a multi-line cause.
            let shown = if error.message.is_empty()
                || (error.kind == "compaction.failed" && error.message.contains('\n'))
            {
                "Compaction failed"
            } else {
                error.message.as_str()
            };
            json!({"status": "failed", "summary": null, "error": shown})
        }
    };
    let mut update = base;
    if let (Some(update), Value::Object(fields)) = (update.as_object_mut(), fields) {
        update.extend(fields);
    }
    update
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child() -> Child {
        Child {
            id: "ses_c".into(),
            parent_id: "ses_r".into(),
            depth: 1,
            title: "Explore".into(),
        }
    }

    #[test]
    fn child_updates_are_projected() {
        let update = child()
            .project(json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "read"}));
        assert_eq!(update["toolCallId"], "ses_c:t1");
        assert_eq!(update["title"], "Explore: read");
        assert_eq!(
            update["_meta"]["opencode/child-session"],
            json!({"id": "ses_c", "parentID": "ses_r", "depth": 1, "title": "Explore"})
        );
    }

    #[test]
    fn compactions_standard_and_marked() {
        assert_eq!(
            compaction_update("msg_1", "auto", Compaction::Started, true),
            json!({"sessionUpdate": "compaction_update", "compactionId": "msg_1", "status": "in_progress"})
        );
        assert_eq!(
            compaction_update("msg_1", "manual", Compaction::Completed("sum"), true)["summary"],
            json!([{"type": "text", "text": "sum"}])
        );
        let cancelled = SessionError::new("aborted", "Compaction cancelled");
        assert_eq!(
            compaction_update("msg_1", "manual", Compaction::Failed(&cancelled), true)["status"],
            "cancelled"
        );
        let defect = SessionError::new("compaction.failed", "a\nb");
        assert_eq!(
            compaction_update("msg_1", "auto", Compaction::Failed(&defect), true)["error"],
            "Compaction failed"
        );
        assert_eq!(
            compaction_update("msg_1", "auto", Compaction::Started, false),
            json!({"sessionUpdate": "session_info_update", "_meta": {"opencode/compaction":
                {"status": "started", "messageId": "msg_1", "reason": "auto"}}})
        );
    }
}
