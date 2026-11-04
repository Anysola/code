//! Permission rules and the gate tools ask through.
//!
//! A ruleset is an ordered list; the last rule matching both action and resource wins and no match
//! means `ask`. The interactive side (asking, replies, saved "always" rules) lives behind
//! [`PermissionGate`], implemented by the session layer. Managed [`policy`] statements can deny
//! what rules allow.

pub mod policy;
mod wildcard;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use wildcard::matches;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub action: String,
    pub resource: String,
    pub effect: Effect,
}

impl Rule {
    pub fn new(action: &str, resource: &str, effect: Effect) -> Self {
        Rule {
            action: action.into(),
            resource: resource.into(),
            effect,
        }
    }
}

pub fn evaluate(action: &str, resource: &str, rules: &[Rule]) -> Effect {
    rules
        .iter()
        .rev()
        .find(|rule| matches(action, &rule.action) && matches(resource, &rule.resource))
        .map(|rule| rule.effect)
        .unwrap_or(Effect::Ask)
}

/// A tool is left out of the request when the last rule matching its action denies `*`.
pub fn wholly_disabled(action: &str, rules: &[Rule]) -> bool {
    rules
        .iter()
        .rev()
        .find(|rule| matches(action, &rule.action))
        .is_some_and(|rule| rule.resource == "*" && rule.effect == Effect::Deny)
}

#[derive(Debug, Clone, Default)]
pub struct PermissionRequest {
    pub action: String,
    pub resources: Vec<String>,
    /// Patterns stored when the user answers "always".
    pub save: Vec<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PermissionError {
    /// A rule or a policy denies the request; a policy says why.
    #[error("{}", reason.clone().unwrap_or_else(|| format!("Permission denied: {action}")))]
    Blocked {
        action: String,
        reason: Option<String>,
    },
    /// The user rejected without a message: the step ends.
    #[error("The user declined this tool call")]
    Declined,
    /// The user rejected with feedback for the model.
    #[error("{0}")]
    Corrected(String),
}

pub trait PermissionGate: Send + Sync {
    /// Resolves when the request is allowed (by rule or by the user).
    fn ask(&self, request: PermissionRequest) -> BoxFuture<'_, Result<(), PermissionError>>;
    /// True only when the rules already allow every resource; never asks.
    fn allows(&self, action: &str, resources: &[String]) -> bool;
    /// True when a rule denies any of the resources; never asks. Gates without rules deny nothing.
    fn denies(&self, _action: &str, _resources: &[String]) -> bool {
        false
    }
}
