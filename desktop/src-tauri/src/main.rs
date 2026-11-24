//! Anysola Code for the desktop: the agent runs in this process (the core crate) and the window
//! drives it through these commands. Sessions live in `~/.anysola/anysola.db`.

// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod actions;
mod branch;
mod config;
mod data;
mod discord;
mod editor;
mod events;
mod git;
mod hooks;
mod live;
mod output;
mod path_repair;
mod prefs;
mod servers;
mod sources;
mod transcript;
mod tree;
mod trust;
mod update;
mod worktree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anysola_core::api::{Core, Prompt, Steered, Target};
use anysola_core::events::{Event, InterruptReason};
use anysola_core::gateway::account::DeviceLogin;
use anysola_core::mcp::FormReply;
use anysola_core::model::{ModelRef, PROVIDER};
use anysola_core::paths::Paths;
use anysola_core::session::TurnOptions;
use anysola_core::session::gate::{Floor, Reply};
use anysola_core::session::message::InvokedSkill;
use anysola_core::storage::{Db, SessionRow, SessionWorktreeRow};
use anysola_core::tool::question::QuestionReply;
use prefs::Notifications;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Mutex, OnceCell};

pub(crate) type Answer = Result<Value, String>;

/// Sessions of every project, newest first, for the sidebar.
const SESSION_LIST: u64 = 200;
/// How long closing the window waits for MCP servers to stop.
const EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(3);
/// How long stopping a core waits for its interrupted turns to end.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// A folder's core, its MCP runtime and the task that sends its events to the window.
struct Running {
    core: Core,
    runtime: Arc<anysola_mcp::Runtime>,
    forward: tauri::async_runtime::JoinHandle<()>,
}

impl Running {
    /// Ends what the core has going: its turns (and the hook commands they run), its MCP
    /// servers, its background commands.
    async fn stop(&self) {
        self.core.interrupt_all();
        // A stopped turn kills its hook commands as it sees the stop: give it the moment, or an
        // exit right after (the updater's on Windows) can leave them running.
        let start = std::time::Instant::now();
        while self.core.any_busy() && start.elapsed() < STOP_WAIT {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if let Some(mcp) = self.core.mcp() {
            mcp.shutdown().await;
        }
        self.core.shutdown();
        // The task holds the core and would otherwise never end.
        self.forward.abort();
    }
}

pub(crate) struct App {
    db: Option<Arc<Db>>,
    /// Why the database didn't open, when it didn't.
    db_fault: Option<String>,
    pub(crate) paths: Paths,
    /// Read without I/O by `notify()` and the commands; written by `prefs_set`.
    pub(crate) prefs: std::sync::RwLock<prefs::Prefs>,
    /// Started in `setup`, once the single-instance check has let this process go on: a second
    /// launch never sets an activity.
    pub(crate) discord: std::sync::OnceLock<discord::Handle>,
    /// `desktop.json` was there but couldn't be parsed: moved aside before the first save.
    pub(crate) prefs_invalid: std::sync::atomic::AtomicBool,
    /// Held across each read-change-write of the global config file.
    pub(crate) config_file: Mutex<()>,
    /// MCP sign-ins waiting for the browser, by folder and server.
    pub(crate) sign_ins: Mutex<HashMap<(PathBuf, String), servers::Waiting>>,
    /// One core per project directory, started when one of its sessions opens. Each has its own
    /// cell, so a folder that is slow to start holds up nothing but itself.
    cores: Mutex<HashMap<PathBuf, Arc<OnceCell<Running>>>>,
    pub(crate) login: Mutex<Option<DeviceLogin>>,
}

/// One key per folder however it is spelled: Windows hands the same folder over with either
/// case and with or without a trailing separator, and two cores on one folder would each run
/// its MCP servers and not see the other's turns.
fn key(directory: &str) -> PathBuf {
    key_for(directory, cfg!(windows))
}

fn key_for(directory: &str, windows: bool) -> PathBuf {
    let trimmed = directory.trim_end_matches(['/', '\\']);
    let trimmed = if trimmed.is_empty() || trimmed.ends_with(':') {
        directory
    } else {
        trimmed
    };
    if windows {
        PathBuf::from(trimmed.replace('/', "\\").to_lowercase())
    } else {
        PathBuf::from(trimmed)
    }
}

impl App {
    pub(crate) fn db(&self) -> Result<&Arc<Db>, String> {
        self.db
            .as_ref()
            .ok_or_else(|| format!("Can’t open {}.", Paths::from_env().database().display()))
    }

    /// A core reads the token when it starts: after signing in or out, start them afresh.
    pub(crate) async fn drop_cores(&self) {
        let cells: Vec<_> = self.cores.lock().await.drain().map(|(_, c)| c).collect();
        // Every folder's turns stop at once: their waits in `stop` overlap.
        for running in cells.iter().filter_map(|cell| cell.get()) {
            running.core.interrupt_all();
        }
        for cell in cells {
            if let Some(running) = cell.get() {
                running.stop().await;
            }
        }
    }

    /// The directory's core if one is running (deleting must not start one).
    pub(crate) async fn running(&self, directory: &str) -> Option<Core> {
        let cell = self.cores.lock().await.get(&key(directory)).cloned()?;
        cell.get().map(|running| running.core.clone())
    }

    /// Every core that has started, with its folder as it was first opened.
    pub(crate) async fn cores_running(&self) -> Vec<(String, Core)> {
        let cells: Vec<_> = self.cores.lock().await.values().cloned().collect();
        cells
            .iter()
            .filter_map(|cell| cell.get())
            .map(|running| {
                let core = running.core.clone();
                (core.directory().display().to_string(), core)
            })
            .collect()
    }

