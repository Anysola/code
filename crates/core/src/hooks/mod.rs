//! Hooks: commands the user configures that run at fixed moments of a session (before and after a
//! tool call, when a prompt is submitted, when the session would stop, when it starts). A hook
//! gets the event as JSON on stdin and answers with its exit code and output. Claude Code's
//! contract, so existing hook scripts run unchanged.
//!
//! Hooks only ever tighten: a hook can block, ask, add context or keep a turn going, never allow
//! something rules, policies or the session's mode would not. Project documents' hooks run only
//! once the user approved that document's hooks as they are now ([`trust`]).

pub mod config;
pub(crate) mod fire;
pub mod matcher;
pub mod output;
mod run;
pub mod tool;

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::storage::Db;
pub use config::Handler;
pub use fire::ToolHooks;

/// The error kind of a turn that ended because a UserPromptSubmit hook blocked its prompt.
pub const BLOCKED: &str = "hook.blocked";

/// The events 1.0.9 runs hooks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    PreToolUse,
    PostToolUse,
    UserPromptSubmit,
    Stop,
    SessionStart,
}

impl Event {
    pub fn parse(name: &str) -> Option<Event> {
        Some(match name {
            "PreToolUse" => Event::PreToolUse,
            "PostToolUse" => Event::PostToolUse,
            "UserPromptSubmit" => Event::UserPromptSubmit,
            "Stop" => Event::Stop,
            "SessionStart" => Event::SessionStart,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Event::PreToolUse => "PreToolUse",
            Event::PostToolUse => "PostToolUse",
            Event::UserPromptSubmit => "UserPromptSubmit",
            Event::Stop => "Stop",
            Event::SessionStart => "SessionStart",
        }
    }

    /// `UserPromptSubmit` and `Stop` run whatever their matcher says.
    pub fn uses_matcher(self) -> bool {
        !matches!(self, Event::UserPromptSubmit | Event::Stop)
    }
}

/// Where a document comes from, which decides whether its hooks need approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookScope {
    /// The global config dir, the legacy global dir, `ANYSOLA_CONFIG`, `ANYSOLA_CONFIG_CONTENT`.
    User,
    /// Found by the upward walk from the session directory: runs only once approved.
    Project,
    /// The system directory or MDM: always runs, `disable_hooks` cannot turn it off.
    Managed,
}

impl HookScope {
    pub fn as_str(self) -> &'static str {
        match self {
            HookScope::User => "user",
            HookScope::Project => "project",
            HookScope::Managed => "managed",
        }
    }
}

/// What a hook did that the user should see (silent successes report nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOutcome {
    /// The tool call, prompt or stop did not go ahead (`Stop`: the turn continued).
    Blocked,
    /// Feedback for the model next to a tool's result (PostToolUse).
    Feedback,
    /// Context added for the model.
    Context,
    /// The hook failed (could not start, timed out, exited with another code, invalid output);
    /// the action went on.
    Error,
    /// A message for the user: `systemMessage`, a `stopReason`, SessionStart's stderr.
    Notice,
}

impl HookOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            HookOutcome::Blocked => "blocked",
            HookOutcome::Feedback => "feedback",
            HookOutcome::Context => "context",
            HookOutcome::Error => "error",
            HookOutcome::Notice => "notice",
        }
    }
}

/// The hooks of one config document.
#[derive(Debug, Clone)]
pub struct Document {
    /// The file, or the variable name for `ANYSOLA_CONFIG_CONTENT`.
    pub path: String,
    pub scope: HookScope,
    /// SHA-256 (hex) of the document's `hooks` value after variable substitution.
    pub digest: String,
    pub handlers: Vec<Handler>,
}

/// Hooks of every config document, in load order (see [`crate::config::Config::hooks`]).
#[derive(Debug, Clone, Default)]
pub struct Configured {
    pub documents: Vec<Document>,
    /// `"disable_hooks": true` in a user document: user and project hooks are off.
    pub disabled_by_user: bool,
    /// `"disable_hooks": true` in a project document: that turns off project hooks only, so a
    /// cloned repository cannot switch off the user's own guards.
    pub disabled_by_project: bool,
    /// `shell` of the last user or managed document: hooks run with it, never with a shell a
    /// project document names (an unapproved repository must not pick what runs trusted hooks).
    pub shell: Option<String>,
}

impl Configured {
    /// Whether any document configures a handler for `event` (before trust or switches).
    pub fn any(&self, event: Event) -> bool {
        self.documents
            .iter()
            .any(|d| d.handlers.iter().any(|h| h.event == event))
    }

    fn off(&self, scope: HookScope) -> bool {
        let env = crate::env::flag("DISABLE_HOOKS");
        match scope {
            HookScope::Managed => false,
            HookScope::User => env || self.disabled_by_user,
            HookScope::Project => env || self.disabled_by_user || self.disabled_by_project,
        }
    }
}

