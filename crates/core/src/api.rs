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

use crate::config::Config;
use crate::credits::Credits;
use crate::events::{CreditsSnapshot, Event};
use crate::gateway::config::Remote;
use crate::gateway::{Gateway, Token};
use crate::model::{Model, ModelRef};
use crate::paths::Paths;
use crate::project::Project;
use crate::session::gate::{Asks, Reply};
use crate::session::inbox::Delivery;
use crate::session::question::Questions;
use crate::session::subagent::Jobs;
use crate::session::{Outcome, TurnOptions};
use crate::storage::{Db, SessionRow};
use crate::tool::question::QuestionReply;
use crate::tool::registry::Registry;
use crate::tool::{PathLocks, SessionNotifier};

mod commands;
mod front;
mod revert;
mod state;

pub use state::Loaded;

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
    #[error("{0}")]
    Image(String),
    #[error("{0}")]
    InstructionEntry(String),
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
    /// Skills the user invoked with `@<skill>`; their content goes before the text.
    pub skills: Vec<crate::session::message::InvokedSkill>,
}

pub(crate) struct Shared {
    pub paths: Arc<Paths>,
    pub db: Arc<Db>,
    pub gateway: Arc<Gateway>,
    pub events: broadcast::Sender<Event>,
    pub credits: Arc<Credits>,
    loaded: std::sync::RwLock<Arc<Loaded>>,
    /// The last `/config` result.
    remote: Mutex<Remote>,
    /// Watched files and the remote document bump this; [`Shared::refresh`] reloads.
    changes: Arc<crate::watch::Changes>,
    /// The generation of `changes` that `loaded` reflects.
    applied: std::sync::atomic::AtomicU64,
    reloading: tokio::sync::Mutex<()>,
    watch: crate::watch::Watch,
    pub registry: Registry,
    /// The MCP runtime, once a front end attaches it.
    pub mcp: std::sync::OnceLock<Arc<dyn crate::mcp::Mcp>>,
    pub forms: crate::mcp::Forms,
    pub asks: Arc<Asks>,
    pub questions: Questions,
    pub jobs: Jobs,
    pub locks: Arc<PathLocks>,
    pub env: Arc<Vec<(OsString, OsString)>>,
    pub notifier: Arc<dyn SessionNotifier>,
    pub directory: PathBuf,
    pub project: Project,
    /// The shadow repository of the project's files; `None` outside git or with `snapshots: false`.
    pub snapshots: Option<Arc<crate::snapshot::Store>>,
    /// `<pid>:<nonce>` for busy markers.
    pub owner: String,
    pub turns: Mutex<std::collections::HashMap<String, CancellationToken>>,
    pub warming: crate::session::Warming,
    pub goals: crate::session::goal::Goals,
    /// Background summaries held until automatic compaction takes them.
    pub precomputed: Mutex<std::collections::HashMap<String, crate::session::compaction::Held>>,
    /// Set once the front end shuts down: background completions no longer wake sessions.
    pub closing: std::sync::atomic::AtomicBool,
}

impl crate::codemode::Host for Shared {
    fn db(&self) -> &Db {
        &self.db
    }

    fn models(&self) -> Arc<[Model]> {
        self.loaded().models.clone()
    }

    fn config(&self) -> Arc<Config> {
        self.loaded().config.clone()
    }

    fn formatters(&self) -> Option<Arc<crate::formatter::Formatters>> {
        self.loaded().formatters.clone()
    }

    fn mcp(&self) -> Option<Arc<dyn crate::mcp::Mcp>> {
        self.mcp.get().cloned()
    }
}

#[derive(Clone)]
pub struct Core {
    pub(crate) shared: Arc<Shared>,
}

/// Background jobs post into the session's inbox: a running turn takes the message at its next
/// step, an idle session starts a turn for it.
struct StoreNotifier {
    shared: std::sync::Weak<Shared>,
}