    pub(crate) fn prefs(&self) -> prefs::Prefs {
        self.prefs.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// `{message, folder}` when the app's own data can't be opened: signing in won't help, the
    /// window says so and offers the folder.
    pub(crate) fn fault(&self) -> Option<Value> {
        self.db_fault.as_ref()?;
        let database = self.paths.database();
        let folder = database.parent().unwrap_or(&database);
        Some(json!({
            "message": "Close any other copy of Anysola Code, then try again. If this keeps happening, the data folder may be damaged.",
            "folder": folder.display().to_string(),
        }))
    }
}

pub(crate) fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A core error in the words a desktop user reads (no terminal commands, no session ids).
pub(crate) fn plain_error(e: anysola_core::api::CoreError) -> String {
    events::core_error(&e).1
}

/// The directory's core, started on first use; its events go to the window from then on.
pub(crate) async fn core_for(
    handle: &AppHandle,
    app: &App,
    directory: &str,
) -> Result<Core, String> {
    Ok(mcp_for(handle, app, directory).await?.0)
}

/// [`core_for`] with the core's MCP runtime.
pub(crate) async fn mcp_for(
    handle: &AppHandle,
    app: &App,
    directory: &str,
) -> Result<(Core, Arc<anysola_mcp::Runtime>), String> {
    let key = key(directory);
    let cell = app
        .cores
        .lock()
        .await
        .entry(key.clone())
        .or_default()
        .clone();
    let running = cell
        .get_or_try_init(|| async {
            if !Path::new(directory).is_dir() {
                return Err(format!("{directory} is not a folder on this computer."));
            }
            let core = Core::start(Path::new(directory)).await.map_err(text)?;
            // A session's budget stops its turns here, woken ones included.
            core.stop_at_budget();
            let runtime = anysola_mcp::Runtime::attach(&core);
            let forward = forward(handle.clone(), &core, directory.to_string());
            Ok(Running {
                core,
                runtime,
                forward,
            })
        })
        .await?;
    // Signing in or out let the cores go while this one was starting: it has the old token.
    let kept = app
        .cores
        .lock()
        .await
        .get(&key)
        .is_some_and(|held| Arc::ptr_eq(held, &cell));
    if !kept {
        running.stop().await;
        return Err("The account changed while this folder was opening. Open it again.".into());
    }
    Ok((running.core.clone(), running.runtime.clone()))
}

fn forward(
    handle: AppHandle,
    core: &Core,
    directory: String,
) -> tauri::async_runtime::JoinHandle<()> {
    let mut events = core.subscribe();
    tauri::async_runtime::spawn(async move {
        // The last failure each MCP server was announced with: one notice per change into it.
        let mut announced: HashMap<String, &'static str> = HashMap::new();
        // Tool inputs being written: the window gets a snapshot of each at most once a tick.
        let mut live = live::Live::default();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Commands running in the foreground: what their output gained, a few times a second.
        let mut tails = output::Tails::default();
        let mut poll = tokio::time::interval(std::time::Duration::from_millis(250));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let received = tokio::select! {
                received = events.recv() => received,
                _ = poll.tick(), if tails.active() => {
                    for payload in tails.poll() {
                        let _ = handle.emit("core", payload);
                    }
                    continue;
                }
                _ = tick.tick(), if live.pending() => {
                    for payload in live.flush(&directory) {
                        let _ = handle.emit("core", payload);
                    }
                    continue;
                }
            };
            match &received {
                Ok(Event::ToolInputDelta {
                    session_id,
                    message_id,
                    call_id,
                    name,
                    at,
                    text,
                }) => {
                    live.piece(session_id, message_id, call_id, name, *at, text);
                    continue;
                }
                Ok(Event::ToolOutputFile {
                    session_id,
                    call_id,
                    file,
                }) => {
                    tails.start(session_id, call_id, file.clone());
                    continue;
                }
                Ok(Event::ToolCalled { call_id, .. }) => live.end_call(call_id),
                Ok(Event::ToolSucceeded { call_id, .. } | Event::ToolFailed { call_id, .. }) => {
                    tails.end_call(call_id)
                }
                Ok(
                    Event::StepEnded { session_id, .. }
                    | Event::StepFailed { session_id, .. }
                    | Event::RetryScheduled { session_id, .. }
                    | Event::Interrupted { session_id, .. }
                    | Event::Failed { session_id, .. }
                    | Event::Succeeded { session_id },
                ) => {
                    live.end_session(session_id);
                    // A step ends only once its calls settled; a turn's end leaves none running.
                    if !matches!(
                        received,
                        Ok(Event::StepEnded { .. } | Event::RetryScheduled { .. })
                    ) {
                        tails.end_session(session_id);
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    live.lagged();
                    // A call's end may be among the lost events; its tail would be read until
                    // the turn ended. The window re-reads the transcript, stored output included.
                    tails = output::Tails::default();
                }
                _ => {}
            }
            match received {
                Ok(Event::McpStatusChanged { server }) => {
                    if let Some(message) = mcp_notice(&mut announced, &server) {
                        let _ = handle.emit("core", json!({"type": "notice", "message": message}));
                    }
                    let _ = handle.emit("core", json!({"type": "mcp", "directory": directory}));
                }
                // A background command's session may be a subagent's: the window has its root open.
                Ok(Event::BackgroundChanged { session_id }) => {
                    let root = root_of(&handle, &session_id).unwrap_or(session_id);
                    let _ = handle.emit("core", json!({"type": "changed", "sessionId": root}));
                }
                Ok(event) => {
                    notify(&handle, &event);
                    if let Some(payload) = request_json(&handle, &event, &directory) {
                        let _ = handle.emit("core", payload);
                    }
                }
                // Events were dropped: the window re-reads what it shows from the database.
                Err(RecvError::Lagged(_)) => {
                    let _ = handle.emit("core", json!({"type": "lagged", "directory": directory}));
                }
                Err(RecvError::Closed) => return,
            }
        }
    })
}

/// An event as the window gets it; a subagent's ask names the session the user has open above
/// it (`rootId`), as live and as listed by `pending`.
fn request_json(handle: &AppHandle, event: &Event, directory: &str) -> Option<Value> {
    let mut payload = events::to_json(event, directory)?;
    if matches!(
        event,
        Event::PermissionAsked { .. } | Event::QuestionAsked { .. }
    ) && let Some(root) = root_of(handle, event.session_id())
    {
        payload["rootId"] = json!(root);
    }
    Some(payload)
}

/// The notice for a server that just failed or now needs a sign-in, once per change: the window
/// shows MCP servers only in Settings. A server that connects or is turned off is forgotten.
fn mcp_notice(
    announced: &mut HashMap<String, &'static str>,
    server: &anysola_core::mcp::Server,
) -> Option<String> {
    use anysola_core::mcp::Status;
    let state = match server.status {
        Status::Failed { .. } => "failed",
        Status::NeedsAuth { .. } => "auth",
        Status::Connected | Status::Disabled => {
            announced.remove(&server.name);
            return None;
        }
        // Retrying: it may come back or fail the same way again.
        Status::Pending => return None,
    };
    if announced.insert(server.name.clone(), state) == Some(state) {
        return None;
    }
    Some(match state {
        "failed" => format!("The MCP server “{}” couldn’t start.", server.name),
        _ => format!("The MCP server “{}” needs you to sign in.", server.name),
    })
}

/// The top session above a subagent's (itself for a root session).
fn root_of(handle: &AppHandle, session_id: &str) -> Option<String> {
    let db = handle.state::<App>().db().ok()?.clone();
    let mut id = session_id.to_string();
    // Subagents nest a few levels at most; the bound only guards a broken chain.
    for _ in 0..16 {
        match db.session(&id).ok()??.parent_id {
            Some(parent) => id = parent,
            None => return Some(id),
        }
    }
    None
}

/// A system notification when a session needs the user or ends while the window is in the
/// background. Subagents' sessions stay quiet: their parent speaks for them.
fn notify(handle: &AppHandle, event: &Event) {
    use tauri_plugin_notification::NotificationExt;
    let (session_id, what, on): (_, _, fn(&Notifications) -> bool) = match event {
        Event::PermissionAsked { session_id, .. } => {
            (session_id, "Needs your approval", |n: &Notifications| {
                n.approval
            })
        }
        // A form belongs to a server, not a session: the server names it.
        Event::FormAsked { form } => {
            let wanted = handle.state::<App>().prefs().notifications;
            let focused = handle
                .get_webview_window("main")
                .and_then(|w| w.is_focused().ok())
                .unwrap_or(false);
            if wanted.enabled && (wanted.approval || wanted.question) && !focused {
                let server = form.metadata["server"].as_str().unwrap_or("An MCP server");
                let _ = handle
                    .notification()
                    .builder()
                    .title(server)
                    .body("Asks for your input")
                    .show();
            }
            return;
        }
        Event::QuestionAsked { session_id, .. } => {
            (session_id, "Has a question for you", |n: &Notifications| {
                n.question
            })
        }
        Event::Succeeded { session_id } => (session_id, "Done", |n: &Notifications| n.finished),
        Event::Failed { session_id, .. } => {
            (session_id, "Stopped with an error", |n: &Notifications| {
                n.failed
            })
        }
        // Only a stop at the budget: the user's own stop and a declined call need no word, and
        // a repeat of a budget stop is silent in the transcript too.
        Event::Interrupted {
            session_id,
            reason: InterruptReason::Budget,
            repeat: false,
        } => (session_id, "Stopped at its budget.", |n: &Notifications| {
            n.finished
        }),
        _ => return,
    };
    let wanted = handle.state::<App>().prefs().notifications;
    if !(wanted.enabled && on(&wanted)) {
        return;
    }
    let focused = handle
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if focused {
        return;
    }
    let Ok(db) = handle.state::<App>().db().cloned() else {
        return;
    };
    let asks = matches!(
        event,
        Event::PermissionAsked { .. } | Event::QuestionAsked { .. }
    );
    let Ok(Some(row)) = db.session(session_id) else {
        return;
    };
    // A subagent's ask needs the user too, named by its root; its end does not.
    let row = match (&row.parent_id, asks) {
        (None, _) => row,
        (Some(_), true) => {
            match root_of(handle, session_id).and_then(|r| db.session(&r).ok().flatten()) {
                Some(root) => root,
                None => return,
            }
        }
        (Some(_), false) => return,
    };
    let _ = handle
        .notification()
        .builder()
        .title(&row.title)
        .body(what)
        .show();
}

/// `SessionRow` in bridge.ts. A session in its own worktree lists under its main folder (every
/// call routes by it); `worktree` says where it works.
pub(crate) fn session_json(row: &SessionRow, worktree: Option<&SessionWorktreeRow>) -> Value {
    let directory = worktree.map_or(row.directory.as_str(), |w| w.folder.as_str());
    json!({"id": row.id, "title": row.title, "directory": directory, "updated": row.time_updated,
           "forkOf": row.fork_session_id, "worktree": worktree.map(worktree::json)})
}

/// The session's model, its effort and every model to pick from, with the 5-hour window when
/// known. Without a session: the folder's default model.
pub(crate) fn info(core: &Core, row: Option<&SessionRow>) -> Value {
    let loaded = core.loaded();
    let reference = row
        .and_then(|r| r.model.as_ref())
        .and_then(ModelRef::from_json)
        .or_else(|| loaded.config.model().and_then(ModelRef::parse))
        .or_else(|| {
            loaded.models.first().map(|m| ModelRef {
                provider: PROVIDER.into(),
                id: m.id.clone(),
                variant: None,
            })
        });
    let current = reference
        .as_ref()
        .and_then(|r| loaded.models.iter().find(|m| m.id == r.id));
    let models: Vec<Value> = loaded
        .models
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "note": m.note,
                "efforts": m.variants.iter().map(|v| v.id.clone()).collect::<Vec<_>>(),
                "images": m.input.iter().any(|i| i == "image"),
            })
        })
        .collect();
    let agent = row
        .and_then(|r| r.agent.clone())
        .or_else(|| loaded.agents.default_agent().map(|a| a.id.clone()));
    let agents: Vec<Value> = loaded
        .agents
        .list
        .iter()
        .filter(|a| a.selectable())
        .map(|a| json!({"id": a.id, "name": a.name, "description": a.description}))
        .collect();
    json!({
        "agent": agent,
        "agents": agents,
        "model": current.map(|m| m.name.clone()),
        "modelId": current.map(|m| m.id.clone()),
        "variant": reference.and_then(|r| r.variant),
        "models": models,
        "fiveHour": row.and_then(|r| core.usage(&r.id).latest).map(|u| u.five_hour.percent),
    })
}