/// One configured hook as front ends list it.
#[derive(Debug, Clone, PartialEq)]
pub struct HookInfo {
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
    /// Seconds.
    pub timeout: u64,
    /// The document path, or the variable name it came from.
    pub source: String,
    pub scope: HookScope,
    /// Will run: trusted (or not a project hook) and not turned off.
    pub active: bool,
}

/// A project document that defines hooks, with the user's decision on its current content.
#[derive(Debug, Clone, PartialEq)]
pub struct HookDocument {
    pub path: String,
    pub digest: String,
    /// `Some(true)` approved, `Some(false)` refused ("Not now"), `None` undecided: never asked,
    /// or the hooks changed since.
    pub decision: Option<bool>,
    pub hooks: Vec<HookInfo>,
}

const TRUST_PREFIX: &str = "hooks.trust:";

/// The stored decision on `path`'s hooks if it was made for this `digest`.
fn decision(db: &Db, path: &str, digest: &str) -> Option<bool> {
    let stored = db.kv_get(&format!("{TRUST_PREFIX}{path}")).ok()??;
    (stored["digest"].as_str() == Some(digest)).then(|| stored["allow"].as_bool())?
}

/// Records the user's decision on `path`'s hooks with this content (`digest` as
/// [`HookDocument::digest`] gave it). A digest that no longer matches is simply undecided.
pub fn trust(db: &Db, path: &str, digest: &str, allow: bool) -> crate::storage::Result<()> {
    db.kv_set(
        &format!("{TRUST_PREFIX}{path}"),
        &json!({"digest": digest, "allow": allow}),
    )
}

/// The decision each project document has, read once per call.
struct Decisions<'a> {
    db: &'a Db,
    read: HashMap<String, Option<bool>>,
}

impl Decisions<'_> {
    fn of(&mut self, document: &Document) -> Option<bool> {
        if document.scope != HookScope::Project {
            return None;
        }
        let db = self.db;
        *self
            .read
            .entry(document.path.clone())
            .or_insert_with(|| decision(db, &document.path, &document.digest))
    }

    fn runs(&mut self, configured: &Configured, document: &Document) -> bool {
        !configured.off(document.scope)
            && (document.scope != HookScope::Project || self.of(document) == Some(true))
    }
}

fn decisions(db: &Db) -> Decisions<'_> {
    Decisions {
        db,
        read: HashMap::new(),
    }
}

fn info(document: &Document, handler: &Handler, active: bool) -> HookInfo {
    HookInfo {
        event: handler.event.as_str().to_string(),
        matcher: handler.matcher.clone(),
        command: handler.command.clone(),
        timeout: handler.timeout,
        source: document.path.clone(),
        scope: document.scope,
        active,
    }
}

