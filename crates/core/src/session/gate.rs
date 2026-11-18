//! The permission gate of a running session. A request is denied when a configured rule denies
//! any resource (saved "always" rules cannot override that), else evaluated with the project's
//! saved rules appended: any `ask` asks, through an [`Event::PermissionAsked`] event answered with
//! [`Asks::reply`]. Managed policies can still turn the result into a denial.
//!
//! A session can also carry a [`Floor`]: what its front end wants asked even where the rules
//! allow it (the desktop app's Ask and Edits modes). It never lifts a denial, and a saved
//! "always" rule still answers for it. A subagent's session works under its root session's floor.

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

/// Actions a session asks about even where its rules allow them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Floor {
    /// The rules alone decide.
    #[default]
    None,
    /// Commands, MCP server tools and work outside the project.
    Commands,
    /// Everything that changes files, runs or fetches, MCP server tools included; reading and
    /// searching never ask.
    Changes,
}

/// Where a session's floor is kept, in its metadata.
pub const FLOOR_KEY: &str = "anysola.permissionFloor";

impl Floor {
    pub fn as_str(self) -> &'static str {
        match self {
            Floor::None => "none",
            Floor::Commands => "commands",
            Floor::Changes => "changes",
        }
    }

    pub fn parse(value: &str) -> Floor {
        match value {
            "commands" => Floor::Commands,
            "changes" => Floor::Changes,
            _ => Floor::None,
        }
    }

    /// The floor a session works under: its root session's, as that one's metadata stores it.
    /// Front ends set the floor on the session the user drives; subagents' sessions under it
    /// carry none of their own.
    pub fn of(db: &Db, session_id: &str) -> Floor {
        db.session_metadata(&root(db, session_id))
            .ok()
            .and_then(|m| m.get(FLOOR_KEY).and_then(|v| v.as_str()).map(Floor::parse))
            .unwrap_or_default()
    }

    /// Whether the floor asks about `action`; `mcp_tool` marks a call of an MCP server's tool
    /// (see [`PermissionRequest::is_mcp_tool`]), which runs code outside Anysola like a command.
    fn asks(self, action: &str, mcp_tool: bool) -> bool {
        match self {
            Floor::None => false,
            Floor::Commands => mcp_tool || matches!(action, "shell" | "external_directory"),
            Floor::Changes => {
                mcp_tool
                    || matches!(
                        action,
                        "shell" | "external_directory" | "edit" | "write" | "webfetch"
                    )
            }
        }
    }
}

/// The topmost session above `session_id` along `parent_id`; a missing parent or a cycle ends
/// the walk at the last session found.
fn root(db: &Db, session_id: &str) -> String {
    let mut current = session_id.to_string();
    let mut seen = std::collections::HashSet::new();
    seen.insert(current.clone());
    while let Some(parent) = db
        .session(&current)
        .ok()
        .flatten()
        .and_then(|row| row.parent_id)
    {
        if !seen.insert(parent.clone()) || !matches!(db.session(&parent), Ok(Some(_))) {
            break;
        }
        current = parent;
    }
    current
}

/// What a session's requests are evaluated against.
pub struct Scope {
    /// The agent's rules followed by the session's own.
    pub configured: Vec<Rule>,
    pub project_id: String,
    pub policies: Arc<Policies>,
    pub floor: Floor,
}

/// The effect of the last rule matching both action and resource, if any does.
fn matched(action: &str, resource: &str, rules: &[Rule]) -> Option<Effect> {
    rules
        .iter()
        .rev()
        .find(|rule| {
            crate::permission::matches(action, &rule.action)
                && crate::permission::matches(resource, &rule.resource)
        })
        .map(|rule| rule.effect)
}

struct Decision {
    effect: Effect,
    reason: Option<String>,
}

impl Scope {
    fn decide(&self, db: &Db, action: &str, resources: &[String], mcp_tool: bool) -> Decision {
        let denied = resources
            .iter()
            .any(|r| evaluate(action, r, &self.configured) == Effect::Deny);
        if denied {
            return Decision {
                effect: Effect::Deny,
                reason: None,
            };
        }
        let saved = db.saved_permissions(&self.project_id).unwrap_or_default();
        let mut rules = self.configured.clone();
        rules.extend(saved.iter().cloned());
        let ask = resources.iter().any(|r| {
            evaluate(action, r, &rules) == Effect::Ask
                || (self.floor.asks(action, mcp_tool)
                    && matched(action, r, &saved) != Some(Effect::Allow))
        });
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
    mcp_tool: bool,
    save: Vec<String>,
    /// What the ask showed with it (an edit's diffs), for [`Asks::pending_events`].
    metadata: serde_json::Value,
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
                    p.scope
                        .decide(&self.db, &p.action, &p.resources, p.mcp_tool)
                        .effect
                        == Effect::Allow
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

    /// Every ask still waiting, as the events that asked them, oldest first.
    pub fn pending_events(&self) -> Vec<Event> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|p| Event::PermissionAsked {
                session_id: p.session_id.clone(),
                request_id: p.id.clone(),
                action: p.action.clone(),
                resources: p.resources.clone(),
                save: p.save.clone(),
                metadata: p.metadata.clone(),
            })
            .collect()
    }
}

