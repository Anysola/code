//! Anysola Code for the desktop: the agent runs in this process (the core crate, as the TUI and
//! `anysola acp` run it) and the window drives it through these commands. Sessions live in the
//! same database as the CLI's, so either can continue the other's.

// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod events;
mod transcript;
mod update;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::gateway::account::DeviceLogin;
use anysola_core::model::{ModelRef, PROVIDER};
use anysola_core::paths::Paths;
use anysola_core::session::TurnOptions;
use anysola_core::session::gate::{Floor, Reply};
use anysola_core::session::inbox::Delivery;
use anysola_core::storage::{Db, SessionRow};
use anysola_core::tool::question::QuestionReply;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;

pub(crate) type Answer = Result<Value, String>;

/// Sessions of every project, newest first, for the sidebar.
const SESSION_LIST: u64 = 200;

pub(crate) struct App {
    db: Option<Arc<Db>>,
    /// One core per project directory, started when one of its sessions opens.
    cores: Mutex<HashMap<PathBuf, Core>>,
    pub(crate) login: Mutex<Option<DeviceLogin>>,
}

impl App {
    pub(crate) fn db(&self) -> Result<&Arc<Db>, String> {
        self.db
            .as_ref()
            .ok_or_else(|| format!("Can’t open {}.", Paths::from_env().database().display()))
    }

    /// A core reads the token when it starts: after signing in or out, start them afresh.
    pub(crate) async fn drop_cores(&self) {
        for (_, core) in self.cores.lock().await.drain() {
            core.shutdown();
        }
    }

    /// The directory's core if one is running (deleting must not start one).
    async fn running(&self, directory: &str) -> Option<Core> {
        self.cores.lock().await.get(Path::new(directory)).cloned()
    }
}

pub(crate) fn text(e: impl std::fmt::Display) -> String {
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

/// The session's model, its effort and every model to pick from, with the 5-hour window when
/// known. Without a session: the folder's default model.
fn info(core: &Core, row: Option<&SessionRow>) -> Value {
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
    json!({
        "model": current.map(|m| m.name.clone()),
        "modelId": current.map(|m| m.id.clone()),
        "variant": reference.and_then(|r| r.variant),
        "models": models,
        "fiveHour": row.and_then(|r| core.usage(&r.id).latest).map(|u| u.five_hour.percent),
    })
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
        "info": info(&core, Some(&row)),
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
    core.select_model(&session_id, &reference).map_err(text)?;
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
    #[serde(default)]
    files: Vec<Attachment>,
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
        files,
    } = request;
    let core = core_for(&handle, &app, &directory).await?;
    let id = match session_id {
        Some(id) => id,
        None => core.open(&Target::New, false).map_err(self::text)?.0.id,
    };
    core.set_permission_floor(&id, floor(&mode))
        .map_err(self::text)?;
    if let Some(model) = model {
        let reference = ModelRef {
            provider: PROVIDER.into(),
            id: model,
            variant,
        };
        core.select_model(&id, &reference).map_err(self::text)?;
    }
    let prompt = Prompt {
        text,
        files: files
            .into_iter()
            .map(|f| (f.mime, f.data, f.name))
            .collect(),
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
    if let Some(core) = app.running(&directory).await
        && core.is_busy(&session_id)
    {
        return Err("Stop the session before deleting it.".into());
    }
    Ok(json!(app.db()?.delete_session(&session_id).map_err(text)?))
}

fn main() {
    anysola_core::version::set_desktop_client();
    let db = Db::open(&Paths::from_env().database()).ok().map(Arc::new);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(update::Pending::default())
        .manage(App {
            db,
            cores: Mutex::default(),
            login: Mutex::default(),
        })
        .invoke_handler(tauri::generate_handler![
            account::auth_status,
            account::login_start,
            account::login_wait,
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
            rename_session,
            delete_session,
        ])
        .run(tauri::generate_context!())
        .expect("the Anysola Code window failed to start");
}
