//! What the composer and the transcript do beyond sending a prompt: rewinding to a message and
//! sending it again, slash commands, skills and files for the composer's menus, the session's
//! agent, and the folder for sessions without a project.

use std::time::Duration;

use anysola_core::api::{Core, Prompt};
use anysola_core::events::Event;
use anysola_core::paths::Paths;
use anysola_core::session::message::{InvokedSkill, Message, UserData};
use anysola_core::session::{Outcome, TurnOptions};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use tokio::sync::broadcast::error::TryRecvError;

use crate::{
    Answer, App, core_for, info, prepare, refused, refused_by, settled, spawn_turn, text,
    transcript,
};

/// Files listed for `@`; a larger tree is cut here (the menu filters this list as you type).
const FILE_LIST: usize = 20_000;

/// How long a rewind waits for the turn it interrupted to store its last step.
const STOP_WAIT: Duration = Duration::from_secs(15);

/// Interrupts a running turn and waits for it to end: a revert must not race the turn's last
/// step.
async fn stop_and_wait(core: &Core, session_id: &str) -> Result<(), String> {
    if !core.is_busy(session_id) {
        return Ok(());
    }
    core.interrupt(session_id);
    let start = std::time::Instant::now();
    while core.is_busy(session_id) {
        if start.elapsed() > STOP_WAIT {
            return Err("The session is still stopping. Try again in a moment.".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

fn user_message(core: &Core, session_id: &str, message_id: &str) -> Result<UserData, String> {
    core.messages(session_id)
        .map_err(text)?
        .into_iter()
        .find_map(|(id, m)| match m {
            Message::User(data) if id == message_id => Some(data),
            _ => None,
        })
        .ok_or_else(|| "That message is no longer in the session.".into())
}

/// Rewinds the session to before `message_id`: the turns from there are hidden and the files
/// they changed put back, until the next prompt makes it final or `unrevert` undoes it.
/// Returns the message's text for the composer.
#[tauri::command]
pub async fn revert(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    message_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    stop_and_wait(&core, &session_id).await?;
    let data = user_message(&core, &session_id, &message_id)?;
    core.revert(&session_id, &message_id).map_err(text)?;
    // A command comes back as the line the user typed, to run again.
    let typed = data
        .extra
        .get("command")
        .and_then(Value::as_str)
        .map_or(data.text, str::to_string);
    Ok(json!({
        "text": typed,
        "view": transcript::view(&core, &session_id, &directory)?,
        // Outside git nothing was snapshotted: the turns hide but their edits stay.
        "filesKept": !core.restores_files(&session_id),
    }))
}

/// Puts back what the staged rewind hid.
#[tauri::command]
pub async fn unrevert(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    core.unrevert(&session_id).map_err(text)?;
    transcript::view(&core, &session_id, &directory)
}

/// Sends a message again from where it was: rewinds to before it and prompts with `text` (the
/// original when absent: a retry), keeping its images and skills.
#[tauri::command]
pub async fn resend(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    message_id: String,
    text: Option<String>,
    mode: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    stop_and_wait(&core, &session_id).await?;
    let data = user_message(&core, &session_id, &message_id)?;
    let files = data
        .files
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            let name = f.name.unwrap_or_else(|| "image".into());
            (f.mime, f.data, name)
        })
        .collect();
    let command = data
        .extra
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);
    let skills: Vec<InvokedSkill> = data
        .extra
        .get("skills")
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .unwrap_or_default();
    let text = text.unwrap_or(data.text);
    core.revert(&session_id, &message_id).map_err(self::text)?;
    let id = prepare(&core, Some(session_id), &mode, None, None, None)?;
    // The prompt makes the rewind final.
    spawn_turn(
        handle.clone(),
        core,
        id.clone(),
        Prompt {
            text,
            files,
            skills,
            command,
        },
    );
    Ok(json!({"sessionId": id}))
}

/// A sent message's images as data URLs, in the order they were attached.
#[tauri::command]
pub async fn message_images(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    message_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let data = user_message(&core, &session_id, &message_id)?;
    Ok(json!(images(&data)))
}

fn images(data: &UserData) -> Vec<String> {
    data.files
        .iter()
        .flatten()
        .filter(|f| f.mime.starts_with("image/"))
        .map(|f| format!("data:{};base64,{}", f.mime, f.data))
        .collect()
}

/// What the session changed in its files, from before its first change to now; `null`
/// outside git, where the window sums the edits instead.
#[tauri::command]
pub async fn changes(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    // A worktree session: its worktree against its base, git-based (commands' changes too).
    if let Some(found) = crate::worktree::changes(app.db()?, &session_id).await? {
        return Ok(found);
    }
    let core = core_for(&handle, &app, &directory).await?;
    let found = tauri::async_runtime::spawn_blocking(move || core.session_changes(&session_id))
        .await
        .map_err(text)?
        .map_err(text)?;
    Ok(json!(found))
}

/// Puts one file back as it was before the session first changed it.
#[tauri::command]
pub async fn restore_file(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    file: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    if crate::worktree::restore(&core, app.db()?, &session_id, &file).await? {
        return Ok(Value::Null);
    }
    if core.is_busy(&session_id) {
        return Err("Stop the session before putting files back.".into());
    }
    core.restore_file(&session_id, &file).map_err(text)?;
    Ok(Value::Null)
}

/// What the composer's `/` and `@` menus list: commands (built-in, the project's and MCP
/// prompts) and skills.
#[tauri::command]
pub async fn palette(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let commands: Vec<Value> = core
        .commands()
        .list()
        .iter()
        .map(|c| json!({"name": c.name, "description": c.description}))
        .collect();
    let skills: Vec<Value> = core
        .loaded()
        .skills
        .iter()
        .map(|s| json!({"name": s.name, "description": s.description}))
        .collect();
    Ok(json!({"commands": commands, "skills": skills}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRequest {
    session_id: Option<String>,
    name: String,
    arguments: String,
    mode: String,
    model: Option<String>,
    variant: Option<String>,
    agent: Option<String>,
}

/// Runs `/<name> <arguments>` in the session (a new one when there is none); the turn's events
/// tell the rest. A command that can't run (a bad `/goal`, a failing template) says why.
#[tauri::command]
pub async fn run_command(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    request: CommandRequest,
) -> Answer {
    let CommandRequest {
        session_id,
        name,
        arguments,
        mode,
        model,
        variant,
        agent,
    } = request;
    let core = core_for(&handle, &app, &directory).await?;
    if core.commands().get(&name).is_none() {
        return Err(format!("There’s no /{name} command here."));
    }
    let id = prepare(&core, session_id, &mode, model, variant, agent)?;
    if core.is_busy(&id) {
        return Err(
            "Wait for the current turn to end, or stop it, before running a command.".into(),
        );
    }
    let session = id.clone();
    tauri::async_runtime::spawn(async move {
        // Whether a turn ran: one that failed has said so itself; a command that never got to
        // one (a bad `/goal`, a failing template) has not.
        let mut events = core.subscribe();
        let outcome = core
            .command(&session, &name, &arguments, TurnOptions::default())
            .await;
        let mut ran = false;
        loop {
            match events.try_recv() {
                Ok(Event::Started { session_id }) if session_id == session => ran = true,
                Ok(_) => {}
                // Dropped events: that many came from a turn.
                Err(TryRecvError::Lagged(_)) => ran = true,
                Err(_) => break,
            }
        }
        match outcome {
            Ok(Outcome::Failed(error)) if !ran => {
                let (kind, _) = crate::events::error_kind(&error.kind, &error.message);
                refused(
                    &handle,
                    &session,
                    kind,
                    crate::events::plain(kind, &error.message),
                );
            }
            Ok(_) => settled(&handle, &core, &session),
            Err(error) => refused_by(&handle, &session, &error),
        }
    });
    Ok(json!({"sessionId": id}))
}

/// `/compact`: summarizes the session so far so its next turns start from the summary.
#[tauri::command]
pub async fn compact(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    if core.is_busy(&session_id) {
        return Err("Wait for the current turn to end, or stop it, before compacting.".into());
    }
    tauri::async_runtime::spawn(async move {
        match core.compact(&session_id).await {
            Ok(_) => settled(&handle, &core, &session_id),
            Err(error) => refused_by(&handle, &session_id, &error),
        }
    });
    Ok(Value::Null)
}

/// The agent for the session's next turns (Build, Plan, or one the project defines).
#[tauri::command]
pub async fn select_agent(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
    agent: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let row = core
        .session(&session_id)
        .map_err(text)?
        .ok_or("This session no longer exists.")?;
    if row.agent.as_deref() != Some(agent.as_str()) {
        core.select_agent(&session_id, &agent).map_err(text)?;
    }
    let row = core.session(&session_id).map_err(text)?;
    Ok(info(&core, row.as_ref()))
}

/// The folder's files and directories (directories end in `/`), relative and ignore-aware, for
/// the `@` menu.
#[tauri::command]
pub async fn project_files(directory: String) -> Answer {
    let list = tauri::async_runtime::spawn_blocking(move || walk(&directory, FILE_LIST))
        .await
        .map_err(text)?;
    Ok(json!(list))
}

fn walk(root: &str, limit: usize) -> Vec<String> {
    let root = std::path::Path::new(root);
    let mut out = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .hidden(true)
        .build()
        .flatten()
    {
        if out.len() >= limit {
            break;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let mut path = relative.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            path.push('/');
        }
        out.push(path);
    }
    out
}

/// Where sessions without a project run: one folder of the app's own, created on first use.
#[tauri::command]
pub async fn scratch_folder() -> Answer {
    let folder = Paths::from_env().data.join("no-project");
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Can’t create {}: {e}", folder.display()))?;
    Ok(json!(folder.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::walk;

    #[test]
    fn images_come_back_as_data_urls_in_order() {
        let data: super::UserData = serde_json::from_value(serde_json::json!({
            "time": {"created": 1},
            "text": "look",
            "files": [
                {"mime": "image/png", "data": "AAA", "source": {"type": "inline"}, "name": "a.png"},
                {"mime": "text/plain", "data": "BBB", "source": {"type": "inline"}},
                {"mime": "image/jpeg", "data": "CCC", "source": {"type": "inline"}},
            ],
        }))
        .unwrap();
        assert_eq!(
            super::images(&data),
            ["data:image/png;base64,AAA", "data:image/jpeg;base64,CCC"]
        );
    }

    #[test]
    fn walk_lists_files_and_directories_relative_and_ignores_hidden() {
        let dir = tempfile_dir();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "").unwrap();
        std::fs::write(dir.join(".secret"), "").unwrap();
        let mut list = walk(dir.to_str().unwrap(), 100);
        list.sort();
        assert_eq!(list, ["src/", "src/a.rs"]);
        assert_eq!(walk(dir.to_str().unwrap(), 1).len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("anysola-walk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