impl SessionNotifier for StoreNotifier {
    fn synthetic(&self, session_id: &str, text: String, description: Option<String>) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let admitted = crate::session::inbox::enqueue_synthetic(
            &shared.db,
            session_id,
            &text,
            description.as_deref(),
            None,
            Delivery::Steer,
        );
        let idle = !shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(session_id);
        if admitted.is_ok() && idle && !shared.closing.load(std::sync::atomic::Ordering::Relaxed) {
            let session_id = session_id.to_string();
            tokio::spawn(async move {
                let _ = drain_session(
                    &shared,
                    &session_id,
                    TurnOptions::default(),
                    CancellationToken::new(),
                )
                .await;
            });
        }
    }
}

impl Core {
    /// Opens storage, loads the remote config (live, else cached) and every config source, and
    /// starts watching them.
    pub async fn start(directory: &Path) -> Result<Core, CoreError> {
        let paths = Arc::new(Paths::from_env());
        let db = Arc::new(Db::open(&paths.database())?);
        let token = Token::resolve(&db)?;
        let gateway = Arc::new(Gateway::new(token));
        let remote = crate::gateway::config::fetch(&gateway, &db).await;
        let loaded = Loaded::load(&paths, &gateway, directory, &remote, None).await;
        let project = crate::project::identify(directory);
        let snapshots = (project.vcs == Some("git")
            && loaded.config.get("snapshots") != Some(&Value::Bool(false)))
        .then(|| crate::snapshot::Store::open(&paths.data, &project.id, directory))
        .flatten()
        .map(Arc::new);
        db.upsert_project(&project)?;
        let (events, _) = broadcast::channel(4096);
        let credits = Arc::new(Credits::new(db.clone(), events.clone()));
        let env = std::env::vars_os()
            .filter(|(k, _)| k != "OPENCODE_PASSWORD" && k != "OPENCODE_SERVER_PASSWORD")
            .collect();
        let asks = Arc::new(Asks::new(db.clone(), events.clone()));
        let shared = Arc::new_cyclic(|weak| Shared {
            notifier: Arc::new(StoreNotifier {
                shared: weak.clone(),
            }),
            paths,
            db,
            gateway,
            events,
            credits,
            loaded: std::sync::RwLock::new(Arc::new(loaded)),
            remote: Mutex::new(remote),
            changes: Arc::default(),
            applied: Default::default(),
            reloading: Default::default(),
            watch: Default::default(),
            registry: Registry::builtin(),
            mcp: Default::default(),
            forms: Default::default(),
            asks,
            questions: Questions::default(),
            jobs: Jobs::default(),
            locks: Arc::default(),
            env: Arc::new(env),
            directory: directory.to_path_buf(),
            project,
            snapshots,
            owner: format!("{}:{}", std::process::id(), crate::ids::part()),
            turns: Mutex::default(),
            warming: Default::default(),
            goals: Default::default(),
            precomputed: Mutex::default(),
            closing: Default::default(),
        });
        shared.arm_watch(&shared.loaded());
        if shared.gateway.token().is_some() {
            tokio::spawn(state::poll_remote(Arc::downgrade(&shared)));
        }
        tokio::spawn(crate::reference::keep_fresh(Arc::downgrade(&shared)));
        Ok(Core { shared })
    }

    /// Hands the MCP runtime its config and keeps it in step with config changes, which are
    /// reloaded as they happen from now on (not only before a step).
    pub fn attach_mcp(&self, mcp: Arc<dyn crate::mcp::Mcp>) {
        if self.shared.mcp.set(mcp.clone()).is_err() {
            return;
        }
        mcp.reconcile(self.loaded().config.clone());
        let shared = Arc::downgrade(&self.shared);
        let changes = self.shared.changes.clone();
        tokio::spawn(async move {
            loop {
                changes.marked().await;
                let Some(shared) = shared.upgrade() else {
                    return;
                };
                shared.refresh().await;
            }
        });
    }

    /// Known projects, most recently used first: `(id, worktree)`.
    pub fn projects(&self, limit: u64) -> Result<Vec<(String, String)>, CoreError> {
        Ok(self.shared.db.projects(limit)?)
    }

    pub fn project_id(&self) -> &str {
        &self.shared.project.id
    }