/// Every configured hook, in run order.
pub fn list(configured: &Configured, db: &Db) -> Vec<HookInfo> {
    let mut decisions = decisions(db);
    configured
        .documents
        .iter()
        .flat_map(|document| {
            let active = decisions.runs(configured, document);
            document
                .handlers
                .iter()
                .map(move |handler| info(document, handler, active))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The project documents that define hooks, in load order.
pub fn documents(configured: &Configured, db: &Db) -> Vec<HookDocument> {
    let mut decisions = decisions(db);
    configured
        .documents
        .iter()
        .filter(|d| d.scope == HookScope::Project)
        .map(|document| {
            let decision = decisions.of(document);
            let active = decisions.runs(configured, document);
            HookDocument {
                path: document.path.clone(),
                digest: document.digest.clone(),
                decision,
                hooks: document
                    .handlers
                    .iter()
                    .map(|handler| info(document, handler, active))
                    .collect(),
            }
        })
        .collect()
}

/// `(path, digest)` of the project documents nobody decided on yet: what a front end asks about.
pub fn undecided(configured: &Configured, db: &Db) -> Vec<(String, String)> {
    documents(configured, db)
        .into_iter()
        .filter(|d| d.decision.is_none())
        .map(|d| (d.path, d.digest))
        .collect()
}

/// How many project hooks are skipped because nobody approved their documents yet (refused ones
/// are not counted: the user already answered).
pub fn unapproved(configured: &Configured, db: &Db) -> usize {
    if configured.off(HookScope::Project) {
        return 0;
    }
    documents(configured, db)
        .iter()
        .filter(|d| d.decision.is_none())
        .map(|d| d.hooks.len())
        .sum()
}

/// The handlers that run for `event` on `names` (a tool's two names, a session source; empty for
/// events that ignore their matcher): trusted, not switched off, matching, and each command
/// once, in config order.
pub(crate) fn matching(
    configured: &Configured,
    db: &Db,
    event: Event,
    names: &[&str],
) -> Vec<Handler> {
    if !configured.any(event) {
        return Vec::new();
    }
    let mut decisions = decisions(db);
    let mut out: Vec<Handler> = Vec::new();
    for document in &configured.documents {
        if !document.handlers.iter().any(|h| h.event == event)
            || !decisions.runs(configured, document)
        {
            continue;
        }
        for handler in &document.handlers {
            let applies = handler.event == event
                && (!event.uses_matcher() || handler.compiled.matches(names));
            if applies && !out.iter().any(|h| h.command == handler.command) {
                out.push(handler.clone());
            }
        }
    }
    out
}

/// Reads a document's `hooks` and `disable_hooks` into `configured` (the config loader calls
/// this for every local document; the gateway's document has no `path` and contributes none).
pub(crate) fn apply(
    configured: &mut Configured,
    data: &serde_json::Map<String, Value>,
    hooks: Option<Value>,
    disable: Option<Value>,
    path: &str,
    scope: HookScope,
    warnings: &mut Vec<String>,
) {
    if scope != HookScope::Project
        && let Some(shell) = data.get("shell").and_then(Value::as_str)
        && !shell.is_empty()
    {
        configured.shell = Some(shell.to_string());
    }
    if disable == Some(Value::Bool(true)) {
        match scope {
            HookScope::User => configured.disabled_by_user = true,
            HookScope::Project => configured.disabled_by_project = true,
            HookScope::Managed => {}
        }
    }
    let Some(value) = hooks else {
        return;
    };
    let handlers = config::parse(&value, path, warnings);
    if handlers.is_empty() {
        return;
    }
    configured.documents.push(Document {
        path: path.to_string(),
        scope,
        digest: config::digest(&value),
        handlers,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(path: &str, scope: HookScope, command: &str) -> Document {
        let value = json!({"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": command}]}]});
        let mut warnings = Vec::new();
        Document {
            path: path.into(),
            scope,
            digest: config::digest(&value),
            handlers: config::parse(&value, path, &mut warnings),
        }
    }

    #[test]
    fn project_hooks_run_once_approved_for_their_content() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        let mut configured = Configured {
            documents: vec![
                document("/u.json", HookScope::User, "user"),
                document("/p/anysola.json", HookScope::Project, "project"),
                document("/p2/anysola.json", HookScope::Project, "user"),
            ],
            ..Configured::default()
        };
        let commands = |configured: &Configured| -> Vec<String> {
            matching(configured, &db, Event::PreToolUse, &["shell", "Bash"])
                .into_iter()
                .map(|h| h.command)
                .collect()
        };
        assert_eq!(commands(&configured), ["user"]);
        assert_eq!(undecided(&configured, &db).len(), 2);
        assert_eq!(unapproved(&configured, &db), 2);
        let digest = configured.documents[1].digest.clone();
        trust(&db, "/p/anysola.json", &digest, true).unwrap();
        trust(&db, "/p2/anysola.json", "stale", true).unwrap();
        assert_eq!(commands(&configured), ["user", "project"]);
        let listed = documents(&configured, &db);
        assert_eq!(listed[0].decision, Some(true));
        assert_eq!(listed[1].decision, None, "approved for other content");
        assert!(!listed[1].hooks[0].active);
        // The same command from an approved project document runs once.
        let digest = configured.documents[2].digest.clone();
        trust(&db, "/p2/anysola.json", &digest, true).unwrap();
        assert_eq!(commands(&configured), ["user", "project"]);
        // A changed command is undecided again.
        configured.documents[1] = document("/p/anysola.json", HookScope::Project, "changed");
        assert_eq!(commands(&configured), ["user"]);
        assert_eq!(undecided(&configured, &db).len(), 1);
        // "Not now" is a decision: not asked again, not counted as skipped.
        let digest = configured.documents[1].digest.clone();
        trust(&db, "/p/anysola.json", &digest, false).unwrap();
        assert!(undecided(&configured, &db).is_empty());
        assert_eq!(unapproved(&configured, &db), 0);
        assert_eq!(commands(&configured), ["user"]);
    }

    #[test]
    fn switches_spare_managed_hooks_and_projects_cannot_turn_off_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        let mut configured = Configured {
            documents: vec![
                document("/u.json", HookScope::User, "user"),
                document("/m.json", HookScope::Managed, "managed"),
            ],
            ..Configured::default()
        };
        let commands = |configured: &Configured| -> Vec<String> {
            list(configured, &db)
                .into_iter()
                .filter(|h| h.active)
                .map(|h| h.command)
                .collect()
        };
        configured.disabled_by_project = true;
        assert_eq!(commands(&configured), ["user", "managed"]);
        configured.disabled_by_user = true;
        assert_eq!(commands(&configured), ["managed"]);
    }
}
