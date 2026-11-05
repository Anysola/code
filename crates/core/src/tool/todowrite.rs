//! The `todowrite` tool (`experimental.todo`): the model's working plan. The session's list is the
//! input of its latest completed `todowrite` call.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolOutput, ToolResult, parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;

embedded_def!(def, "todowrite");

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Todo {
    pub content: String,
    pub status: String,
}

#[derive(Deserialize)]
struct Input {
    todos: Vec<Todo>,
}

/// Whether config `experimental.todo` is `true`.
pub fn enabled(config: &crate::config::Config) -> bool {
    config
        .get("experimental")
        .and_then(|e| e.get("todo"))
        .and_then(Value::as_bool)
        == Some(true)
}

/// The model-visible result text.
fn summarize(todos: &[Todo]) -> String {
    if todos.is_empty() {
        return "Todos cleared".into();
    }
    let count = |status: &str| todos.iter().filter(|t| t.status == status).count();
    let mut counts = vec![format!("{}/{} completed", count("completed"), todos.len())];
    for status in ["in_progress", "pending", "cancelled"] {
        let n = count(status);
        if n > 0 {
            counts.push(format!("{n} {}", status.replacen('_', " ", 1)));
        }
    }
    let active: Vec<String> = todos
        .iter()
        .filter(|t| t.status == "in_progress")
        .map(|t| crate::json::compact(&Value::String(t.content.clone())))
        .collect();
    let active = if active.is_empty() {
        String::new()
    } else {
        format!(". In progress: {}", active.join(", "))
    };
    format!("Todos updated: {}{active}", counts.join(", "))
}

pub struct TodowriteTool;

impl Tool for TodowriteTool {
    fn name(&self) -> &'static str {
        "todowrite"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            ctx.ask(PermissionRequest {
                action: self.name().into(),
                resources: vec!["*".into()],
                save: vec!["*".into()],
                ..Default::default()
            })
            .await?;
            Ok(
                ToolOutput::text(summarize(&input.todos))
                    .with_metadata("todos", json!(input.todos)),
            )
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionError;
    use crate::tool::ToolContent;
    use crate::tool::test_support::context;

    fn todo(content: &str, status: &str) -> Todo {
        Todo {
            content: content.into(),
            status: status.into(),
        }
    }

    #[test]
    fn summaries() {
        assert_eq!(summarize(&[]), "Todos cleared");
        assert_eq!(
            summarize(&[todo("a", "completed"), todo("b", "pending")]),
            "Todos updated: 1/2 completed, 1 pending"
        );
        assert_eq!(
            summarize(&[
                todo("say \"hi\"", "in_progress"),
                todo("b", "in_progress"),
                todo("c", "cancelled"),
                todo("d", "pending"),
            ]),
            "Todos updated: 0/4 completed, 2 in progress, 1 pending, 1 cancelled. In progress: \"say \\\"hi\\\"\", \"b\""
        );
    }

    #[tokio::test]
    async fn asks_and_records_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let test = context(dir.path());
        let todos = json!([{"content": "a", "status": "in_progress"}]);
        let out = TodowriteTool
            .execute(json!({"todos": todos}), &test.ctx)
            .await
            .unwrap();
        assert_eq!(
            out.content,
            [ToolContent::text(
                "Todos updated: 0/1 completed, 1 in progress. In progress: \"a\""
            )]
        );
        assert_eq!(out.metadata["todos"], todos);
        let asked = test.gate.asked.lock().unwrap().clone();
        assert_eq!(
            (
                asked[0].action.as_str(),
                asked[0].resources.clone(),
                asked[0].save.clone()
            ),
            ("todowrite", vec!["*".to_string()], vec!["*".to_string()])
        );

        *test.gate.answer.lock().unwrap() = Some(PermissionError::Blocked {
            action: "todowrite".into(),
            reason: Some("Blocked by configuration policy".into()),
        });
        let error = TodowriteTool
            .execute(json!({"todos": []}), &test.ctx)
            .await
            .unwrap_err();
        assert_eq!(error.message, "Blocked by configuration policy");
        let invalid = TodowriteTool
            .execute(
                json!({"todos": [{"content": "a", "status": "doing"}]}),
                &test.ctx,
            )
            .await
            .unwrap_err();
        assert!(
            invalid
                .message
                .starts_with("Invalid arguments for tool \"todowrite\"")
        );
    }
}
