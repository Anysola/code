//! Core events as the window receives them (`core` channel). Text streams as deltas; anything
//! that changes stored parts (a tool call, a step's end) says so, and the window re-reads the
//! transcript instead of rebuilding it from events.

use anysola_core::api::CoreError;
use anysola_core::events::{Event, InterruptReason};
use anysola_core::mcp::Form;
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
        // A compaction's note is stored as it starts: "Compacting…" shows at once.
        | Event::CompactionStarted { session_id, .. }
        | Event::CompactionEnded { session_id, .. } => {
            json!({"type": "changed", "sessionId": session_id})
        }
        Event::StepFailed {
            session_id, error, ..
        }
        | Event::CompactionFailed {
            session_id, error, ..
        } => {
            let (kind, _) = error_kind(&error.kind, &error.message);
            json!({"type": "changed", "sessionId": session_id, "error": plain(kind, &error.message)})
        }
        Event::RetryScheduled {
            session_id,
            attempt,
            delay_ms,
            error,
        } => {
            let (kind, _) = error_kind(&error.kind, &error.message);
            json!({"type": "retrying", "sessionId": session_id, "attempt": attempt,
                   "delayMs": delay_ms, "error": plain(kind, &error.message)})
        }
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
        Event::QuestionReplied {
            session_id,
            request_id,
        } => json!({"type": "questionReplied", "sessionId": session_id, "requestId": request_id}),
        Event::FormAsked { form } => form_json(form, directory),
        Event::FormReplied { id } => {
            json!({"type": "formReplied", "directory": directory, "formId": id})
        }
        Event::TitleChanged { session_id, title } => {
            json!({"type": "title", "sessionId": session_id, "title": title})
        }
        Event::Succeeded { session_id } => json!({"type": "done", "sessionId": session_id}),
        Event::Failed {
            session_id,
            error,
            unsent,
        } => {
            let (kind, _) = error_kind(&error.kind, &error.message);
            json!({"type": "failed", "sessionId": session_id, "error": plain(kind, &error.message),
                   "kind": kind, "unsent": unsent})
        }
        Event::Interrupted {
            session_id, reason, ..
        } => json!({
            "type": "interrupted",
            "sessionId": session_id,
            "reason": match reason {
                InterruptReason::User => "user",
                InterruptReason::Budget => "budget",
                InterruptReason::Declined => "declined",
            },
        }),
        Event::Usage(snapshot) => {
            let mut v = crate::branch::usage_json(snapshot);
            v["type"] = json!("usage");
            v
        }
        Event::SessionCreated {
            session_id,
            parent_id: None,
            title,
        } => {
            json!({"type": "created", "sessionId": session_id, "title": title, "directory": directory})
        }
        // A tool's hooks reach the window on its item. A blocked prompt's comes just before its
        // `failed`, whose `error` is the same reason: the window tells a hook's stop by that.
        Event::Hook {
            session_id,
            event,
            command,
            outcome,
            message,
        } => {
            if matches!(event.as_str(), "PreToolUse" | "PostToolUse") {
                return None;
            }
            json!({"type": "hook", "sessionId": session_id, "event": event, "command": command,
                   "outcome": outcome.as_str(), "message": message})
        }
        Event::HooksChanged => json!({"type": "hooks", "directory": directory}),
        Event::TrustChanged => json!({"type": "trust", "directory": directory}),
        // The window re-reads the folder's server list.
        Event::McpStatusChanged { .. } => json!({"type": "mcp", "directory": directory}),
        // The forwarder names the root session the window has open.
        Event::BackgroundChanged { session_id } => {
            json!({"type": "changed", "sessionId": session_id})
        }
        _ => return None,
    };
    Some(v)
}

/// An MCP server's form as the window shows it (`FormRequest`). The runtime's field shapes are
/// loose (a URL form's field has no title, a label may be missing): they are made whole here.
pub fn form_json(form: &Form, directory: &str) -> Value {
    let server = form.metadata["server"].as_str().unwrap_or("An MCP server");
    let as_text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let fields: Vec<Value> = form
        .fields
        .iter()
        .map(|field| {
            let mut out = field.clone();
            let key = as_text(&field["key"]);
            let title = field["title"]
                .as_str()
                .filter(|t| !t.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if field["type"] == "external" {
                        "Open the page".to_string()
                    } else {
                        key.clone()
                    }
                });
            out["key"] = json!(key);
            out["title"] = json!(title);
            if !out["type"].is_string() {
                out["type"] = json!("string");
            }
            if let Some(options) = field["options"].as_array() {
                out["options"] = Value::Array(
                    options
                        .iter()
                        .map(|o| {
                            let value = as_text(&o["value"]);
                            let label = Some(as_text(&o["label"]))
                                .filter(|l| !l.is_empty())
                                .unwrap_or_else(|| value.clone());
                            json!({"value": value, "label": label})
                        })
                        .collect(),
                );
                // The window picks among the options' values, which are text: a default must be too.
                match field.get("default") {
                    Some(Value::Array(items)) => {
                        out["default"] =
                            Value::Array(items.iter().map(|i| json!(as_text(i))).collect());
                    }
                    Some(Value::Null) | None => {}
                    Some(other) => out["default"] = json!(as_text(other)),
                }
            }
            out
        })
        .collect();
    json!({
        "type": "form",
        "directory": directory,
        "formId": form.id,
        "server": server,
        "title": form.title,
        "message": form.metadata["message"].as_str().unwrap_or_default(),
        "fields": fields,
    })
}