/// One session the list may not hold (it shows the latest [`SESSION_LIST`]): a fork's source.
/// Null when it is gone or a subagent's.
#[tauri::command]
async fn session_row(app: State<'_, App>, session_id: String) -> Answer {
    let db = app.db()?;
    let row = db.session(&session_id).map_err(text)?;
    Ok(row
        .filter(|r| r.parent_id.is_none())
        .map_or(Value::Null, |r| worktree::session_value(db, &r)))
}

#[tauri::command]
async fn sessions(app: State<'_, App>) -> Answer {
    // Worktrees left from the last run are settled first, so none lists as still being made.
    worktree::cleaned().await;
    let db = app.db()?;
    let rows = db.list_sessions(None, None, SESSION_LIST).map_err(text)?;
    let worktrees: HashMap<String, SessionWorktreeRow> = db
        .session_worktrees()
        .map_err(text)?
        .into_iter()
        .map(|w| (w.session_id.clone(), w))
        .collect();
    Ok(Value::Array(
        rows.iter()
            .map(|r| session_json(r, worktrees.get(&r.id)))
            .collect(),
    ))
}

#[tauri::command]
async fn open_session(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    worktree::cleaned().await;
    let core = core_for(&handle, &app, &directory).await?;
    // `open` would start an empty session under the id of one deleted elsewhere.
    let row = core
        .session(&session_id)
        .map_err(text)?
        .ok_or("This session no longer exists.")?;
    // A turn the app was closed under (or that crashed) shows as interrupted, not as still
    // running; one this or another process drives is left alone.
    for id in transcript::family(&core, &row.id) {
        if !core.is_busy(&id) {
            let _ = core.settle_interrupted(&id);
        }
    }
    Ok(json!({
        "session": worktree::session_value(app.db()?, &row),
        "view": transcript::view(&core, &row.id, &row.directory)?,
        "info": info(&core, Some(&row)),
        "busy": core.is_busy(&row.id),
        "pending": pending_of(&handle, &core, &row.id, &directory),
    }))
}

