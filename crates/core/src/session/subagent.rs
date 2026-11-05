//! Child sessions for the `subagent` tool: creation, the depth limit, model overrides, foreground
//! and background runs, continuing a direct child, and interrupt propagation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::message::{Created, Message, Part, SyntheticData, encode};
use super::{Outcome, TurnOptions};
use crate::agent::Mode;
use crate::api::{CoreError, Prompt, Shared};
use crate::model::ModelRef;
use crate::permission::PermissionRequest;
use crate::storage::SessionRow;
use crate::tool::subagent::{
    ChildOutcome, FIRST_PROMPT, NO_TEXT, SubagentRequest, background_text, completed_text,
    completion_notice,
};
use crate::tool::{ToolContext, ToolError, ToolOutput, ToolResult};

type Result<T> = std::result::Result<T, ToolError>;

/// Child runs of this process: each child id maps to the outcome of its current run, `None` while
/// it runs.
#[derive(Default)]
pub struct Jobs {
    running: Mutex<HashMap<String, watch::Receiver<Option<ChildOutcome>>>>,
}

impl Jobs {
    fn current(&self, child: &str) -> Option<watch::Receiver<Option<ChildOutcome>>> {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(child)
            .cloned()
    }
}

/// Whether a child of the session runs in this process (a background subagent at idle).
pub(crate) fn has_running_child(shared: &Shared, session_id: &str) -> bool {
    let running: Vec<String> = shared
        .jobs
        .running
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect();
    running.iter().any(|child| {
        session(shared, child).is_some_and(|c| c.parent_id.as_deref() == Some(session_id))
    })
}

/// Cancels the child's run when the parent's tool call goes away (interrupted or dropped).
struct CancelOnDrop(Option<CancellationToken>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            token.cancel();
        }
    }
}

fn execution(message: String) -> ToolError {
    ToolError::execution(message)
}

fn session(shared: &Shared, id: &str) -> Option<SessionRow> {
    shared.db.session(id).ok().flatten()
}

fn depth(shared: &Shared, parent: &SessionRow) -> Result<u64> {
    let mut depth = 0;
    let mut current = parent.clone();
    while let Some(id) = current.parent_id.clone() {
        depth += 1;
        current = session(shared, &id)
            .ok_or_else(|| execution(format!("Parent session not found: {id}")))?;
    }
    Ok(depth)
}

fn resolve_model(shared: &Shared, input: &str) -> Result<ModelRef> {
    let reference = ModelRef::parse(input).ok_or_else(|| {
        execution(format!(
            "Invalid model \"{input}\". Use \"providerID/modelID\" or \"providerID/modelID#variant\"."
        ))
    })?;
    let label = reference.label();
    let loaded = shared.loaded();
    let model = loaded
        .models
        .iter()
        .find(|m| reference.provider == crate::model::PROVIDER && m.id == reference.id)
        .ok_or_else(|| {
            execution(format!(
                "Model \"{label}\" is not available. Use the models tool to see what is available."
            ))
        })?;
    if let Some(variant) = &reference.variant
        && model.variant(variant).is_none()
    {
        return Err(execution(if model.variants.is_empty() {
            format!("Model \"{label}\" has no variants. Omit the variant.")
        } else {
            let available: Vec<&str> = model.variants.iter().map(|v| v.id.as_str()).collect();
            format!(
                "Variant \"{variant}\" is not available for \"{label}\". Available: {}.",
                available.join(", ")
            )
        }));
    }
    Ok(reference)
}

