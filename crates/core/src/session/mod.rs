//! The agent loop: a turn is a series of steps, each one model request plus the tool calls it
//! makes. Tools start as soon as their `tool_use` block completes and run while the stream goes on.

pub mod compaction;
pub mod gate;
pub mod goal;
pub(crate) mod host;
pub mod inbox;
pub mod lower;
pub mod message;
pub mod question;
pub mod recent;
pub mod request;
pub(crate) mod step;
mod stream;
pub mod subagent;
mod title;
mod warming;

pub(crate) use step::session_headers as step_headers;
pub(crate) use warming::Warming;

use std::sync::Arc;

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::agent::Agent;
use crate::api::{Loaded, Shared};
use crate::events::{Event, InterruptReason};
use crate::gateway::error::SessionError;
use crate::model::{Model, ModelRef};
use crate::storage::SessionRow;
use crate::time::now_ms;
use message::{Created, Message, SyntheticData, encode};

pub const PLAN_LEAVE: &str = "<system-reminder>\nYou are NO LONGER in Plan mode. The previous Plan restrictions no longer apply. Any Plan mode instructions from earlier in this conversation are no longer active.\n</system-reminder>";

pub fn plan_enter(directory: &str) -> String {
    format!(
        "<system-reminder>\nYou are in Plan mode. Discuss the plan with the user directly in the conversation. Do not create or update plan files unless the user explicitly asks you to; when they do, write them only in:\n{directory}\n\nDo not modify any other files or ask a subagent to do so.\n\nYou remain in Plan mode until the user switches agents. If the user asks you to implement changes, do not do so. Tell them they need to switch agents.\n</system-reminder>"
    )
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TurnOptions {
    /// Stop inside the turn, at the next step start, once the session's budget is spent
    /// (`anysola run --budget`).
    pub stop_at_budget: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Succeeded,
    Failed(SessionError),
    Interrupted(InterruptReason),
}

/// The config sources, agent, model and variant a step runs with.
#[derive(Clone)]
pub(crate) struct Selection {
    pub sources: Arc<Loaded>,
    pub agent: Agent,
    pub model: Model,
    pub reference: ModelRef,
}

pub(crate) fn select(shared: &Shared, session: &SessionRow) -> Result<Selection, SessionError> {
    let sources = shared.loaded();
    let agent = match session.agent.as_deref() {
        Some(id) => sources.agents.get(id).cloned(),
        None => sources.agents.default_agent().cloned(),
    }
    .ok_or_else(|| {
        SessionError::new(
            "unknown",
            format!(
                "Agent not found: \"{}\"",
                session.agent.as_deref().unwrap_or("build")
            ),
        )
    })?;
    let no_route = || {
        SessionError::new(
            "provider.no-route",
            format!("No model is available for session {}", session.id),
        )
    };
    let reference = agent
        .model
        .as_deref()
        .and_then(ModelRef::parse)
        .or_else(|| session.model.as_ref().and_then(ModelRef::from_json))
        .or_else(|| sources.config.model().and_then(ModelRef::parse))
        .ok_or_else(no_route)?;
    let model = crate::model::resolve(&sources.models, &reference)
        .map_err(|error| SessionError::new("provider.no-route", error.to_string()))?;
    Ok(Selection {
        sources,
        agent,
        model,
        reference,
    })
}

fn synthetic(shared: &Shared, session_id: &str, text: String) {
    let data = encode(&SyntheticData {
        time: Created { created: now_ms() },
        text,
        description: None,
        extra: Default::default(),
    });
    let _ = shared
        .db
        .append_message(session_id, &crate::ids::message(), "synthetic", &data);
}

/// Keeps the plan-mode reminders in step with the agent: entering plan adds the enter reminder,
/// leaving it adds the leave reminder.
fn plan_reminders(shared: &Shared, session_id: &str, agent: &str) {
    let enter = plan_enter(&shared.paths.plan_dir().display().to_string());
    let last = shared
        .db
        .messages(session_id)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find_map(|row| match Message::decode(&row.kind, &row.data) {
            Message::Synthetic(data) if data.text == enter || data.text == PLAN_LEAVE => {
                Some(data.text)
            }
            _ => None,
        });
    let in_plan = last.as_deref() == Some(enter.as_str());
    if agent == "plan" && !in_plan {
        synthetic(shared, session_id, enter);
    } else if agent != "plan" && in_plan {
        synthetic(shared, session_id, PLAN_LEAVE.to_string());
    }
}

/// Plan mode per request: compaction or a committed revert can strip the reminders while the
/// agent stays put. A missing enter reminder (or a stale one after leaving plan) is added before
/// the user's prompt and recorded.
fn plan_context(shared: &Shared, session_id: &str, agent: &str, entries: &mut Vec<lower::Entry>) {
    let enter = plan_enter(&shared.paths.plan_dir().display().to_string());
    let user = |entry: &lower::Entry| match entry {
        lower::Entry::Turn(message) => {
            message["role"] == "user"
                && message["content"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|b| b["type"] != "tool_result"))
        }
        _ => false,
    };
    let last = entries.iter().rev().find_map(|entry| match entry {
        lower::Entry::Turn(message) if user(entry) => match message["content"].as_array() {
            Some(content) if content.len() == 1 && content[0]["type"] == "text" => {
                content[0]["text"]
                    .as_str()
                    .filter(|t| *t == enter || *t == PLAN_LEAVE)
            }
            _ => None,
        },
        _ => None,
    });
    let text = match (agent == "plan", last == Some(enter.as_str())) {
        (true, false) => enter,
        (false, true) => PLAN_LEAVE.to_string(),
        _ => return,
    };
    let at = match entries.last() {
        Some(entry) if user(entry) => entries.len() - 1,
        _ => entries.len(),
    };
    entries.insert(
        at,
        lower::Entry::Turn(json!({"role": "user", "content": [{"type": "text", "text": text}]})),
    );
    synthetic(shared, session_id, text);
}

