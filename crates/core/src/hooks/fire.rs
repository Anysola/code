//! Firing hooks from a session: the payload every hook gets, running the matching handlers
//! concurrently, and combining their answers (a block beats an ask beats nothing; reasons and
//! contexts are all kept, in config order). Every noteworthy answer is reported as
//! [`CoreEvent::Hook`] and returned as an entry for the tool part's `hooks` metadata.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::output::{self, Verdict};
use super::run::{Job, run};
use super::{Event, Handler, HookOutcome};
use crate::api::Shared;
use crate::config::Config;
use crate::events::Event as CoreEvent;
use crate::session::inbox;
use crate::session::message::{Created, Message, Part, SyntheticData, encode};
use crate::storage::{Db, SessionRow};

/// Description of the synthetic rows that carry hook context to the model.
pub const CONTEXT: &str = "Hook context";
/// Description of the synthetic row that keeps a turn going for a Stop hook.
pub const STOP: &str = "Stop hook";
/// Consecutive Stop-hook continuations without a tool call after which a block is overridden.
const STOP_ROUNDS: u32 = 8;

/// What a session's hooks need: the config they come from and the session they run for.
#[derive(Clone)]
pub(crate) struct Context {
    pub config: Arc<Config>,
    pub db: Arc<Db>,
    pub events: broadcast::Sender<CoreEvent>,
    pub env: Arc<Vec<(OsString, OsString)>>,
    pub shell: Option<String>,
    /// The session whose work fires the hooks (a subagent's own session); events name it.
    pub session_id: String,
    /// The root session: what hooks get as `session_id`.
    pub root_id: String,
    pub child: bool,
    pub cwd: PathBuf,
    pub project_dir: PathBuf,
    pub home: PathBuf,
    pub agent: String,
    pub approved: Arc<super::Approved>,
}

impl Context {
    pub fn new(shared: &Shared, session: &SessionRow, config: Arc<Config>, agent: &str) -> Self {
        let root_id = match session.parent_id {
            Some(_) => crate::session::gate::root(&shared.db, &session.id),
            None => session.id.clone(),
        };
        Context {
            shell: config.hooks.shell.clone(),
            config,
            db: shared.db.clone(),
            events: shared.events.clone(),
            env: shared.env.clone(),
            session_id: session.id.clone(),
            root_id,
            child: session.parent_id.is_some(),
            cwd: PathBuf::from(&session.directory),
            project_dir: shared.project.worktree.clone(),
            home: shared.paths.home.clone(),
            approved: shared.hooks.approved.clone(),
            agent: agent.to_string(),
        }
    }

    /// The session's context with its current agent (or the default one).
    pub fn of(shared: &Shared, session: &SessionRow) -> Self {
        let loaded = shared.loaded();
        let agent = match crate::session::select(shared, session) {
            Ok(selection) => selection.agent.id,
            Err(_) => session.agent.clone().unwrap_or_else(|| "build".into()),
        };
        Context::new(shared, session, loaded.config.clone(), &agent)
    }

