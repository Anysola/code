//! What the interactive UI reads besides prompting: the directory and project, recent sessions,
//! a session's decoded messages, account routes, and `!` shell commands.

use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{Core, CoreError};
use crate::gateway::account::{Method, Reply};
use crate::session::message::{AssistantTime, Message, ShellData, ShellOutput};
use crate::storage::SessionSummary;
use crate::time::now_ms;

/// A hung `!` command must not hold the composer forever.
const SHELL_TIMEOUT: Duration = Duration::from_secs(120);

impl Core {
    pub fn directory(&self) -> &Path {
        &self.shared.directory
    }

    /// The latest root sessions of this project, newest first.
    pub fn recent_sessions(&self, limit: u64) -> Result<Vec<SessionSummary>, CoreError> {
        Ok(self
            .shared
            .db
            .root_sessions(&self.shared.project.id, limit)?)
    }

    /// Top-level sessions newest first, of this project or of all, whose title contains `query`.
    pub fn sessions(
        &self,
        all: bool,
        query: &str,
        limit: u64,
    ) -> Result<Vec<SessionSummary>, CoreError> {
        let project = (!all).then_some(self.shared.project.id.as_str());
        Ok(self.shared.db.search_sessions(project, query, limit)?)
    }

    /// Deletes a session and its children; false when it did not exist.
    pub fn delete_session(&self, session_id: &str) -> Result<bool, CoreError> {
        Ok(self.shared.db.delete_session(session_id)?)
    }

    /// `{info, messages}` as `anysola session export` writes it.
    pub fn export(&self, session_id: &str) -> Result<Option<Value>, CoreError> {
        Ok(self.shared.db.export_session(session_id)?)
    }

    pub fn default_budget(&self) -> Option<u64> {
        self.shared.credits.default_budget()
    }

    pub fn set_default_budget(&self, budget: Option<u64>) -> Result<(), CoreError> {
        Ok(self.shared.credits.set_default_budget(budget)?)
    }

    /// Admits a hidden note (editor context) that goes before the next prompt; it does not
    /// start a turn by itself.
    pub fn admit_note(&self, session_id: &str, text: &str) -> Result<(), CoreError> {
        crate::session::inbox::enqueue_synthetic(
            &self.shared.db,
            session_id,
            text,
            None,
            None,
            crate::session::inbox::Delivery::Steer,
        )?;
        Ok(())
    }

    /// `/btw`: a one-shot answer from the session's context; nothing is added to the session.
    pub async fn aside(&self, session_id: &str, question: &str) -> Result<String, String> {
        const INSTRUCTIONS: &str = "The user is asking a quick side question about the conversation so far. Answer directly and concisely in markdown from what you already know. Do not call any tools and do not take any actions.";
        let session = self
            .session(session_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| CoreError::SessionNotFound.to_string())?;
        let prompt = format!("{INSTRUCTIONS}\n\n{question}");
        match crate::session::step::generate(&self.shared, &session, &prompt).await {
            Ok(text) => Ok(text.trim().to_string()),
            Err(crate::session::step::GenerateError::Overflow) => {
                Err("The conversation is too long for a side question.".into())
            }
            Err(crate::session::step::GenerateError::Failed(error)) => Err(error.message),
        }
    }

    /// The project's other checkouts.
    pub fn worktrees(&self) -> Vec<crate::worktree::Worktree> {
        crate::worktree::list(&self.shared.db, &self.shared.project.id)
    }

    /// Adds a detached worktree of the project named `name` (suffixed when taken).
    pub async fn create_worktree(&self, name: &str) -> Result<std::path::PathBuf, String> {
        let shared = &self.shared;
        let base = crate::worktree::base(
            &self.loaded().config,
            &shared.paths,
            &shared.project.worktree,
            &shared.project.id,
        );
        crate::worktree::create(
            &shared.db,
            &base,
            &shared.project.id,
            &shared.project.worktree,
            name,
        )
        .await
    }

    /// Removes a worktree this project created; `force` drops its file changes.
    pub async fn remove_worktree(
        &self,
        directory: &Path,
        force: bool,
    ) -> Result<(), crate::worktree::RemoveError> {
        crate::worktree::remove(&self.shared.db, &self.shared.project.id, directory, force).await
    }

    /// The session's stored messages, decoded, oldest first.
    pub fn messages(&self, session_id: &str) -> Result<Vec<(String, Message)>, CoreError> {
        Ok(self
            .shared
            .db
            .messages(session_id)?
            .into_iter()
            .map(|row| {
                let message = Message::decode(&row.kind, &row.data);
                (row.id, message)
            })
            .collect())
    }

