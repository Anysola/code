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

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::storage::Db;
pub use config::Handler;
pub use fire::ToolHooks;

/// The error kind of a turn that ended because a UserPromptSubmit hook blocked its prompt.
pub const BLOCKED: &str = "hook.blocked";

/// `text` safe to show in a terminal: control characters (C0 including newline and tab, DEL,
/// C1, and so every escape sequence's ESC) and bidirectional overrides are written out as
/// `\x1b` / `\u{202e}`, so a command cannot hide behind what it makes the terminal draw.
pub fn display(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let code = c as u32;
        let bidi = matches!(code, 0x200e | 0x200f | 0x202a..=0x202e | 0x2066..=0x2069);
        if code < 0x20 || code == 0x7f || (0x80..0xa0).contains(&code) {
            out.push_str(&format!("\\x{code:02x}"));
        } else if bidi {
            out.push_str(&format!("\\u{{{code:04x}}}"));
        } else {
            out.push(c);
        }
    }
    out
}

/// The timeout a handler gets when none is configured, in seconds.
pub fn default_timeout(event: &str) -> u64 {
    if event == Event::UserPromptSubmit.as_str() {
        30
    } else {
        600
    }
}

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
    /// Always false since 1.0.9: `disable_hooks` in a project document is ignored (the agent can
    /// edit that file). Kept for source compatibility.
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
    /// An earlier approved version of this document's hooks runs in place of the listed content
    /// right now (in this process): the listed content changed and is undecided or refused.
    pub standing: bool,
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
                standing: false,
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

/// Precedence when one command is configured in several documents.
fn rank(scope: HookScope) -> u8 {
    match scope {
        HookScope::Managed => 2,
        HookScope::User => 1,
        HookScope::Project => 0,
    }
}

/// The handlers that run for `event` on `names` (a tool's two names, a session source; empty for
/// events that ignore their matcher): trusted, not switched off and matching, in config order.
/// A command configured in several documents runs once, as the highest scope configured it
/// (managed, user, project; the longer timeout within a scope), so a weaker document repeating
/// a guard cannot weaken it. With `approved`, a project document whose current content is
/// undecided runs its last approved version; the second value names those documents with their
/// current digest.
pub(crate) fn matching(
    configured: &Configured,
    db: &Db,
    approved: Option<&Approved>,
    event: Event,
    names: &[&str],
) -> (Vec<Handler>, Vec<(String, String)>) {
    let standing = approved
        .map(|a| a.standing(configured, db))
        .unwrap_or_default();
    let wanted = |d: &Document| d.handlers.iter().any(|h| h.event == event);
    if !configured.any(event) && !standing.values().any(|(d, _)| wanted(d)) {
        return (Vec::new(), Vec::new());
    }
    let mut decisions = decisions(db);
    let mut runs: Vec<(Handler, HookScope)> = Vec::new();
    let mut stood = Vec::new();
    let mut add = |document: &Document| {
        for handler in &document.handlers {
            let applies = handler.event == event
                && (!event.uses_matcher() || handler.compiled.matches(names));
            if !applies {
                continue;
            }
            match runs.iter_mut().find(|(h, _)| h.command == handler.command) {
                Some(existing) => {
                    let stronger = rank(document.scope) > rank(existing.1)
                        || (rank(document.scope) == rank(existing.1)
                            && handler.timeout > existing.0.timeout);
                    if stronger {
                        *existing = (handler.clone(), document.scope);
                    }
                }
                None => runs.push((handler.clone(), document.scope)),
            }
        }
    };
    let projects_on = !configured.off(HookScope::Project);
    for document in &configured.documents {
        if document.scope == HookScope::Project
            && let Some((snapshot, digest)) = standing.get(&document.path)
        {
            if projects_on && wanted(snapshot) {
                add(snapshot);
                stood.push((document.path.clone(), digest.clone()));
            }
            continue;
        }
        if wanted(document) && decisions.runs(configured, document) {
            add(document);
        }
    }
    // Approved documents whose hooks were removed since (or whose file is gone).
    let mut removed: Vec<_> = standing
        .iter()
        .filter(|(path, _)| !configured.documents.iter().any(|d| &d.path == *path))
        .collect();
    removed.sort_by(|a, b| a.0.cmp(b.0));
    for (path, (snapshot, digest)) in removed {
        if projects_on && wanted(snapshot) {
            add(snapshot);
            stood.push((path.clone(), digest.clone()));
        }
    }
    (runs.into_iter().map(|(h, _)| h).collect(), stood)
}

/// The digest of a project document that no longer defines hooks (or no longer exists).
pub fn removed_digest() -> String {
    config::digest(&Value::Null)
}

