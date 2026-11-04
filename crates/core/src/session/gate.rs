//! The permission gate of a running session: rules decide allow/deny, anything else is asked
//! through a [`Event::PermissionAsked`] event and waits for [`Asks::reply`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use tokio::sync::{broadcast, oneshot};

use crate::events::Event;
use crate::permission::{
    Effect, PermissionError, PermissionGate, PermissionRequest, Rule, evaluate,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Once,
    Always,
    /// Reject; with a message the model sees it and continues, without one the step ends.
    Reject(Option<String>),
}

/// Pending asks of every session in this process.
#[derive(Default)]
pub struct Asks {
    pending: Mutex<HashMap<String, oneshot::Sender<Reply>>>,
}

impl Asks {
    /// Answers a pending ask; false when it is unknown or already answered.
    pub fn reply(&self, request_id: &str, reply: Reply) -> bool {
        let sender = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(request_id);
        sender.is_some_and(|s| s.send(reply).is_ok())
    }
}

pub struct RuleGate {
    pub session_id: String,
    pub rules: Vec<Rule>,
    pub asks: Arc<Asks>,
    pub events: broadcast::Sender<Event>,
}

impl RuleGate {
    fn decide(&self, request: &PermissionRequest) -> Effect {
        let effects: Vec<Effect> = request
            .resources
            .iter()
            .map(|r| evaluate(&request.action, r, &self.rules))
            .collect();
        if effects.contains(&Effect::Deny) {
            Effect::Deny
        } else if effects.contains(&Effect::Ask) {
            Effect::Ask
        } else {
            Effect::Allow
        }
    }
}

impl PermissionGate for RuleGate {
    fn ask(&self, request: PermissionRequest) -> BoxFuture<'_, Result<(), PermissionError>> {
        async move {
            match self.decide(&request) {
                Effect::Allow => return Ok(()),
                Effect::Deny => {
                    return Err(PermissionError::Blocked {
                        action: request.action,
                    });
                }
                Effect::Ask => {}
            }
            let id = crate::ids::permission();
            let (sender, receiver) = oneshot::channel();
            self.asks
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id.clone(), sender);
            let _ = self.events.send(Event::PermissionAsked {
                session_id: self.session_id.clone(),
                request_id: id,
                action: request.action.clone(),
                resources: request.resources.clone(),
            });
            match receiver.await {
                Ok(Reply::Once | Reply::Always) => Ok(()),
                Ok(Reply::Reject(Some(message))) => Err(PermissionError::Corrected(message)),
                Ok(Reply::Reject(None)) | Err(_) => Err(PermissionError::Declined),
            }
        }
        .boxed()
    }

    fn allows(&self, action: &str, resources: &[String]) -> bool {
        resources
            .iter()
            .all(|r| evaluate(action, r, &self.rules) == Effect::Allow)
    }
}
