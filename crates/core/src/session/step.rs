//! Steps of a turn: build the request, stream the answer, run tool calls as they complete,
//! record the assistant message, and decide whether the turn continues.

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use serde_json::{Map, Value, json};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::gate::{RuleGate, Scope};
use super::lower::{Entry, Target, lower};
use super::message::{
    AssistantData, AssistantTime, Message, Part, StepSnapshot, StoredModel, ToolState, encode,
};
use super::request::{Request, build, output_limit};
use super::stream::{Action, Assembly};
use super::{Outcome, Selection, TurnOptions, compaction};
use crate::api::Shared;
use crate::events::{Event, InterruptReason};
use crate::gateway::error::{ApiError, Reason, SessionError};
use crate::gateway::retry::Retry;
use crate::gateway::{MessageStream, StreamItem};
use crate::instructions::{self, Environment};
use crate::storage::SessionRow;
use crate::time::now_ms;
use crate::tool::registry::{Registry, process_year};
use crate::tool::{DefinitionEnv, Tool, ToolContext, ToolError, ToolErrorKind, ToolOutput};

const MAX_STEPS_PROMPT: &str = include_str!("../prompt/max-steps.txt");
const CONTINUE_AFTER_INCOMPLETE_STREAM: &str = "The previous response was interrupted. Continue from where you left off without repeating completed content.";
const CONTINUE_AFTER_OUTPUT_LIMIT: &str = "Your last response hit the output token limit (stop reason: length). Do not apologize, recap, or repeat yourself. Break the remaining work into smaller pieces.";
const WORKTREE_HINT: &str = "When you create a worktree outside the current working directory and intend to use it as your primary working directory, consider using `execute` to call `tools.opencode.session_move` and make the worktree the session's working directory.";
const SYSTEM_TEMPLATE: &str = include_str!("../prompt/system.txt");
const ANTHROPIC_PROMPT: &str = include_str!("../prompt/anthropic.txt");
const OUTPUT_LIMIT_CONTINUATIONS: usize = 2;

/// Tool guidance lines for the tools actually advertised.
pub fn default_system(tools: &[&str]) -> String {
    let mut lines = Vec::new();
    if tools.contains(&"shell") {
        lines.push("- Prefer dedicated tools over shell commands; fall back to the shell when a tool cannot do what you need.");
        lines.push("- Do not chain shell commands with separators like `echo \"====\";` or `printf '---'`; the output becomes noisy in a way that makes the user's side of the conversation worse.");
    }
    if tools.contains(&"write") {
        lines.push("- Use the write tool to create files or completely replace their content. Prefer using the edit tool for targeted changes.");
    }
    if tools.contains(&"edit") {
        lines.push("- Use the edit tool for targeted changes to existing text files. It replaces the exact text in `oldString` with `newString`, and the values must differ. By default, `oldString` must occur exactly once. If it occurs multiple times, include more surrounding context to make it unique or set `replaceAll` to true to replace every occurrence.");
    }
    SYSTEM_TEMPLATE.replace("${OPENCODE_TOOL_GUIDANCE}", &lines.join("\n"))
}

pub fn identity(name: &str, id: &str) -> String {
    format!("# Your Model\n- Name: {name}\n- Provider ID: anysola\n- Model ID: {id}")
}

struct Context {
    system: Vec<String>,
    tools: Vec<Arc<dyn Tool>>,
    definitions: Vec<crate::tool::ToolDefinition>,
    rules: Vec<crate::permission::Rule>,
}

/// Commits the instruction sources' current values (the baseline on the first step, an update
/// message on later changes) and returns block 2, the frozen baseline.
fn instructions(
    shared: &Shared,
    session: &SessionRow,
    rules: &[crate::permission::Rule],
) -> Result<String, SessionError> {
    let directory = std::path::PathBuf::from(&session.directory);
    let worktree = shared.project.worktree.clone();
    let loaded = shared.loaded();
    let files = instructions::discover(
        &shared.paths,
        &directory,
        &worktree,
        &loaded.config.instructions,
        &loaded.remote_instructions,
    );
    let environment = Environment {
        git: shared.project.vcs == Some("git"),
        directory,
        worktree,
    };
    let catalog = shared
        .registry
        .code_mode(rules)
        .map(|inventory| inventory.summary());
    let sources = instructions::sources(
        catalog.as_ref(),
        crate::skill::guidance(&loaded.skills, rules),
        files,
        &crate::time::date_string(),
        &environment,
    );
    Ok(instructions::prepare(&shared.db, &session.id, &sources)?.baseline)
}