    fn common(&self, event: Event) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("session_id".into(), json!(self.root_id));
        out.insert("cwd".into(), json!(self.cwd.display().to_string()));
        out.insert("hook_event_name".into(), json!(event.as_str()));
        let mode = if self.agent == "plan" {
            "plan"
        } else {
            "default"
        };
        out.insert("permission_mode".into(), json!(mode));
        out.insert("agent".into(), json!(self.agent));
        out.insert("transcript_path".into(), json!(""));
        if self.child {
            out.insert("agent_id".into(), json!(self.session_id));
            out.insert("agent_type".into(), json!(self.agent));
        }
        out
    }

    fn environment(&self) -> Vec<(OsString, OsString)> {
        let names = ["ANYSOLA_PROJECT_DIR", "CLAUDE_PROJECT_DIR"];
        let mut env: Vec<(OsString, OsString)> = self
            .env
            .iter()
            .filter(|(k, _)| !names.iter().any(|n| k == n))
            .cloned()
            .collect();
        for name in names {
            env.push((name.into(), self.project_dir.clone().into_os_string()));
        }
        env
    }

    /// Runs the handlers for `event` matching `names` with the payload's event fields `extra`.
    pub async fn fire(
        &self,
        event: Event,
        names: &[&str],
        extra: Map<String, Value>,
        cancel: &CancellationToken,
    ) -> Fired {
        let hooks = &self.config.hooks;
        if hooks
            .documents
            .iter()
            .any(|d| d.scope == super::HookScope::Project)
        {
            self.approved.sync(hooks, &self.db);
        }
        let (handlers, stood) =
            super::matching(hooks, &self.db, Some(&self.approved), event, names);
        let mut notices = Fired::default();
        for (path, digest) in stood {
            if self.approved.first_notice(&path, &digest) {
                let message = format!(
                    "The approved hooks of {path} keep running until the change to them is allowed."
                );
                self.note(&mut notices, event, "", HookOutcome::Notice, &message);
            }
        }
        if handlers.is_empty() {
            return notices;
        }
        let mut payload = self.common(event);
        payload.extend(extra);
        let input = serde_json::to_vec(&Value::Object(payload)).unwrap_or_default();
        let env = self.environment();
        let shell = crate::tool::shell::select::resolve(self.shell.as_deref());
        let cwd = working_dir(&[&self.cwd, &self.project_dir, &self.home]);
        let runs = handlers.iter().map(|handler| {
            run(
                Job {
                    shell: &shell,
                    command: &handler.command,
                    cwd: &cwd,
                    env: env.clone(),
                    input: input.clone(),
                    timeout: handler.timeout,
                },
                cancel,
            )
        });
        let ran = futures_util::future::join_all(runs).await;
        let mut fired = notices;
        for (handler, ran) in handlers.iter().zip(ran) {
            let verdict = match output::unrunnable(event, &handler.command, &ran) {
                Some(reason) => Verdict {
                    block: Some(reason),
                    ..Verdict::default()
                },
                None => output::interpret(event, &ran),
            };
            self.record(event, handler, verdict, &mut fired);
        }
        fired
    }

    fn note(
        &self,
        fired: &mut Fired,
        event: Event,
        command: &str,
        outcome: HookOutcome,
        message: &str,
    ) {
        fired.entries.push(json!({
            "event": event.as_str(),
            "command": command,
            "outcome": outcome.as_str(),
            "message": message,
        }));
        if outcome == HookOutcome::Error {
            crate::log::warn(
                "hook failed",
                &[
                    ("event", event.as_str()),
                    ("command", command),
                    ("error", message),
                ],
            );
        }
        let _ = self.events.send(CoreEvent::Hook {
            session_id: self.session_id.clone(),
            event: event.as_str().to_string(),
            command: command.to_string(),
            outcome,
            message: message.to_string(),
        });
    }

    fn record(&self, event: Event, handler: &Handler, verdict: Verdict, fired: &mut Fired) {
        let command = handler.command.as_str();
        if let Some(reason) = verdict.block {
            let outcome = if event == Event::PostToolUse {
                HookOutcome::Feedback
            } else {
                HookOutcome::Blocked
            };
            self.note(fired, event, command, outcome, &reason);
            fired.block.push(reason);
        }
        fired.ask |= verdict.ask;
        for context in verdict.context {
            self.note(fired, event, command, HookOutcome::Context, &context);
            fired.context.push(context);
        }
        if let Some(notice) = verdict.notice {
            self.note(fired, event, command, HookOutcome::Notice, &notice);
        }
        if let Some(reason) = verdict.stop {
            if event == Event::UserPromptSubmit {
                let reason = if reason.is_empty() {
                    "A hook stopped this prompt".to_string()
                } else {
                    reason
                };
                self.note(fired, event, command, HookOutcome::Blocked, &reason);
                fired.block.push(reason);
            } else {
                if !reason.is_empty() {
                    self.note(fired, event, command, HookOutcome::Notice, &reason);
                }
                fired.stop.get_or_insert(reason);
            }
        }
        if let Some(error) = verdict.error {
            self.note(fired, event, command, HookOutcome::Error, &error);
        }
    }
}

