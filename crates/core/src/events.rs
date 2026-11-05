//! Events a front end follows while a session runs. Every event names its session.

use serde_json::{Map, Value};

use crate::gateway::error::SessionError;
use crate::session::message::Tokens;
use crate::tool::ToolContent;
use crate::tool::question::Question;

#[derive(Debug, Clone, PartialEq)]
pub struct CreditsSnapshot {
    pub session_id: String,
    pub remaining: Option<u64>,
    pub turn: u64,
    pub total: u64,
    pub turns: u64,
    pub budget: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptReason {
    /// The user (or the run's SIGINT) interrupted.
    User,
    /// The session reached its credit budget.
    Budget,
    /// The user declined a tool call.
    Declined,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StepStarted {
        session_id: String,
        message_id: String,
        event_id: String,
        agent: String,
        model_id: String,
        /// The tree captured as the step started.
        snapshot: Option<String>,
    },
    TextStarted {
        session_id: String,
        message_id: String,
        event_id: String,
        ordinal: usize,
    },
    /// Text streamed into a text block; front ends render it before the block ends.
    TextDelta {
        session_id: String,
        message_id: String,
        ordinal: usize,
        text: String,
    },
    ReasoningDelta {
        session_id: String,
        message_id: String,
        ordinal: usize,
        text: String,
    },
    TextEnded {
        session_id: String,
        message_id: String,
        event_id: String,
        ordinal: usize,
        text: String,
    },
    ReasoningStarted {
        session_id: String,
        message_id: String,
        event_id: String,
        ordinal: usize,
    },
    ReasoningEnded {
        session_id: String,
        message_id: String,
        event_id: String,
        ordinal: usize,
        text: String,
        native: Option<Value>,
    },
    ToolInputStarted {
        session_id: String,
        message_id: String,
        event_id: String,
        call_id: String,
        name: String,
    },
    ToolCalled {
        session_id: String,
        message_id: String,
        call_id: String,
        input: Value,
        raw: String,
    },
    ToolSucceeded {
        session_id: String,
        message_id: String,
        call_id: String,
        content: Vec<ToolContent>,
        metadata: Map<String, Value>,
    },
    ToolFailed {
        session_id: String,
        message_id: String,
        call_id: String,
        error: SessionError,
        content: Vec<ToolContent>,
        metadata: Map<String, Value>,
    },
    StepEnded {
        session_id: String,
        message_id: String,
        event_id: String,
        finish: String,
        tokens: Tokens,
        /// The turn ends after this step.
        last: bool,
        /// The tree captured as the step ended.
        snapshot: Option<String>,
    },
    StepFailed {
        session_id: String,
        message_id: String,
        error: SessionError,
    },
    RetryScheduled {
        session_id: String,
        attempt: u32,
        delay_ms: u64,
        error: SessionError,
    },
    PermissionAsked {
        session_id: String,
        request_id: String,
        action: String,
        resources: Vec<String>,
        /// Patterns "always" would save; empty when the request offers no "always".
        save: Vec<String>,
        /// What the tool attached for the prompt (an edit's file diffs, a search's input).
        metadata: Value,
    },
    /// A pending ask was answered: by the user, by an "always" that now covers it, or rejected
    /// along with another ask of its session or at shutdown.
    PermissionReplied {
        session_id: String,
        request_id: String,
        reply: PermissionReply,
    },
    /// The question tool waits for [`crate::api::Core::reply_question`].
    QuestionAsked {
        session_id: String,
        request_id: String,
        questions: Vec<Question>,
    },
    Credits(CreditsSnapshot),
    TitleChanged {
        session_id: String,
        title: String,
    },
    Succeeded {
        session_id: String,
    },
    Failed {
        session_id: String,
        error: SessionError,
    },
    Interrupted {
        session_id: String,
        reason: InterruptReason,
    },
    /// An MCP server asks for input; answered with [`crate::api::Core::reply_form`]. Session
    /// [`crate::mcp::GLOBAL`].
    FormAsked {
        form: crate::mcp::Form,
    },
    /// An MCP server's status changed. Session [`crate::mcp::GLOBAL`].
    McpStatusChanged {
        server: crate::mcp::Server,
    },
    /// The session's `/goal` was set, changed or cleared (`None`).
    GoalChanged {
        session_id: String,
        goal: Option<crate::session::goal::Goal>,
    },
    /// A session was created; subagents create children of the running session.
    SessionCreated {
        session_id: String,
        parent_id: Option<String>,
        title: String,
    },
    /// The session started working through its inbox.
    Started {
        session_id: String,
    },
    /// A summary of the history started; `message_id` is its compaction message, `reason`
    /// `auto` or `manual`.
    CompactionStarted {
        session_id: String,
        message_id: String,
        reason: String,
    },
    CompactionEnded {
        session_id: String,
        message_id: String,
        reason: String,
        summary: String,
    },
    CompactionFailed {
        session_id: String,
        message_id: String,
        reason: String,
        error: SessionError,
    },
}

impl Event {
    pub fn session_id(&self) -> &str {
        match self {
            Event::StepStarted { session_id, .. }
            | Event::TextStarted { session_id, .. }
            | Event::TextDelta { session_id, .. }
            | Event::ReasoningDelta { session_id, .. }
            | Event::TextEnded { session_id, .. }
            | Event::ReasoningStarted { session_id, .. }
            | Event::ReasoningEnded { session_id, .. }
            | Event::ToolInputStarted { session_id, .. }
            | Event::ToolCalled { session_id, .. }
            | Event::ToolSucceeded { session_id, .. }
            | Event::ToolFailed { session_id, .. }
            | Event::StepEnded { session_id, .. }
            | Event::StepFailed { session_id, .. }
            | Event::RetryScheduled { session_id, .. }
            | Event::PermissionAsked { session_id, .. }
            | Event::QuestionAsked { session_id, .. }
            | Event::PermissionReplied { session_id, .. }
            | Event::TitleChanged { session_id, .. }
            | Event::Succeeded { session_id }
            | Event::Failed { session_id, .. }
            | Event::Interrupted { session_id, .. }
            | Event::GoalChanged { session_id, .. }
            | Event::SessionCreated { session_id, .. }
            | Event::Started { session_id }
            | Event::CompactionStarted { session_id, .. }
            | Event::CompactionEnded { session_id, .. }
            | Event::CompactionFailed { session_id, .. } => session_id,
            Event::Credits(snapshot) => &snapshot.session_id,
            Event::FormAsked { .. } | Event::McpStatusChanged { .. } => crate::mcp::GLOBAL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionReply {
    Once,
    Always,
    Reject,
}