/// Settles the step context before new input is admitted, so instruction changes are recorded
/// ahead of the prompt that follows them.
pub(crate) fn settle_context(
    shared: &Shared,
    session: &SessionRow,
    selection: &Selection,
) -> Result<(), SessionError> {
    context(shared, session, selection).map(drop)
}

fn context(
    shared: &Shared,
    session: &SessionRow,
    selection: &Selection,
) -> Result<Context, SessionError> {
    let rules = selection.agent.permissions.clone();
    let mut tools = shared.registry.active(&rules);
    let loaded = shared.loaded();
    if !loaded.config.websearch_enabled() {
        tools.retain(|t| t.name() != "websearch");
    }
    let env = DefinitionEnv {
        shell_name: crate::tool::shell::shell_name(loaded.config.shell()),
        year: process_year(),
        subagents: loaded.agents.subagents_for(&rules),
    };
    let definitions = Registry::definitions(&tools, &env);
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    let block0 = match &selection.agent.system {
        Some(system) => system.clone(),
        None => format!("{}\n\n{ANTHROPIC_PROMPT}", default_system(&names)),
    };
    let block2 = instructions(shared, session, &rules)?;
    let system = vec![
        block0,
        identity(&selection.model.name, &selection.model.id),
        block2,
        WORKTREE_HINT.into(),
    ];
    Ok(Context {
        system,
        tools,
        definitions,
        rules,
    })
}

fn send(shared: &Shared, event: Event) {
    let _ = shared.events.send(event);
}

enum StepEnd {
    Continue,
    Stop,
    /// Auto compaction replaced the history; the step runs again on the checkpoint.
    Compacted,
    /// The provider rejected the prompt as too long before any output.
    Overflow(ApiError),
    Failed(SessionError),
    Interrupted(InterruptReason),
}

pub(crate) async fn drive(
    shared: &Arc<Shared>,
    session: &SessionRow,
    selection: &Selection,
    options: TurnOptions,
    cancel: &CancellationToken,
) -> Outcome {
    let mut length_finishes = 0;
    let mut number = 0;
    let mut overflowed = false;
    loop {
        number += 1;
        if cancel.is_cancelled() {
            return Outcome::Interrupted(InterruptReason::User);
        }
        if options.stop_at_budget {
            let credits = shared.credits.snapshot(&session.id);
            if credits.budget.is_some_and(|budget| credits.total >= budget) {
                return Outcome::Interrupted(InterruptReason::Budget);
            }
        }
        // A tool may have moved the session; the next step runs in its new directory.
        let current = shared.db.session(&session.id).ok().flatten();
        let session = current.as_ref().unwrap_or(session);
        if number > 1 {
            // Steered input joins the turn at this boundary, after the context it follows.
            if let Err(error) = settle_context(shared, session, selection) {
                return Outcome::Failed(error);
            }
            if let Ok(Some(promoted)) =
                super::inbox::promote(&shared.db, &session.id, super::inbox::Scope::Steer)
                && promoted > 0
            {
                number = 1;
            }
        }
        let capped = selection.agent.steps == Some(number);
        match step(shared, session, selection, capped, cancel).await {
            StepEnd::Compacted => {
                number -= 1;
                continue;
            }
            // One overflow compaction per step; a second rejection surfaces.
            StepEnd::Overflow(error) if !overflowed => {
                match overflow(shared, session, selection, cancel).await {
                    compaction::Outcome::Completed => {
                        overflowed = true;
                        number -= 1;
                        continue;
                    }
                    compaction::Outcome::Failed(error) => return Outcome::Failed(error),
                    compaction::Outcome::Skipped => {
                        return Outcome::Failed(error.to_session_error());
                    }
                }
            }
            StepEnd::Overflow(error) => return Outcome::Failed(error.to_session_error()),
            StepEnd::Continue if capped => return Outcome::Succeeded,
            StepEnd::Continue => {}
            StepEnd::Stop => return Outcome::Succeeded,
            StepEnd::Failed(error) => return Outcome::Failed(error),
            StepEnd::Interrupted(reason) => return Outcome::Interrupted(reason),
        }
        overflowed = false;
        let last_finish = compaction::history(shared, &session.id)
            .messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::Assistant(a) => a.finish.clone(),
                _ => None,
            });
        if last_finish.as_deref() == Some("length") {
            length_finishes += 1;
            if length_finishes > OUTPUT_LIMIT_CONTINUATIONS {
                return Outcome::Failed(SessionError::new(
                    "output-limit",
                    "Response still truncated after two output token limit continuations",
                ));
            }
            super::synthetic(shared, &session.id, CONTINUE_AFTER_OUTPUT_LIMIT.into());
        } else {
            length_finishes = 0;
        }
    }
}