    /// The session's running background shells.
    pub fn background_shells(&self, session_id: &str) -> Vec<crate::tool::shell::BackgroundShell> {
        crate::tool::shell::background_shells(session_id)
    }

    /// Ends a background shell by its shell ID.
    pub fn kill_shell(&self, id: &str) -> bool {
        crate::tool::shell::kill_background(id)
    }

    /// The session's direct children, newest first: `(id, title, agent)`.
    pub fn children(
        &self,
        session_id: &str,
    ) -> Result<Vec<(String, String, Option<String>)>, CoreError> {
        Ok(self.shared.db.children(session_id)?)
    }

    /// Whether a subagent session runs in this process now.
    pub fn child_running(&self, session_id: &str) -> bool {
        crate::session::subagent::is_running(&self.shared, session_id)
    }

    /// Moves a session into this core's project and directory.
    pub fn adopt_session(&self, id: &str) -> Result<(), CoreError> {
        let directory = self.shared.directory.display().to_string();
        Ok(self
            .shared
            .db
            .move_session(id, &self.shared.project.id, &directory)?)
    }

    pub fn mcp(&self) -> Option<Arc<dyn crate::mcp::Mcp>> {
        self.shared.mcp.get().cloned()
    }

    /// Adds or replaces a Code Mode namespace (an MCP server's tools).
    pub fn set_namespace(&self, namespace: Arc<crate::codemode::Namespace>) {
        self.shared.registry.set_namespace(namespace);
    }

    pub fn remove_namespace(&self, name: &str) {
        self.shared.registry.remove_namespace(name);
    }

    pub fn emit(&self, event: Event) {
        let _ = self.shared.events.send(event);
    }

    /// Asks the front ends to fill in a form; [`Core::reply_form`] answers it.
    pub async fn ask_form(&self, form: crate::mcp::Form) -> crate::mcp::FormReply {
        self.shared.forms.ask(&self.shared.events, form).await
    }

    pub fn reply_form(&self, id: &str, reply: crate::mcp::FormReply) -> bool {
        self.shared.forms.reply(id, reply)
    }

    pub fn paths(&self) -> &Paths {
        &self.shared.paths
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.shared.db
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    /// The configured references (the composer's `@alias` list leaves out hidden ones).
    pub fn references(&self) -> Vec<crate::reference::Reference> {
        self.loaded().references.clone()
    }

    /// `@alias[/path]` (without the `@`) as the path it names.
    pub fn resolve_reference(&self, mention: &str) -> Option<PathBuf> {
        crate::reference::resolve(&self.loaded().references, mention)
    }

    /// The current config and what was built from it.
    pub fn loaded(&self) -> Arc<Loaded> {
        self.shared.loaded()
    }

    /// Reloads the config sources if any changed (the loop does this before every step).
    pub async fn refresh(&self) {
        self.shared.refresh().await
    }

    /// Fetches the remote config document again (done every 10 minutes while signed in).
    pub async fn refresh_remote(&self) {
        self.shared.refresh_remote().await
    }

    /// `Run \`anysola login\` to sign in to Anysola` after `/config` answered 401.
    pub fn connection_status(&self) -> Option<&'static str> {
        self.shared
            .remote()
            .needs_auth
            .then_some(crate::gateway::config::SIGNED_OUT)
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
        self.fork_before(source, None)
    }

    /// Forks a session: all of it, or only the messages before `before`.
    pub fn fork_session(
        &self,
        session_id: &str,
        before: Option<&str>,
    ) -> Result<SessionRow, CoreError> {
        let source = self
            .shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        self.fork_before(&source, before)
    }