    /// The staged revert boundary (`messageID`), if any.
    pub fn revert_boundary(&self, session_id: &str) -> Result<Option<String>, CoreError> {
        Ok(self.shared.db.revert(session_id)?.and_then(|r| {
            r.get("messageID")
                .and_then(Value::as_str)
                .map(str::to_string)
        }))
    }

    /// The staged revert's file changes (`file`, `status`, `additions`, `deletions`).
    pub fn revert_files(&self, session_id: &str) -> Result<Vec<Value>, CoreError> {
        Ok(self
            .shared
            .db
            .revert(session_id)?
            .and_then(|r| r.get("files").and_then(Value::as_array).cloned())
            .unwrap_or_default())
    }

    pub fn signed_in(&self) -> bool {
        self.shared.gateway.token().is_some()
    }

    /// An account route (`/me`, `/usage?days=7`, …); `Err` is the transport failure text.
    pub async fn account(&self, method: Method, route: &str) -> Result<Reply, String> {
        self.shared.gateway.account_call(method, route, None).await
    }

    /// Runs a `!` command in the session directory and stores it as a `shell` message: its
    /// combined output, and `exit` (or `timedOut`) in the metadata.
    pub async fn run_shell(&self, session_id: &str, command: &str) -> Result<(), CoreError> {
        let session = self
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        let configured = self
            .loaded()
            .config
            .get("shell")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (program, args) = crate::tool::shell::invocation(configured.as_deref(), command);
        let created = now_ms();
        let child = tokio::process::Command::new(program)
            .args(args)
            .current_dir(&session.directory)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let (output, metadata) = match tokio::time::timeout(SHELL_TIMEOUT, child).await {
            Ok(Ok(out)) => {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                (text, json!({ "exit": out.status.code() }))
            }
            Ok(Err(error)) => (error.to_string(), json!({ "exit": 1 })),
            Err(_) => (String::new(), json!({ "timedOut": true })),
        };
        let data = ShellData {
            time: AssistantTime {
                created,
                streamed: None,
                completed: Some(now_ms()),
            },
            command: command.to_string(),
            output: Some(ShellOutput { output }),
            metadata: Some(metadata),
            extra: Map::new(),
        };
        let value = serde_json::to_value(&data).unwrap_or(Value::Null);
        self.shared
            .db
            .append_message(session_id, &crate::ids::message(), "shell", &value)?;
        Ok(())
    }
}

/// Session metadata key of the directories an ACP client granted.
const GRANTED: &str = "opencode.acp.additionalDirectories";

impl Core {
    /// The directories granted with [`Core::grant_directories`].
    pub fn granted_directories(&self, session_id: &str) -> Result<Vec<String>, CoreError> {
        let metadata = self.shared.db.session_metadata(session_id)?;
        Ok(metadata
            .get(GRANTED)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default())
    }

    /// Replaces the directories outside the session's own that its tools may use without asking.
    /// The grants lead the session's rules, so its other rules keep precedence for those paths.
    pub fn grant_directories(
        &self,
        session_id: &str,
        directories: &[String],
    ) -> Result<(), CoreError> {
        use crate::permission::{Effect, Rule};
        let db = &self.shared.db;
        let rules = |directories: &[String]| -> Vec<Rule> {
            let mut resources: Vec<String> = Vec::new();
            for directory in directories {
                // Tools compare written paths without following symlinks: grant both spellings.
                let resolved = crate::paths::canonicalize(directory)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| directory.clone());
                for root in [directory.clone(), resolved] {
                    let resource = Path::new(&root).join("*").display().to_string();
                    if !resources.contains(&resource) {
                        resources.push(resource);
                    }
                }
            }
            resources
                .into_iter()
                .map(|r| Rule::new("external_directory", &r, Effect::Allow))
                .collect()
        };
        let previous = self.granted_directories(session_id)?;
        if previous == directories {
            return Ok(());
        }
        let owned = rules(&previous);
        let mut granted = rules(directories);
        granted.extend(
            db.session_permissions(session_id)?
                .into_iter()
                .filter(|rule| !owned.contains(rule)),
        );
        let mut metadata = db.session_metadata(session_id)?;
        metadata.remove(GRANTED);
        if !directories.is_empty() {
            metadata.insert(GRANTED.into(), json!(directories));
        }
        Ok(db.set_session_access(session_id, &granted, &metadata)?)
    }
}