/// The transcript alone (with its revert and todos), re-read after a tool call or a step
/// changed it.
#[tauri::command]
async fn transcript(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    transcript::view(&core, &session_id, &directory)
}

/// The model and window for a folder with no session yet (a new session's composer).
#[tauri::command]
async fn folder_info(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    Ok(info(&core, None))
}

#[tauri::command]
async fn select_model(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    model: String,
    variant: Option<String>,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let reference = ModelRef {
        provider: PROVIDER.into(),
        id: model,
        variant,
    };
    core.select_model(&session_id, &reference)
        .map_err(plain_error)?;
    let row = core
        .session(&session_id)
        .map_err(text)?
        .ok_or("This session no longer exists.")?;
    Ok(info(&core, Some(&row)))
}

/// An image pasted or dropped on the composer, base64 encoded.
#[derive(Deserialize)]
struct Attachment {
    mime: String,
    data: String,
    name: String,
}

/// The composer's mode as the session's permission floor: Ask asks before changes, Edits
/// before commands, Auto leaves it to the rules. A mode this build doesn't know asks.
pub(crate) fn floor(mode: &str) -> Floor {
    match mode {
        "auto" => Floor::None,
        "edits" => Floor::Commands,
        _ => Floor::Changes,
    }
}

#[tauri::command]
async fn set_mode(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    mode: String,
) -> Answer {
    core_for(&handle, &app, &directory)
        .await?
        .set_permission_floor(&session_id, floor(&mode))
        .map_err(text)?;
    Ok(Value::Null)
}

/// What the composer sends; without a session it starts one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Send {
    session_id: Option<String>,
    text: String,
    mode: String,
    /// A new session's composer picks a model before the session exists.
    model: Option<String>,
    variant: Option<String>,
    /// A new session's agent, picked before it existed.
    agent: Option<String>,
    #[serde(default)]
    files: Vec<Attachment>,
    /// Skills mentioned as `@<name>`: their content goes before the text.
    #[serde(default)]
    skills: Vec<String>,
    /// A new session in a git folder: its own worktree (default) or the main folder.
    #[serde(default)]
    place: Option<String>,
}

/// A new session's settings from the composer: the session (created when there is none), its
/// permission floor, and for a new one the model and agent picked before it existed.
pub(crate) fn prepare(
    core: &Core,
    session_id: Option<String>,
    mode: &str,
    model: Option<String>,
    variant: Option<String>,
    agent: Option<String>,
) -> Result<String, String> {
    let id = match session_id {
        Some(id) => id,
        None => core.open(&Target::New, false).map_err(plain_error)?.0.id,
    };
    core.set_permission_floor(&id, floor(mode))
        .map_err(plain_error)?;
    if let Some(model) = model {
        let reference = ModelRef {
            provider: PROVIDER.into(),
            id: model,
            variant,
        };
        core.select_model(&id, &reference).map_err(plain_error)?;
    }
    if let Some(agent) = agent {
        let current = core.session(&id).map_err(text)?.and_then(|r| r.agent);
        if current.as_deref() != Some(agent.as_str()) {
            core.select_agent(&id, &agent).map_err(text)?;
        }
    }
    Ok(id)
}