/// The `<subagent>` message a parent receives when a child session ends: the child's last
/// assistant text, or the failure or cancellation note.
pub(crate) fn subagent_completion(
    shared: &Shared,
    child_id: &str,
    description: &str,
    outcome: &Outcome,
) -> String {
    use crate::tool::subagent::{ChildOutcome, completion_notice};
    let child = match outcome {
        Outcome::Succeeded => ChildOutcome::Completed(subagent::final_text(shared, child_id)),
        Outcome::Failed(error) if !error.message.is_empty() => {
            ChildOutcome::Failed(error.message.clone())
        }
        Outcome::Failed(_) => ChildOutcome::Failed("Subagent failed".into()),
        Outcome::Interrupted(_) => ChildOutcome::Cancelled,
    };
    completion_notice(child_id, description, &child)
}

pub(crate) async fn regenerate_title(shared: Arc<Shared>, session: SessionRow) {
    title::generate(shared, session).await;
}

pub(crate) fn is_fallback_title(session: &SessionRow) -> bool {
    session.parent_id.is_none()
        && session.title == format!("New session - {}", crate::time::iso(session.time_created))
}

/// Recovery after a process died mid-turn: running compactions failed as interrupted, unsettled
/// tool calls failed with the tool's name.
fn settle_stale(shared: &Shared, session_id: &str) {
    for row in shared.db.messages(session_id).unwrap_or_default() {
        match Message::decode(&row.kind, &row.data) {
            Message::Compaction(mut data) if data.status == "running" => {
                data.status = "failed".into();
                data.summary = None;
                data.error = Some(SessionError::new(
                    "compaction.interrupted",
                    "Compaction was interrupted",
                ));
                let _ = shared.db.update_message(&row.id, &encode(&data));
            }
            Message::Assistant(mut data) => {
                let mut changed = false;
                for part in &mut data.content {
                    let message::Part::Tool {
                        name, state, time, ..
                    } = part
                    else {
                        continue;
                    };
                    let (input, metadata) = match state {
                        message::ToolState::Streaming { input } => (
                            serde_json::from_str(input).unwrap_or(json!({})),
                            Default::default(),
                        ),
                        message::ToolState::Running { input, metadata } => {
                            (input.clone(), metadata.clone())
                        }
                        _ => continue,
                    };
                    let child = (name == "subagent")
                        .then(|| metadata.get("sessionID").and_then(|v| v.as_str()))
                        .flatten()
                        .map(|id| format!(" (sessionID: {id})"))
                        .unwrap_or_default();
                    *state = message::ToolState::Error {
                        input,
                        error: SessionError::new(
                            "aborted",
                            format!("Tool execution interrupted: {name}{child}"),
                        ),
                        content: Vec::new(),
                        metadata,
                    };
                    time.completed = Some(now_ms());
                    changed = true;
                }
                if changed {
                    let _ = shared.db.update_message(&row.id, &encode(&data));
                }
            }
            _ => {}
        }
    }
}

