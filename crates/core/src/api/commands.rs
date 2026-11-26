//! Running slash commands through the core API.

use super::{Core, CoreError, Prompt};
use crate::command::{Command, Commands, Template};
use crate::gateway::error::SessionError;
use crate::model::ModelRef;
use crate::session::goal::{self, Goal};
use crate::session::inbox::Delivery;
use crate::session::{Outcome, TurnOptions};

impl Core {
    /// Built-in, markdown and config commands, then the MCP prompts.
    pub fn commands(&self) -> Commands {
        let commands = self.shared.loaded().commands.clone();
        match self.shared.mcp.get() {
            Some(mcp) => commands.with_prompts(mcp.prompts()),
            None => commands,
        }
    }

    /// Runs `/<name> <arguments>`: switches to the command's agent and model, then prompts the
    /// session with the rendered template. A subagent command runs in a child session whose
    /// result comes back to this one as a `<subagent>` message.
    pub async fn command(
        &self,
        session_id: &str,
        name: &str,
        arguments: &str,
        options: TurnOptions,
    ) -> Result<Outcome, CoreError> {
        let loaded = self.shared.loaded();
        let command = self
            .commands()
            .get(name)
            .cloned()
            .ok_or_else(|| CoreError::CommandNotFound(name.to_string()))?;
        let session = self
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        let mcp = self.shared.mcp.get();
        let agent = command.agent.as_deref().and_then(|a| loaded.agents.get(a));
        // A project's command never moves the user's session to another agent: it runs in a
        // child session, which its parent's rules bound.
        let current = session
            .agent
            .clone()
            .or_else(|| loaded.agents.default_agent().map(|a| a.id.clone()));
        let switches = command
            .agent
            .as_deref()
            .is_some_and(|a| current.as_deref() != Some(a));
        let subtask = command.subagent
            || agent.is_some_and(|a| a.mode == crate::agent::Mode::Subagent)
            || (command.project && switches);
        // A project's ``!`cmd` `` runs only as the session it runs in may run shell commands.
        let gate = if command.project && matches!(command.template, Template::User(_)) {
            let child = subtask
                .then(|| command.agent.clone().or_else(|| current.clone()))
                .flatten();
            let floor = crate::session::gate::Floor::of(&self.shared.db, &session.id);
            match crate::schedule::check::context(&self.shared, &session, floor, child.as_deref()) {
                Ok(ctx) => Some(ctx),
                Err(message) => return Ok(Outcome::Failed(SessionError::new("unknown", message))),
            }
        } else {
            None
        };
        let checkout = self.shared.checkout_of(&session);
        let sandbox =
            crate::tool::shell::sandbox::SandboxConfig::from_value(loaded.config.get("sandbox"));
        let sandbox_cache = self.shared.paths.sandbox_cache_dir(&session.id);
        let at = crate::command::RenderEnv {
            project: &checkout.root,
            shell: loaded.config.shell(),
            cwd: std::path::Path::new(&session.directory),
            env: &self.shared.env,
            mcp: mcp.map(|m| m.as_ref()),
            gate: gate.as_ref(),
            sandbox,
            sandbox_cache: sandbox_cache.as_deref(),
        };
        // While a question it raised waits, Stop (or closing, or deleting the session) cancels
        // the whole command: the question is taken down and nothing of it runs.
        let preparing = gate
            .is_some()
            .then(|| Preparing::start(&self.shared, session_id));
        let rendered = match &preparing {
            Some(preparing) => {
                let cancelled = preparing.token.clone();
                let text = tokio::select! {
                    biased;
                    _ = cancelled.cancelled() => None,
                    text = crate::command::render(&command, arguments, &at) => Some(text),
                };
                // Closing rejects the question as it cancels: never run on that answer.
                text.filter(|_| !cancelled.is_cancelled())
            }
            None => Some(crate::command::render(&command, arguments, &at).await),
        };
        drop(preparing);
        let text = match rendered {
            None => return Ok(Outcome::Interrupted(crate::events::InterruptReason::User)),
            Some(Ok(text)) => text,
            Some(Err(message)) => {
                return Ok(Outcome::Failed(SessionError::new("unknown", message)));
            }
        };
        if gate.is_some() && self.session(session_id)?.is_none() {
            return Err(CoreError::SessionNotFound);
        }
        if command.template == Template::Goal {
            return self.goal_command(session_id, &text, options).await;
        }
        let model = command
            .model
            .as_deref()
            .or_else(|| agent.and_then(|a| a.model.as_deref()))
            .and_then(ModelRef::parse);
        let line = match arguments.trim() {
            "" => format!("/{name}"),
            arguments => format!("/{name} {arguments}"),
        };
        if subtask {
            return self
                .subagent_command(&session, &command, &line, model, text, options)
                .await;
        }
        if let Some(agent) = &command.agent
            && session.agent.as_deref() != Some(agent)
        {
            self.select_agent(session_id, agent)?;
        }
        if let Some(model) = &model {
            self.select_model(session_id, model)?;
        }
        let prompt = Prompt {
            text,
            files: Vec::new(),
            skills: Vec::new(),
            command: Some(line),
        };
        self.prompt(session_id, prompt, options).await
    }