/// Where hooks run: the first of `candidates` that still exists (the session directory may have
/// been moved or deleted; a guard that cannot start would let the call through), else the system
/// temp directory.
fn working_dir(candidates: &[&std::path::Path]) -> PathBuf {
    candidates
        .iter()
        .find(|dir| dir.is_dir())
        .map(|dir| dir.to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
}

/// The combined answer of every hook that ran for one event.
#[derive(Debug, Default)]
pub(crate) struct Fired {
    /// Blocking reasons (PostToolUse: feedback), in config order.
    pub block: Vec<String>,
    pub ask: bool,
    pub context: Vec<String>,
    /// `continue: false`: its `stopReason`, possibly empty.
    pub stop: Option<String>,
    /// `{event, command, outcome, message}` for the tool part's `hooks` metadata.
    pub entries: Vec<Value>,
}

/// Process-wide hook state of the sessions a core drives.
#[derive(Default)]
pub(crate) struct State {
    /// Approved project hooks that keep running while a change to them waits for the user.
    pub approved: Arc<super::Approved>,
    /// Sessions this process fired SessionStart for (or decided not to).
    started: Mutex<HashSet<String>>,
    /// Root sessions whose turn a tool hook ended with `continue: false`: their Stop hooks do not
    /// run for that turn.
    stopped: Mutex<HashSet<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

struct ToolScope {
    context: Context,
    cancel: CancellationToken,
    /// `continue: false` from a tool hook during this step.
    stop: Mutex<Option<String>>,
}

/// Tool-call hooks of one step, carried by its [`crate::tool::ToolContext`]: direct calls, the
/// calls a Code Mode program makes and its `fetch()` all fire them. The default fires nothing.
#[derive(Clone, Default)]
pub struct ToolHooks(Option<Arc<ToolScope>>);

/// One tool call as hooks see it.
pub(crate) struct Call<'a> {
    /// Our tool's name (`shell`, `github_list_issues`, …).
    pub anysola: &'a str,
    /// The name hooks match (`Bash`, `mcp__github__list_issues`, …).
    pub name: &'a str,
    /// The call's input with Claude Code's keys added.
    pub input: &'a Value,
    pub id: &'a str,
}

impl ToolHooks {
    pub(crate) fn new(context: Context, cancel: CancellationToken) -> Self {
        ToolHooks(Some(Arc::new(ToolScope {
            context,
            cancel,
            stop: Mutex::new(None),
        })))
    }

    /// Whether any tool hook is configured at all (before trust or matching).
    pub(crate) fn wants(&self) -> bool {
        self.0.as_ref().is_some_and(|scope| {
            let hooks = &scope.context.config.hooks;
            let approved = &scope.context.approved;
            hooks.any(Event::PreToolUse)
                || hooks.any(Event::PostToolUse)
                || approved.any(Event::PreToolUse)
                || approved.any(Event::PostToolUse)
        })
    }

    /// Whether `target` is a file a hook command that can run here names (its script): a write
    /// to it always asks.
    pub(crate) fn guards(&self, target: &std::path::Path) -> bool {
        let Some(scope) = &self.0 else {
            return false;
        };
        let context = &scope.context;
        let places = super::scripts::Places {
            home: &context.home,
            project: &context.project_dir,
            session: &context.cwd,
        };
        let target = crate::config::real_path(target);
        super::guarded_files(
            &context.config.hooks,
            &context.db,
            &context.approved,
            &places,
        )
        .iter()
        .any(|file| crate::config::real_path(file) == target)
    }

    /// The `stopReason` of a tool hook that said `continue: false` during this step.
    pub(crate) fn stopped(&self) -> Option<String> {
        self.0.as_ref().and_then(|scope| lock(&scope.stop).clone())
    }

    fn fields(call: &Call<'_>) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("tool_name".into(), json!(call.name));
        out.insert("tool_input".into(), call.input.clone());
        out.insert("tool_use_id".into(), json!(call.id));
        out.insert("anysola_tool".into(), json!(call.anysola));
        out
    }

    async fn fire(&self, event: Event, call: &Call<'_>, extra: Map<String, Value>) -> Fired {
        let Some(scope) = &self.0 else {
            return Fired::default();
        };
        let mut fields = Self::fields(call);
        fields.extend(extra);
        let names = [call.anysola, call.name];
        let fired = scope
            .context
            .fire(event, &names, fields, &scope.cancel)
            .await;
        if let Some(reason) = &fired.stop {
            lock(&scope.stop).get_or_insert_with(|| reason.clone());
        }
        fired
    }

    pub(crate) async fn pre(&self, call: &Call<'_>) -> Fired {
        self.fire(Event::PreToolUse, call, Map::new()).await
    }

    /// `response`: `{"output", "metadata"}` or `{"error"}`.
    pub(crate) async fn post(&self, call: &Call<'_>, response: Value, duration_ms: u64) -> Fired {
        let mut extra = Map::new();
        extra.insert("tool_response".into(), response);
        extra.insert("duration_ms".into(), json!(duration_ms));
        self.fire(Event::PostToolUse, call, extra).await
    }
}

/// The tool error a PreToolUse block gives the model.
pub(crate) fn blocked(reasons: &[String]) -> String {
    format!(
        "A PreToolUse hook blocked this call: {}",
        reasons.join("\n")
    )
}

/// The tool error when a hook asked for approval and nobody can give it (unattended runs).
pub(crate) const UNANSWERABLE: &str =
    "A PreToolUse hook asked for approval of this call, and nobody can approve it here.";

/// What the model gets next to a tool's result from its hooks.
pub(crate) fn notes(pre: &Fired, post: &Fired) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(
        pre.context
            .iter()
            .map(|c| format!("PreToolUse hook context: {c}")),
    );
    out.extend(
        post.block
            .iter()
            .map(|r| format!("PostToolUse hook feedback: {r}")),
    );
    out.extend(
        post.context
            .iter()
            .map(|c| format!("PostToolUse hook context: {c}")),
    );
    out
}