/// Runs a turn in the background. A prompt the core refuses before any turn runs (an image it
/// can't take, a session another process drives) comes back as `refused`: no turn failed, so
/// the window offers no retry.
pub(crate) fn spawn_turn(handle: AppHandle, core: Core, session: String, prompt: Prompt) {
    tauri::async_runtime::spawn(async move {
        match core.prompt(&session, prompt, TurnOptions::default()).await {
            // Queued for a running turn, or what the turn left in the inbox: the window re-reads it.
            Ok(_) => changed(&handle, &session),
            Err(error) => refused_by(&handle, &session, &error),
        }
    });
}

/// The session's view changed outside a turn's events (its inbox, its background commands).
pub(crate) fn changed(handle: &AppHandle, session: &str) {
    let _ = handle.emit("core", json!({"type": "changed", "sessionId": session}));
}

/// Something the user asked for didn't run (as opposed to a turn that failed); `kind` is an
/// `ErrorKind`.
pub(crate) fn refused(handle: &AppHandle, session: &str, kind: &str, error: String) {
    let _ = handle.emit(
        "core",
        json!({"type": "refused", "sessionId": session, "error": error, "kind": kind}),
    );
}

/// [`refused`] for a core error, in plain words.
pub(crate) fn refused_by(handle: &AppHandle, session: &str, error: &anysola_core::api::CoreError) {
    let (kind, message) = events::core_error(error);
    refused(handle, session, kind, message);
}

/// A command or compaction ended: when no turn is left running, the window stops showing work
/// (a `/goal pause` runs no turn and sends no other event).
pub(crate) fn settled(handle: &AppHandle, core: &Core, session: &str) {
    if !core.is_busy(session) {
        let _ = handle.emit("core", json!({"type": "settled", "sessionId": session}));
    }
}

/// Sends a prompt. The turn runs on; its events tell the rest.
#[tauri::command]
async fn prompt(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    request: Send,
) -> Answer {
    let Send {
        session_id,
        text,
        mode,
        model,
        variant,
        agent,
        files,
        skills,
        place,
    } = request;
    let core = core_for(&handle, &app, &directory).await?;
    let new = session_id.is_none();
    let id = prepare(&core, session_id, &mode, model, variant, agent)?;
    // A new session in a git folder works in its own worktree unless asked otherwise: the
    // message waits until it is ready (`worktree` events say when).
    let isolated = new
        && place.as_deref() != Some("folder")
        && worktree::available(&app, &directory).await
        && worktree::start_new(&handle, &core, app.db()?, &id).await;
    let loaded = core.loaded();
    let skills = skills
        .iter()
        .filter_map(|name| {
            let skill = loaded
                .skills
                .iter()
                .find(|s| s.name == *name || s.id == *name)?;
            Some(InvokedSkill::new(skill, &format!("@{name}")))
        })
        .collect();
    let prompt = Prompt {
        text,
        files: files
            .into_iter()
            .map(|f| (f.mime, f.data, f.name))
            .collect(),
        skills,
        command: None,
    };
    // A running turn takes it when it ends: the core queues it for that turn, or starts one if
    // the turn ended meanwhile. Messages left from a stopped or failed turn run after it, in the
    // order `View.queued` shows them.
    let queued = isolated || core.is_busy(&id);
    spawn_turn(handle.clone(), core, id.clone(), prompt);
    Ok(json!({"sessionId": id, "queued": queued}))
}

#[tauri::command]
async fn interrupt(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    core_for(&handle, &app, &directory)
        .await?
        .interrupt(&session_id);
    Ok(Value::Null)
}

#[tauri::command]
async fn reply_permission(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    request_id: String,
    reply: String,
) -> Answer {
    let reply = match reply.as_str() {
        "once" => Reply::Once,
        "always" => Reply::Always,
        _ => Reply::Reject(None),
    };
    let answered = core_for(&handle, &app, &directory)
        .await?
        .reply_permission(&request_id, reply);
    Ok(json!(answered))
}

/// The model's questions: one answer list per question, or none to dismiss them.
#[tauri::command]
async fn reply_question(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    request_id: String,
    answers: Option<Vec<Vec<String>>>,
) -> Answer {
    let reply = match answers {
        Some(answers) => QuestionReply::Answered(answers),
        None => QuestionReply::Dismissed(None),
    };
    let answered = core_for(&handle, &app, &directory)
        .await?
        .reply_question(&request_id, reply);
    Ok(json!(answered))
}

/// Answers an MCP server's form: accepted with its values, declined, or cancelled.
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
enum FormAnswer {
    Accept {
        #[serde(default)]
        values: serde_json::Map<String, Value>,
    },
    Decline,
    Cancel,
}

impl FormAnswer {
    fn reply(self) -> FormReply {
        match self {
            FormAnswer::Accept { values } => FormReply::Answered(values),
            FormAnswer::Decline => FormReply::Declined,
            FormAnswer::Cancel => FormReply::Cancelled,
        }
    }
}

#[tauri::command]
async fn reply_form(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    form_id: String,
    answer: FormAnswer,
) -> Answer {
    let answered = core_for(&handle, &app, &directory)
        .await?
        .reply_form(&form_id, answer.reply());
    Ok(json!(answered))
}

/// What waits for the user in the session (a root), its subagents and the folder's MCP servers,
/// as the live events showed it.
fn pending_of(handle: &AppHandle, core: &Core, session_id: &str, directory: &str) -> Vec<Value> {
    core.pending_requests()
        .iter()
        .filter(|event| match event {
            Event::FormAsked { .. } => true,
            other => root_of(handle, other.session_id()).as_deref() == Some(session_id),
        })
        .filter_map(|event| request_json(handle, event, directory))
        .collect()
}

