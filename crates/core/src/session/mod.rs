//! The agent loop: a turn is a series of steps, each one model request plus the tool calls it
//! makes. Tools start as soon as their `tool_use` block completes and run while the stream goes on.

pub mod gate;
mod host;
pub mod lower;
pub mod message;
pub mod question;
pub mod request;
mod step;
mod stream;
pub mod subagent;
mod title;

pub(crate) use step::session_headers as step_headers;

use std::sync::Arc;

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::agent::Agent;
use crate::api::{Prompt, Shared};
use crate::events::{Event, InterruptReason};
use crate::gateway::error::SessionError;
use crate::model::{Model, ModelRef};
use crate::storage::SessionRow;
use crate::time::now_ms;
use message::{Created, FileAttachment, Message, SyntheticData, UserData, encode};

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

/// The agent, model and variant a turn runs with.
pub(crate) struct Selection {
    pub agent: Agent,
    pub model: Model,
    pub reference: ModelRef,
}

fn select(shared: &Shared, session: &SessionRow) -> Result<Selection, SessionError> {
    let agent = match session.agent.as_deref() {
        Some(id) => shared.agents.get(id).cloned(),
        None => shared.agents.default_agent().cloned(),
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
        .or_else(|| shared.config.model().and_then(ModelRef::parse))
        .ok_or_else(no_route)?;
    let model = crate::model::resolve(&shared.models, &reference)
        .map_err(|error| SessionError::new("provider.no-route", error.to_string()))?;
    Ok(Selection {
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

fn is_fallback_title(session: &SessionRow) -> bool {
    session.parent_id.is_none()
        && session.title == format!("New session - {}", crate::time::iso(session.time_created))
}

pub(crate) async fn run_turn(
    shared: Arc<Shared>,
    session: SessionRow,
    prompt: Prompt,
    options: TurnOptions,
    cancel: CancellationToken,
) -> Outcome {
    let session_id = session.id.clone();
    let selection = select(&shared, &session);
    if let Ok(selection) = &selection {
        plan_reminders(&shared, &session_id, &selection.agent.id);
    }
    let files = prompt
        .files
        .iter()
        .map(|(mime, data, name)| FileAttachment {
            data: data.clone(),
            mime: mime.clone(),
            source: json!({"type": "inline"}),
            name: Some(name.clone()),
        })
        .collect();
    let user = UserData {
        time: Created { created: now_ms() },
        text: prompt.text.clone(),
        files,
    };
    let _ = shared
        .db
        .append_message(&session_id, &crate::ids::message(), "user", &encode(&user));

    let title = (is_fallback_title(&session) && selection.is_ok()).then(|| {
        tokio::spawn(title::generate(
            shared.clone(),
            session.clone(),
            prompt.text.clone(),
        ))
    });

    let outcome = match selection {
        Ok(selection) => step::drive(&shared, &session, &selection, options, &cancel).await,
        Err(error) => Outcome::Failed(error),
    };
    if let Some(title) = title {
        let _ = title.await;
    }
    shared.credits.close_turn(&session_id);
    let (outcome_label, event) = match &outcome {
        Outcome::Succeeded => (
            "succeeded",
            Event::Succeeded {
                session_id: session_id.clone(),
            },
        ),
        Outcome::Failed(error) => (
            "failed",
            Event::Failed {
                session_id: session_id.clone(),
                error: error.clone(),
            },
        ),
        Outcome::Interrupted(reason) => (
            "interrupted",
            Event::Interrupted {
                session_id: session_id.clone(),
                reason: *reason,
            },
        ),
    };
    let mut idle = json!({"time": {"created": now_ms()}, "outcome": outcome_label});
    if let Outcome::Failed(error) = &outcome {
        idle["error"] = encode(error);
    }
    let _ = shared
        .db
        .append_message(&session_id, &crate::ids::message(), "idle", &idle);
    let _ = shared.db.set_idle(&session_id, outcome_label);
    crate::log::info(
        "session idle",
        &[("session", &session_id), ("outcome", outcome_label)],
    );
    let _ = shared.events.send(event);
    outcome
}
