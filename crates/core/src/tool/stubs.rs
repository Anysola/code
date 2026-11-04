//! Tools whose full behaviour arrives in M1b. They are registered so the request's tool array has
//! 0.5.1's exact shape; their execution is minimal.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;

use super::{DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolResult, parse_input};
use crate::embedded_def;
use crate::permission::PermissionRequest;

pub use super::websearch::WebsearchTool;

embedded_def!(question_def, "question");
embedded_def!(skill_def, "skill");
embedded_def!(subagent_def, "subagent");
embedded_def!(webfetch_def, "webfetch");
embedded_def!(execute_def, "execute");

fn plain(name: &str, def: &super::registry::Def) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: def.description.clone(),
        input_schema: def.input_schema.clone(),
    }
}

const QUESTION_CANCELLED: &str = "This non-interactive run cannot ask the user questions, so the question was cancelled. Continue without an answer; make reasonable assumptions and state them.";

// M1b: question forms (TUI/ACP answers, the `Questions` form and answer template).
pub struct QuestionTool;

impl Tool for QuestionTool {
    fn name(&self) -> &'static str {
        "question"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        plain(self.name(), question_def())
    }

    fn execute<'a>(&'a self, _input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            ctx.ask(PermissionRequest {
                action: "question".into(),
                resources: vec!["*".into()],
                ..Default::default()
            })
            .await?;
            Err(ToolError::execution(QUESTION_CANCELLED))
        }
        .boxed()
    }
}

// M1b: skill discovery and `<skill_content>` loading.
pub struct SkillTool;

#[derive(Deserialize)]
struct SkillInput {
    id: String,
}

impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "skill"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        plain(self.name(), skill_def())
    }

    fn execute<'a>(&'a self, input: Value, _ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: SkillInput = parse_input(self.name(), &skill_def().input_schema, input)?;
            Err(ToolError::execution(format!(
                "Unable to load skill {}",
                input.id
            )))
        }
        .boxed()
    }
}

// M1b: child sessions (foreground, background, continuation).
pub struct SubagentTool;

impl Tool for SubagentTool {
    fn name(&self) -> &'static str {
        "subagent"
    }

    fn definition(&self, env: &DefinitionEnv) -> ToolDefinition {
        let def = subagent_def();
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

    fn execute<'a>(&'a self, _input: Value, _ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            Err(ToolError::execution(
                "Subagents are not available in this build yet.",
            ))
        }
        .boxed()
    }
}

// M1b: fetching, redirects, HTML to Markdown.
pub struct WebfetchTool;

impl Tool for WebfetchTool {
    fn name(&self) -> &'static str {
        "webfetch"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        plain(self.name(), webfetch_def())
    }

    fn execute<'a>(&'a self, _input: Value, _ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            Err(ToolError::execution(
                "Web fetch is not available in this build yet.",
            ))
        }
        .boxed()
    }
}

// M1b: the Code Mode runtime (QuickJS) and the `opencode` namespace.
pub struct ExecuteTool;

impl Tool for ExecuteTool {
    fn name(&self) -> &'static str {
        "execute"
    }

    fn code_mode(&self) -> bool {
        true
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        plain(self.name(), execute_def())
    }

    fn execute<'a>(&'a self, _input: Value, _ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            Err(ToolError::execution(
                "Code Mode is not available in this build yet.",
            ))
        }
        .boxed()
    }
}