/// A tool's result as PostToolUse gets it.
pub(crate) fn response(output: Result<(&str, &Map<String, Value>), &str>) -> Value {
    match output {
        Ok((text, metadata)) => json!({"output": text, "metadata": metadata}),
        Err(error) => json!({"error": error}),
    }
}

/// Whether any hook may run for `event`: configured, or approved earlier in this process.
fn wanted(shared: &Shared, config: &Config, event: Event) -> bool {
    config.hooks.any(event) || shared.hooks.approved.any(event)
}

fn synthetic(db: &Db, session_id: &str, text: &str, description: &str) {
    let data = encode(&SyntheticData {
        time: Created {
            created: crate::time::now_ms(),
        },
        text: text.to_string(),
        description: Some(description.to_string()),
        extra: Default::default(),
    });
    let _ = db.append_message(session_id, &crate::ids::message(), "synthetic", &data);
}

/// SessionStart, once per session and process: `startup` for a session without history,
/// `resume` for one that has some. Subagents' sessions do not fire it.
pub(crate) async fn session_start(
    shared: &Arc<Shared>,
    session: &SessionRow,
    cancel: &CancellationToken,
) {
    if session.parent_id.is_some() || !lock(&shared.hooks.started).insert(session.id.clone()) {
        return;
    }
    if !wanted(shared, &shared.loaded().config, Event::SessionStart) {
        return;
    }
    let history = shared
        .db
        .messages(&session.id)
        .unwrap_or_default()
        .iter()
        .any(|row| matches!(row.kind.as_str(), "user" | "assistant"));
    let source = if history { "resume" } else { "startup" };
    let model = crate::session::select(shared, session)
        .map(|selection| selection.model.id)
        .unwrap_or_default();
    let context = Context::of(shared, session);
    let mut extra = Map::new();
    extra.insert("source".into(), json!(source));
    extra.insert("model".into(), json!(model));
    let fired = context
        .fire(Event::SessionStart, &[source], extra, cancel)
        .await;
    for text in &fired.context {
        synthetic(&shared.db, &session.id, text, CONTEXT);
    }
}

pub(crate) enum Promoted {
    /// As [`inbox::promote`] counts it.
    Rows(Option<usize>),
    /// A UserPromptSubmit hook blocked a prompt, which was removed; the reason.
    Blocked(String),
}

/// [`inbox::promote`] with UserPromptSubmit: each real prompt of a root session runs its hooks
/// as it enters the conversation. A blocked prompt is removed (only that one: what was promoted
/// before it stays, what follows stays pending) and its reason returned.
pub(crate) async fn promote(
    shared: &Arc<Shared>,
    session: &SessionRow,
    scope: inbox::Scope,
    cancel: &CancellationToken,
) -> crate::storage::Result<Promoted> {
    let config = shared.loaded().config.clone();
    if session.parent_id.is_some() || !wanted(shared, &config, Event::UserPromptSubmit) {
        return inbox::promote(&shared.db, &session.id, scope).map(Promoted::Rows);
    }
    let Some(rows) = inbox::plan(&shared.db, &session.id, scope)? else {
        return Ok(Promoted::Rows(None));
    };
    let context = Context::of(shared, session);
    let mut promoted = 0;
    for row in rows {
        let mut added = Vec::new();
        if row.kind == "user" {
            let mut extra = Map::new();
            extra.insert("prompt".into(), row.payload["text"].clone());
            let fired = context
                .fire(Event::UserPromptSubmit, &[], extra, cancel)
                .await;
            if cancel.is_cancelled() {
                // The prompt stays pending; the interrupted turn ends at its next check.
                break;
            }
            if !fired.block.is_empty() {
                let _ = shared.db.inbox_remove(&row.id);
                return Ok(Promoted::Blocked(fired.block.join("\n")));
            }
            added = fired.context;
        }
        if inbox::publish_one(&shared.db, &session.id, &row)? {
            promoted += 1;
            for text in &added {
                synthetic(&shared.db, &session.id, text, CONTEXT);
            }
        }
    }
    Ok(Promoted::Rows(Some(promoted)))
}