/// Text parts of the child's last completed, error-free assistant message.
pub(crate) fn final_text(shared: &Shared, child: &str) -> String {
    let rows = shared.db.messages(child).unwrap_or_default();
    let text = rows
        .iter()
        .rev()
        .find_map(|row| match Message::decode(&row.kind, &row.data) {
            Message::Assistant(a) if a.time.completed.is_some() && a.error.is_none() => Some(
                a.content
                    .iter()
                    .filter_map(|p| match p {
                        Part::Text { text, .. } if !text.is_empty() => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            _ => None,
        });
    text.filter(|t| !t.is_empty())
        .unwrap_or_else(|| NO_TEXT.to_string())
}

fn create_child(
    shared: &Shared,
    parent: &SessionRow,
    title: &str,
    agent: &str,
    model: Option<&ModelRef>,
) -> Result<SessionRow> {
    let now = crate::time::now_ms();
    let row = SessionRow {
        id: crate::ids::session(),
        project_id: parent.project_id.clone(),
        parent_id: Some(parent.id.clone()),
        fork_session_id: None,
        fork_boundary: None,
        slug: crate::slug::create(),
        directory: parent.directory.clone(),
        title: title.to_string(),
        version: crate::version::session_version(),
        agent: Some(agent.to_string()),
        model: model.map(ModelRef::to_json),
        time_created: now,
        time_updated: now,
    };
    let failed = || execution(format!("Parent session not found: {}", parent.id));
    shared.db.create_session(&row).map_err(|_| failed())?;
    shared.db.set_agent(&row.id, agent).map_err(|_| failed())?;
    if let Some(model) = &row.model {
        shared.db.set_model(&row.id, model).map_err(|_| failed())?;
    }
    Ok(row)
}

/// Starts a turn of the child with `text`, after any run of it still going in this process.
fn start(
    shared: &Arc<Shared>,
    child: &str,
    text: String,
    cancel: CancellationToken,
) -> watch::Receiver<Option<ChildOutcome>> {
    let previous = shared.jobs.current(child);
    let (sender, receiver) = watch::channel(None);
    shared
        .jobs
        .running
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(child.to_string(), receiver.clone());
    let shared = shared.clone();
    let child = child.to_string();
    tokio::spawn(async move {
        if let Some(mut previous) = previous {
            let _ = previous.wait_for(Option::is_some).await;
        }
        let prompt = Prompt {
            text,
            files: Vec::new(),
            skills: Vec::new(),
        };
        let outcome =
            crate::api::prompt_session(&shared, &child, prompt, TurnOptions::default(), cancel)
                .await;
        let outcome = match outcome {
            Ok(Outcome::Succeeded) => ChildOutcome::Completed(final_text(&shared, &child)),
            Ok(Outcome::Failed(error)) => ChildOutcome::Failed(error.message),
            Ok(Outcome::Interrupted(_)) => ChildOutcome::Cancelled,
            Err(CoreError::Busy(_)) => {
                ChildOutcome::Failed(format!("Failed to prompt subagent: {child}"))
            }
            Err(error) => ChildOutcome::Failed(error.to_string()),
        };
        let mut running = shared
            .jobs
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if running
            .get(&child)
            .is_some_and(|current| current.same_channel(&sender.subscribe()))
        {
            running.remove(&child);
        }
        drop(running);
        let _ = sender.send(Some(outcome));
    });
    receiver
}

fn notify_parent(
    shared: Arc<Shared>,
    parent: String,
    child: String,
    description: String,
    mut outcome: watch::Receiver<Option<ChildOutcome>>,
) {
    tokio::spawn(async move {
        let Ok(result) = outcome.wait_for(Option::is_some).await else {
            return;
        };
        let Some(result) = result.clone() else {
            return;
        };
        let data = encode(&SyntheticData {
            time: Created {
                created: crate::time::now_ms(),
            },
            text: completion_notice(&child, &description, &result),
            description: Some(description),
            extra: Default::default(),
        });
        let _ = shared
            .db
            .append_message(&parent, &crate::ids::message(), "synthetic", &data);
    });
}

pub(crate) async fn run(
    shared: Arc<Shared>,
    ctx: &ToolContext,
    request: SubagentRequest,
) -> ToolResult {
    let parent = session(&shared, &ctx.session_id)
        .ok_or_else(|| execution(format!("Parent session not found: {}", ctx.session_id)))?;
    let loaded = shared.loaded();
    let limit = loaded
        .config
        .get("experimental")
        .and_then(|e| e.get("subagent_depth"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    if depth(&shared, &parent)? >= limit {
        return Err(execution(format!(
            "Subagent depth limit reached ({limit}). Increase \"experimental.subagent_depth\" to allow nested subagents."
        )));
    }
    let agent = loaded
        .agents
        .get(&request.agent)
        .cloned()
        .ok_or_else(|| execution(format!("Unknown agent: {}", request.agent)))?;
    if agent.mode == Mode::Primary {
        return Err(execution(format!(
            "Agent {} cannot run as a subagent",
            request.agent
        )));
    }
    ctx.ask(PermissionRequest {
        action: "subagent".into(),
        resources: vec![agent.id.clone()],
        save: vec![agent.id.clone()],
        metadata: json!({"agent": agent.id, "description": request.description}),
    })
    .await?;
    let existing = match &request.session_id {
        Some(id) => Some(
            session(&shared, id)
                .ok_or_else(|| execution(format!("Subagent session not found: {id}")))?,
        ),
        None => None,
    };
    if let Some(existing) = &existing
        && existing.parent_id.as_deref() != Some(ctx.session_id.as_str())
    {
        return Err(execution(format!(
            "Session {} is not a child of the current session",
            existing.id
        )));
    }
    let override_model = request
        .model
        .as_deref()
        .map(|m| resolve_model(&shared, m))
        .transpose()?;
    let agent_model = agent.model.as_deref().and_then(ModelRef::parse);
    let child = match existing {
        Some(existing) => {
            let switched = existing.agent.as_deref() != Some(agent.id.as_str());
            let switch_failed = || {
                execution(format!(
                    "Failed to switch subagent session: {}",
                    existing.id
                ))
            };
            if switched {
                shared
                    .db
                    .set_agent(&existing.id, &agent.id)
                    .map_err(|_| switch_failed())?;
            }
            if let Some(model) = override_model
                .as_ref()
                .or(agent_model.as_ref().filter(|_| switched))
            {
                shared
                    .db
                    .set_model(&existing.id, &model.to_json())
                    .map_err(|_| switch_failed())?;
            }
            existing.id
        }
        None => {
            let model = override_model
                .or(agent_model)
                .or_else(|| parent.model.as_ref().and_then(ModelRef::from_json));
            create_child(
                &shared,
                &parent,
                &request.description,
                &agent.id,
                model.as_ref(),
            )?
            .id
        }
    };
    let text = match request.session_id {
        None => format!("{FIRST_PROMPT}\n{}", request.prompt),
        Some(_) => request.prompt,
    };
    let cancel = CancellationToken::new();
    if request.background {
        let outcome = start(&shared, &child, text, cancel);
        notify_parent(
            shared.clone(),
            ctx.session_id.clone(),
            child.clone(),
            request.description,
            outcome,
        );
        return Ok(ToolOutput::text(background_text(&child))
            .with_metadata("sessionID", child.clone())
            .with_metadata("status", "running"));
    }
    let mut guard = CancelOnDrop(Some(cancel.clone()));
    let mut outcome = start(&shared, &child, text, cancel);
    let result = outcome
        .wait_for(Option::is_some)
        .await
        .ok()
        .and_then(|r| r.clone());
    guard.0 = None;
    match result {
        Some(ChildOutcome::Completed(text)) => Ok(ToolOutput::text(completed_text(&child, &text))
            .with_metadata("sessionID", child.clone())
            .with_metadata("status", "completed")),
        Some(ChildOutcome::Failed(error)) => Err(execution(format!(
            "Subagent failed (sessionID: {child}): {error}"
        ))),
        Some(ChildOutcome::Cancelled) | None => Err(execution(format!(
            "Subagent cancelled (sessionID: {child})"
        ))),
    }
}
