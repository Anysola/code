//! The permission gate of a running session. A request is denied when a configured rule denies
//! any resource (saved "always" rules cannot override that), else evaluated with the project's
//! saved rules appended: any `ask` asks, through an [`Event::PermissionAsked`] event answered with
//! [`Asks::reply`]. Managed policies can still turn the result into a denial.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use tokio::sync::{broadcast, oneshot};

use crate::events::{Event, PermissionReply};
use crate::permission::policy::Policies;
use crate::permission::{
    Effect, PermissionError, PermissionGate, PermissionRequest, Rule, evaluate,
};
use crate::storage::Db;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Once,
    Always,
    /// Reject; with a message the model sees it and continues, without one the step ends.
    Reject(Option<String>),
}

/// What a session's requests are evaluated against.
pub struct Scope {
    /// The agent's rules followed by the session's own.
    pub configured: Vec<Rule>,
    pub project_id: String,
    pub policies: Arc<Policies>,
}

struct Decision {
    effect: Effect,
    reason: Option<String>,
}

impl Scope {
    fn decide(&self, db: &Db, action: &str, resources: &[String]) -> Decision {
        let denied = resources
            .iter()
            .any(|r| evaluate(action, r, &self.configured) == Effect::Deny);
        if denied {
            return Decision {
                effect: Effect::Deny,
                reason: None,
            };
        }
        let mut rules = self.configured.clone();
        rules.extend(db.saved_permissions(&self.project_id).unwrap_or_default());
        let ask = resources
            .iter()
            .any(|r| evaluate(action, r, &rules) == Effect::Ask);
        if let Some(reason) = self.policies.tool_denial(action, resources) {
            return Decision {
                effect: Effect::Deny,
                reason: Some(reason),
            };
        }
        Decision {
            effect: if ask { Effect::Ask } else { Effect::Allow },
            reason: None,
        }
    }
}

struct Pending {
    id: String,
    session_id: String,
    action: String,
    resources: Vec<String>,
    save: Vec<String>,
    scope: Arc<Scope>,
    answer: oneshot::Sender<Result<(), PermissionError>>,
}

/// Pending asks of every session in this process, in the order they were asked.
pub struct Asks {
    db: Arc<Db>,
    events: broadcast::Sender<Event>,
    pending: Mutex<Vec<Pending>>,
    closed: AtomicBool,
}

impl Asks {
    pub fn new(db: Arc<Db>, events: broadcast::Sender<Event>) -> Asks {
        Asks {
            db,
            events,
            pending: Mutex::default(),
            closed: AtomicBool::new(false),
        }
    }

    fn replied(&self, pending: &Pending, reply: PermissionReply) {
        let _ = self.events.send(Event::PermissionReplied {
            session_id: pending.session_id.clone(),
            request_id: pending.id.clone(),
            reply,
        });
    }

    /// Answers a pending ask; false when it is unknown or already answered. A rejection rejects
    /// every other pending ask of the session the same way; "always" saves the request's patterns
    /// for the project and approves the other pending asks they now allow.
    pub fn reply(&self, request_id: &str, reply: Reply) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let Some(index) = pending.iter().position(|p| p.id == request_id) else {
            return false;
        };
        let answered = pending.remove(index);
        match reply {
            Reply::Reject(message) => {
                let error = || match &message {
                    Some(message) => PermissionError::Corrected(message.clone()),
                    None => PermissionError::Declined,
                };
                self.replied(&answered, PermissionReply::Reject);
                let session_id = answered.session_id.clone();
                let _ = answered.answer.send(Err(error()));
                // Feedback applies to the whole batch, so parallel asks don't end the step.
                let (same, others): (Vec<_>, Vec<_>) =
                    pending.drain(..).partition(|p| p.session_id == session_id);
                *pending = others;
                for item in same {
                    self.replied(&item, PermissionReply::Reject);
                    let _ = item.answer.send(Err(error()));
                }
            }
            Reply::Once | Reply::Always => {
                let always = reply == Reply::Always && !answered.save.is_empty();
                self.replied(
                    &answered,
                    if reply == Reply::Always {
                        PermissionReply::Always
                    } else {
                        PermissionReply::Once
                    },
                );
                if always {
                    let saved = self.db.save_permissions(
                        &answered.scope.project_id,
                        &answered.action,
                        &answered.save,
                    );
                    if let Err(error) = saved {
                        crate::log::warn(
                            "failed to save permission",
                            &[("error", &error.to_string())],
                        );
                    }
                }
                let _ = answered.answer.send(Ok(()));
                if !always {
                    return true;
                }
                let (allowed, rest): (Vec<_>, Vec<_>) = pending.drain(..).partition(|p| {
                    p.scope.decide(&self.db, &p.action, &p.resources).effect == Effect::Allow
                });
                *pending = rest;
                for item in allowed {
                    self.replied(&item, PermissionReply::Always);
                    let _ = item.answer.send(Ok(()));
                }
            }
        }
        true
    }

    /// Rejects every pending ask and every later one: the process is shutting down.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let drained: Vec<Pending> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect();
        for item in drained {
            self.replied(&item, PermissionReply::Reject);
            let _ = item.answer.send(Err(PermissionError::Declined));
        }
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// Drops a pending ask whose waiter went away (an interrupted turn drops the tool call).
struct Forget<'a> {
    asks: &'a Asks,
    id: String,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.asks
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|p| p.id != self.id);
    }
}