/// `/compact`: a manual compaction of the session's history.
pub(crate) async fn compact_manually(
    shared: &Arc<Shared>,
    session: &SessionRow,
    selection: &Selection,
    cancel: &CancellationToken,
) -> compaction::Outcome {
    triggered(shared, session, selection, cancel, false).await
}

/// The overflow compaction after a "too long" rejection, sized from the rejected prompt.
async fn overflow(
    shared: &Arc<Shared>,
    session: &SessionRow,
    selection: &Selection,
    cancel: &CancellationToken,
) -> compaction::Outcome {
    triggered(shared, session, selection, cancel, true).await
}

async fn triggered(
    shared: &Arc<Shared>,
    session: &SessionRow,
    selection: &Selection,
    cancel: &CancellationToken,
    overflow: bool,
) -> compaction::Outcome {
    let ctx = match context(shared, session, selection) {
        Ok(ctx) => ctx,
        Err(error) => return compaction::Outcome::Failed(error),
    };
    let target = Target {
        provider: &selection.reference.provider,
        id: &selection.model.id,
        input: &selection.model.input,
    };
    let history = compaction::history(shared, &session.id).messages;
    let estimate = compaction::estimate_prompt(&history, &target, &ctx.system, &ctx.definitions);
    let prefix = compaction::Prefix {
        system: &ctx.system,
        tools: &ctx.definitions,
    };
    compaction::compact(
        shared,
        session,
        selection,
        prefix,
        if overflow {
            compaction::Trigger::Overflow(estimate)
        } else {
            compaction::Trigger::Manual
        },
        cancel,
    )
    .await
}

struct Running {
    session_id: String,
    message_id: String,
    data: AssistantData,
}

impl Running {
    fn save(&self, shared: &Shared) {
        let _ = shared
            .db
            .update_message(&self.message_id, &encode(&self.data));
    }
}