fn finish(shared: &Arc<Shared>, session_id: &str, outcome: &Outcome, unsent: bool) {
    goal::ended(shared, session_id, outcome);
    shared.usage.close_turn(session_id);
    let (outcome_label, event) = match outcome {
        Outcome::Succeeded => (
            "succeeded",
            Event::Succeeded {
                session_id: session_id.to_string(),
            },
        ),
        Outcome::Failed(error) => (
            "failed",
            Event::Failed {
                session_id: session_id.to_string(),
                error: error.clone(),
                unsent,
            },
        ),
        Outcome::Interrupted(reason) => (
            "interrupted",
            Event::Interrupted {
                session_id: session_id.to_string(),
                reason: *reason,
            },
        ),
    };
    let mut idle = json!({"time": {"created": now_ms()}, "outcome": outcome_label});
    match outcome {
        Outcome::Failed(error) => idle["error"] = encode(error),
        Outcome::Interrupted(reason) => idle["reason"] = json!(interrupt_label(*reason)),
        Outcome::Succeeded => {}
    }
    let _ = shared
        .db
        .append_message(session_id, &crate::ids::message(), "idle", &idle);
    let _ = shared.db.set_idle(session_id, outcome_label);
    crate::log::info(
        "session idle",
        &[("session", session_id), ("outcome", outcome_label)],
    );
    let _ = shared.events.send(event);
}

/// Why a turn was interrupted, as its `idle` row keeps it; `closed` marks a turn whose process
/// ended under it (see [`settle_interrupted`]).
fn interrupt_label(reason: InterruptReason) -> &'static str {
    match reason {
        InterruptReason::User => "user",
        InterruptReason::Budget => "budget",
        InterruptReason::Declined => "declined",
    }
}

/// Whether the stored rows end in a turn nobody finished: a prompt or a model step after the
/// last `idle` row, unless that step already ended the turn.
fn turn_unfinished(rows: &[crate::storage::MessageRow]) -> bool {
    let Some(last) = rows
        .iter()
        .rev()
        .find(|r| matches!(r.kind.as_str(), "user" | "assistant" | "idle"))
    else {
        return false;
    };
    match last.kind.as_str() {
        "user" => true,
        "assistant" => match Message::decode(&last.kind, &last.data) {
            Message::Assistant(step) => {
                step.time.completed.is_none() || step.finish.as_deref() == Some("tool-calls")
            }
            _ => false,
        },
        _ => false,
    }
}

/// The session's own rows: a fork's copied history ends wherever its source was cut, often on a
/// prompt or a step with tool calls, and is not a turn of the fork's.
fn own_rows(
    session: Option<&SessionRow>,
    mut rows: Vec<crate::storage::MessageRow>,
) -> Vec<crate::storage::MessageRow> {
    let through = session
        .filter(|s| s.fork_session_id.is_some())
        .and_then(|s| s.fork_boundary.as_ref())
        .and_then(|b| b["messageID"].as_str());
    if let Some(through) = through {
        // Copied rows are named `<source row>_<seq>`.
        let prefix = format!("{through}_");
        if let Some(at) = rows.iter().position(|r| r.id.starts_with(&prefix)) {
            rows.drain(..=at);
        }
    }
    rows
}

