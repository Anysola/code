//! Running slash commands through the core API.

use super::{Core, CoreError, Prompt};
use crate::command::{Command, Commands};
use crate::model::ModelRef;
use crate::session::inbox::Delivery;
use crate::session::{Outcome, TurnOptions};

impl Core {
    pub fn commands(&self) -> Commands {
        self.shared.loaded().commands.clone()
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
        let command = loaded
            .commands
            .get(name)
            .cloned()
            .ok_or_else(|| CoreError::CommandNotFound(name.to_string()))?;
        let session = self
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        let text = match crate::command::render(
            &command,
            arguments,
            &self.shared.project.worktree,
            loaded.config.shell(),
            std::path::Path::new(&session.directory),
            &self.shared.env,
        )
        .await
        {
            Ok(text) => text,
            Err(message) => {
                return Ok(Outcome::Failed(crate::gateway::error::SessionError::new(
                    "unknown", message,
                )));
            }
        };
        let agent = command.agent.as_deref().and_then(|a| loaded.agents.get(a));
        let model = command
            .model
            .as_deref()
            .or_else(|| agent.and_then(|a| a.model.as_deref()))
            .and_then(ModelRef::parse);
        if command.subagent || agent.is_some_and(|a| a.mode == crate::agent::Mode::Subagent) {
            return self
                .subagent_command(&session, &command, model, text, options)
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
        };
        self.prompt(session_id, prompt, options).await
    }

    async fn subagent_command(
        &self,
        parent: &crate::storage::SessionRow,
        command: &Command,
        model: Option<ModelRef>,
        text: String,
        options: TurnOptions,
    ) -> Result<Outcome, CoreError> {
        let description = command
            .description
            .clone()
            .unwrap_or_else(|| command.name.clone());
        let child = self.create_child(&parent.id, Some(description.clone()))?;
        let loaded = self.shared.loaded();
        let agent = command
            .agent
            .clone()
            .or_else(|| parent.agent.clone())
            .or_else(|| loaded.agents.default_agent().map(|a| a.id.clone()));
        if let Some(agent) = &agent {
            self.select_agent(&child.id, agent)?;
        }
        let model = model
            .or_else(|| {
                agent
                    .as_deref()
                    .and_then(|a| loaded.agents.get(a))
                    .and_then(|a| a.model.as_deref())
                    .and_then(ModelRef::parse)
            })
            .or_else(|| parent.model.as_ref().and_then(ModelRef::from_json));
        if let Some(model) = &model {
            self.select_model(&child.id, model)?;
        }
        let prompt = Prompt {
            text: format!("{}\n{text}", crate::tool::subagent::FIRST_PROMPT),
            files: Vec::new(),
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
            &description,
            Delivery::Steer,
        )?;
        self.drain(&parent.id, options).await
    }
}
