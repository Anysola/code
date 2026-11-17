//! Core events as the window receives them (`core` channel). Text streams as deltas; anything
//! that changes stored parts (a tool call, a step's end) says so, and the window re-reads the
//! transcript instead of rebuilding it from events.

use anysola_core::events::{Event, InterruptReason};
use serde_json::{Value, json};

pub fn to_json(event: &Event, directory: &str) -> Option<Value> {
    let v = match event {
        Event::Started { session_id } => json!({"type": "started", "sessionId": session_id}),
        Event::TextDelta {
            session_id,
            message_id,
            ordinal,
            text,
        } => json!({"type": "text", "sessionId": session_id, "messageId": message_id,
                    "ordinal": ordinal, "text": text}),
        Event::ReasoningDelta {
            session_id,
            message_id,
            ordinal,
            text,
        } => json!({"type": "reasoning", "sessionId": session_id, "messageId": message_id,
                    "ordinal": ordinal, "text": text}),
        Event::ToolInputStarted { session_id, .. }
        | Event::ToolCalled { session_id, .. }
        | Event::ToolSucceeded { session_id, .. }
        | Event::ToolFailed { session_id, .. }
        | Event::TextEnded { session_id, .. }
        | Event::ReasoningEnded { session_id, .. }
        | Event::StepStarted { session_id, .. }
        | Event::StepEnded { session_id, .. }
        | Event::CompactionEnded { session_id, .. } => {
            json!({"type": "changed", "sessionId": session_id})
        }
        Event::StepFailed {
            session_id, error, ..
        } => json!({"type": "changed", "sessionId": session_id, "error": error.message}),
        Event::RetryScheduled {
            session_id,
            attempt,
            delay_ms,
            error,
        } => json!({"type": "retrying", "sessionId": session_id, "attempt": attempt,
                    "delayMs": delay_ms, "error": error.message}),
        Event::PermissionAsked {
            session_id,
            request_id,
            action,
            resources,
            save,
            metadata,
        } => json!({
            "type": "permission",
            "sessionId": session_id,
            "directory": directory,
            "requestId": request_id,
            "action": action,
            // A shell ask's resources are its commands; file asks name paths.
            "resources": resources.iter()
                .map(|r| if action == "shell" {
                    r.clone()
                } else {
                    crate::transcript::target("read", &json!({"path": r}), directory)
                })
                .collect::<Vec<_>>(),
            "canAlways": !save.is_empty(),
            "files": metadata.get("files"),
        }),
        Event::PermissionReplied {
            session_id,
            request_id,
            ..
        } => json!({"type": "permissionReplied", "sessionId": session_id, "requestId": request_id}),
        Event::QuestionAsked {
            session_id,
            request_id,
            questions,
        } => json!({"type": "question", "sessionId": session_id, "directory": directory,
                    "requestId": request_id, "questions": questions}),
        Event::TitleChanged { session_id, title } => {
            json!({"type": "title", "sessionId": session_id, "title": title})
        }
        Event::Succeeded { session_id } => json!({"type": "done", "sessionId": session_id}),
        Event::Failed { session_id, error } => {
            json!({"type": "failed", "sessionId": session_id, "error": error.message})
        }
        Event::Interrupted { session_id, reason } => json!({
            "type": "interrupted",
            "sessionId": session_id,
            "reason": match reason {
                InterruptReason::User => "user",
                InterruptReason::Budget => "budget",
                InterruptReason::Declined => "declined",
            },
        }),
        Event::Usage(snapshot) => json!({
            "type": "usage",
            "sessionId": snapshot.session_id,
            "fiveHour": snapshot.latest.as_ref().map(|u| u.five_hour.percent),
            "weekly": snapshot.latest.as_ref().map(|u| u.weekly.percent),
        }),
        Event::SessionCreated {
            session_id,
            parent_id: None,
            title,
        } => {
            json!({"type": "created", "sessionId": session_id, "title": title, "directory": directory})
        }
        // The window re-reads the folder's server list.
        Event::McpStatusChanged { .. } => json!({"type": "mcp", "directory": directory}),
        _ => return None,
    };
    Some(v)
}