/// Drops a pending ask whose waiter went away (an interrupted turn drops the tool call).
struct Forget<'a> {
    asks: &'a Asks,
    id: String,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        let dropped = {
            let mut pending = self.asks.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending
                .iter()
                .position(|p| p.id == self.id)
                .map(|at| pending.remove(at))
        };
        // Front ends showing it take it down.
        if let Some(dropped) = dropped {
            self.asks.replied(&dropped, PermissionReply::Reject);
        }
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
            let mcp_tool = request.is_mcp_tool();
            let decision =
                self.scope
                    .decide(&self.asks.db, &request.action, &request.resources, mcp_tool);
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
                    mcp_tool,
                    save: request.save.clone(),
                    metadata: request.metadata.clone(),
                    scope: self.scope.clone(),
                    answer,
                });
            let _ = self.asks.events.send(Event::PermissionAsked {
                session_id: self.session_id.clone(),
                request_id: id.clone(),
                action: request.action,
                resources: request.resources,
                save: request.save,
                metadata: request.metadata,
            });
            let _forget = Forget {
                asks: &self.asks,
                id,
            };
            receiver.await.unwrap_or(Err(PermissionError::Declined))
        }
        .boxed()
    }

    /// Without a request to carry the marker, an MCP tool's action is judged like any other.
    fn allows(&self, action: &str, resources: &[String]) -> bool {
        !self.asks.closed()
            && self
                .scope
                .decide(&self.asks.db, action, resources, false)
                .effect
                == Effect::Allow
    }

    fn denies(&self, action: &str, resources: &[String]) -> bool {
        self.scope
            .decide(&self.asks.db, action, resources, false)
            .effect
            == Effect::Deny
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
                floor: Floor::None,
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

    #[test]
    fn a_floor_asks_where_rules_allow_but_keeps_denials_and_saved_allows() {
        let setup = setup();
        let db = &setup.asks.db;
        let scope = |floor| Scope {
            configured: vec![
                Rule::new("*", "*", Effect::Allow),
                Rule::new("shell", "rm *", Effect::Deny),
            ],
            project_id: "prj".into(),
            policies: Arc::new(Policies::default()),
            floor,
        };
        let effect = |scope: &Scope, action: &str, resource: &str| {
            scope
                .decide(db, action, &[resource.to_string()], false)
                .effect
        };
        let none = scope(Floor::None);
        assert_eq!(effect(&none, "shell", "ls"), Effect::Allow);
        assert_eq!(effect(&none, "edit", "a.rs"), Effect::Allow);

        let commands = scope(Floor::Commands);
        assert_eq!(effect(&commands, "shell", "ls"), Effect::Ask);
        assert_eq!(effect(&commands, "edit", "a.rs"), Effect::Allow);
        assert_eq!(effect(&commands, "shell", "rm x"), Effect::Deny);

        let changes = scope(Floor::Changes);
        assert_eq!(effect(&changes, "edit", "a.rs"), Effect::Ask);
        assert_eq!(effect(&changes, "write", "b.rs"), Effect::Ask);
        assert_eq!(effect(&changes, "read", "a.rs"), Effect::Allow);
        assert_eq!(effect(&changes, "grep", "x"), Effect::Allow);
        assert_eq!(effect(&changes, "shell", "rm x"), Effect::Deny);

        // "Always" answers for the floor.
        db.save_permissions("prj", "shell", &["ls".into()]).unwrap();
        assert_eq!(effect(&commands, "shell", "ls"), Effect::Allow);
        assert_eq!(effect(&commands, "shell", "make"), Effect::Ask);
    }

    #[test]
    fn a_floor_reads_back_from_the_session() {
        assert_eq!(Floor::parse(Floor::Changes.as_str()), Floor::Changes);
        assert_eq!(Floor::parse(Floor::Commands.as_str()), Floor::Commands);
        assert_eq!(Floor::parse("anything else"), Floor::None);
        let setup = setup();
        assert_eq!(Floor::of(&setup.asks.db, "ses_missing"), Floor::None);
    }

    fn create(db: &Db, id: &str, parent: Option<&str>) {
        db.create_session(&crate::storage::SessionRow {
            id: id.into(),
            project_id: "global".into(),
            parent_id: parent.map(str::to_string),
            fork_session_id: None,
            fork_boundary: None,
            slug: "s".into(),
            directory: "/".into(),
            title: "t".into(),
            version: "v".into(),
            agent: None,
            model: None,
            time_created: 1,
            time_updated: 1,
        })
        .unwrap();
    }

    fn set_floor(db: &Db, id: &str, floor: Floor) {
        let mut metadata = db.session_metadata(id).unwrap();
        metadata.insert(FLOOR_KEY.into(), json!(floor.as_str()));
        db.set_session_access(id, &[], &metadata).unwrap();
    }

    #[test]
    fn subagents_work_under_the_root_sessions_floor() {
        let setup = setup();
        let db = &setup.asks.db;
        create(db, "ses_root", None);
        create(db, "ses_child", Some("ses_root"));
        create(db, "ses_grandchild", Some("ses_child"));
        create(db, "ses_orphan", Some("ses_gone"));
        assert_eq!(Floor::of(db, "ses_grandchild"), Floor::None);
        set_floor(db, "ses_root", Floor::Changes);
        assert_eq!(Floor::of(db, "ses_root"), Floor::Changes);
        assert_eq!(Floor::of(db, "ses_child"), Floor::Changes);
        assert_eq!(Floor::of(db, "ses_grandchild"), Floor::Changes);
        // A missing parent ends the walk at the last session found.
        set_floor(db, "ses_orphan", Floor::Commands);
        assert_eq!(Floor::of(db, "ses_orphan"), Floor::Commands);
        // A cycle ends too.
        create(db, "ses_a", Some("ses_b"));
        create(db, "ses_b", Some("ses_a"));
        assert_eq!(Floor::of(db, "ses_a"), Floor::None);
    }

    #[tokio::test]
    async fn a_floor_asks_for_mcp_tools_but_not_for_built_in_reads() {
        let mut setup = setup();
        let rules = vec![
            Rule::new("*", "*", Effect::Allow),
            Rule::new("blocked_tool", "*", Effect::Deny),
        ];
        let mcp = |action: &str| PermissionRequest {
            action: action.into(),
            resources: vec!["*".into()],
            save: vec!["*".into()],
            metadata: json!({"input": {}, crate::permission::MCP_TOOL_KEY: {"server": "github", "tool": "x"}}),
        };
        for floor in [Floor::Commands, Floor::Changes] {
            let gate = Arc::new(RuleGate {
                session_id: "s1".into(),
                scope: Arc::new(Scope {
                    configured: rules.clone(),
                    // Each floor saves its "always" answer for a project of its own.
                    project_id: format!("prj_{}", floor.as_str()),
                    policies: Arc::new(Policies::default()),
                    floor,
                }),
                asks: setup.asks.clone(),
            });
            // The actions the built-in tools ask for that no floor asks about.
            for action in [
                "read",
                "glob",
                "grep",
                "question",
                "skill",
                "todowrite",
                "websearch",
                "subagent",
                "execute",
                "opencode_list_mcp_resources",
                "opencode_read_mcp_resource",
            ] {
                assert!(
                    gate.ask(request(action, &["x"], &[])).await.is_ok(),
                    "{action}"
                );
            }
            // The same name without the marker is not an MCP tool.
            assert!(gate.ask(request("github_x", &["*"], &[])).await.is_ok());
            // A configured deny still denies.
            assert_eq!(
                gate.ask(mcp("blocked_tool")).await,
                Err(PermissionError::Blocked {
                    action: "blocked_tool".into(),
                    reason: None
                })
            );
            let asking = tokio::spawn({
                let gate = gate.clone();
                async move { gate.ask(mcp("github_x")).await }
            });
            let id = asked(&mut setup.events).await;
            // "Always" saves the answer, and the floor then lets it through.
            assert!(setup.asks.reply(&id, Reply::Always));
            assert!(asking.await.unwrap().is_ok());
            assert!(gate.ask(mcp("github_x")).await.is_ok());
        }
        let none = gate(&setup, "s1", rules, Policies::default());
        assert!(none.ask(mcp("github_y")).await.is_ok());
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

    #[tokio::test]
    async fn pending_asks_keep_what_they_showed_until_answered_or_dropped() {
        let mut setup = setup();
        let gate = Arc::new(gate(&setup, "s1", Vec::new(), Policies::default()));
        let ask = |gate: &Arc<RuleGate>| {
            let gate = gate.clone();
            tokio::spawn(async move {
                let mut asked = request("edit", &["a.rs"], &["a.rs"]);
                asked.metadata = json!({"files": [{"file": "a.rs"}]});
                gate.ask(asked).await
            })
        };
        let answered = ask(&gate);
        let id = asked(&mut setup.events).await;
        let dropped = ask(&gate);
        let dropped_id = asked(&mut setup.events).await;
        let pending = setup.asks.pending_events();
        assert_eq!(pending.len(), 2);
        let Event::PermissionAsked {
            session_id,
            request_id,
            action,
            resources,
            save,
            metadata,
        } = &pending[0]
        else {
            panic!("not a permission ask");
        };
        assert_eq!(
            (session_id.as_str(), request_id, action.as_str()),
            ("s1", &id, "edit")
        );
        assert_eq!(
            (resources.as_slice(), save.as_slice()),
            (
                ["a.rs".to_string()].as_slice(),
                ["a.rs".to_string()].as_slice()
            )
        );
        assert_eq!(metadata, &json!({"files": [{"file": "a.rs"}]}));
        assert!(setup.asks.reply(&id, Reply::Once));
        assert!(answered.await.unwrap().is_ok());
        // An interrupted turn drops its tool call and the ask with it.
        dropped.abort();
        let _ = dropped.await;
        assert!(setup.asks.pending_events().is_empty());
        assert!(!setup.asks.reply(&dropped_id, Reply::Once));
    }
}