async fn step(
    shared: &Arc<Shared>,
    session: &SessionRow,
    selection: &Selection,
    capped: bool,
    cancel: &CancellationToken,
) -> StepEnd {
    let ctx = match context(shared, session, selection) {
        Ok(ctx) => ctx,
        Err(error) => return StepEnd::Failed(error),
    };
    let history = compaction::history(shared, &session.id).messages;
    let target = Target {
        provider: &selection.reference.provider,
        id: &selection.model.id,
        input: &selection.model.input,
    };
    let estimate = compaction::estimate_prompt(&history, &target, &ctx.system, &ctx.definitions);
    let prefix = compaction::Prefix {
        system: &ctx.system,
        tools: &ctx.definitions,
    };
    let trigger = compaction::Trigger::Auto(estimate);
    match compaction::compact(shared, session, selection, prefix, trigger, cancel).await {
        compaction::Outcome::Skipped => {}
        compaction::Outcome::Completed => return StepEnd::Compacted,
        compaction::Outcome::Failed(error) => return StepEnd::Failed(error),
    }
    let mut messages = lower(&history, &target);
    if capped {
        messages.push(Entry::Turn(
            json!({"role": "assistant", "content": [{"type": "text", "text": MAX_STEPS_PROMPT}]}),
        ));
    }
    let max_tokens = output_limit(
        &selection.model,
        estimate.measured,
        estimate.estimated,
        0.01,
    );
    let variant = selection
        .reference
        .variant
        .as_deref()
        .and_then(|v| selection.model.variant(v));
    let (body, mut headers) = build(Request {
        model: &selection.model,
        variant,
        system: ctx.system.clone(),
        tools: ctx.definitions.clone(),
        messages,
        max_tokens,
        tools_disabled: capped,
    });
    headers.extend(session_headers(shared, session));

    let message_id = crate::ids::message();
    let start = capture(shared);
    let running = Running {
        session_id: session.id.clone(),
        message_id: message_id.clone(),
        data: AssistantData {
            time: AssistantTime {
                created: now_ms(),
                ..AssistantTime::default()
            },
            agent: selection.agent.id.clone(),
            model: StoredModel {
                id: selection.model.id.clone(),
                provider_id: selection.reference.provider.clone(),
                variant: selection.reference.variant.clone(),
            },
            content: Vec::new(),
            snapshot: start.clone().map(|start| StepSnapshot {
                start: Some(start),
                ..StepSnapshot::default()
            }),
            finish: None,
            raw_finish: None,
            cost: None,
            tokens: None,
            error: None,
            extra: Default::default(),
        },
    };
    let _ = shared.db.append_message(
        &session.id,
        &message_id,
        "assistant",
        &encode(&running.data),
    );

    let started = |shared: &Shared| {
        send(
            shared,
            Event::StepStarted {
                session_id: session.id.clone(),
                message_id: message_id.clone(),
                event_id: crate::ids::event(),
                agent: selection.agent.id.clone(),
                model_id: selection.model.id.clone(),
                snapshot: start.clone(),
            },
        )
    };
    started(shared);

    let mut retry = Retry::default();
    let stream = loop {
        let attempt = tokio::select! {
            result = shared.gateway.messages(body.clone(), &headers) => result,
            _ = cancel.cancelled() => return interrupt(shared, running, Vec::new(), InterruptReason::User),
        };
        match attempt {
            Ok(stream) => break stream,
            Err(error) if error.reason == Reason::ContextOverflow => {
                return overflowed(shared, &running, error);
            }
            Err(error) => match retry.next(&error) {
                Some(delay) => {
                    wait_retry(shared, &session.id, retry.attempt(), delay, &error);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => started(shared),
                        _ = cancel.cancelled() => return interrupt(shared, running, Vec::new(), InterruptReason::User),
                    }
                }
                None => return fail(shared, running, error.to_session_error()),
            },
        }
    };
    run_stream(
        shared, session, selection, &ctx, stream, running, capped, cancel, retry, body, headers,
    )
    .await
}

fn wait_retry(shared: &Shared, session_id: &str, attempt: u32, delay: Duration, error: &ApiError) {
    send(
        shared,
        Event::RetryScheduled {
            session_id: session_id.into(),
            attempt,
            delay_ms: delay.as_millis() as u64,
            error: error.to_session_error(),
        },
    );
}

pub(crate) fn session_headers(shared: &Shared, session: &SessionRow) -> Vec<(String, String)> {
    // Children share their parent's cache affinity, forks their source's.
    let affinity = session
        .parent_id
        .clone()
        .or_else(|| session.fork_session_id.clone())
        .unwrap_or_else(|| session.id.clone());
    let mut headers = vec![
        ("x-opencode-client".to_string(), "cli".to_string()),
        ("x-opencode-project".to_string(), shared.project.id.clone()),
        ("x-opencode-session".to_string(), affinity.clone()),
        ("x-opencode-session-id".to_string(), session.id.clone()),
        ("x-session-affinity".to_string(), affinity.clone()),
        ("x-session-id".to_string(), affinity),
    ];
    if let Some(parent) = &session.parent_id {
        headers.push(("x-opencode-parent-session-id".into(), parent.clone()));
        headers.push(("x-parent-session-id".into(), parent.clone()));
    }
    headers
}