/// Approved project hooks of this process. Once a project document's hooks were approved and
/// loaded, a change to them (an edit, their removal, the file's deletion) takes effect only once
/// the user decided on the new content; until then the approved version keeps running. A new
/// process starts with none: undecided content runs nothing there.
#[derive(Default)]
pub struct Approved {
    documents: Mutex<HashMap<String, Document>>,
    /// `(path, current digest)` the user was told about.
    noticed: Mutex<HashSet<(String, String)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Approved {
    /// Content approved now becomes the snapshot; a decision on the current content (or on the
    /// removal of the hooks) replaces or drops it.
    pub(crate) fn sync(&self, configured: &Configured, db: &Db) {
        let mut documents = lock(&self.documents);
        let projects: Vec<&Document> = configured
            .documents
            .iter()
            .filter(|d| d.scope == HookScope::Project)
            .collect();
        for document in &projects {
            match decision(db, &document.path, &document.digest) {
                Some(true) => {
                    documents.insert(document.path.clone(), (*document).clone());
                }
                // Refusing a change keeps the approved version running here: only refusing the
                // approved content itself ("stop allowing") ends it.
                Some(false)
                    if documents
                        .get(&document.path)
                        .is_some_and(|s| s.digest == document.digest) =>
                {
                    documents.remove(&document.path);
                }
                _ => {}
            }
        }
        // Allowing the removal of approved hooks accepts it; refusing it keeps them running.
        let removed = removed_digest();
        documents.retain(|path, _| {
            projects.iter().any(|d| &d.path == path) || decision(db, path, &removed) != Some(true)
        });
    }

    /// The snapshots standing in for content that is not allowed (undecided or refused):
    /// path → (snapshot, current digest).
    fn standing(&self, configured: &Configured, db: &Db) -> HashMap<String, (Document, String)> {
        if lock(&self.documents).is_empty() {
            return HashMap::new();
        }
        self.sync(configured, db);
        let documents = lock(&self.documents);
        documents
            .iter()
            .filter_map(|(path, snapshot)| {
                let digest = configured
                    .documents
                    .iter()
                    .find(|d| d.scope == HookScope::Project && &d.path == path)
                    .map_or_else(removed_digest, |d| d.digest.clone());
                (decision(db, path, &digest) != Some(true) && digest != snapshot.digest)
                    .then(|| (path.clone(), (snapshot.clone(), digest)))
            })
            .collect()
    }

    /// Whether an approved snapshot has a handler for `event` (it may not be standing in).
    pub(crate) fn any(&self, event: Event) -> bool {
        lock(&self.documents)
            .values()
            .any(|d| d.handlers.iter().any(|h| h.event == event))
    }

    /// Whether the user still has to be told that `path`'s approved hooks keep running for this
    /// change; true once.
    pub(crate) fn first_notice(&self, path: &str, digest: &str) -> bool {
        lock(&self.noticed).insert((path.to_string(), digest.to_string()))
    }

    /// Project documents that no longer define hooks while their approved hooks still run here.
    fn removed(&self, configured: &Configured, db: &Db) -> Vec<HookDocument> {
        let mut out: Vec<HookDocument> = self
            .standing(configured, db)
            .into_iter()
            .filter(|(path, _)| !configured.documents.iter().any(|d| &d.path == path))
            .map(|(path, (_, digest))| HookDocument {
                decision: decision(db, &path, &digest),
                path,
                digest,
                hooks: Vec::new(),
                standing: true,
            })
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }
}

/// [`documents`] plus, with `approved`, the documents whose approved hooks were removed:
/// `{path, digest: removed_digest(), decision: None, hooks: []}` until the user decides.
pub(crate) fn documents_with(
    configured: &Configured,
    db: &Db,
    approved: &Approved,
) -> Vec<HookDocument> {
    approved.sync(configured, db);
    let standing = approved.standing(configured, db);
    let mut out = documents(configured, db);
    for document in &mut out {
        document.standing = standing.contains_key(&document.path);
    }
    out.extend(approved.removed(configured, db));
    out
}

/// [`undecided`] with the removed documents of [`documents_with`].
pub(crate) fn undecided_with(
    configured: &Configured,
    db: &Db,
    approved: &Approved,
) -> Vec<(String, String)> {
    documents_with(configured, db, approved)
        .into_iter()
        .filter(|d| d.decision.is_none())
        .map(|d| (d.path, d.digest))
        .collect()
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
            HookScope::Project => warnings.push(format!(
                "Ignoring \"disable_hooks\" in {path}: it only works in your own config (~/.anysola/config.json)."
            )),
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
    fn display_writes_out_what_a_terminal_would_interpret() {
        assert_eq!(display("npm test"), "npm test");
        assert_eq!(
            display("rm -rf ~ \x1b[2K\x1b[1Gecho ok"),
            "rm -rf ~ \\x1b[2K\\x1b[1Gecho ok"
        );
        assert_eq!(display("a\nb\tc\u{7f}\u{9b}"), "a\\x0ab\\x09c\\x7f\\x9b");
        assert_eq!(display("x\u{202e}y"), "x\\u{202e}y");
        assert_eq!(display("café ✓"), "café ✓");
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
            matching(configured, &db, None, Event::PreToolUse, &["shell", "Bash"])
                .0
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
        configured.disabled_by_user = true;
        assert_eq!(commands(&configured), ["managed"]);
    }
}