    /// The session's `/goal`, if one is set.
    pub fn goal(&self, session_id: &str) -> Option<Goal> {
        goal::read(&self.shared.db, &self.shared.events, session_id)
    }

    /// `/goal <objective> | pause | resume | clear`; usage errors come back as a failed outcome
    /// with the text to show.
    async fn goal_command(
        &self,
        session_id: &str,
        arguments: &str,
        options: TurnOptions,
    ) -> Result<Outcome, CoreError> {
        match goal::command(&self.shared, session_id, arguments) {
            Err(text) => Ok(Outcome::Failed(SessionError::new("unknown", text))),
            Ok(goal::Command::Done) => Ok(Outcome::Succeeded),
            Ok(goal::Command::Run {
                text,
                description,
                metadata,
            }) => {
                super::revert::commit(&self.shared, session_id)?;
                crate::session::inbox::enqueue_synthetic(
                    &self.shared.db,
                    session_id,
                    &text,
                    description.as_deref(),
                    Some(&metadata),
                    Delivery::Steer,
                )?;
                self.drain(session_id, options).await
            }
        }
    }

    async fn subagent_command(
        &self,
        parent: &crate::storage::SessionRow,
        command: &Command,
        line: &str,
        model: Option<ModelRef>,
        text: String,
        options: TurnOptions,
    ) -> Result<Outcome, CoreError> {
        // The parent shows what the user typed, as a command run in it would: a user message
        // holding only the command line (the template goes to the child), ahead of the result.
        super::revert::commit(&self.shared, &parent.id)?;
        let typed = Prompt {
            text: line.to_string(),
            files: Vec::new(),
            skills: Vec::new(),
            command: Some(line.to_string()),
        };
        self.shared.db.append_message(
            &parent.id,
            &crate::ids::message(),
            "user",
            &crate::session::inbox::user_data(&typed),
        )?;
        let description = command
            .description
            .clone()
            .unwrap_or_else(|| command.name.clone());
        let child = self.create_child(&parent.id, Some(description.clone()))?;
        let agent = command
            .agent
            .clone()
            .or_else(|| parent.agent.clone())
            .or_else(|| {
                let loaded = self.shared.loaded();
                loaded.agents.default_agent().map(|a| a.id.clone())
            });
        if let Some(agent) = &agent {
            self.select_agent(&child.id, agent)?;
        }
        let model = model
            .or_else(|| {
                agent.as_deref().and_then(|a| {
                    let loaded = self.shared.loaded();
                    loaded
                        .agents
                        .get(a)
                        .and_then(|a| a.model.as_deref())
                        .and_then(ModelRef::parse)
                })
            })
            .or_else(|| parent.model.as_ref().and_then(ModelRef::from_json));
        if let Some(model) = &model {
            self.select_model(&child.id, model)?;
        }
        let prompt = Prompt {
            text: format!("{}\n{text}", crate::tool::subagent::FIRST_PROMPT),
            files: Vec::new(),
            skills: Vec::new(),
            command: None,
        };
        let outcome = self
            .prompt(&child.id, prompt, TurnOptions::default())
            .await?;
        let completion =
            crate::session::subagent_completion(&self.shared, &child.id, &description, &outcome);
        crate::session::inbox::enqueue_synthetic(
            &self.shared.db,
            &parent.id,
            &completion,
            Some(&description),
            None,
            Delivery::Steer,
        )?;
        self.drain(&parent.id, options).await
    }
}

/// A command of `session` waiting before its turn, registered for [`Core::interrupt`] until
/// dropped.
struct Preparing<'a> {
    shared: &'a super::Shared,
    session_id: String,
    nonce: String,
    token: tokio_util::sync::CancellationToken,
}

impl<'a> Preparing<'a> {
    fn start(shared: &'a super::Shared, session_id: &str) -> Self {
        let token = tokio_util::sync::CancellationToken::new();
        let nonce = crate::ids::part();
        shared
            .preparing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(session_id.to_string())
            .or_default()
            .push((nonce.clone(), token.clone()));
        Preparing {
            shared,
            session_id: session_id.to_string(),
            nonce,
            token,
        }
    }
}

impl Drop for Preparing<'_> {
    fn drop(&mut self) {
        let mut preparing = self
            .shared
            .preparing
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(waiting) = preparing.get_mut(&self.session_id) {
            waiting.retain(|(nonce, _)| *nonce != self.nonce);
            if waiting.is_empty() {
                preparing.remove(&self.session_id);
            }
        }
    }
}