/// The project's files as a snapshot tree, when snapshots are on.
fn capture(shared: &Shared) -> Option<String> {
    shared.snapshots.as_ref()?.capture()
}

/// Captures the tree after the step and records the files the step changed.
fn end_snapshot(shared: &Shared, running: &mut Running) -> Option<String> {
    let store = shared.snapshots.as_ref()?;
    let snapshot = running.data.snapshot.as_mut()?;
    let end = store.capture()?;
    if let Some(start) = &snapshot.start {
        snapshot.files = Some(store.files(start, &end));
    }
    snapshot.end = Some(end.clone());
    Some(end)
}

/// A rejected prompt leaves no trace: its step reruns after the overflow compaction.
fn overflowed(shared: &Shared, running: &Running, error: ApiError) -> StepEnd {
    let _ = shared.db.delete_message(&running.message_id);
    StepEnd::Overflow(error)
}

fn fail(shared: &Shared, mut running: Running, error: SessionError) -> StepEnd {
    running.data.error = Some(error.clone());
    running.data.time.completed = Some(now_ms());
    running.save(shared);
    send(
        shared,
        Event::StepFailed {
            session_id: running.session_id.clone(),
            message_id: running.message_id.clone(),
            error: error.clone(),
        },
    );
    StepEnd::Failed(error)
}

/// Ends the step as interrupted: unsettled tools fail, the step reports `Step interrupted`.
fn interrupt(
    shared: &Shared,
    mut running: Running,
    unsettled: Vec<usize>,
    reason: InterruptReason,
) -> StepEnd {
    let error = SessionError::new("aborted", "Tool execution interrupted");
    for index in unsettled {
        settle(
            shared,
            &mut running,
            index,
            Err(ToolError::interrupted()),
            &error,
        );
    }
    running.data.error = Some(SessionError::new("aborted", "Step interrupted"));
    running.data.finish.get_or_insert_with(|| "error".into());
    running.data.time.completed = Some(now_ms());
    running.save(shared);
    let end = end_snapshot(shared, &mut running);
    if let Some(tokens) = running.data.tokens {
        send(
            shared,
            Event::StepEnded {
                session_id: running.session_id.clone(),
                message_id: running.message_id.clone(),
                event_id: crate::ids::event(),
                finish: "error".into(),
                tokens,
                last: true,
                snapshot: end,
            },
        );
    }
    send(
        shared,
        Event::StepFailed {
            session_id: running.session_id.clone(),
            message_id: running.message_id.clone(),
            error: SessionError::new("aborted", "Step interrupted"),
        },
    );
    StepEnd::Interrupted(reason)
}

/// Records a tool result on its part and reports it.
fn settle(
    shared: &Shared,
    running: &mut Running,
    index: usize,
    result: Result<ToolOutput, ToolError>,
    interrupted: &SessionError,
) {
    let Some(Part::Tool {
        id, state, time, ..
    }) = running.data.content.get_mut(index)
    else {
        return;
    };
    let input = match state {
        ToolState::Running { input, .. } => input.clone(),
        ToolState::Streaming { input } => serde_json::from_str(input).unwrap_or(json!({})),
        _ => return,
    };
    time.completed = Some(now_ms());
    let (session_id, message_id, call_id) = (
        running.session_id.clone(),
        running.message_id.clone(),
        id.clone(),
    );
    match result {
        Ok(output) => {
            *state = ToolState::Completed {
                input,
                content: output.content.clone(),
                metadata: output.metadata.clone(),
            };
            send(
                shared,
                Event::ToolSucceeded {
                    session_id,
                    message_id,
                    call_id,
                    content: output.content,
                    metadata: output.metadata,
                },
            );
        }
        Err(error) => {
            let error = match error.kind {
                ToolErrorKind::Interrupted => interrupted.clone(),
                _ => SessionError::new(error.type_tag(), error.message),
            };
            *state = ToolState::Error {
                input,
                error: error.clone(),
                content: Vec::new(),
                metadata: Map::new(),
            };
            send(
                shared,
                Event::ToolFailed {
                    session_id,
                    message_id,
                    call_id,
                    error,
                    content: Vec::new(),
                    metadata: Map::new(),
                },
            );
        }
    }
}

