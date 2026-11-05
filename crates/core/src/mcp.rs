//! What the core needs from the MCP runtime (crate `anysola-mcp`), which attaches itself with
//! [`crate::api::Core::attach_mcp`]: server statuses, server instructions for block 2, resources
//! for the `opencode` namespace, prompts as slash commands, and the forms servers ask for.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::{broadcast, oneshot};

use crate::config::Config;
use crate::events::Event;

/// The session id of asks that belong to no session (MCP elicitation forms, status changes).
pub const GLOBAL: &str = "global";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    Connected,
    Pending,
    Disabled,
    Failed { error: String },
    NeedsAuth { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    pub status: Status,
    /// `mcp_<hash>`: the credentials integration of a remote server that may sign in.
    pub integration: Option<String>,
}

/// A connected server's `instructions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instructions {
    pub server: String,
    pub text: String,
    /// False when the server's tools are direct tools.
    pub codemode: bool,
    /// The permission actions (`<namespace>_<tool>`) of its tools.
    pub tools: Vec<String>,
}

/// An MCP prompt as the slash command `<server>:<prompt>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCommand {
    pub name: String,
    pub description: Option<String>,
    pub server: String,
    pub prompt: String,
    /// Argument names in order; the command's words fill them positionally.
    pub arguments: Vec<String>,
}

/// A form an MCP server asks the user to fill in (elicitation).
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub id: String,
    pub title: String,
    /// `{kind:"mcp-elicitation", server, message[, elicitationID]}`.
    pub metadata: Value,
    /// Field descriptions (`{key, type, title, …}`); a URL elicitation has one `external` field.
    pub fields: Vec<Value>,
}

/// An OAuth sign-in in progress: open `url`; `done` settles when the browser comes back.
pub struct SignIn {
    pub url: String,
    pub instructions: String,
    pub done: oneshot::Receiver<Result<(), String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormReply {
    Answered(Map<String, Value>),
    Cancelled,
}

pub trait Mcp: Send + Sync {
    /// Every configured server, sorted by name.
    fn servers(&self) -> Vec<Server>;

    /// The config changed: start, stop or restart the servers whose entries changed.
    fn reconcile(&self, config: Arc<Config>);

    /// Instructions of connected servers, in config order.
    fn instructions(&self) -> Vec<Instructions>;

    /// `{resources, templates}` of one server or of all connected ones.
    fn list_resources<'a>(
        &'a self,
        server: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Value, String>>;

    /// `{server, uri, contents}`; `None` when the server is not connected or has no resources.
    /// Errors are the tool's message (`MCP server not found: …`, `Unable to read MCP resource …`).
    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Option<Value>, String>>;

    fn prompts(&self) -> Vec<PromptCommand>;

    /// The prompt's messages as one user prompt.
    fn expand_prompt<'a>(
        &'a self,
        command: &'a PromptCommand,
        arguments: Vec<String>,
    ) -> BoxFuture<'a, Result<String, String>>;

    /// Connects a server that is disabled for this process, failed, or not signed in.
    fn connect(&self, name: &str);

    /// Disables a server until [`Mcp::connect`] or the end of the process (config is untouched).
    fn disconnect(&self, name: &str);

    /// Starts signing in to a remote server; the server reconnects with the new credential.
    fn authorize<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<SignIn, String>>;
}

/// Forms waiting for a front end.
#[derive(Default)]
pub struct Forms {
    pending: Mutex<HashMap<String, oneshot::Sender<FormReply>>>,
}

impl Forms {
    pub async fn ask(&self, events: &broadcast::Sender<Event>, form: Form) -> FormReply {
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(form.id.clone(), sender);
        let id = form.id.clone();
        let _ = events.send(Event::FormAsked { form });
        let reply = receiver.await.unwrap_or(FormReply::Cancelled);
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        reply
    }

    pub fn reply(&self, id: &str, reply: FormReply) -> bool {
        let sender = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        sender.is_some_and(|s| s.send(reply).is_ok())
    }
}

/// The servers whose instructions a session with these rules sees (one of their tools is
/// reachable), sorted by name, as the block-2 source stores them.
pub fn visible_instructions(
    list: &[Instructions],
    rules: &[crate::permission::Rule],
) -> Option<Value> {
    let execute = !crate::permission::wholly_disabled("execute", rules);
    let mut visible: Vec<&Instructions> = list
        .iter()
        .filter(|i| {
            i.tools
                .iter()
                .any(|t| !crate::permission::wholly_disabled(t, rules))
        })
        .collect();
    visible.sort_by_key(|i| i.server.to_lowercase());
    (!visible.is_empty()).then(|| {
        Value::Array(
            visible
                .iter()
                .map(|i| {
                    let mut entry = serde_json::json!({"server": i.server, "instructions": i.text});
                    if !(execute && i.codemode) {
                        entry["codemode"] = Value::Bool(false);
                    }
                    entry
                })
                .collect(),
        )
    })
}

fn entries(servers: &[Value]) -> Vec<String> {
    servers
        .iter()
        .flat_map(|server| {
            let name = server["server"].as_str().unwrap_or_default();
            let mut lines = vec![format!("  <server name=\"{name}\">")];
            if server.get("codemode") != Some(&Value::Bool(false)) {
                lines.push(format!(
                    "    Use tools from this server through `execute` under `tools[{}]`.",
                    crate::json::compact(&Value::String(crate::codemode::sanitize(name)))
                ));
            }
            lines.extend(
                server["instructions"]
                    .as_str()
                    .unwrap_or_default()
                    .split('\n')
                    .map(|l| format!("    {l}")),
            );
            lines.push("  </server>".into());
            lines
        })
        .collect()
}

pub(crate) fn render_instructions(value: &Value) -> Option<String> {
    let servers = value.as_array()?;
    let mut lines = vec!["<mcp_instructions>".to_string()];
    lines.extend(entries(servers));
    lines.push("</mcp_instructions>".into());
    Some(lines.join("\n"))
}

pub(crate) fn instructions_update(previous: &Value, current: &Value) -> Option<String> {
    let (before, after) = (previous.as_array()?, current.as_array()?);
    let find =
        |list: &[Value], server: &Value| list.iter().find(|s| s["server"] == *server).cloned();
    let added: Vec<Value> = after
        .iter()
        .filter(|a| find(before, &a["server"]).is_none())
        .cloned()
        .collect();
    let changed: Vec<Value> = after
        .iter()
        .filter(|a| find(before, &a["server"]).is_some_and(|b| b != **a))
        .cloned()
        .collect();
    let removed: Vec<&str> = before
        .iter()
        .filter(|b| find(after, &b["server"]).is_none())
        .filter_map(|b| b["server"].as_str())
        .collect();
    if added.is_empty() && changed.is_empty() && removed.is_empty() {
        return Some(format!(
            "The available MCP server instructions have changed. This list supersedes the previous one.\n{}",
            render_instructions(current)?
        ));
    }
    let mut lines = Vec::new();
    if !added.is_empty() {
        lines.push(
            "New MCP server instructions are available in addition to those previously listed:"
                .to_string(),
        );
        lines.extend(entries(&added));
    }
    if !changed.is_empty() {
        lines.push("The following MCP server instructions have changed. Each entry replaces the previously listed server with the same name:".into());
        lines.extend(entries(&changed));
    }
    if !removed.is_empty() {
        lines.push(format!(
            "Instructions for the following MCP servers are no longer available: {}.",
            removed.join(", ")
        ));
    }
    Some(lines.join("\n"))
}
