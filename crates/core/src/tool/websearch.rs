//! The `websearch` tool: Anysola's gateway is the only provider.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;

use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult,
    parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;

embedded_def!(def, "websearch");

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub url: String,
    pub title: Option<String>,
    pub content: Option<String>,
    /// Epoch milliseconds.
    pub published: Option<u64>,
}

/// Runs a search for a session. Errors are the model-visible failure texts.
pub trait WebSearch: Send + Sync {
    fn search<'a>(
        &'a self,
        session_id: &'a str,
        query: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, String>>;
}

pub struct WebsearchTool;

#[derive(Deserialize)]
struct Input {
    query: String,
}

pub fn format_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No search results found. Please try a different query.".into();
    }
    results
        .iter()
        .map(|result| {
            let mut text = format!(
                "## [{}]({})",
                result.title.as_deref().unwrap_or(&result.url),
                result.url
            );
            if let Some(published) = result.published {
                text.push_str(&format!("\nPublished: {}", crate::time::iso(published)));
            }
            if let Some(content) = &result.content {
                text.push_str(&format!("\n\n{content}"));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

impl Tool for WebsearchTool {
    fn name(&self) -> &'static str {
        "websearch"
    }

    fn definition(&self, env: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.replace("{year}", &env.year.to_string()),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let raw = input.clone();
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            ctx.ask(PermissionRequest {
                action: self.name().into(),
                resources: vec![input.query.clone()],
                save: vec!["*".into()],
                metadata: raw,
            })
            .await?;
            let results = ctx
                .web_search
                .search(&ctx.session_id, &input.query)
                .await
                .map_err(ToolError::execution)?;
            Ok(ToolOutput::text(format_results(&results)).with_metadata("provider", "anysola"))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::registry::process_year;

    #[test]
    fn output_format() {
        let results = vec![
            SearchResult {
                url: "https://a.b".into(),
                title: None,
                content: None,
                published: Some(1_790_000_000_000),
            },
            SearchResult {
                url: "https://c.d".into(),
                title: Some("C".into()),
                content: Some("cc".into()),
                published: None,
            },
        ];
        assert_eq!(
            format_results(&results),
            "## [https://a.b](https://a.b)\nPublished: 2026-09-21T14:13:20.000Z\n\n## [C](https://c.d)\n\ncc"
        );
        assert_eq!(
            format_results(&[]),
            "No search results found. Please try a different query."
        );
    }

    #[test]
    fn year_in_description() {
        let env = DefinitionEnv {
            shell_name: "bash".into(),
            year: process_year(),
            subagents: vec![],
        };
        assert!(
            WebsearchTool
                .definition(&env)
                .description
                .contains(&format!("The current year is {}.", process_year()))
        );
    }
}