type ToolTask = (usize, Result<ToolOutput, ToolError>);

#[allow(clippy::too_many_arguments)]
async fn run_stream(
    shared: &Arc<Shared>,
    session: &SessionRow,
    selection: &Selection,
    ctx: &Context,
    mut stream: MessageStream,
    mut running: Running,
    capped: bool,
    cancel: &CancellationToken,
    mut retry: Retry,
    body: String,
    headers: Vec<(String, String)>,
) -> StepEnd {
    let tool_cancel = cancel.child_token();
    let mut configured = ctx.rules.clone();
    configured.extend(
        shared
            .db
            .session_permissions(&session.id)
            .unwrap_or_default(),
    );
    let gate = Arc::new(RuleGate {
        session_id: session.id.clone(),
        scope: Arc::new(Scope {
            configured,
            project_id: shared.project.id.clone(),
            policies: Arc::new(shared.loaded().config.policies()),
        }),
        asks: shared.asks.clone(),
    });
    let mut tasks: JoinSet<ToolTask> = JoinSet::new();
    let mut assembly = Assembly::default();
    let mut ended_ok = false;
    let mut stream_error: Option<ApiError> = None;
    loop {
        let item = tokio::select! {
            item = stream.next() => item,
            _ = cancel.cancelled() => break,
        };
        let Some(item) = item else { break };
        let event = match item {
            Ok(StreamItem::Credits { charged, remaining }) => {
                shared.credits.record(&session.id, charged, remaining);
                continue;
            }
            Ok(StreamItem::Event(event)) => event,
            Err(error) => {
                stream_error = Some(error);
                break;
            }
        };
        let actions = match assembly.on_event(&event) {
            Ok(actions) => actions,
            Err(error) => {
                stream_error = Some(error);
                break;
            }
        };
        running.data.content = assembly.parts.clone();
        for action in actions {
            handle(
                shared,
                session,
                selection,
                ctx,
                &running,
                action,
                capped,
                &gate,
                &tool_cancel,
                &mut tasks,
            );
        }
        if assembly.finished {
            ended_ok = true;
        }
    }
    running.data.content = assembly.parts.clone();
    running.data.tokens = Some(assembly.tokens);

    if cancel.is_cancelled() {
        tool_cancel.cancel();
        let unsettled = collect(shared, &mut running, &mut tasks).await;
        return interrupt(shared, running, unsettled, InterruptReason::User);
    }
    if !ended_ok {
        let error = stream_error
            .unwrap_or_else(|| ApiError::incomplete("The provider response ended unexpectedly."));
        if error.reason == Reason::ContextOverflow && !assembly.has_output() {
            tool_cancel.cancel();
            return overflowed(shared, &running, error);
        }
        if !assembly.has_output()
            && let Some(delay) = retry.next(&error)
        {
            // Nothing was produced: send the whole request again.
            wait_retry(shared, &session.id, retry.attempt(), delay, &error);
            tokio::time::sleep(delay).await;
            return resend(
                shared, session, selection, ctx, running, capped, cancel, retry, body, headers,
            )
            .await;
        }
        tool_cancel.cancel();
        let unsettled = collect(shared, &mut running, &mut tasks).await;
        if error.retryable() && assembly.has_output() {
            for index in unsettled {
                settle(
                    shared,
                    &mut running,
                    index,
                    Err(ToolError::interrupted()),
                    &interrupted_error(),
                );
            }
            running.data.finish = Some("unknown".into());
            running.data.time.completed = Some(now_ms());
            running.save(shared);
            super::synthetic(shared, &session.id, CONTINUE_AFTER_INCOMPLETE_STREAM.into());
            return StepEnd::Continue;
        }
        return fail(shared, running, error.to_session_error());
    }

    let unsettled = collect(shared, &mut running, &mut tasks).await;
    debug_assert!(unsettled.is_empty());
    let declined = running.data.content.iter().any(|p| {
        matches!(p, Part::Tool { state: ToolState::Error { error, .. }, .. }
            if error.message == "The user declined this tool call"
                || error.message == crate::tool::question::DISMISSED)
    });
    let finish = assembly.finish();
    running.data.finish = Some(finish.into());
    running.data.raw_finish = assembly.stop_reason.clone();
    running.data.cost = Some(0.0);
    running.data.time.completed = Some(now_ms());
    let ran_tools = running
        .data
        .content
        .iter()
        .any(|p| matches!(p, Part::Tool { .. }));
    let _ = shared.db.add_tokens(
        &session.id,
        crate::storage::TokenTotals {
            input: assembly.tokens.input,
            output: assembly.tokens.output,
            reasoning: assembly.tokens.reasoning,
            cache_read: assembly.tokens.cache.read,
            cache_write: assembly.tokens.cache.write,
        },
    );
    if declined {
        return interrupt(shared, running, Vec::new(), InterruptReason::Declined);
    }
    let end = end_snapshot(shared, &mut running);
    running.save(shared);
    let continues = (ran_tools && !capped) || finish == "length";
    send(
        shared,
        Event::StepEnded {
            session_id: session.id.clone(),
            message_id: running.message_id.clone(),
            event_id: crate::ids::event(),
            finish: finish.into(),
            tokens: assembly.tokens,
            last: !continues,
            snapshot: end,
        },
    );
    if continues {
        StepEnd::Continue
    } else {
        StepEnd::Stop
    }
}