/// Whether the session ends in a turn nobody finished (see [`settle_interrupted`]).
pub(crate) fn interrupted(shared: &Shared, session_id: &str) -> bool {
    let session = shared.db.session(session_id).ok().flatten();
    let rows = shared.db.messages(session_id).unwrap_or_default();
    turn_unfinished(&own_rows(session.as_ref(), rows))
}

/// For a session no process drives: settles what a process that died mid-turn left running and
/// ends that turn with an `idle` row marked `closed`, so it reads as interrupted rather than
/// still working. Returns whether the session had such a turn.
pub(crate) fn settle_interrupted(shared: &Shared, session_id: &str) -> bool {
    if !interrupted(shared, session_id) {
        return false;
    }
    settle_stale(shared, session_id);
    let idle = json!({"time": {"created": now_ms()}, "outcome": "interrupted", "reason": "closed"});
    let _ = shared
        .db
        .append_message(session_id, &crate::ids::message(), "idle", &idle);
    let _ = shared.db.set_idle(session_id, "interrupted");
    true
}

fn storage(error: crate::storage::StorageError) -> SessionError {
    SessionError::new("unknown", error.to_string())
}

/// How long a turn waits for its auto-title before going idle without it.
const TITLE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Clears the input a turn that cannot start was admitted with; left in the inbox it would go
/// out unseen with whatever is sent next. Returns whether the user's prompt was dropped, which
/// the front end can then give back.
///
/// The desktop app drops only the prompt the session was taken for (and the notes steered with
/// it); any other message stays, queued, where the user sees it. The terminal drops every
/// pending prompt (not control items such as a compaction), as it always has.
fn drop_next_input(shared: &Shared, session_id: &str) -> bool {
    let rows = shared.db.inbox(session_id).unwrap_or_default();
    if crate::version::client() != "desktop" {
        let mut unsent = false;
        for row in rows.iter().filter(|r| r.kind != "compaction") {
            if matches!(shared.db.inbox_remove(&row.id), Ok(true)) {
                unsent |= row.kind == "user";
            }
        }
        return unsent;
    }
    let admitted = shared
        .admitted
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session_id);
    let steers = rows
        .iter()
        .filter(|r| r.delivery == "steer")
        .take_while(|r| r.kind != "compaction");
    let mut unsent = false;
    for row in steers {
        if row.kind != "user" {
            let _ = shared.db.inbox_remove(&row.id);
        } else if admitted.as_deref() == Some(row.id.as_str()) {
            unsent |= matches!(shared.db.inbox_remove(&row.id), Ok(true));
        } else {
            let _ = shared
                .db
                .inbox_set_delivery(&row.id, inbox::Delivery::Queue.as_str());
        }
    }
    unsent
}

/// One turn: settle the context (instructions are recorded before the input that follows them),
/// promote pending input, then run steps until the model stops.
async fn turn(
    shared: &Arc<Shared>,
    session: &SessionRow,
    options: TurnOptions,
    cancel: &CancellationToken,
    unsent: &mut bool,
) -> Outcome {
    shared.refresh().await;
    let prepare = |shared: &Arc<Shared>| {
        select(shared, session).and_then(|selection| {
            plan_reminders(shared, &session.id, &selection.agent.id);
            step::settle_context(shared, session, &selection)
        })
    };
    let mut prepared = prepare(shared);
    if let Err(error) = &prepared
        && error.kind == "provider.no-route"
        && shared.remote().body.is_none()
    {
        // No remote document yet (the first fetch failed): try the gateway again right now
        // rather than failing every prompt until the periodic refresh.
        shared.refresh_remote().await;
        shared.refresh().await;
        prepared = prepare(shared);
    }
    if let Err(error) = &mut prepared
        && error.kind == "provider.no-route"
        && let Some(reason) = no_models(shared)
    {
        error.message = reason;
    }
    if let Err(error) = prepared {
        *unsent = drop_next_input(shared, &session.id);
        return Outcome::Failed(error);
    }
    let promoted = match inbox::promote(&shared.db, &session.id, inbox::Scope::Input) {
        Ok(Some(promoted)) => promoted,
        Ok(None) => return Outcome::Succeeded,
        Err(error) => return Outcome::Failed(storage(error)),
    };
    let title = (promoted > 0 && is_fallback_title(session))
        .then(|| tokio::spawn(title::generate(shared.clone(), session.clone())));
    let outcome = step::drive(shared, session, options, cancel).await;
    if let Some(title) = title {
        // The turn waits briefly for the title (a normal call lands within a few seconds); a
        // slow or stalled one finishes on its own and must not keep the session busy, and an
        // interrupted turn does not wait at all.
        tokio::select! {
            _ = title => {}
            _ = tokio::time::sleep(TITLE_WAIT) => {}
            _ = cancel.cancelled() => {}
        }
    }
    outcome
}

