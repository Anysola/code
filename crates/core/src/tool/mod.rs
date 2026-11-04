//! Tools the model can call. Each tool owns its verbatim description and JSON schema (sent to the
//! model exactly as 0.5.1 sent them) and its execution. The [`registry`] builds the request's tool
//! array in 0.5.1's order.

pub mod access;
pub mod bound;
mod input;
pub mod registry;
mod search;

pub mod edit;
pub mod glob;
pub mod grep;
pub mod question;
pub mod read;
pub mod shell;
pub mod skill;
pub mod stubs;
pub mod subagent;
pub mod websearch;
pub mod write;

#[cfg(test)]
pub mod test_support;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::paths::Paths;
use crate::permission::{PermissionError, PermissionGate, PermissionRequest};

pub use input::parse_input;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolContent {
    Text {
        text: String,
    },
    File {
        uri: String,
        mime: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl ToolContent {
    pub fn text(text: impl Into<String>) -> Self {
        ToolContent::Text { text: text.into() }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutput {
    /// Model-visible content, at least one item.
    pub content: Vec<ToolContent>,
    /// UI/storage metadata. A `truncated` key means the tool bounded its own output, so the
    /// generic 2000-line / 51200-byte bounding is skipped.
    pub metadata: Map<String, Value>,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        ToolOutput {
            content: vec![ToolContent::text(text)],
            metadata: Map::new(),
        }
    }

    pub fn with_metadata(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.metadata.insert(key.to_string(), value.into());
        self
    }
}

/// How a failure reaches the model: `{"error":{"type":<tag>,"message":…},"content":[]}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorKind {
    /// `tool.execution`: a plain tool failure.
    Execution,
    /// `unknown`: a failure carrying an underlying error's own message.
    Unknown,
    /// `permission.rejected`: denied by a rule or rejected with feedback.
    PermissionRejected,
    /// `aborted`: the user declined without feedback; the step ends.
    Declined,
    /// `aborted`: the turn was interrupted while the tool ran.
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ToolError {
    pub kind: ToolErrorKind,
    pub message: String,
}

impl ToolError {
    pub fn execution(message: impl Into<String>) -> Self {
        ToolError {
            kind: ToolErrorKind::Execution,
            message: message.into(),
        }
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        ToolError {
            kind: ToolErrorKind::Unknown,
            message: message.into(),
        }
    }

    pub fn interrupted() -> Self {
        ToolError {
            kind: ToolErrorKind::Interrupted,
            message: "Tool execution interrupted".into(),
        }
    }

    pub fn type_tag(&self) -> &'static str {
        match self.kind {
            ToolErrorKind::Execution => "tool.execution",
            ToolErrorKind::Unknown => "unknown",
            ToolErrorKind::PermissionRejected => "permission.rejected",
            ToolErrorKind::Declined | ToolErrorKind::Interrupted => "aborted",
        }
    }
}

impl From<PermissionError> for ToolError {
    fn from(error: PermissionError) -> Self {
        let kind = match error {
            PermissionError::Declined => ToolErrorKind::Declined,
            PermissionError::Blocked { .. } | PermissionError::Corrected(_) => {
                ToolErrorKind::PermissionRejected
            }
        };
        ToolError {
            kind,
            message: error.to_string(),
        }
    }
}

pub type ToolResult = Result<ToolOutput, ToolError>;

/// What a definition may depend on besides the tool itself.
#[derive(Debug, Clone)]
pub struct DefinitionEnv {
    /// Shell named in the shell tool description (`bash`, `zsh`, `pwsh`, …).
    pub shell_name: String,
    /// Year for the websearch description (local time at process start).
    pub year: i32,
    /// Subagents the caller may spawn: `(id, description)`, sorted by id.
    pub subagents: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;

    /// Permission action used to hide the whole tool (`write` shares `edit`).
    fn permission(&self) -> &'static str {
        self.name()
    }

    /// Tools that only run inside Code Mode's `execute`. Every M1 tool is direct.
    fn code_mode(&self) -> bool {
        false
    }

    fn definition(&self, env: &DefinitionEnv) -> ToolDefinition;

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult>;
}

/// Limits of the generic output bounding (`tool_output` config).
#[derive(Debug, Clone, Copy)]
pub struct OutputLimits {
    pub max_lines: usize,
    pub max_bytes: usize,
}

impl Default for OutputLimits {
    fn default() -> Self {
        OutputLimits {
            max_lines: 2000,
            max_bytes: 51200,
        }
    }
}

/// Receives text that a background job (a background shell) posts back into its session.
pub trait SessionNotifier: Send + Sync {
    fn synthetic(&self, session_id: &str, text: String);
}

/// Session services behind the agent-facing tools: asking the user, skills, child sessions.
pub trait SessionHost: Send + Sync {
    /// Shows the questions to whoever drives the session and waits for the reply.
    fn ask_questions(
        &self,
        session_id: &str,
        questions: Vec<question::Question>,
    ) -> BoxFuture<'_, question::QuestionReply>;

    fn skill(&self, id: &str) -> Option<crate::skill::Skill>;

    /// Runs (or continues) a child session for the `subagent` tool.
    fn subagent<'a>(
        &'a self,
        ctx: &'a ToolContext,
        request: subagent::SubagentRequest,
    ) -> BoxFuture<'a, ToolResult>;
}

/// Everything a tool call can see.
#[derive(Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub message_id: String,
    pub call_id: String,
    pub agent: String,
    /// The session's working directory.
    pub directory: PathBuf,
    /// The project root (VCS root, else the directory).
    pub worktree: PathBuf,
    pub project_id: String,
    pub paths: Arc<Paths>,
    pub permissions: Arc<dyn PermissionGate>,
    pub cancel: CancellationToken,
    /// Base environment for child processes.
    pub env: Arc<Vec<(OsString, OsString)>>,
    /// `shell` from config, if set.
    pub configured_shell: Option<String>,
    pub limits: OutputLimits,
    pub locks: Arc<PathLocks>,
    pub notifier: Arc<dyn SessionNotifier>,
    pub web_search: Arc<dyn websearch::WebSearch>,
    pub host: Arc<dyn SessionHost>,
}

impl ToolContext {
    pub async fn ask(&self, request: PermissionRequest) -> Result<(), ToolError> {
        self.permissions.ask(request).await.map_err(ToolError::from)
    }
}

/// Per-path async mutexes: edits and writes of one file never interleave.
#[derive(Default)]
pub struct PathLocks {
    locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

impl PathLocks {
    pub async fn lock(&self, path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.locks.lock().unwrap_or_else(|e| e.into_inner());
            map.entry(path.to_path_buf()).or_default().clone()
        };
        lock.lock_owned().await
    }
}