#[tauri::command]
async fn pending(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    Ok(json!(pending_of(&handle, &core, &session_id, &directory)))
}

/// Takes a waiting message out of the session's inbox.
#[tauri::command]
async fn queue_cancel(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let removed = core.cancel_pending(&id).map_err(text)?;
    changed(&handle, &session_id);
    // Promoted meanwhile: the turn has it, so the window must not put it back or call it removed.
    if !removed {
        return Err("That message already went to the session.".into());
    }
    transcript::view(&core, &session_id, &directory)
}

/// Hands a waiting message to the running turn at its next step.
#[tauri::command]
async fn queue_steer(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    // Switched only while a turn holds the session: with no turn to join, a steer would merge
    // into the next prompt's turn unseen.
    let steered = core.steer_pending(&session_id, &id).map_err(text)?;
    changed(&handle, &session_id);
    match steered {
        Steered::Taken => transcript::view(&core, &session_id, &directory),
        Steered::NoTurn => Err("The turn ended before it could take the message.".into()),
        Steered::Gone => Err("That message already went to the session.".into()),
    }
}

/// Runs what waits in the inbox when no turn does (after a stop or a failure), under `mode`.
#[tauri::command]
async fn queue_send(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    mode: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    core.set_permission_floor(&session_id, floor(&mode))
        .map_err(text)?;
    tauri::async_runtime::spawn(async move {
        match core.send_queued(&session_id, TurnOptions::default()).await {
            Ok(_) => {
                changed(&handle, &session_id);
                settled(&handle, &core, &session_id);
            }
            Err(error) => refused_by(&handle, &session_id, &error),
        }
    });
    Ok(Value::Null)
}

/// Stops a background command; false when it already ended.
#[tauri::command]
async fn background_stop(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    Ok(json!(core.kill_shell(&id)))
}

/// The root sessions with work running, over every folder: a turn of their own or of one of
/// their subagents (a background one goes on after its parent's turn ended).
async fn busy_roots(app: &App) -> usize {
    let mut roots = std::collections::HashSet::new();
    for (_, core) in app.cores_running().await {
        for id in core.busy_sessions() {
            roots.insert(root_session(&core, &id));
        }
    }
    roots.len()
}

/// The root of a session's subagent chain.
fn root_session(core: &Core, session_id: &str) -> String {
    let mut id = session_id.to_string();
    // Subagents nest a few levels at most; the bound only guards a broken chain.
    for _ in 0..16 {
        match core.session(&id) {
            Ok(Some(row)) => match row.parent_id {
                Some(parent) => id = parent,
                None => break,
            },
            _ => break,
        }
    }
    id
}

/// How long quitting waits for stopped turns to store their end.
const QUIT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// A second close within this long closes the app even if the window never answered the first.
const CLOSE_AGAIN: std::time::Duration = std::time::Duration::from_secs(4);

/// When the window was last asked to confirm a close.
static CLOSE_ASKED: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// What a close request does while work runs.
#[derive(Debug, PartialEq)]
enum Close {
    /// Ask the window to confirm.
    Ask,
    /// Part of the request just asked about (a double click): nothing more.
    Same,
    /// Asked again soon after: stop the work and close.
    Anyway,
}

/// Clicks closer than this are one request.
const CLOSE_GESTURE: std::time::Duration = std::time::Duration::from_millis(500);

fn close_again(asked: Option<std::time::Instant>, now: std::time::Instant) -> Close {
    match asked.map(|at| now.saturating_duration_since(at)) {
        Some(since) if since < CLOSE_GESTURE => Close::Same,
        Some(since) if since < CLOSE_AGAIN => Close::Anyway,
        _ => Close::Ask,
    }
}

