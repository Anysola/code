//! The in-process API every front end uses (`run` now, the TUI and ACP later): open, continue or
//! fork a session, prompt it, interrupt it, answer permission asks, and follow its events.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::time::now_ms;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::agent::Agents;
use crate::config::{Config, LoadOptions};
use crate::credits::Credits;
use crate::events::{CreditsSnapshot, Event};
use crate::gateway::{ConfigFetch, Gateway, Token};
use crate::model::{Model, ModelRef};
use crate::paths::Paths;
use crate::project::Project;
use crate::session::gate::{Asks, Reply};
use crate::session::inbox::Delivery;
use crate::session::{Outcome, TurnOptions};
use crate::storage::{Db, SessionRow};
use crate::tool::registry::Registry;
use crate::tool::{PathLocks, SessionNotifier};

mod commands;
mod revert;

const CONFIG_CACHE_KEY: &str = "anysola:config";

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Storage(#[from] crate::storage::StorageError),
    #[error("Session not found")]
    SessionNotFound,
    #[error("Session {0} is busy in another Anysola Code process.")]
    Busy(String),
    #[error("Agent not found: \"{0}\"")]
    AgentNotFound(String),
    #[error(transparent)]
    Model(#[from] crate::model::ModelError),
    #[error("Cannot fork empty session: {0}")]
    ForkEmpty(String),
    #[error("Command not found: \"{0}\"")]
    CommandNotFound(String),
    #[error("Message not found: {0}")]
    MessageNotFound(String),
    #[error("{0}")]
    Snapshot(String),
}

/// Which session a front end works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    New,
    /// The latest root session of the directory, else a new one.
    Continue,
    /// This id; a new session with exactly this id when it does not exist.
    Id(String),
}

/// What a new session is.
enum Kind {
    Root,
    Child {
        parent: String,
        title: Option<String>,
    },
    Fork {
        source: String,
        title: String,
        /// The source's last message, which the fork copies through.
        through: String,
    },
}

/// A user prompt.
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub text: String,
    /// `(mime, base64 data, file name)` attachments sent as media.
    pub files: Vec<(String, String, String)>,
}

pub(crate) struct Shared {
    pub paths: Arc<Paths>,
    pub db: Arc<Db>,
    pub gateway: Arc<Gateway>,
    pub events: broadcast::Sender<Event>,
    pub credits: Arc<Credits>,
    pub config: Config,
    pub agents: Agents,
    pub commands: crate::command::Commands,
    pub models: Vec<Model>,
    pub registry: Registry,
    pub asks: Arc<Asks>,
    pub locks: Arc<PathLocks>,
    pub env: Arc<Vec<(OsString, OsString)>>,
    pub notifier: Arc<dyn SessionNotifier>,
    pub directory: PathBuf,
    pub project: Project,
    /// The shadow repository of the project's files; `None` outside git or with `snapshots: false`.
    pub snapshots: Option<Arc<crate::snapshot::Store>>,
    /// The configured `instructions` URLs, fetched when the config loaded.
    pub remote_instructions: Vec<crate::instructions::InstructionFile>,
    /// `<pid>:<nonce>` for busy markers.
    pub owner: String,
    pub turns: Mutex<std::collections::HashMap<String, CancellationToken>>,
}

#[derive(Clone)]
pub struct Core {
    pub(crate) shared: Arc<Shared>,
}

/// Background shells post their completion here; with no running turn it is stored for the next.
struct StoreNotifier {
    db: std::sync::Weak<Db>,
}

impl SessionNotifier for StoreNotifier {
    fn synthetic(&self, session_id: &str, text: String) {
        if let Some(db) = self.db.upgrade() {
            let data =
                serde_json::json!({"time": {"created": crate::time::now_ms()}, "text": text});
            let _ = db.append_message(session_id, &crate::ids::message(), "synthetic", &data);
        }
    }
}

