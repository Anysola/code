//! Code Mode: tools that live in namespaces (`tools.<namespace>.<tool>`) and are only reachable
//! from JavaScript run by the `execute` tool, their catalog in system block 2, and the confined
//! QuickJS runtime that runs the programs.
//!
//! The inventory is a list of namespaces. Built-in tools form the `opencode` namespace; MCP
//! servers each add one when they connect (M5), and the session then announces the catalog change
//! with [`catalog::update`].

pub mod catalog;
pub mod execute;
pub mod opencode;
mod runtime;
pub mod search;
pub mod typescript;

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::Value;

use crate::permission::{Rule, wholly_disabled};
use crate::tool::{ToolContent, ToolContext, ToolError};

/// What a namespace tool returns: structured `output` (what a program receives) and the text or
/// files a direct call shows the model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Called {
    pub output: Option<Value>,
    pub content: Vec<ToolContent>,
}

impl Called {
    pub fn output(output: Value) -> Self {
        Called {
            output: Some(output),
            ..Called::default()
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.content.push(ToolContent::text(text));
        self
    }

    /// A program's value: the structured output, else the text parts joined (or null).
    pub fn value(&self) -> Value {
        if let Some(output) = &self.output {
            return output.clone();
        }
        let text: Vec<&str> = self
            .content
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.as_str()),
                ToolContent::File { .. } => None,
            })
            .collect();
        if text.is_empty() || text.concat().is_empty() {
            Value::Null
        } else {
            Value::String(text.join("\n"))
        }
    }

    /// What a direct call shows the model: the content, else the output as compact JSON.
    pub fn content(&self) -> Vec<ToolContent> {
        if !self.content.is_empty() {
            return self.content.clone();
        }
        let text = match &self.output {
            Some(Value::String(text)) => text.clone(),
            Some(value) => crate::json::compact(value),
            None => String::new(),
        };
        vec![ToolContent::text(text)]
    }
}

/// What the `opencode` namespace tools need from the running core besides the tool context.
pub trait Host: Send + Sync {
    fn db(&self) -> &crate::storage::Db;
    fn models(&self) -> std::sync::Arc<[crate::model::Model]>;
    fn config(&self) -> std::sync::Arc<crate::config::Config>;
    /// The formatters edits and writes run, when config `formatter` enables them.
    fn formatters(&self) -> Option<std::sync::Arc<crate::formatter::Formatters>> {
        None
    }
}

pub trait NamespaceTool: Send + Sync {
    fn name(&self) -> &str;

    fn description(&self) -> &str;

    fn input_schema(&self) -> &Value;

    /// `None`: the tool returns text (`string | null`).
    fn output_schema(&self) -> Option<&Value> {
        None
    }

    /// Always listed in the catalog, before the budget is shared out.
    fn pinned(&self) -> bool {
        false
    }

    /// The permission action that hides the tool when wholly denied, if not `<namespace>_<name>`.
    fn permission(&self) -> Option<&str> {
        None
    }

    fn call<'a>(
        &'a self,
        input: Value,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<Called, ToolError>>;
}

pub struct Namespace {
    pub name: String,
    pub description: Option<String>,
    /// In registration order, which is the order `Object.keys(tools.<namespace>)` shows.
    pub tools: Vec<Arc<dyn NamespaceTool>>,
}

/// Tool names as JavaScript paths and direct-tool names allow them.
pub fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `<namespace>_<tool>`: the name a namespace tool has as a direct tool, and its permission action.
pub fn direct_name(namespace: &str, tool: &str) -> String {
    format!("{}_{}", namespace.replace('.', "_"), sanitize(tool))
}

/// The Code Mode tools one agent can reach.
#[derive(Clone, Default)]
pub struct Inventory {
    namespaces: Vec<Arc<Namespace>>,
}

impl Inventory {
    pub fn new(namespaces: Vec<Arc<Namespace>>) -> Self {
        Inventory { namespaces }
    }

    /// The same inventory without the tools these rules wholly deny.
    pub fn visible(&self, rules: &[Rule]) -> Inventory {
        let namespaces = self
            .namespaces
            .iter()
            .map(|namespace| {
                let tools: Vec<_> = namespace
                    .tools
                    .iter()
                    .filter(|tool| {
                        let action = tool
                            .permission()
                            .map(str::to_string)
                            .unwrap_or_else(|| direct_name(&namespace.name, tool.name()));
                        !wholly_disabled(&action, rules)
                    })
                    .cloned()
                    .collect();
                Arc::new(Namespace {
                    name: namespace.name.clone(),
                    description: namespace.description.clone(),
                    tools,
                })
            })
            .collect();
        Inventory { namespaces }
    }

    pub fn namespaces(&self) -> &[Arc<Namespace>] {
        &self.namespaces
    }

    /// Every tool with its namespace, in registration order.
    pub fn tools(&self) -> impl Iterator<Item = (&Namespace, &Arc<dyn NamespaceTool>)> {
        self.namespaces
            .iter()
            .flat_map(|n| n.tools.iter().map(move |t| (n.as_ref(), t)))
    }

    /// Finds `namespace.tool`.
    pub fn find(&self, path: &str) -> Option<(&Namespace, &Arc<dyn NamespaceTool>)> {
        self.tools()
            .find(|(n, t)| path_of(&n.name, t.name()) == path)
    }

    pub fn summary(&self) -> catalog::Summary {
        let entries: Vec<catalog::Entry> = self
            .tools()
            .map(|(namespace, tool)| {
                let path = path_of(&namespace.name, tool.name());
                catalog::Entry {
                    signature: typescript::signature(
                        &path,
                        tool.input_schema(),
                        tool.output_schema(),
                    ),
                    description: tool.description().to_string(),
                    pinned: tool.pinned(),
                    path,
                }
            })
            .collect();
        let namespaces: Vec<(String, Option<String>)> = self
            .namespaces
            .iter()
            .map(|n| (n.name.clone(), n.description.clone()))
            .collect();
        catalog::summarize(&entries, &namespaces)
    }
}

pub fn path_of(namespace: &str, tool: &str) -> String {
    format!("{namespace}.{}", sanitize(tool))
}
#[cfg(test)]
mod tests;