/// The window answered the close question with "keep working": the next close asks again,
/// however soon it comes.
#[tauri::command]
fn keep_open() {
    *CLOSE_ASKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Closes the app even while turns run: they are stopped first, so each ends as "Stopped".
#[tauri::command]
async fn quit(handle: AppHandle, app: State<'_, App>) -> Answer {
    stop_and_exit(&handle, &app).await;
    Ok(Value::Null)
}

async fn stop_and_exit(handle: &AppHandle, app: &App) {
    let cores = app.cores_running().await;
    for (_, core) in &cores {
        core.interrupt_all();
    }
    let start = std::time::Instant::now();
    while cores.iter().any(|(_, core)| core.any_busy()) && start.elapsed() < QUIT_WAIT {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // The exit handler ends the MCP servers and background commands.
    handle.exit(0);
}

#[tauri::command]
async fn rename_session(app: State<'_, App>, session_id: String, title: String) -> Answer {
    let title = title.trim();
    if title.is_empty() {
        return Err("A session needs a name.".into());
    }
    app.db()?.set_title(&session_id, title).map_err(text)?;
    Ok(Value::Null)
}

#[tauri::command]
async fn delete_session(app: State<'_, App>, directory: String, session_id: String) -> Answer {
    let db = app.db()?.clone();
    // A running core also drops the session's usage it keeps in memory.
    let core = app.running(&directory).await;
    if core.as_ref().is_some_and(|core| core.is_busy(&session_id)) {
        return Err("Stop the session before deleting it.".into());
    }
    // Its worktree and branch go with it.
    worktree::before_delete(core.as_ref(), &db, &session_id).await?;
    let removed = match &core {
        Some(core) => core.delete_session(&session_id).map_err(text)?,
        None => db.delete_session(&session_id).map_err(text)?,
    };
    worktree::after_delete(&db, &session_id).await;
    Ok(json!(removed))
}

/// Shows one notification now, focused or not, so the user sees what they look like.
#[tauri::command]
fn notify_test(handle: AppHandle) -> Answer {
    use tauri_plugin_notification::NotificationExt;
    handle
        .notification()
        .builder()
        .title("Anysola Code")
        .body("Notifications are on.")
        .show()
        .map_err(|e| format!("The system didn’t show it: {e}"))?;
    Ok(Value::Null)
}

fn main() {
    // First, while this is the only thread: the PATH a terminal would have.
    path_repair::repair();
    anysola_core::version::set_desktop_client();
    let paths = Paths::from_env();
    // Core's log file, each line marked `role=desktop`; nothing on stdout.
    anysola_core::log::init_as(&paths, None, false, "desktop");
    path_repair::report();
    let (db, db_fault) = match Db::open(&paths.database()) {
        Ok(db) => (Some(Arc::new(db)), None),
        Err(error) => {
            anysola_core::log::warn("database failed to open", &[("error", &error.to_string())]);
            (None, Some(error.to_string()))
        }
    };
    let (prefs, prefs_invalid) = prefs::Prefs::read(&paths.desktop_prefs());
    anysola_core::skill::set_claude_skills(prefs.sources.claude_skills);
    tauri::Builder::default()
        // A second start shows the window that is open: two apps on one database would each
        // run the folders' MCP servers and miss each other's turns.
        .plugin(tauri_plugin_single_instance::init(|handle, _, _| {
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        // Size, position and maximized state come back on the next start.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(update::Pending::default())
        .manage(App {
            db,
            db_fault,
            paths,
            prefs: std::sync::RwLock::new(prefs),
            discord: std::sync::OnceLock::new(),
            prefs_invalid: prefs_invalid.into(),
            config_file: Mutex::default(),
            sign_ins: Mutex::default(),
            cores: Mutex::default(),
            login: Mutex::default(),
        })
        // Closing the window while turns run asks first; `quit` then stops them and exits. A
        // second close soon after does the same without the window, which may not be answering.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let (handle, window) = (window.app_handle().clone(), window.clone());
                tauri::async_runtime::spawn(async move {
                    let app = handle.state::<App>();
                    let busy = busy_roots(&app).await;
                    if busy == 0 {
                        let _ = window.destroy();
                        return;
                    }
                    let now = std::time::Instant::now();
                    let close = {
                        let mut asked = CLOSE_ASKED.lock().unwrap_or_else(|e| e.into_inner());
                        let close = close_again(*asked, now);
                        if close != Close::Same {
                            *asked = Some(now);
                        }
                        close
                    };
                    match close {
                        Close::Ask => {
                            let _ = handle
                                .emit("core", json!({"type": "closeRequested", "busy": busy}));
                        }
                        Close::Same => {}
                        Close::Anyway => stop_and_exit(&handle, &app).await,
                    }
                });
            }
        })
        .setup(|app| {
            let state = app.state::<App>();
            let wanted = state.prefs().discord;
            let _ = state.discord.set(discord::Handle::start(wanted));
            // Session worktrees left from the last run, settled in the background.
            worktree::cleanup_at_start(state.db.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            account::auth_status,
            account::login_start,
            account::login_wait,
            account::login_cancel,
            account::account,
            account::logout,
            update::update_check,
            update::update_install,
            sessions,
            open_session,
            transcript,
            folder_info,
            select_model,
            prompt,
            set_mode,
            interrupt,
            reply_permission,
            reply_question,
            reply_form,
            pending,
            queue_cancel,
            queue_steer,
            queue_send,
            background_stop,
            actions::message_images,
            quit,
            keep_open,
            rename_session,
            delete_session,
            actions::revert,
            actions::unrevert,
            actions::resend,
            actions::palette,
            actions::run_command,
            actions::compact,
            actions::select_agent,
            actions::project_files,
            actions::scratch_folder,
            actions::changes,
            actions::restore_file,
            worktree::worktree_info,
            worktree::worktree_unapplied,
            worktree::worktrees_with_work,
            worktree::worktree_apply_check,
            worktree::worktree_apply,
            worktree::worktree_discard,
            worktree::worktree_bring_base,
            worktree::worktree_recreate,
            worktree::worktree_to_folder,
            worktree::setup_output,
            worktree::setup_run,
            worktree::setup_stop,
            session_row,
            branch::fork_session,
            branch::set_budget,
            branch::session_usage,
            git::git_status,
            git::git_branch,
            git::git_commit,
            git::git_push,
            git::git_pull,
            git::git_identity,
            git::git_pr,
            git::suggest_commit_message,
            git::reveal,
            editor::editors,
            editor::open_in_editor,
            prefs::prefs_get,
            prefs::prefs_set,
            account::devices,
            account::device_revoke,
            account::usage,
            account::limits_redeem,
            config::config_get,
            config::config_set,
            servers::mcp_list,
            servers::mcp_add,
            servers::mcp_remove,
            servers::mcp_enable,
            servers::mcp_retry,
            servers::mcp_sign_in,
            servers::mcp_sign_in_wait,
            servers::mcp_sign_in_cancel,
            servers::mcp_sign_out,
            hooks::hooks,
            hooks::hook_add,
            hooks::hook_update,
            hooks::hook_remove,
            hooks::hooks_trust,
            hooks::hooks_disable,
            trust::trust,
            trust::trust_answer,
            sources::sources,
            sources::sources_retry,
            sources::personal_instructions,
            sources::save_personal_instructions,
            sources::reveal_path,
            data::permissions,
            data::permission_forget,
            data::permission_forget_all,
            data::data_info,
            data::data_clear,
            data::delete_all_sessions,
            data::export_session,
            data::about,
            notify_test,
            discord::discord_status,
            discord::presence,
        ])
        .build(tauri::generate_context!())
        .expect("the Anysola Code window failed to start")
        .run(|handle, event| {
            // Windows leaves a closed app's children running: background commands and MCP
            // servers would outlive the window.
            if let tauri::RunEvent::Exit = event {
                let app = handle.state::<App>();
                worktree::stop_all_setups();
                tauri::async_runtime::block_on(async {
                    let _ = tokio::time::timeout(EXIT_GRACE, app.drop_cores()).await;
                });
                // Nobody is working in Anysola Code once it is closed.
                if let Some(discord) = app.discord.get() {
                    discord.stop(EXIT_GRACE);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fork_names_its_source_in_the_list() {
        let mut row = SessionRow {
            id: "ses_2".into(),
            project_id: "p".into(),
            parent_id: None,
            fork_session_id: None,
            fork_boundary: None,
            slug: "s".into(),
            directory: "/work".into(),
            title: "Renamed".into(),
            version: "1".into(),
            agent: None,
            model: None,
            time_created: 1,
            time_updated: 2,
        };
        assert_eq!(session_json(&row, None)["forkOf"], Value::Null);
        row.fork_session_id = Some("ses_1".into());
        assert_eq!(
            session_json(&row, None),
            json!({"id": "ses_2", "title": "Renamed", "directory": "/work", "updated": 2, "forkOf": "ses_1",
                   "worktree": null})
        );
        // A worktree session lists under its main folder and says where it works.
        row.directory = "/data/worktrees/work-abc123/s".into();
        let worktree = SessionWorktreeRow {
            session_id: "ses_2".into(),
            project_id: "p".into(),
            folder: "/work".into(),
            directory: "/data/worktrees/work-abc123/s".into(),
            branch: "anysola/s".into(),
            base_branch: Some("main".into()),
            base_commit: "abc".into(),
            snapshot_dir: None,
            state: anysola_core::storage::WorktreeState::Ready,
            error: None,
            applied_commit: None,
            apply_from: None,
            apply_tree: None,
            restored: None,
            leftover: false,
            time_created: 1,
            time_updated: 2,
        };
        assert_eq!(
            session_json(&row, Some(&worktree)),
            json!({"id": "ses_2", "title": "Renamed", "directory": "/work", "updated": 2, "forkOf": "ses_1",
                   "worktree": {"path": "/data/worktrees/work-abc123/s", "branch": "anysola/s",
                                "baseBranch": "main", "baseCommit": "abc", "state": "ready", "error": null,
                                "appliedCommit": null, "restored": null, "leftover": false}})
        );
    }

    #[test]
    fn one_key_per_folder() {
        let windows = |d| key_for(d, true);
        assert_eq!(windows(r"C:\Work\Shop"), PathBuf::from(r"c:\work\shop"));
        assert_eq!(windows(r"c:\work\shop\"), windows(r"C:\Work\Shop"));
        assert_eq!(windows("C:/Work/Shop/"), windows(r"C:\Work\Shop"));
        assert_eq!(windows(r"C:\"), PathBuf::from(r"c:\"));
        assert_eq!(windows("C:/"), PathBuf::from(r"c:\"));
        assert_eq!(windows("C:"), PathBuf::from("c:"));
        assert_eq!(
            windows(r"\\Server\Share\Proj\"),
            PathBuf::from(r"\\server\share\proj")
        );
        assert_eq!(
            windows("//server/share/proj"),
            windows(r"\\SERVER\share\proj")
        );
        let unix = |d| key_for(d, false);
        assert_eq!(unix("/home/u/Shop/"), PathBuf::from("/home/u/Shop"));
        assert_ne!(unix("/home/u/Shop"), unix("/home/u/shop"));
        assert_eq!(unix("/"), PathBuf::from("/"));
    }

    #[test]
    fn a_second_close_soon_after_closes() {
        let now = std::time::Instant::now();
        let ago = |ms: u64| Some(now - std::time::Duration::from_millis(ms));
        assert_eq!(close_again(None, now), Close::Ask);
        assert_eq!(close_again(ago(100), now), Close::Same, "a double click");
        assert_eq!(close_again(ago(1_000), now), Close::Anyway);
        assert_eq!(close_again(ago(4_000), now), Close::Ask);
        assert_eq!(close_again(ago(60_000), now), Close::Ask);
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;
    use anysola_core::mcp::{Server, Status};

    #[test]
    fn mcp_failures_are_announced_once_per_change() {
        let server = |status| Server {
            name: "docs".into(),
            status,
            integration: None,
        };
        let failed = || {
            server(Status::Failed {
                error: "spawn failed".into(),
            })
        };
        let mut announced = HashMap::new();
        assert_eq!(
            mcp_notice(&mut announced, &failed()).as_deref(),
            Some("The MCP server “docs” couldn’t start.")
        );
        // A retry that fails the same way says nothing new.
        assert_eq!(mcp_notice(&mut announced, &server(Status::Pending)), None);
        assert_eq!(mcp_notice(&mut announced, &failed()), None);
        assert_eq!(
            mcp_notice(
                &mut announced,
                &server(Status::NeedsAuth {
                    error: "401".into()
                })
            )
            .as_deref(),
            Some("The MCP server “docs” needs you to sign in.")
        );
        assert_eq!(mcp_notice(&mut announced, &server(Status::Connected)), None);
        assert!(mcp_notice(&mut announced, &failed()).is_some());
    }

    #[test]
    fn form_answers_become_three_different_replies() {
        let reply = |answer: Value| {
            serde_json::from_value::<FormAnswer>(answer)
                .unwrap()
                .reply()
        };
        assert_eq!(
            reply(json!({"action": "accept", "values": {"env": "prod", "n": 2, "tags": ["a"]}})),
            FormReply::Answered(
                json!({"env": "prod", "n": 2, "tags": ["a"]})
                    .as_object()
                    .unwrap()
                    .clone()
            )
        );
        assert_eq!(reply(json!({"action": "decline"})), FormReply::Declined);
        assert_eq!(reply(json!({"action": "cancel"})), FormReply::Cancelled);
    }
}