impl Core {
    /// Opens storage, loads the remote config (live, else cached) and every config source.
    pub async fn start(directory: &Path) -> Result<Core, CoreError> {
        let paths = Arc::new(Paths::from_env());
        let db = Arc::new(Db::open(&paths.database())?);
        let token = match std::env::var("ANYSOLA_TOKEN") {
            Ok(token) if !token.trim().is_empty() => Some(Token::Env(token.trim().to_string())),
            _ => db.anysola_login()?.map(Token::Login),
        };
        let gateway = Arc::new(Gateway::new(token));
        let remote = match gateway.config().await {
            ConfigFetch::Ok(body) => {
                db.kv_set(CONFIG_CACHE_KEY, &Value::String(body.clone()))?;
                Some(body)
            }
            ConfigFetch::Unauthorized | ConfigFetch::Failed => db
                .kv_get(CONFIG_CACHE_KEY)?
                .and_then(|v| v.as_str().map(str::to_string)),
        };
        let options = LoadOptions {
            remote,
            api_url: gateway.base().to_string(),
        };
        let config = crate::config::load(&paths, directory, &options);
        for warning in &config.warnings {
            crate::log::warn(warning, &[]);
        }
        let agents = Agents::load(&config, &paths);
        let commands = crate::command::Commands::load(&config);
        let remote_instructions =
            crate::instructions::fetch_remote(&gateway, &config.instructions).await;
        let models = crate::model::catalog(&config);
        let project = crate::project::identify(directory);
        let snapshots = (project.vcs == Some("git")
            && config.get("snapshots") != Some(&Value::Bool(false)))
        .then(|| crate::snapshot::Store::open(&paths.data, &project.id, directory))
        .flatten()
        .map(Arc::new);
        db.upsert_project(&project, directory)?;
        let (events, _) = broadcast::channel(4096);
        let credits = Arc::new(Credits::new(db.clone(), events.clone()));
        let env = std::env::vars_os()
            .filter(|(k, _)| k != "OPENCODE_PASSWORD" && k != "OPENCODE_SERVER_PASSWORD")
            .collect();
        let notifier = Arc::new(StoreNotifier {
            db: Arc::downgrade(&db),
        });
        let shared = Shared {
            paths,
            db,
            gateway,
            events,
            credits,
            config,
            commands,
            agents,
            models,
            registry: Registry::builtin(),
            asks: Arc::default(),
            locks: Arc::default(),
            env: Arc::new(env),
            notifier,
            directory: directory.to_path_buf(),
            project,
            remote_instructions,
            snapshots,
            owner: format!("{}:{}", std::process::id(), crate::ids::part()),
            turns: Mutex::default(),
        };
        Ok(Core {
            shared: Arc::new(shared),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    pub fn config(&self) -> &Config {
        &self.shared.config
    }

    pub fn agents(&self) -> &Agents {
        &self.shared.agents
    }

    pub fn models(&self) -> &[Model] {
        &self.shared.models
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionRow>, CoreError> {
        Ok(self.shared.db.session(id)?)
    }

    /// Resolves the target to a session, creating it when needed. Returns it and whether it
    /// already existed.
    pub fn open(&self, target: &Target, fork: bool) -> Result<(SessionRow, bool), CoreError> {
        let db = &self.shared.db;
        let directory = self.shared.directory.display().to_string();
        let existing = match target {
            Target::New => None,
            Target::Continue => db.latest_root_session(&directory)?,
            Target::Id(id) => db.session(id)?,
        };
        match (existing, fork) {
            (Some(source), true) => Ok((self.fork(&source)?, false)),
            (None, true) => Err(CoreError::SessionNotFound),
            (Some(session), false) => Ok((session, true)),
            (None, false) => {
                let id = match target {
                    Target::Id(id) => id.clone(),
                    _ => crate::ids::session(),
                };
                Ok((self.create(id, Kind::Root)?, false))
            }
        }
    }

    fn create(&self, id: String, kind: Kind) -> Result<SessionRow, CoreError> {
        let now = crate::time::now_ms();
        let iso = crate::time::iso(now);
        let (title, parent_id, fork_session_id, fork_boundary) = match kind {
            Kind::Root => (format!("New session - {iso}"), None, None, None),
            Kind::Child { parent, title } => (
                title.unwrap_or_else(|| format!("Child session - {iso}")),
                Some(parent),
                None,
                None,
            ),
            Kind::Fork {
                source,
                title,
                through,
            } => (
                title,
                None,
                Some(source),
                Some(serde_json::json!({"type": "through", "messageID": through})),
            ),
        };
        let row = SessionRow {
            id,
            project_id: self.shared.project.id.clone(),
            parent_id,
            fork_session_id,
            fork_boundary,
            slug: crate::slug::create(),
            directory: self.shared.directory.display().to_string(),
            title,
            version: crate::version::session_version(),
            agent: None,
            model: None,
            time_created: now,
            time_updated: now,
        };
        self.shared.db.create_session(&row)?;
        Ok(row)
    }

    /// A child session of `parent` (subagents, subagent commands); without a title it is
    /// `Child session - <ISO>`.
    pub fn create_child(
        &self,
        parent: &str,
        title: Option<String>,
    ) -> Result<SessionRow, CoreError> {
        self.create(
            crate::ids::session(),
            Kind::Child {
                parent: parent.to_string(),
                title,
            },
        )
    }

    fn fork(&self, source: &SessionRow) -> Result<SessionRow, CoreError> {
        let number = self.shared.db.fork_count(&source.id)? + 1;
        let title = format!("{} (fork #{number})", source.title);
        let through = self
            .shared
            .db
            .messages(&source.id)?
            .pop()
            .ok_or_else(|| CoreError::ForkEmpty(source.id.clone()))?
            .id;
        let kind = Kind::Fork {
            source: source.id.clone(),
            title,
            through,
        };
        let mut row = self.create(crate::ids::session(), kind)?;
        row.agent = source.agent.clone();
        row.model = source.model.clone();
        if let Some(agent) = &row.agent {
            self.shared.db.set_agent(&row.id, agent)?;
        }
        if let Some(model) = &row.model {
            self.shared.db.set_model(&row.id, model)?;
        }
        self.shared.db.copy_messages(&source.id, &row.id)?;
        // The fork starts from the instructions its source last saw, not its source's baseline.
        if let Some(state) = self.shared.db.instruction_state(&source.id)? {
            self.shared
                .db
                .instruction_initialize(&row.id, 0, &state.current)?;
        }
        Ok(row)
    }

    pub fn set_title(&self, session_id: &str, title: &str) -> Result<(), CoreError> {
        Ok(self.shared.db.set_title(session_id, title)?)
    }

    /// Selects the agent for the session's next turns, recorded as an `agent-switched` message.
    pub fn select_agent(&self, session_id: &str, agent: &str) -> Result<(), CoreError> {
        if self.shared.agents.get(agent).is_none() {
            return Err(CoreError::AgentNotFound(agent.to_string()));
        }
        let db = &self.shared.db;
        let session = db.session(session_id)?.ok_or(CoreError::SessionNotFound)?;
        let mut data = json!({"time": {"created": now_ms()}, "agent": agent});
        if let Some(previous) = session.agent {
            data["previous"] = Value::String(previous);
        }
        db.append_message(session_id, &crate::ids::message(), "agent-switched", &data)?;
        Ok(db.set_agent(session_id, agent)?)
    }

    /// Selects the model (and variant) for the session's next turns. A change is recorded as a
    /// `model-switched` message, which becomes an effort marker when only the effort changed.
    pub fn select_model(&self, session_id: &str, model: &ModelRef) -> Result<(), CoreError> {
        crate::model::resolve(&self.shared.models, model)?;
        let db = &self.shared.db;
        let session = db.session(session_id)?.ok_or(CoreError::SessionNotFound)?;
        let previous = session.model.as_ref().and_then(ModelRef::from_json);
        if previous.as_ref() == Some(model) {
            return Ok(());
        }
        let mut data = json!({"time": {"created": now_ms()}, "model": model.to_json()});
        if let Some(previous) = &session.model {
            data["previous"] = previous.clone();
        }
        db.append_message(session_id, &crate::ids::message(), "model-switched", &data)?;
        Ok(db.set_model(session_id, &model.to_json())?)
    }

    pub fn credits(&self, session_id: &str) -> CreditsSnapshot {
        self.shared.credits.snapshot(session_id)
    }

    pub fn set_budget(&self, session_id: &str, budget: Option<u64>) -> CreditsSnapshot {
        self.shared.credits.set_budget(session_id, budget)
    }

    /// Admits the prompt and runs the session until its inbox is empty. Fails fast when another
    /// process drives the session.
    pub async fn prompt(
        &self,
        session_id: &str,
        prompt: Prompt,
        options: TurnOptions,
    ) -> Result<Outcome, CoreError> {
        self.shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        self.commit_revert(session_id)?;
        crate::session::inbox::enqueue(&self.shared.db, session_id, &prompt, Delivery::Steer)?;
        self.drain(session_id, options).await
    }

    /// Admits a prompt without waiting: a running turn picks up `steer` input at its next step
    /// boundary and `queue` input when it ends. Returns the pending item's id.
    pub fn enqueue(
        &self,
        session_id: &str,
        prompt: Prompt,
        delivery: Delivery,
    ) -> Result<String, CoreError> {
        Ok(crate::session::inbox::enqueue(
            &self.shared.db,
            session_id,
            &prompt,
            delivery,
        )?)
    }

    /// The session's pending input, oldest first.
    pub fn pending(&self, session_id: &str) -> Result<Vec<crate::storage::InboxRow>, CoreError> {
        Ok(self.shared.db.inbox(session_id)?)
    }

    /// Switches a pending item between `steer` and `queue`; false once it was promoted.
    pub fn set_delivery(&self, inbox_id: &str, delivery: Delivery) -> Result<bool, CoreError> {
        Ok(self
            .shared
            .db
            .inbox_set_delivery(inbox_id, delivery.as_str())?)
    }

    /// Cancels a pending item; false once it was promoted.
    pub fn cancel_pending(&self, inbox_id: &str) -> Result<bool, CoreError> {
        Ok(self.shared.db.inbox_remove(inbox_id)?)
    }

    /// `/compact`: summarizes the session's history now.
    pub async fn compact(&self, session_id: &str) -> Result<Outcome, CoreError> {
        crate::session::inbox::enqueue_compaction(&self.shared.db, session_id)?;
        self.drain(session_id, TurnOptions::default()).await
    }

    async fn drain(&self, session_id: &str, options: TurnOptions) -> Result<Outcome, CoreError> {
        let shared = &self.shared;
        if !shared.db.acquire(session_id, &shared.owner)? {
            return Err(CoreError::Busy(session_id.to_string()));
        }
        let cancel = CancellationToken::new();
        shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id.to_string(), cancel.clone());
        let heartbeat = {
            let db = shared.db.clone();
            let (id, owner) = (session_id.to_string(), shared.owner.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let _ = db.heartbeat(&id, &owner);
                }
            })
        };
        let outcome = crate::session::drain(self.shared.clone(), session_id, options, cancel).await;
        heartbeat.abort();
        shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        shared.db.release(session_id, &shared.owner)?;
        Ok(outcome)
    }

    /// Interrupts the session's running turn, if any.
    pub fn interrupt(&self, session_id: &str) {
        if let Some(token) = self
            .shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
        {
            token.cancel();
        }
    }

    pub fn reply_permission(&self, request_id: &str, reply: Reply) -> bool {
        self.shared.asks.reply(request_id, reply)
    }
}