/// Stop-hook continuations of one execution of a session.
#[derive(Debug, Default)]
pub(crate) struct StopRounds {
    /// The turn is continuing because of a Stop hook.
    active: bool,
    /// Consecutive continuations in which the model called no tool.
    idle: u32,
    /// The message id of the last continuation.
    mark: Option<String>,
}

/// Whether the model called a tool after message `mark`.
fn called_tools_since(db: &Db, session_id: &str, mark: &str) -> bool {
    db.messages(session_id)
        .unwrap_or_default()
        .iter()
        .skip_while(|row| row.id != mark)
        .any(|row| match Message::decode(&row.kind, &row.data) {
            Message::Assistant(step) => step.content.iter().any(|p| matches!(p, Part::Tool { .. })),
            _ => false,
        })
}

fn last_assistant_message(db: &Db, session_id: &str) -> String {
    db.messages(session_id)
        .unwrap_or_default()
        .iter()
        .rev()
        .find_map(|row| match Message::decode(&row.kind, &row.data) {
            Message::Assistant(step) if step.error.is_none() => Some(
                step.content
                    .iter()
                    .filter_map(|p| match p {
                        Part::Text { text, .. } if !text.is_empty() => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

/// A new turn starts: a `continue: false` of an earlier one no longer holds back Stop hooks.
pub(crate) fn turn_started(shared: &Shared, session_id: &str) {
    lock(&shared.hooks.stopped).remove(session_id);
}

/// Remembers that a tool hook ended a root session's turn with `continue: false`.
pub(crate) fn mark_stopped(shared: &Shared, session: &SessionRow) {
    if session.parent_id.is_none() {
        lock(&shared.hooks.stopped).insert(session.id.clone());
    }
}

/// Stop hooks, when a root session's turn ended successfully and the session would go idle.
/// Returns whether a hook keeps the turn going: its reasons were admitted as a steer.
pub(crate) async fn stop(
    shared: &Arc<Shared>,
    session_id: &str,
    rounds: &mut StopRounds,
    cancel: &CancellationToken,
) -> bool {
    if lock(&shared.hooks.stopped).remove(session_id) {
        *rounds = StopRounds::default();
        return false;
    }
    let Ok(Some(session)) = shared.db.session(session_id) else {
        return false;
    };
    if session.parent_id.is_some() || !wanted(shared, &shared.loaded().config, Event::Stop) {
        return false;
    }
    if let Some(mark) = &rounds.mark {
        if called_tools_since(&shared.db, session_id, mark) {
            rounds.idle = 0;
        } else {
            rounds.idle += 1;
        }
    }
    let context = Context::of(shared, &session);
    let mut extra = Map::new();
    extra.insert("stop_hook_active".into(), json!(rounds.active));
    extra.insert(
        "last_assistant_message".into(),
        json!(last_assistant_message(&shared.db, session_id)),
    );
    let fired = context.fire(Event::Stop, &[], extra, cancel).await;
    let reasons: Vec<String> = fired.block.into_iter().chain(fired.context).collect();
    if cancel.is_cancelled() || fired.stop.is_some() || reasons.is_empty() {
        *rounds = StopRounds::default();
        return false;
    }
    if rounds.idle >= STOP_ROUNDS {
        *rounds = StopRounds::default();
        let message = format!(
            "Stop hooks kept the turn going {STOP_ROUNDS} times in a row without a tool call, so the turn ended."
        );
        let _ = shared.events.send(CoreEvent::Hook {
            session_id: session_id.to_string(),
            event: Event::Stop.as_str().to_string(),
            command: String::new(),
            outcome: HookOutcome::Notice,
            message,
        });
        return false;
    }
    let text = reasons.join("\n\n");
    match inbox::enqueue_synthetic(
        &shared.db,
        session_id,
        &text,
        Some(STOP),
        None,
        inbox::Delivery::Steer,
    ) {
        Ok(id) => {
            rounds.active = true;
            rounds.mark = Some(id);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_run_in_the_first_directory_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        assert_eq!(working_dir(&[&gone, &root, dir.path()]), root);
        assert_eq!(working_dir(&[&gone]), std::env::temp_dir());
    }
}