fn interrupted_error() -> SessionError {
    SessionError::new("aborted", "Tool execution interrupted")
}

/// Retries a stream that failed before producing anything, in the same step.
#[allow(clippy::too_many_arguments)]
fn resend<'a>(
    shared: &'a Arc<Shared>,
    session: &'a SessionRow,
    selection: &'a Selection,
    ctx: &'a Context,
    running: Running,
    capped: bool,
    cancel: &'a CancellationToken,
    mut retry: Retry,
    body: String,
    headers: Vec<(String, String)>,
) -> futures_util::future::BoxFuture<'a, StepEnd> {
    async move {
        send(
            shared,
            Event::StepStarted {
                session_id: session.id.clone(),
                message_id: running.message_id.clone(),
                event_id: crate::ids::event(),
                agent: selection.agent.id.clone(),
                model_id: selection.model.id.clone(),
                snapshot: running.data.snapshot.as_ref().and_then(|s| s.start.clone()),
            },
        );
        let _ = shared
            .db
            .update_message(&running.message_id, &encode(&running.data));
        loop {
            match shared.gateway.messages(body.clone(), &headers).await {
                Ok(stream) => {
                    return run_stream(
                        shared, session, selection, ctx, stream, running, capped, cancel, retry,
                        body, headers,
                    )
                    .await;
                }
                Err(error) => match retry.next(&error) {
                    Some(delay) => {
                        wait_retry(shared, &session.id, retry.attempt(), delay, &error);
                        tokio::time::sleep(delay).await;
                    }
                    None => return fail(shared, running, error.to_session_error()),
                },
            }
        }
    }
    .boxed()
}

