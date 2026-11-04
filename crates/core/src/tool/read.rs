//! The `read` tool.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::Value;

use super::{DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolResult};
use crate::embedded_def;

embedded_def!(def, "read");

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, _input: Value, _ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move { Err(ToolError::execution("not implemented")) }.boxed()
    }
}