pub struct RuleGate {
    pub session_id: String,
    pub scope: Arc<Scope>,
    pub asks: Arc<Asks>,
}

impl PermissionGate for RuleGate {
    fn ask(&self, request: PermissionRequest) -> BoxFuture<'_, Result<(), PermissionError>> {
        async move {
            if self.asks.closed() {
                return Err(PermissionError::Declined);
            }
            let decision = self
                .scope
                .decide(&self.asks.db, &request.action, &request.resources);
            match decision.effect {
                Effect::Allow => return Ok(()),
                Effect::Deny => {
                    return Err(PermissionError::Blocked {
                        action: request.action,
                        reason: decision.reason,
                    });
                }
                Effect::Ask => {}
            }
            let id = crate::ids::permission();
            let (answer, receiver) = oneshot::channel();
            self.asks
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Pending {
                    id: id.clone(),
                    session_id: self.session_id.clone(),
                    action: request.action.clone(),
                    resources: request.resources.clone(),
                    save: request.save.clone(),
                    scope: self.scope.clone(),
                    answer,
                });
            let _ = self.asks.events.send(Event::PermissionAsked {
                session_id: self.session_id.clone(),
                request_id: id.clone(),
                action: request.action,
                resources: request.resources,
                save: request.save,
            });
            let _forget = Forget {
                asks: &self.asks,
                id,
            };
            receiver.await.unwrap_or(Err(PermissionError::Declined))
        }
        .boxed()
    }

    fn allows(&self, action: &str, resources: &[String]) -> bool {
        !self.asks.closed()
            && self.scope.decide(&self.asks.db, action, resources).effect == Effect::Allow
    }

    fn denies(&self, action: &str, resources: &[String]) -> bool {
        self.scope.decide(&self.asks.db, action, resources).effect == Effect::Deny
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::permission::policy::Policy;

    struct Setup {
        _dir: tempfile::TempDir,
        asks: Arc<Asks>,
        events: broadcast::Receiver<Event>,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(&dir.path().join("a.db")).unwrap());
        let (sender, events) = broadcast::channel(64);
        Setup {
            _dir: dir,
            asks: Arc::new(Asks::new(db, sender)),
            events,
        }
    }

    fn gate(setup: &Setup, session: &str, rules: Vec<Rule>, policies: Policies) -> RuleGate {
        RuleGate {
            session_id: session.into(),
            scope: Arc::new(Scope {
                configured: rules,
                project_id: "prj".into(),
                policies: Arc::new(policies),
            }),
            asks: setup.asks.clone(),
        }
    }

    fn request(action: &str, resources: &[&str], save: &[&str]) -> PermissionRequest {
        PermissionRequest {
            action: action.into(),
            resources: resources.iter().map(|r| r.to_string()).collect(),
            save: save.iter().map(|r| r.to_string()).collect(),
            ..Default::default()
        }
    }

    async fn asked(events: &mut broadcast::Receiver<Event>) -> String {
        loop {
            if let Event::PermissionAsked { request_id, .. } = events.recv().await.unwrap() {
                return request_id;
            }
        }
    }

    #[tokio::test]
    async fn rules_decide_before_anyone_is_asked() {
        let setup = setup();
        let rules = vec![
            Rule::new("*", "*", Effect::Allow),
            Rule::new("shell", "rm *", Effect::Deny),
            Rule::new("read", "*.env", Effect::Ask),
        ];
        let gate = gate(&setup, "s1", rules, Policies::default());
        assert!(gate.ask(request("shell", &["ls"], &[])).await.is_ok());
        let denied = gate.ask(request("shell", &["ls", "rm -rf x"], &[])).await;
        assert_eq!(denied.unwrap_err().to_string(), "Permission denied: shell");
        // A saved allow never overrides a configured deny.
        setup
            .asks
            .db
            .save_permissions("prj", "shell", &["rm *".into()])
            .unwrap();
        assert!(gate.ask(request("shell", &["rm x"], &[])).await.is_err());
        setup
            .asks
            .db
            .save_permissions("prj", "read", &["*".into()])
            .unwrap();
        assert!(gate.allows("read", &["a.env".into()]));
    }

    #[tokio::test]
    async fn policies_deny_with_their_message() {
        let setup = setup();
        let policy = Policy::parse(
            &json!({"action": "tool.use", "resource": "shell:git *", "effect": "deny"}),
        );
        let gate = gate(
            &setup,
            "s1",
            vec![Rule::new("*", "*", Effect::Allow)],
            Policies(policy.into_iter().collect()),
        );
        let error = gate
            .ask(request("shell", &["git push"], &[]))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Blocked by configuration policy");
        assert!(gate.ask(request("shell", &["ls"], &[])).await.is_ok());
    }

    #[tokio::test]
    async fn always_saves_and_approves_what_it_now_covers() {
        let mut setup = setup();
        let gate = Arc::new(gate(&setup, "s1", Vec::new(), Policies::default()));
        let first = tokio::spawn({
            let gate = gate.clone();
            async move {
                gate.ask(request("shell", &["git status"], &["git status *"]))
                    .await
            }
        });
        let first_id = asked(&mut setup.events).await;
        let second = tokio::spawn({
            let gate = gate.clone();
            async move {
                gate.ask(request("shell", &["git status -s"], &["git status *"]))
                    .await
            }
        });
        asked(&mut setup.events).await;
        let third = tokio::spawn({
            let gate = gate.clone();
            async move { gate.ask(request("shell", &["ls"], &["ls *"])).await }
        });
        let third_id = asked(&mut setup.events).await;
        assert!(setup.asks.reply(&first_id, Reply::Always));
        assert!(first.await.unwrap().is_ok());
        assert!(second.await.unwrap().is_ok());
        assert_eq!(
            setup.asks.db.saved_permissions("prj").unwrap(),
            [Rule::new("shell", "git status *", Effect::Allow)]
        );
        assert!(!setup.asks.reply(&first_id, Reply::Once));
        assert!(setup.asks.reply(&third_id, Reply::Once));
        assert!(third.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn rejection_rejects_the_whole_session() {
        let mut setup = setup();
        let one = Arc::new(gate(&setup, "s1", Vec::new(), Policies::default()));
        let other = Arc::new(gate(&setup, "s2", Vec::new(), Policies::default()));
        let spawn = |gate: &Arc<RuleGate>, action: &'static str| {
            let gate = gate.clone();
            tokio::spawn(async move { gate.ask(request(action, &["x"], &[])).await })
        };
        let a = spawn(&one, "read");
        let a_id = asked(&mut setup.events).await;
        let b = spawn(&one, "edit");
        asked(&mut setup.events).await;
        let c = spawn(&other, "edit");
        asked(&mut setup.events).await;
        setup.asks.reply(&a_id, Reply::Reject(Some("No.".into())));
        assert_eq!(
            a.await.unwrap(),
            Err(PermissionError::Corrected("No.".into()))
        );
        assert_eq!(
            b.await.unwrap(),
            Err(PermissionError::Corrected("No.".into()))
        );
        setup.asks.close();
        assert_eq!(c.await.unwrap(), Err(PermissionError::Declined));
        assert_eq!(
            one.ask(request("read", &["x"], &[])).await,
            Err(PermissionError::Declined)
        );
    }
}