/// What went wrong, as the window's `ErrorKind`, and whether sending again can work. `kind` is
/// a session error's kind; no route to a model also covers being signed out and an unreachable
/// service, told apart by the core's messages.
pub fn error_kind(kind: &str, message: &str) -> (&'static str, bool) {
    use anysola_core::gateway::config::SIGNED_OUT;
    use anysola_core::session::NOT_SIGNED_IN;
    match kind {
        anysola_core::gateway::error::USAGE_LIMIT_KIND => ("limit", false),
        "provider.transport" => ("offline", true),
        "provider.auth" => ("auth", false),
        "provider.no-route" if message == SIGNED_OUT || message == NOT_SIGNED_IN => ("auth", false),
        "provider.no-route" if message.starts_with("Cannot reach ") => ("offline", true),
        "provider.no-route" => ("model", false),
        "busy" => ("elsewhere", true),
        // The same message would be blocked again.
        anysola_core::hooks::BLOCKED => ("other", false),
        _ => ("other", true),
    }
}

/// The message a desktop user reads for an error of `kind` (from [`error_kind`]): no terminal
/// commands, no raw network errors.
pub fn plain(kind: &str, message: &str) -> String {
    match kind {
        "auth" => "Your Anysola sign-in has expired or was revoked. Sign in again.".into(),
        "offline" => "Can’t reach Anysola. Check your connection.".into(),
        "elsewhere" => "This session is running in another copy of Anysola Code. \
                        Let it finish or stop it there, then try again."
            .into(),
        "model"
            if [
                "No model is available",
                "Model unavailable",
                "Invalid model reference",
                "Variant unavailable",
            ]
            .iter()
            .any(|p| message.starts_with(p)) =>
        {
            "This model isn’t available. Pick another model.".into()
        }
        _ => message.to_string(),
    }
}

