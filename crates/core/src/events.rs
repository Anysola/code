//! Events a front end follows while a session runs. Every event names its session.

use serde_json::{Map, Value};

use crate::gateway::error::SessionError;
use crate::session::message::Tokens;
use crate::tool::ToolContent;

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
    },
    TextStarted {
        session_id: String,
        message_id: String,
        event_id: String,
        ordinal: usize,
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
}

impl Event {
    pub fn session_id(&self) -> &str {
        match self {
            Event::StepStarted { session_id, .. }
            | Event::TextStarted { session_id, .. }
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
            | Event::TitleChanged { session_id, .. }
            | Event::Succeeded { session_id }
            | Event::Failed { session_id, .. }
            | Event::Interrupted { session_id, .. } => session_id,
            Event::Credits(snapshot) => &snapshot.session_id,
        }
    }
}
