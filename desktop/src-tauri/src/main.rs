//! Anysola Code for the desktop: the agent runs in this process (the core crate, as the TUI and
//! `anysola acp` run it) and the window drives it through these commands. Sessions live in the
//! same database as the CLI's, so either can continue the other's.

// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod events;
mod transcript;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::gateway::account::DeviceLogin;
use anysola_core::gateway::{Gateway, Token};
use anysola_core::model::{ModelRef, PROVIDER};
use anysola_core::paths::Paths;
use anysola_core::session::TurnOptions;
use anysola_core::session::gate::{Floor, Reply};
use anysola_core::session::inbox::Delivery;
use anysola_core::storage::{Db, SessionRow};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;

type Answer = Result<Value, String>;

/// Sessions of every project, newest first, for the sidebar.
const SESSION_LIST: u64 = 200;

struct App {
    db: Option<Arc<Db>>,
    /// One core per project directory, started when one of its sessions opens.
    cores: Mutex<HashMap<PathBuf, Core>>,
    login: Mutex<Option<DeviceLogin>>,
}

impl App {
    fn db(&self) -> Result<&Arc<Db>, String> {
        self.db
            .as_ref()
            .ok_or_else(|| format!("Can’t open {}.", Paths::from_env().database().display()))
    }
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The directory's core, started on first use; its events go to the window from then on.
async fn core_for(handle: &AppHandle, app: &App, directory: &str) -> Result<Core, String> {
    let mut cores = app.cores.lock().await;
    if let Some(core) = cores.get(Path::new(directory)) {
        return Ok(core.clone());
    }
    if !Path::new(directory).is_dir() {
        return Err(format!("{directory} is not a folder on this computer."));
    }
    let core = Core::start(Path::new(directory)).await.map_err(text)?;
    anysola_mcp::Runtime::attach(&core);
    forward(handle.clone(), &core, directory.to_string());
    cores.insert(PathBuf::from(directory), core.clone());
    Ok(core)
}

fn forward(handle: AppHandle, core: &Core, directory: String) {
    let mut events = core.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Some(payload) = events::to_json(&event, &directory) {
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
    });
}

fn session_json(row: &SessionRow) -> Value {
    json!({"id": row.id, "title": row.title, "directory": row.directory, "updated": row.time_updated})
}

/// The session's model as the composer shows it, and the 5-hour window when known.
fn info(core: &Core, row: &SessionRow) -> Value {
    let loaded = core.loaded();
    let reference = row
        .model
        .as_ref()
        .and_then(ModelRef::from_json)
        .or_else(|| loaded.config.model().and_then(ModelRef::parse))
        .or_else(|| {
            loaded.models.first().map(|m| ModelRef {
                provider: PROVIDER.into(),
                id: m.id.clone(),
                variant: None,
            })
        });
    let model = reference
        .as_ref()
        .and_then(|r| loaded.models.iter().find(|m| m.id == r.id));
    let usage = core.usage(&row.id);
    json!({
        "model": model.map(|m| m.name.clone()),
        "variant": reference.and_then(|r| r.variant),
        "fiveHour": usage.latest.as_ref().map(|u| u.five_hour.percent),
    })
}

#[tauri::command]
async fn auth_status(app: State<'_, App>) -> Answer {
    let token = Token::resolve(app.db()?).map_err(text)?;
    Ok(json!({"signedIn": token.is_some()}))
}

/// Starts the device sign-in: the window shows the code and opens the address.
#[tauri::command]
async fn login_start(app: State<'_, App>) -> Answer {
    let login = Gateway::new(None).start_device_login().await?;
    let started = json!({"url": login.url, "code": login.user_code});
    *app.login.lock().await = Some(login);
    Ok(started)
}

/// Waits for the browser approval and stores the token.
#[tauri::command]
async fn login_wait(app: State<'_, App>) -> Answer {
    let login = app
        .login
        .lock()
        .await
        .take()
        .ok_or("No sign-in is in progress.")?;
    let credential = Gateway::new(None).finish_device_login(&login).await?;
    app.db()?.insert_anysola_login(&credential).map_err(text)?;
    // A core reads the token when it starts: drop any started before this sign-in.
    for (_, core) in app.cores.lock().await.drain() {
        core.shutdown();
    }
    Ok(json!({"signedIn": true}))
}

#[tauri::command]
async fn sessions(app: State<'_, App>) -> Answer {
    let rows = app
        .db()?
        .list_sessions(None, None, SESSION_LIST)
        .map_err(text)?;
    Ok(Value::Array(rows.iter().map(session_json).collect()))
}

#[tauri::command]
async fn open_session(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let (row, _) = core.open(&Target::Id(session_id), false).map_err(text)?;
    let messages = core.messages(&row.id).map_err(text)?;
    Ok(json!({
        "session": session_json(&row),
        "items": transcript::items(messages, &row.directory),
        "info": info(&core, &row),
        "busy": core.is_busy(&row.id),
    }))
}

/// The transcript alone, re-read after a tool call or a step changed it.
#[tauri::command]
async fn transcript(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let messages = core.messages(&session_id).map_err(text)?;
    Ok(Value::Array(transcript::items(messages, &directory)))
}

/// The model and window for a folder with no session yet (a new session's composer).
#[tauri::command]
async fn folder_info(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let loaded = core.loaded();
    let model = loaded
        .config
        .model()
        .and_then(ModelRef::parse)
        .and_then(|r| loaded.models.iter().find(|m| m.id == r.id).cloned())
        .or_else(|| loaded.models.first().cloned());
    Ok(json!({"model": model.map(|m| m.name), "variant": null, "fiveHour": null}))
}

/// The composer's mode as the session's permission floor: Ask asks before changes, Edits
/// before commands, Auto leaves it to the rules.
fn floor(mode: &str) -> Floor {
    match mode {
        "ask" => Floor::Changes,
        "edits" => Floor::Commands,
        _ => Floor::None,
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

/// Sends a prompt; without a session it starts one. The turn runs on; its events tell the rest.
#[tauri::command]
async fn prompt(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: Option<String>,
    text: String,
    mode: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let id = match session_id {
        Some(id) => id,
        None => core.open(&Target::New, false).map_err(self::text)?.0.id,
    };
    core.set_permission_floor(&id, floor(&mode))
        .map_err(self::text)?;
    let prompt = Prompt {
        text,
        files: Vec::new(),
        skills: Vec::new(),
    };
    if core.is_busy(&id) {
        // A running turn takes it when it ends.
        core.enqueue(&id, prompt, Delivery::Queue)
            .map_err(self::text)?;
        return Ok(json!({"sessionId": id, "queued": true}));
    }
    let (session, events) = (id.clone(), handle.clone());
    tauri::async_runtime::spawn(async move {
        if let Err(error) = core.prompt(&session, prompt, TurnOptions::default()).await {
            let _ = events.emit(
                "core",
                json!({"type": "failed", "sessionId": session, "error": error.to_string()}),
            );
        }
    });
    Ok(json!({"sessionId": id, "queued": false}))
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

fn main() {
    anysola_core::version::set_desktop_client();
    let db = Db::open(&Paths::from_env().database()).ok().map(Arc::new);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(App {
            db,
            cores: Mutex::default(),
            login: Mutex::default(),
        })
        .invoke_handler(tauri::generate_handler![
            auth_status,
            login_start,
            login_wait,
            sessions,
            open_session,
            transcript,
            folder_info,
            prompt,
            set_mode,
            interrupt,
            reply_permission,
        ])
        .run(tauri::generate_context!())
        .expect("the Anysola Code window failed to start");
}
