//! Tools whose full behaviour arrives in M1b. They are registered so the request's tool array has
//! 0.5.1's exact shape; their execution is minimal.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::Value;

use super::{DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolResult};
use crate::embedded_def;

pub use super::websearch::WebsearchTool;

embedded_def!(execute_def, "execute");

fn plain(name: &str, def: &super::registry::Def) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: def.description.clone(),
        input_schema: def.input_schema.clone(),
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
