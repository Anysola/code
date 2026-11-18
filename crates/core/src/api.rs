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
use crate::events::{Event, UsageSnapshot};
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
use crate::usage::Usage;

mod commands;
mod front;
mod revert;
mod state;

pub use crate::hooks::{HookDocument, HookInfo, HookScope};
pub use crate::trust::{TrustEntry, TrustItem, TrustKind};
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
    /// No models at all: not signed in, or the gateway refused the token or did not answer.
    #[error("{0}")]
    NoModels(String),
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

/// What [`Core::steer_pending`] did with a waiting item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Steered {
    /// The running turn takes it at its next step.
    Taken,
    /// No turn of this process runs the session: it stays queued.
    NoTurn,
    /// It was promoted or removed meanwhile.
    Gone,
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
    /// The `/<name> <arguments>` line a command's text was rendered from, kept with the message
    /// so a front end can show and restore what the user typed.
    pub command: Option<String>,
}

pub(crate) struct Shared {
    pub paths: Arc<Paths>,
    pub db: Arc<Db>,
    pub gateway: Arc<Gateway>,
    pub events: broadcast::Sender<Event>,
    pub usage: Arc<Usage>,
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
    /// The inbox row of the prompt each held session was taken for: the one input a turn that
    /// cannot start may drop as unsent (the desktop app).
    pub admitted: Mutex<std::collections::HashMap<String, String>>,
    pub warming: crate::session::Warming,
    pub goals: crate::session::goal::Goals,
    /// Background summaries held until automatic compaction takes them.
    pub precomputed: Mutex<std::collections::HashMap<String, crate::session::compaction::Held>>,
    /// Set once the front end shuts down: background completions no longer wake sessions.
    pub closing: std::sync::atomic::AtomicBool,
    pub hooks: crate::hooks::fire::State,
    /// The runtime the core started on: work a synchronous call starts runs there.
    pub runtime: tokio::runtime::Handle,
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

    fn code_mode(&self, agent: &str) -> Option<crate::codemode::Inventory> {
        let loaded = self.loaded();
        let rules = &loaded.agents.get(agent)?.permissions;
        let gates = crate::tool::registry::Gates::from_config(&loaded.config);
        self.registry.code_mode_gated(rules, gates)
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
        if admitted.is_ok() {
            wake(&shared, session_id);
        }
    }

    fn background_changed(&self, session_id: &str) {
        if let Some(shared) = self.shared.upgrade() {
            let _ = shared.events.send(Event::BackgroundChanged {
                session_id: session_id.to_string(),
            });
        }
    }
}

