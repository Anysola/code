//! The `subagent` tool: runs an agent in a child session (foreground or background) or continues
//! a direct child. The session layer does the work behind [`super::SessionHost::subagent`].

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;

use super::{DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolResult, parse_input};
use crate::embedded_def;

embedded_def!(def, "subagent");

pub const NO_TEXT: &str = "Subagent completed without a text response.";
pub const FIRST_PROMPT: &str = "You are a subagent spawned by another session.";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SubagentRequest {
    pub agent: String,
    pub description: String,
    pub prompt: String,
    pub model: Option<String>,
    #[serde(rename = "sessionID")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub background: bool,
}

/// How a child session's run ended, as reported back to the parent.
#[derive(Debug, Clone, PartialEq)]
pub enum ChildOutcome {
    /// The child's final text (or [`NO_TEXT`]).
    Completed(String),
    Failed(String),
    Cancelled,
}

impl ChildOutcome {
    fn state(&self) -> &'static str {
        match self {
            ChildOutcome::Completed(_) => "completed",
            ChildOutcome::Failed(_) => "error",
            ChildOutcome::Cancelled => "cancelled",
        }
    }
}

pub fn completed_text(session_id: &str, text: &str) -> String {
    format!("<subagent sessionID=\"{session_id}\" state=\"completed\">\n{text}\n</subagent>")
}

pub fn background_text(session_id: &str) -> String {
    [
        format!(
            "The subagent is working in the background (sessionID: {session_id}). You will be notified automatically when it finishes."
        ),
        "DO NOT sleep, poll for progress, ask the subagent for status, or duplicate this subagent's work; avoid working with the same files or topics it is using.".into(),
        "Work on non-overlapping tasks, or briefly tell the user what you launched and end your response.".into(),
        "To send it follow-ups or change its instructions while it runs, call subagent again with this sessionID.".into(),
    ]
    .join("\n")
}

/// The synthetic message a background child posts into its parent when it ends.
pub fn completion_notice(session_id: &str, description: &str, outcome: &ChildOutcome) -> String {
    let text = match outcome {
        ChildOutcome::Completed(text) | ChildOutcome::Failed(text) => text.as_str(),
        ChildOutcome::Cancelled => "Subagent cancelled",
    };
    format!(
        "<subagent sessionID=\"{session_id}\" state=\"{}\" description=\"{description}\">\n{text}\n</subagent>",
        outcome.state()
    )
}

pub struct SubagentTool;

impl Tool for SubagentTool {
    fn name(&self) -> &'static str {
        "subagent"
    }

    fn definition(&self, env: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        let mut description = def.description.clone();
        if !env.subagents.is_empty() {
            description.push_str("\n\nAvailable subagents:");
            for (id, text) in &env.subagents {
                let text = text
                    .as_deref()
                    .unwrap_or("This subagent should only be called when explicitly requested.");
                description.push_str(&format!("\n- {id}: {text}"));
            }
        }
        ToolDefinition {
            name: self.name().into(),
            description,
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            // Empty optional strings count as absent, before the `^ses` pattern sees them.
            let mut input = input;
            if let Value::Object(map) = &mut input {
                map.retain(|key, value| {
                    !((key == "model" || key == "sessionID") && value.as_str() == Some(""))
                });
            }
            let request: SubagentRequest = parse_input(self.name(), &def().input_schema, input)?;
            ctx.host.subagent(ctx, request).await
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::test_support::context;
    use serde_json::json;

    #[test]
    fn texts() {
        assert_eq!(
            completed_text("ses_1", "child general result"),
            "<subagent sessionID=\"ses_1\" state=\"completed\">\nchild general result\n</subagent>"
        );
        assert!(background_text("ses_1").starts_with(
            "The subagent is working in the background (sessionID: ses_1). You will be notified"
        ));
        assert_eq!(background_text("ses_1").lines().count(), 4);
        assert_eq!(
            completion_notice("ses_1", "find x", &ChildOutcome::Cancelled),
            "<subagent sessionID=\"ses_1\" state=\"cancelled\" description=\"find x\">\nSubagent cancelled\n</subagent>"
        );
        assert_eq!(
            completion_notice("ses_1", "d", &ChildOutcome::Failed("boom".into())),
            "<subagent sessionID=\"ses_1\" state=\"error\" description=\"d\">\nboom\n</subagent>"
        );
    }

    #[tokio::test]
    async fn empty_optionals_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let test = context(dir.path());
        let _ = SubagentTool
            .execute(
                json!({"agent": "general", "description": "d", "prompt": "p", "model": "", "sessionID": ""}),
                &test.ctx,
            )
            .await;
        let requests = test.host.subagents.lock().unwrap();
        assert_eq!(
            requests[0],
            SubagentRequest {
                agent: "general".into(),
                description: "d".into(),
                prompt: "p".into(),
                model: None,
                session_id: None,
                background: false,
            }
        );
    }
}