/// A core error as its kind and plain message (a prompt refused before any turn ran).
pub fn core_error(error: &CoreError) -> (&'static str, String) {
    let (kind, message) = match error {
        CoreError::Busy(_) => ("elsewhere", String::new()),
        // The reason no model loaded: not signed in, signed out, or unreachable.
        CoreError::NoModels(message) => {
            (error_kind("provider.no-route", message).0, message.clone())
        }
        CoreError::Model(_) => ("model", error.to_string()),
        other => ("other", other.to_string()),
    };
    (kind, plain(kind, &message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anysola_core::gateway::error::SessionError;
    use anysola_core::hooks::HookOutcome;

    fn failed(kind: &str, message: &str, unsent: bool) -> Value {
        let event = Event::Failed {
            session_id: "ses_1".into(),
            error: SessionError::new(kind, message),
            unsent,
        };
        to_json(&event, "/p").unwrap()
    }

    #[test]
    fn usage_reaches_the_window_with_totals_and_budget() {
        let event = Event::Usage(anysola_core::events::UsageSnapshot {
            session_id: "ses_1".into(),
            latest: None,
            turn: 0.5,
            total: 3.0,
            turns: 2,
            budget: Some(10),
        });
        assert_eq!(
            to_json(&event, "/p").unwrap(),
            json!({"type": "usage", "sessionId": "ses_1", "turn": 0.5, "total": 3.0, "turns": 2,
                   "budget": 10, "fiveHour": null, "weekly": null})
        );
    }

    #[test]
    fn errors_carry_a_kind_and_plain_words() {
        assert_eq!(
            failed(
                "provider.transport",
                "error sending request for url (https://x/v1)",
                false
            ),
            json!({"type": "failed", "sessionId": "ses_1", "kind": "offline", "unsent": false,
                   "error": "Can’t reach Anysola. Check your connection."})
        );
        let signed_out = failed(
            "provider.no-route",
            anysola_core::gateway::config::SIGNED_OUT,
            true,
        );
        assert_eq!(signed_out["kind"], "auth");
        assert_eq!(signed_out["unsent"], true);
        assert!(
            !signed_out["error"]
                .as_str()
                .unwrap()
                .contains("anysola login")
        );
        assert_eq!(
            failed(
                "provider.no-route",
                anysola_core::session::NOT_SIGNED_IN,
                false
            )["kind"],
            "auth"
        );
        assert_eq!(
            failed(
                "provider.no-route",
                "Cannot reach https://api.anysola.com",
                false
            )["kind"],
            "offline"
        );
        let model = failed("provider.no-route", "Model unavailable: x", false);
        assert_eq!(model["kind"], "model");
        assert_eq!(
            model["error"],
            "This model isn’t available. Pick another model."
        );
        assert_eq!(
            failed(
                "provider.usage-limit",
                "Your 5-hour limit is reached.",
                false
            )["error"],
            "Your 5-hour limit is reached."
        );
        assert_eq!(error_kind("provider.usage-limit", ""), ("limit", false));
        assert_eq!(error_kind("provider.auth", "bad"), ("auth", false));
        assert_eq!(
            error_kind("provider.internal", "overloaded"),
            ("other", true)
        );
        let (kind, message) = core_error(&CoreError::Busy("ses_1".into()));
        assert_eq!(kind, "elsewhere");
        assert!(!message.contains("ses_1"));
        assert_eq!(
            core_error(&CoreError::NoModels(
                anysola_core::gateway::config::SIGNED_OUT.into()
            ))
            .0,
            "auth"
        );
    }

    #[test]
    fn hooks_outside_tool_calls_reach_the_window() {
        let hook = |event: &str, outcome| Event::Hook {
            session_id: "ses_1".into(),
            event: event.into(),
            command: "./guard.sh".into(),
            outcome,
            message: "Tests fail".into(),
        };
        assert_eq!(
            to_json(&hook("Stop", HookOutcome::Blocked), "/p").unwrap(),
            json!({"type": "hook", "sessionId": "ses_1", "event": "Stop", "command": "./guard.sh",
                   "outcome": "blocked", "message": "Tests fail"})
        );
        assert_eq!(
            to_json(&hook("UserPromptSubmit", HookOutcome::Context), "/p").unwrap()["outcome"],
            "context"
        );
        // On the tool's item.
        assert_eq!(
            to_json(&hook("PreToolUse", HookOutcome::Blocked), "/p"),
            None
        );
        assert_eq!(
            to_json(&hook("PostToolUse", HookOutcome::Error), "/p"),
            None
        );
        assert_eq!(
            to_json(&hook("UserPromptSubmit", HookOutcome::Blocked), "/p").unwrap()["message"],
            "Tests fail"
        );
        assert_eq!(
            to_json(&Event::HooksChanged, "/p").unwrap(),
            json!({"type": "hooks", "directory": "/p"})
        );
        assert_eq!(
            to_json(&Event::TrustChanged, "/p").unwrap(),
            json!({"type": "trust", "directory": "/p"})
        );
        let blocked = failed(anysola_core::hooks::BLOCKED, "No secrets in prompts", true);
        assert_eq!(
            blocked,
            json!({"type": "failed", "sessionId": "ses_1", "kind": "other", "unsent": true,
                   "error": "No secrets in prompts"})
        );
        assert_eq!(
            error_kind(anysola_core::hooks::BLOCKED, "x"),
            ("other", false)
        );
    }

    #[test]
    fn requests_and_their_ends_as_the_window_reads_them() {
        let form = Form {
            id: "frm_1".into(),
            title: "docs is requesting input".into(),
            metadata: json!({"kind": "mcp-elicitation", "server": "docs", "message": "Pick one"}),
            fields: vec![
                json!({"key": "env", "title": "Env", "type": "string", "custom": false,
                       "options": [{"value": "prod", "label": null}, {"value": 2, "label": "Two"}]}),
                json!({"key": "elicitation", "type": "external", "url": "https://x"}),
            ],
            session: None,
        };
        assert_eq!(
            to_json(&Event::FormAsked { form }, "/p").unwrap(),
            json!({"type": "form", "directory": "/p", "formId": "frm_1", "server": "docs",
            "title": "docs is requesting input", "message": "Pick one", "fields": [
                {"key": "env", "title": "Env", "type": "string", "custom": false,
                 "options": [{"value": "prod", "label": "prod"}, {"value": "2", "label": "Two"}]},
                {"key": "elicitation", "title": "Open the page", "type": "external", "url": "https://x"},
            ]})
        );
        assert_eq!(
            to_json(&Event::FormReplied { id: "frm_1".into() }, "/p").unwrap(),
            json!({"type": "formReplied", "directory": "/p", "formId": "frm_1"})
        );
        assert_eq!(
            to_json(
                &Event::QuestionReplied {
                    session_id: "ses_1".into(),
                    request_id: "frm_2".into()
                },
                "/p"
            )
            .unwrap(),
            json!({"type": "questionReplied", "sessionId": "ses_1", "requestId": "frm_2"})
        );
        assert_eq!(
            to_json(
                &Event::CompactionStarted {
                    session_id: "ses_1".into(),
                    message_id: "msg_1".into(),
                    reason: "auto".into()
                },
                "/p"
            )
            .unwrap(),
            json!({"type": "changed", "sessionId": "ses_1"})
        );
    }
}