/// Why no model is available at all: no credential, or no remote document (none cached) because
/// the gateway refused the token or did not answer. `None` when the models were loaded.
pub(crate) fn no_models(shared: &Shared) -> Option<String> {
    if shared.gateway.token().is_none() {
        return Some(NOT_SIGNED_IN.into());
    }
    let remote = shared.remote();
    if remote.body.is_some() {
        return None;
    }
    Some(if remote.needs_auth {
        crate::gateway::config::SIGNED_OUT.into()
    } else {
        format!("Cannot reach {}", shared.gateway.base())
    })
}

/// The turn error without a credential (`anysola run`, ACP, the TUI).
pub const NOT_SIGNED_IN: &str = "Not signed in. Run `anysola login` or set ANYSOLA_TOKEN.";

/// Runs the session until its inbox is empty: a manual compaction or a turn per pending item,
/// queued prompts one at a time. Stops at the first turn that does not succeed. A turn that
/// leaves nothing pending stays open for the idle hook (`/goal`): input it admits continues the
/// same execution.
pub(crate) async fn drain(
    shared: Arc<Shared>,
    session_id: &str,
    options: TurnOptions,
    cancel: CancellationToken,
) -> Outcome {
    settle_stale(&shared, session_id);
    goal::started(&shared, session_id);
    let _ = shared.events.send(Event::Started {
        session_id: session_id.to_string(),
    });
    let mut outcome = Outcome::Succeeded;
    let mut open = false;
    let mut unsent = false;
    loop {
        let session = match shared.db.session(session_id) {
            Ok(Some(session)) => session,
            Ok(None) => return Outcome::Failed(SessionError::new("unknown", "Session not found")),
            Err(error) => return Outcome::Failed(storage(error)),
        };
        let next = match inbox::next(&shared.db, session_id, inbox::Scope::Input) {
            Ok(next) => next,
            Err(error) => return Outcome::Failed(storage(error)),
        };
        let Some(next) = next else {
            if open {
                if goal::idle(&shared, session_id).await {
                    continue;
                }
                finish(&shared, session_id, &outcome, false);
            }
            return outcome;
        };
        if next.kind == "compaction" {
            let _ = shared.db.inbox_remove(&next.id);
            outcome = match manual_compaction(&shared, &session, &cancel).await {
                compaction::Outcome::Failed(error) => Outcome::Failed(error),
                _ => Outcome::Succeeded,
            };
            if open && outcome != Outcome::Succeeded {
                finish(&shared, session_id, &outcome, false);
            }
        } else {
            outcome = turn(&shared, &session, options, &cancel, &mut unsent).await;
            open = outcome == Outcome::Succeeded
                && matches!(
                    inbox::next(&shared.db, session_id, inbox::Scope::Input),
                    Ok(None)
                );
            if !open {
                finish(&shared, session_id, &outcome, unsent);
            }
        }
        if outcome != Outcome::Succeeded {
            return outcome;
        }
    }
}

async fn manual_compaction(
    shared: &Arc<Shared>,
    session: &SessionRow,
    cancel: &CancellationToken,
) -> compaction::Outcome {
    match select(shared, session) {
        Ok(selection) => step::compact_manually(shared, session, &selection, cancel).await,
        Err(error) => compaction::Outcome::Failed(error),
    }
}