/// Starts a turn for input already in the session's inbox unless one runs in this process; a
/// running turn takes it at its next step.
pub(crate) fn wake(shared: &Arc<Shared>, session_id: &str) {
    let idle = !shared
        .turns
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(session_id);
    if idle && !shared.closing.load(std::sync::atomic::Ordering::Relaxed) {
        let (shared, session_id) = (shared.clone(), session_id.to_string());
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

impl Core {
    /// Opens storage, loads the remote config (live, else cached) and every config source, and
    /// starts watching them.
    pub async fn start(directory: &Path) -> Result<Core, CoreError> {
        let paths = Arc::new(Paths::from_env());
        let db = Arc::new(Db::open(&paths.database())?);
        let token = Token::resolve(&db)?;
        let gateway = Arc::new(Gateway::new(token));
        let remote = crate::gateway::config::fetch(&gateway, &db).await;
        let trust = crate::trust::Decisions::read(&db);
        let loaded = Loaded::load(&paths, &gateway, directory, &remote, None, trust).await;
        let project = crate::project::identify(directory);
        let snapshots = (project.vcs == Some("git")
            && loaded.config.get("snapshots") != Some(&Value::Bool(false)))
        .then(|| crate::snapshot::Store::open(&paths.data, &project.id, directory))
        .flatten()
        .map(Arc::new);
        db.upsert_project(&project)?;
        let (events, _) = broadcast::channel(4096);
        let usage = Arc::new(Usage::new(db.clone(), events.clone()));
        let env = std::env::vars_os()
            .filter(|(k, _)| {
                ![
                    crate::env::names("PASSWORD"),
                    crate::env::names("SERVER_PASSWORD"),
                ]
                .iter()
                .flatten()
                .any(|name| k == name.as_str())
            })
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
            usage,
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
            admitted: Mutex::default(),
            warming: Default::default(),
            goals: Default::default(),
            precomputed: Mutex::default(),
            closing: Default::default(),
            hooks: Default::default(),
            runtime: tokio::runtime::Handle::current(),
        });
        shared.arm_watch(&shared.loaded());
        shared
            .hooks
            .approved
            .sync(&shared.loaded().config.hooks, &shared.db);
        if shared.gateway.token().is_some() {
            tokio::spawn(state::poll_remote(Arc::downgrade(&shared)));
        }
        tokio::spawn(crate::reference::keep_fresh(Arc::downgrade(&shared)));
        tokio::spawn(crate::schedule::run(Arc::downgrade(&shared)));
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

    /// The project's root: its VCS root, else the directory itself.
    pub fn worktree(&self) -> &Path {
        &self.shared.project.worktree
    }

    /// The session's running background shells.
    pub fn background_shells(&self, session_id: &str) -> Vec<crate::tool::shell::BackgroundShell> {
        crate::tool::shell::background_shells(session_id)
    }

    /// Whether a background shell of this process is still running (they outlive their turn and
    /// keep writing their output under `~/.anysola/shell`).
    pub fn any_background_shell(&self) -> bool {
        crate::tool::shell::background_running() > 0
    }

    /// How a background shell ended; `None` while it runs.
    pub fn shell_end(&self, id: &str) -> Option<crate::tool::shell::ShellEnd> {
        crate::tool::shell::background_end(id)
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

    /// Everything waiting for the user in this core, oldest first of each kind, as the events
    /// that asked: permission asks and questions of every session, and the MCP servers' forms.
    pub fn pending_requests(&self) -> Vec<Event> {
        let mut out = self.shared.asks.pending_events();
        out.extend(self.shared.questions.pending());
        out.extend(
            self.shared
                .forms
                .pending()
                .into_iter()
                .map(|form| Event::FormAsked { form }),
        );
        out
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

    /// Reads every config source again now, without waiting for the file watcher: after a front
    /// end wrote a config file itself, or flipped a switch such as
    /// [`crate::skill::set_claude_skills`].
    pub async fn reload(&self) {
        self.shared.changes.mark();
        self.shared.refresh().await
    }

    /// Every configured hook, in run order, with its source and whether it runs.
    pub fn hooks(&self) -> Vec<HookInfo> {
        crate::hooks::list(&self.loaded().config.hooks, &self.shared.db)
    }

    /// The project documents that define hooks, with the user's decision on their current
    /// content ([`HookDocument::decision`] `None`: ask the user).
    pub fn hook_documents(&self) -> Vec<HookDocument> {
        crate::hooks::documents_with(
            &self.loaded().config.hooks,
            &self.shared.db,
            &self.shared.hooks.approved,
        )
    }

    /// Records the user's decision on a project document's hooks as they are now (`digest` from
    /// [`HookDocument::digest`]); takes effect at once. "Not now" is `allow: false`. Sends
    /// [`Event::HooksChanged`].
    pub fn trust_hooks(&self, path: &str, digest: &str, allow: bool) -> Result<(), CoreError> {
        crate::hooks::trust(&self.shared.db, path, digest, allow)?;
        self.shared
            .hooks
            .approved
            .sync(&self.loaded().config.hooks, &self.shared.db);
        self.emit(Event::HooksChanged);
        self.emit(Event::TrustChanged);
        Ok(())
    }

    /// What every project document wants to run, per kind (hooks, MCP servers, formatters), with
    /// the user's decision on its current content; documents in load order.
    pub fn trust_items(&self) -> Vec<crate::trust::TrustItem> {
        crate::trust::items_with(
            &self.loaded().config,
            &self.shared.db,
            &self.shared.hooks.approved,
        )
    }

    /// Records the user's decision on one project document's `kind` as it is now (`digest` from
    /// [`crate::trust::TrustItem::digest`]). It takes effect at once: the config reloads, MCP
    /// servers it approved start and those it refused stop. Sends [`Event::TrustChanged`] (and
    /// [`Event::HooksChanged`] for hooks).
    pub fn trust(
        &self,
        path: &str,
        kind: crate::trust::TrustKind,
        digest: &str,
        allow: bool,
    ) -> Result<(), CoreError> {
        if kind == crate::trust::TrustKind::Hooks {
            return self.trust_hooks(path, digest, allow);
        }
        crate::trust::record(&self.shared.db, path, kind, digest, allow)?;
        self.shared.changes.mark();
        let shared = self.shared.clone();
        self.shared
            .runtime
            .spawn(async move { shared.refresh().await });
        self.emit(Event::TrustChanged);
        Ok(())
    }

    /// Fetches the remote config document again (done every 10 minutes while signed in).
    pub async fn refresh_remote(&self) {
        self.shared.refresh_remote().await
    }

    /// [`SIGNED_OUT`](crate::gateway::config::SIGNED_OUT) after `/config` answered 401.
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
        let row = self.insert(id, kind)?;
        self.announce(&row);
        Ok(row)
    }

    fn announce(&self, row: &SessionRow) {
        self.emit(Event::SessionCreated {
            session_id: row.id.clone(),
            parent_id: row.parent_id.clone(),
            title: row.title.clone(),
        });
    }

    /// Stores a new session without telling listeners about it yet.
    fn insert(&self, id: String, kind: Kind) -> Result<SessionRow, CoreError> {
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
        let mut row = self.insert(crate::ids::session(), kind)?;
        row.agent = source.agent.clone();
        row.model = source.model.clone();
        // Listeners hear of the fork once it has its history; one that cannot be filled is
        // removed rather than left behind empty.
        if let Err(error) = self.fill_fork(source, &row, &through) {
            let _ = self.shared.db.delete_session(&row.id);
            return Err(error);
        }
        self.announce(&row);
        Ok(row)
    }

    fn fill_fork(
        &self,
        source: &SessionRow,
        row: &SessionRow,
        through: &str,
    ) -> Result<(), CoreError> {
        let db = &self.shared.db;
        if let Some(agent) = &row.agent {
            db.set_agent(&row.id, agent)?;
        }
        if let Some(model) = &row.model {
            db.set_model(&row.id, model)?;
        }
        db.copy_messages(&source.id, &row.id, through)?;
        // The fork starts from the instructions its source last saw, not its source's baseline.
        if let Some(state) = db.instruction_state(&source.id)? {
            db.instruction_initialize(&row.id, 0, &state.current)?;
        }
        db.instruction_entries_copy(&source.id, &row.id)?;
        Ok(())
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
        if let Err(error) = crate::model::resolve(&self.shared.loaded().models, model) {
            return Err(crate::session::no_models(&self.shared)
                .map_or(CoreError::Model(error), CoreError::NoModels));
        }
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

    pub fn usage(&self, session_id: &str) -> UsageSnapshot {
        self.shared.usage.snapshot(session_id)
    }

    pub fn set_budget(
        &self,
        session_id: &str,
        budget: Option<u64>,
    ) -> Result<UsageSnapshot, CoreError> {
        Ok(self.shared.usage.set_budget(session_id, budget)?)
    }

    /// Admits the prompt and runs the session until its inbox is empty. Fails fast when another
    /// process drives the session. When a turn of this process already drives it, the prompt is
    /// queued for that turn and this returns `Succeeded` at once.
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

    /// Hands a waiting item to the session's running turn at its next step. Done under the
    /// `turns` lock, so the turn either takes it or has not ended yet: an item switched with no
    /// turn to join would go out unseen with the next prompt.
    pub fn steer_pending(&self, session_id: &str, inbox_id: &str) -> Result<Steered, CoreError> {
        let turns = self.shared.turns.lock().unwrap_or_else(|e| e.into_inner());
        if !turns.contains_key(session_id) {
            return Ok(Steered::NoTurn);
        }
        let switched = self
            .shared
            .db
            .inbox_set_delivery(inbox_id, Delivery::Steer.as_str())?;
        drop(turns);
        Ok(if switched {
            Steered::Taken
        } else {
            Steered::Gone
        })
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

    /// Runs what waits in the session's inbox when no turn does (input left after a stop or a
    /// failure), committing a staged revert first as a prompt would. A turn of this process that
    /// already drives the session takes the inbox itself.
    pub async fn send_queued(
        &self,
        session_id: &str,
        options: TurnOptions,
    ) -> Result<Outcome, CoreError> {
        let shared = &self.shared;
        shared
            .db
            .session(session_id)?
            .ok_or(CoreError::SessionNotFound)?;
        // Nothing waits: no turn to start (a drain would announce one and never end it).
        if crate::session::inbox::next(&shared.db, session_id, crate::session::inbox::Scope::Input)?
            .is_none()
        {
            return Ok(Outcome::Succeeded);
        }
        let Some(held) = Held::acquire(shared, session_id, CancellationToken::new(), None)? else {
            return Ok(Outcome::Succeeded);
        };
        revert::commit(shared, session_id)?;
        Ok(held.drain(options).await)
    }

    /// Ends a turn that a process which died under it left open (tool calls shown running, no
    /// end), unless a process drives the session now. Returns whether there was one.
    pub fn settle_interrupted(&self, session_id: &str) -> Result<bool, CoreError> {
        let shared = &self.shared;
        // Under the `turns` lock but without taking a turn's place: a prompt arriving meanwhile
        // waits for the lock and then starts its turn as usual.
        let turns = shared.turns.lock().unwrap_or_else(|e| e.into_inner());
        if turns.contains_key(session_id)
            || crate::session::subagent::is_running(shared, session_id)
            // Read first: most sessions ended their turns, and taking one rewrites its busy time.
            || !crate::session::interrupted(shared, session_id)
            || !shared.db.acquire(session_id, &shared.owner)?
        {
            return Ok(false);
        }
        let settled = crate::session::settle_interrupted(shared, session_id);
        let _ = shared.db.release(session_id, &shared.owner);
        drop(turns);
        Ok(settled)
    }

    async fn drain(&self, session_id: &str, options: TurnOptions) -> Result<Outcome, CoreError> {
        drain_session(&self.shared, session_id, options, CancellationToken::new()).await
    }

    /// Whether this process is driving a turn of the session right now.
    pub fn is_busy(&self, session_id: &str) -> bool {
        self.shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(session_id)
    }

    /// The sessions this process drives a turn of right now.
    pub fn busy_sessions(&self) -> Vec<String> {
        self.shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Whether this process is driving any turn of this core.
    pub fn any_busy(&self) -> bool {
        !self
            .shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Interrupts every running turn (the front end is letting this core go).
    pub fn interrupt_all(&self) {
        for token in self
            .shared
            .turns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            token.cancel();
        }
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
        // The firing loop sees `closing` and releases its lease now rather than after a sleep.
        crate::schedule::ring();
    }
}

const BACKGROUNDED: &str = "The backgrounded work is still unfinished. Move on to other work if you can. If there is nothing else useful to do, finish your response. Do not wait, sleep, poll, or report the backgrounded work as complete until a later completion notification is added to the conversation.";

/// Attached images as the model should get them.
fn normalize(shared: &Shared, mut prompt: Prompt) -> Result<Prompt, CoreError> {
    let limits = crate::media::Limits::from_config(shared.loaded().config.get("media"));
    for (mime, data, name) in &mut prompt.files {
        // Every image goes through: one the model can't take is converted or refused here,
        // before it is stored and sent with every later turn.
        if !mime.to_ascii_lowercase().starts_with("image/") {
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
    let prompt = normalize(shared, prompt)?;
    // The session is taken before the prompt is admitted: enqueued first, a prompt refused
    // with `Busy` would still go out later from the process that drives the session. When a
    // turn of this process already drives it, the prompt is queued for that turn instead, as
    // `Core::enqueue` would have done.
    let Some(held) = Held::acquire(shared, session_id, cancel, Some(&prompt))? else {
        return Ok(Outcome::Succeeded);
    };
    revert::commit(shared, session_id)?;
    let admitted =
        crate::session::inbox::enqueue(&shared.db, session_id, &prompt, Delivery::Steer)?;
    shared
        .admitted
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_id.to_string(), admitted);
    Ok(held.drain(options).await)
}

/// A session marked busy for this process: the heartbeat task, the cancel token registered for
/// [`Core::interrupt`] and the marker itself are released on drop, so a turn that unwinds or is
/// dropped cannot leave the session busy for the rest of the process's life.
///
/// One turn at a time drives a session in this process: taking and releasing it happen under
/// the `turns` lock, and so does queueing a prompt for the turn that holds it, so a prompt is
/// either seen by that turn's last look at the inbox or finds the session free.
struct Held<'a> {
    shared: &'a Arc<Shared>,
    session_id: String,
    cancel: CancellationToken,
    heartbeat: tokio::task::JoinHandle<()>,
    released: bool,
    _tracked: crate::teardown::Tracked,
}

impl<'a> Held<'a> {
    /// Takes the session for this process; `None` when a turn of this process already holds it,
    /// after queueing `queue` for that turn.
    fn acquire(
        shared: &'a Arc<Shared>,
        session_id: &str,
        cancel: CancellationToken,
        queue: Option<&Prompt>,
    ) -> Result<Option<Self>, CoreError> {
        let mut turns = shared.turns.lock().unwrap_or_else(|e| e.into_inner());
        if turns.contains_key(session_id) {
            if let Some(prompt) = queue {
                crate::session::inbox::enqueue(&shared.db, session_id, prompt, Delivery::Queue)?;
            }
            return Ok(None);
        }
        if !shared.db.acquire(session_id, &shared.owner)? {
            return Err(CoreError::Busy(session_id.to_string()));
        }
        turns.insert(session_id.to_string(), cancel.clone());
        drop(turns);
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
        Ok(Some(Held {
            shared,
            session_id: session_id.to_string(),
            cancel,
            heartbeat,
            released: false,
            _tracked: crate::teardown::track(format!("session {session_id} running")),
        }))
    }

    /// Runs the session until its inbox is empty. Input queued while the last turn was ending
    /// is run too; once the session is released, the next prompt starts a turn of its own. A
    /// turn that fails or is interrupted leaves what is queued for the user to send.
    async fn drain(mut self, options: TurnOptions) -> Outcome {
        loop {
            let outcome = crate::session::drain(
                self.shared.clone(),
                &self.session_id,
                options,
                self.cancel.clone(),
            )
            .await;
            let shared = self.shared;
            let mut turns = shared.turns.lock().unwrap_or_else(|e| e.into_inner());
            if outcome != Outcome::Succeeded {
                // Held and released in one step: a prompt sent meanwhile is either among the
                // leftovers the window shows, or finds the session free and starts its own turn.
                // The terminal keeps them as they were: it has no way to send a queue.
                if crate::version::client() == "desktop" {
                    let _ = crate::session::inbox::hold_leftovers(&shared.db, &self.session_id);
                }
                self.release(&mut turns);
                return outcome;
            }
            let next = crate::session::inbox::next(
                &shared.db,
                &self.session_id,
                crate::session::inbox::Scope::Input,
            );
            if !matches!(next, Ok(Some(_))) {
                self.release(&mut turns);
                return outcome;
            }
        }
    }

    fn release(&mut self, turns: &mut std::collections::HashMap<String, CancellationToken>) {
        self.heartbeat.abort();
        turns.remove(&self.session_id);
        self.shared
            .admitted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.session_id);
        let _ = self.shared.db.release(&self.session_id, &self.shared.owner);
        self.released = true;
        // A form an MCP server asked for a stopped turn's call has nobody left to wait for it
        // (the server may never end its request itself). Once no turn is left, nor is anyone
        // for the rest.
        if self.cancel.is_cancelled() {
            let (id, idle) = (&self.session_id, turns.is_empty());
            self.shared
                .forms
                .cancel_if(|form| idle || form.session.as_ref() == Some(id));
        }
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.heartbeat.abort();
        if !self.released {
            let shared = self.shared;
            let mut turns = shared.turns.lock().unwrap_or_else(|e| e.into_inner());
            self.release(&mut turns);
        }
    }
}

/// Runs the session until its inbox is empty, registered so [`Core::interrupt`] reaches it.
/// Fails fast when another process drives it.
async fn drain_session(
    shared: &Arc<Shared>,
    session_id: &str,
    options: TurnOptions,
    cancel: CancellationToken,
) -> Result<Outcome, CoreError> {
    match Held::acquire(shared, session_id, cancel, None)? {
        Some(held) => Ok(held.drain(options).await),
        // The turn that holds the session takes its inbox.
        None => Ok(Outcome::Succeeded),
    }
}