    fn fork_before(
        &self,
        source: &SessionRow,
        before: Option<&str>,
    ) -> Result<SessionRow, CoreError> {
        let number = self.shared.db.fork_count(&source.id)? + 1;
        let title = format!("{} (fork #{number})", source.title);
        let mut messages = self.shared.db.messages(&source.id)?;
        if let Some(before) = before
            && let Some(at) = messages.iter().position(|m| m.id == before)
        {
            messages.truncate(at);
        }
        let through = messages
            .pop()
            .ok_or_else(|| CoreError::ForkEmpty(source.id.clone()))?
            .id;
        let kind = Kind::Fork {
            source: source.id.clone(),
            title,
            through: through.clone(),
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
        self.shared
            .db
            .copy_messages(&source.id, &row.id, &through)?;
        // The fork starts from the instructions its source last saw, not its source's baseline.
        if let Some(state) = self.shared.db.instruction_state(&source.id)? {
            self.shared
                .db
                .instruction_initialize(&row.id, 0, &state.current)?;
        }
        self.shared
            .db
            .instruction_entries_copy(&source.id, &row.id)?;
        Ok(row)
    }

    /// Names the session again from its conversation so far.
    pub async fn regenerate_title(&self, session_id: &str) -> Result<(), CoreError> {
        let session = self
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        crate::session::regenerate_title(self.shared.clone(), session).await;
        Ok(())
    }

    /// Attaches `value` to the session's instructions under `key` (shown to the model as a
    /// `<context key="…">` block from the next step), or detaches it when `value` is `None`.
    pub fn set_instruction_entry(
        &self,
        session_id: &str,
        key: &str,
        value: Option<&Value>,
    ) -> Result<(), CoreError> {
        let valid = key.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && key.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
            });
        if !valid {
            return Err(CoreError::InstructionEntry(format!(
                "Invalid instruction entry key: {key}"
            )));
        }
        const MAX_BYTES: usize = 256 * 1024;
        if let Some(value) = value {
            let bytes = crate::json::compact(value).len();
            if bytes > MAX_BYTES {
                return Err(CoreError::InstructionEntry(format!(
                    "Instruction entry value is {bytes} bytes; the limit is {MAX_BYTES} bytes"
                )));
            }
        }
        self.session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        Ok(self
            .shared
            .db
            .instruction_entry_set(session_id, key, value)?)
    }

    pub fn set_title(&self, session_id: &str, title: &str) -> Result<(), CoreError> {
        Ok(self.shared.db.set_title(session_id, title)?)
    }

    /// Selects the agent for the session's next turns, recorded as an `agent-switched` message.
    pub fn select_agent(&self, session_id: &str, agent: &str) -> Result<(), CoreError> {
        if self.shared.loaded().agents.get(agent).is_none() {
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
        crate::model::resolve(&self.shared.loaded().models, model)?;
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

    pub fn set_budget(
        &self,
        session_id: &str,
        budget: Option<u64>,
    ) -> Result<CreditsSnapshot, CoreError> {
        Ok(self.shared.credits.set_budget(session_id, budget)?)
    }

    /// Admits the prompt and runs the session until its inbox is empty. Fails fast when another
    /// process drives the session.
    pub async fn prompt(
        &self,
        session_id: &str,
        prompt: Prompt,
        options: TurnOptions,
    ) -> Result<Outcome, CoreError> {
        prompt_session(
            &self.shared,
            session_id,
            prompt,
            options,
            CancellationToken::new(),
        )
        .await
    }

    /// Admits a prompt without waiting: a running turn picks up `steer` input at its next step
    /// boundary and `queue` input when it ends. Returns the pending item's id.
    pub fn enqueue(
        &self,
        session_id: &str,
        prompt: Prompt,
        delivery: Delivery,
    ) -> Result<String, CoreError> {
        let prompt = normalize(&self.shared, prompt)?;
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
        drain_session(&self.shared, session_id, options, CancellationToken::new()).await
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

    pub fn reply_question(&self, request_id: &str, reply: QuestionReply) -> bool {
        self.shared.questions.reply(request_id, reply)
    }

    /// Moves the session's running foreground shell commands to the background (ctrl+b) and tells
    /// the model; false when nothing was running.
    pub fn background(&self, session_id: &str) -> bool {
        let moved = crate::tool::shell::background_all(session_id);
        if moved.is_empty() {
            return false;
        }
        let mut lines = vec![
            "User requested that active blocking work be moved to the background.".to_string(),
            String::new(),
            "Backgrounded work:".to_string(),
        ];
        lines.extend(moved.iter().map(|command| format!("- shell: {command}")));
        lines.push(String::new());
        lines.push(BACKGROUNDED.to_string());
        self.shared
            .notifier
            .synthetic(session_id, lines.join("\n"), None);
        true
    }

    /// The session's todo list: the input of its latest completed `todowrite` call before a
    /// staged revert.
    pub fn todos(&self, session_id: &str) -> Result<Vec<crate::tool::todowrite::Todo>, CoreError> {
        use crate::session::message::{Message, Part, ToolState};
        let boundary = self.shared.db.revert(session_id)?;
        let boundary = boundary
            .as_ref()
            .and_then(|r| r.get("messageID"))
            .and_then(Value::as_str);
        let latest = self
            .shared
            .db
            .messages(session_id)?
            .into_iter()
            .take_while(|row| Some(row.id.as_str()) != boundary)
            .filter_map(|row| match Message::decode(&row.kind, &row.data) {
                Message::Assistant(data) => Some(data.content),
                _ => None,
            })
            .flatten()
            .filter_map(|part| match part {
                Part::Tool {
                    name,
                    state: ToolState::Completed { input, .. },
                    ..
                } if name == "todowrite" => Some(input),
                _ => None,
            })
            .last();
        Ok(latest
            .and_then(|input| serde_json::from_value(input["todos"].clone()).ok())
            .unwrap_or_default())
    }

    /// For process exit: rejects every pending permission ask and every later one, stops background
    /// completions from waking sessions and kills the background shell commands.
    pub fn shutdown(&self) {
        self.shared
            .closing
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.shared.asks.close();
        crate::tool::shell::kill_background_shells();
    }
}

const BACKGROUNDED: &str = "The backgrounded work is still unfinished. Move on to other work if you can. If there is nothing else useful to do, finish your response. Do not wait, sleep, poll, or report the backgrounded work as complete until a later completion notification is added to the conversation.";

/// Attached images as the model should get them.
fn normalize(shared: &Shared, mut prompt: Prompt) -> Result<Prompt, CoreError> {
    let limits = crate::media::Limits::from_config(shared.loaded().config.get("media"));
    for (mime, data, name) in &mut prompt.files {
        if !crate::media::RESIZABLE.contains(&mime.as_str()) {
            continue;
        }
        match crate::media::normalize(mime, data, &limits) {
            Ok(Some((normalized, encoded))) => (*mime, *data) = (normalized, encoded),
            Ok(None) => {}
            Err(error) => return Err(CoreError::Image(error.message(name))),
        }
    }
    Ok(prompt)
}

/// Admits a prompt (committing a staged revert first) and runs the session under `cancel` until
/// its inbox is empty.
pub(crate) async fn prompt_session(
    shared: &Arc<Shared>,
    session_id: &str,
    prompt: Prompt,
    options: TurnOptions,
    cancel: CancellationToken,
) -> Result<Outcome, CoreError> {
    shared
        .db
        .session(session_id)?
        .ok_or(CoreError::SessionNotFound)?;
    revert::commit(shared, session_id)?;
    let prompt = normalize(shared, prompt)?;
    crate::session::inbox::enqueue(&shared.db, session_id, &prompt, Delivery::Steer)?;
    drain_session(shared, session_id, options, cancel).await
}

/// Runs the session until its inbox is empty, registered so [`Core::interrupt`] reaches it.
/// Fails fast when another process drives it.
async fn drain_session(
    shared: &Arc<Shared>,
    session_id: &str,
    options: TurnOptions,
    cancel: CancellationToken,
) -> Result<Outcome, CoreError> {
    if !shared.db.acquire(session_id, &shared.owner)? {
        return Err(CoreError::Busy(session_id.to_string()));
    }
    shared
        .turns
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_id.to_string(), cancel.clone());
    let _tracked = crate::teardown::track(format!("session {session_id} running"));
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
    let outcome = crate::session::drain(shared.clone(), session_id, options, cancel).await;
    heartbeat.abort();
    shared
        .turns
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session_id);
    shared.db.release(session_id, &shared.owner)?;
    Ok(outcome)
}