/// Waits for every tool task, recording each result as it lands. Returns the tool parts that
/// never settled.
async fn collect(
    shared: &Shared,
    running: &mut Running,
    tasks: &mut JoinSet<ToolTask>,
) -> Vec<usize> {
    while let Some(joined) = tasks.join_next().await {
        if let Ok((index, result)) = joined {
            settle(shared, running, index, result, &interrupted_error());
            running.save(shared);
        }
    }
    running
        .data
        .content
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            matches!(
                p,
                Part::Tool {
                    state: ToolState::Running { .. } | ToolState::Streaming { .. },
                    ..
                }
            )
        })
        .map(|(i, _)| i)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn handle(
    shared: &Arc<Shared>,
    session: &SessionRow,
    selection: &Selection,
    ctx: &Context,
    running: &Running,
    action: Action,
    capped: bool,
    gate: &Arc<RuleGate>,
    cancel: &CancellationToken,
    tasks: &mut JoinSet<ToolTask>,
) {
    let session_id = session.id.clone();
    let message_id = running.message_id.clone();
    match action {
        Action::TextStarted { ordinal } => send(
            shared,
            Event::TextStarted {
                session_id,
                message_id,
                event_id: crate::ids::event(),
                ordinal,
            },
        ),
        Action::TextEnded { ordinal, text } => send(
            shared,
            Event::TextEnded {
                session_id,
                message_id,
                event_id: crate::ids::event(),
                ordinal,
                text,
            },
        ),
        Action::ReasoningStarted { ordinal } => send(
            shared,
            Event::ReasoningStarted {
                session_id,
                message_id,
                event_id: crate::ids::event(),
                ordinal,
            },
        ),
        Action::ReasoningEnded {
            ordinal,
            text,
            native,
        } => send(
            shared,
            Event::ReasoningEnded {
                session_id,
                message_id,
                event_id: crate::ids::event(),
                ordinal,
                text,
                native,
            },
        ),
        Action::ToolStarted { call_id, name } => send(
            shared,
            Event::ToolInputStarted {
                session_id,
                message_id,
                event_id: crate::ids::event(),
                call_id,
                name,
            },
        ),
        Action::ToolReady {
            call_id,
            name,
            input,
            raw,
            part,
        } => {
            send(
                shared,
                Event::ToolCalled {
                    session_id: session_id.clone(),
                    message_id: message_id.clone(),
                    call_id: call_id.clone(),
                    input: input.clone(),
                    raw,
                },
            );
            let tool_ctx = ToolContext {
                session_id,
                message_id,
                call_id,
                agent: selection.agent.id.clone(),
                directory: session.directory.clone().into(),
                worktree: shared.project.worktree.clone(),
                project_id: shared.project.id.clone(),
                paths: shared.paths.clone(),
                permissions: gate.clone(),
                cancel: cancel.clone(),
                env: shared.env.clone(),
                configured_shell: shared.loaded().config.shell().map(str::to_string),
                limits: shared.loaded().config.tool_output(),
                locks: shared.locks.clone(),
                notifier: shared.notifier.clone(),
                web_search: Arc::new(crate::search::GatewaySearch::new(
                    shared.gateway.clone(),
                    shared.credits.clone(),
                )),
                host: shared.clone(),
                session: Arc::new(super::host::Host {
                    shared: shared.clone(),
                }),
            };
            let tools = ctx.tools.clone();
            let output_dir = shared.paths.tool_output_dir();
            let images = crate::media::Limits::from_config(shared.loaded().config.get("media"));
            tasks.spawn(async move {
                let result = execute(&tools, &name, input, &tool_ctx, capped).await;
                let result = result.map(|out| {
                    let mut out = crate::tool::bound::bound(out, tool_ctx.limits, &output_dir);
                    out.content = crate::media::normalize_content(out.content, &images);
                    out
                });
                (part, result)
            });
        }
    }
}

async fn execute(
    tools: &[Arc<dyn Tool>],
    name: &str,
    input: Value,
    ctx: &ToolContext,
    capped: bool,
) -> Result<ToolOutput, ToolError> {
    if capped {
        return Err(ToolError::execution(
            "Tools are disabled after the maximum agent steps",
        ));
    }
    let find = |name: &str| tools.iter().find(|t| t.name() == name).cloned();
    let (tool, alias) = match find(name) {
        Some(tool) => (tool, None),
        None if name == "bash" => match find("shell") {
            Some(tool) => (
                tool,
                Some(
                    "There is no \"bash\" tool, so this ran as \"shell\". Call \"shell\" directly.",
                ),
            ),
            None => return Err(not_available(name)),
        },
        None => return Err(not_available(name)),
    };
    let result = tokio::select! {
        result = tool.execute(input, ctx) => result,
        _ = ctx.cancel.cancelled() => Err(ToolError::interrupted()),
    };
    match (result, alias) {
        (Ok(mut output), Some(note)) => {
            output.content.push(crate::tool::ToolContent::text(note));
            Ok(output)
        }
        (Err(mut error), Some(note)) => {
            error.message = format!("{}\n\n{note}", error.message);
            Err(error)
        }
        (result, None) => result,
    }
}

fn not_available(name: &str) -> ToolError {
    ToolError::execution(format!(
        "No tool named \"{name}\" is currently available. Please use a tool from the available tool list."
    ))
}
