//! MCP servers in Settings: the folder's merged servers with their live status, and adding,
//! removing, turning off and signing in. Servers of the global config file are changed in that
//! file (as `anysola mcp add --global` does); others only for this run of the app.
//!
//! Nothing here waits for a server to connect: the list comes back as it stands and `mcp` events
//! tell the window when a status changes.

use std::sync::Arc;

use anysola_core::api::Core;
use anysola_core::config;
use anysola_core::mcp::{Mcp, Status};
use anysola_mcp::{Runtime, oauth};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, State};
use tokio::sync::{Notify, oneshot};

use crate::{Answer, App, key, mcp_for, text};

/// A sign-in waiting for the browser; a newer one for the same server ends its wait.
pub struct Waiting {
    done: Option<oneshot::Receiver<Result<(), String>>>,
    replaced: Arc<Notify>,
}

/// What the window's add form sends.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Draft {
    Remote {
        name: String,
        url: String,
        #[serde(default)]
        headers: Map<String, Value>,
    },
    Local {
        name: String,
        command: Vec<String>,
        #[serde(default)]
        environment: Map<String, Value>,
    },
}

fn status(status: &Status) -> (&'static str, Option<&str>) {
    match status {
        Status::Connected => ("connected", None),
        Status::Pending => ("pending", None),
        Status::Disabled => ("disabled", None),
        Status::Failed { error } => ("failed", Some(error)),
        Status::NeedsAuth { error } => ("needs_auth", Some(error)),
    }
}

/// The address, or the command line.
fn target(server: &Value) -> String {
    match server["url"].as_str() {
        Some(url) => url.to_string(),
        None => server["command"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|part| {
                if part.contains(' ') {
                    format!("\"{part}\"")
                } else {
                    part.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn list(app: &App, core: &Core, runtime: &Runtime) -> Answer {
    let loaded = core.loaded();
    let global = app.paths.config_file().display().to_string();
    let prompts = runtime.prompts();
    let servers: Vec<Value> = runtime
        .servers()
        .iter()
        .map(|server| {
            let entry = runtime.config(&server.name).unwrap_or(Value::Null);
            let (state, error) = status(&server.status);
            let source = loaded.config.mcp_source(&server.name).unwrap_or_default();
            let signed_in = server.integration.as_deref().is_some_and(|integration| {
                core.db()
                    .active_credential(integration)
                    .ok()
                    .flatten()
                    .is_some()
            });
            json!({
                "name": server.name,
                "kind": if entry["type"] == "local" { "local" } else { "remote" },
                "target": target(&entry),
                "status": state,
                "error": error,
                "source": source,
                "editable": source == global,
                "oauth": server.integration.is_some(),
                "signedIn": signed_in,
                "tools": runtime.tool_count(&server.name),
                "prompts": prompts.iter().filter(|p| p.server == server.name).count(),
            })
        })
        .collect();
    Ok(Value::Array(servers))
}

/// Where a server of the global file is: `mcp.servers.<name>`, or the 0.5.x `mcp.<name>`.
fn entry_path<'a>(data: &Map<String, Value>, name: &'a str) -> Vec<&'a str> {
    let legacy = data
        .get("mcp")
        .and_then(|m| m.get(name))
        .is_some_and(|s| s.get("type").is_some());
    let native = data
        .get("mcp")
        .and_then(|m| m.get("servers"))
        .and_then(|s| s.get(name))
        .is_some();
    if legacy && !native {
        vec!["mcp", name]
    } else {
        vec!["mcp", "servers", name]
    }
}

/// A server of the global file, or why it can't be changed here.
fn editable(app: &App, core: &Core, name: &str) -> Result<(), String> {
    let global = app.paths.config_file().display().to_string();
    match core.loaded().config.mcp_source(name) {
        Some(source) if source == global => Ok(()),
        Some(source) => Err(format!("“{name}” is defined in {source}. Change it there.")),
        None => Err(format!("There’s no MCP server named “{name}”.")),
    }
}

/// Edits the global file, then has the core take it at once.
async fn edit(
    app: &App,
    core: &Core,
    change: impl FnOnce(&str, &Map<String, Value>) -> Result<String, String>,
) -> Result<(), String> {
    let file = app.paths.config_file();
    // One read-change-write of the global file at a time, with `config_set` too.
    let _writing = app.config_file.lock().await;
    let (before, data) = crate::config::read(&app.paths)?;
    let after = change(&before, &data)?;
    if after != before {
        crate::config::write(&file, &after)?;
    }
    core.reload().await;
    Ok(())
}

fn find(runtime: &Runtime, name: &str) -> Result<(), String> {
    runtime
        .servers()
        .iter()
        .any(|s| s.name == name)
        .then_some(())
        .ok_or_else(|| format!("There’s no MCP server named “{name}”."))
}

#[tauri::command]
pub async fn mcp_list(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let (core, runtime) = mcp_for(&handle, &app, &directory).await?;
    list(&app, &core, &runtime)
}

/// Adds a server to the global file, as `anysola mcp add --global` writes it.
#[tauri::command]
pub async fn mcp_add(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    server: Draft,
) -> Answer {
    let (core, runtime) = mcp_for(&handle, &app, &directory).await?;
    let (name, entry) = match server {
        Draft::Remote { name, url, headers } => (name, config::remote_server(url.trim(), headers)?),
        Draft::Local {
            name,
            command,
            environment,
        } => {
            let command: Vec<String> = command
                .into_iter()
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect();
            (name, config::local_server(command, environment)?)
        }
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("A server needs a name.".into());
    }
    if name.contains(['/', '\\', ':']) || name.chars().any(char::is_whitespace) {
        return Err("Use letters, digits, - and _ in a server’s name.".into());
    }
    if core.loaded().config.mcp_source(&name).is_some() {
        return Err(format!("There’s already a server named “{name}”."));
    }
    edit(&app, &core, |text, _| {
        config::jsonc::set(text, &["mcp", "servers", &name], entry)
    })
    .await?;
    list(&app, &core, &runtime)
}

/// Removes a server of the global file and its stored sign-in.
#[tauri::command]
pub async fn mcp_remove(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    name: String,
) -> Answer {
    let (core, runtime) = mcp_for(&handle, &app, &directory).await?;
    editable(&app, &core, &name)?;
    let entry = runtime.config(&name);
    edit(&app, &core, |text, data| {
        Ok(config::jsonc::remove(text, &entry_path(data, &name)))
    })
    .await?;
    if let Some(entry) = entry.filter(oauth::capable) {
        let url = entry["url"].as_str().unwrap_or_default();
        core.db()
            .remove_integration_credentials(&oauth::integration_id(&name, url))
            .map_err(text)?;
    }
    app.sign_ins.lock().await.remove(&(key(&directory), name));
    list(&app, &core, &runtime)
}

/// Turns a server on or off: kept in the global file for its own servers, for this run of the
/// app for the others.
#[tauri::command]
pub async fn mcp_enable(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    name: String,
    enabled: bool,
) -> Answer {
    let (core, runtime) = mcp_for(&handle, &app, &directory).await?;
    find(&runtime, &name)?;
    if editable(&app, &core, &name).is_ok() {
        edit(&app, &core, |text, data| {
            let mut path = entry_path(data, &name);
            // 0.5.x entries say `enabled`, the native ones `disabled`; the default is on.
            let legacy = path.len() == 2;
            path.push(if legacy { "enabled" } else { "disabled" });
            Ok(if enabled {
                config::jsonc::remove(text, &path)
            } else {
                config::jsonc::set(text, &path, json!(!legacy))?
            })
        })
        .await?;
        // Turned on in the file while turned off for this run: start it.
        if enabled
            && runtime
                .servers()
                .iter()
                .any(|s| s.name == name && s.status == Status::Disabled)
        {
            Mcp::connect(&*runtime, &name);
        }
    } else if enabled {
        Mcp::connect(&*runtime, &name);
    } else {
        Mcp::disconnect(&*runtime, &name);
    }
    list(&app, &core, &runtime)
}

/// Connects a server again (after it failed, or to pick up a change it can't see).
#[tauri::command]
pub async fn mcp_retry(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    name: String,
) -> Answer {
    let (core, runtime) = mcp_for(&handle, &app, &directory).await?;
    find(&runtime, &name)?;
    Mcp::connect(&*runtime, &name);
    list(&app, &core, &runtime)
}

/// Starts the OAuth sign-in; the window opens the address and calls `mcp_sign_in_wait`.
#[tauri::command]
pub async fn mcp_sign_in(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    name: String,
) -> Answer {
    let (_, runtime) = mcp_for(&handle, &app, &directory).await?;
    let server = runtime
        .servers()
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("There’s no MCP server named “{name}”."))?;
    if server.integration.is_none() {
        return Err(format!("“{name}” doesn’t sign in with OAuth."));
    }
    let attempt = runtime.authorize(&name).await?;
    let waiting = Waiting {
        done: Some(attempt.done),
        replaced: Arc::new(Notify::new()),
    };
    if let Some(earlier) = app
        .sign_ins
        .lock()
        .await
        .insert((key(&directory), name), waiting)
    {
        earlier.replaced.notify_one();
    }
    let instructions = Some(attempt.instructions).filter(|i| !i.trim().is_empty());
    Ok(json!({"url": attempt.url, "instructions": instructions}))
}

/// Waits for the browser to finish the sign-in, then lists the servers (the server reconnects
/// with the new credential). A newer sign-in for the same server ends this wait.
#[tauri::command]
pub async fn mcp_sign_in_wait(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    name: String,
) -> Answer {
    let (core, runtime) = mcp_for(&handle, &app, &directory).await?;
    let slot = (key(&directory), name);
    let (done, replaced) = {
        let mut sign_ins = app.sign_ins.lock().await;
        match sign_ins.get_mut(&slot) {
            Some(waiting) => (waiting.done.take(), waiting.replaced.clone()),
            None => (None, Arc::new(Notify::new())),
        }
    };
    if let Some(done) = done {
        let result = tokio::select! {
            result = done => Some(result.unwrap_or_else(|_| Err("The sign-in expired. Try again.".into()))),
            _ = replaced.notified() => None,
        };
        if let Some(result) = result {
            let mut sign_ins = app.sign_ins.lock().await;
            if sign_ins
                .get(&slot)
                .is_some_and(|w| Arc::ptr_eq(&w.replaced, &replaced))
            {
                sign_ins.remove(&slot);
            }
            drop(sign_ins);
            result?;
        }
    }
    list(&app, &core, &runtime)
}

/// Removes the server's stored sign-in, as `anysola mcp logout`, and reconnects so its status
/// says so.
#[tauri::command]
pub async fn mcp_sign_out(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    name: String,
) -> Answer {
    let (core, runtime) = mcp_for(&handle, &app, &directory).await?;
    let entry = runtime
        .config(&name)
        .ok_or_else(|| format!("There’s no MCP server named “{name}”."))?;
    if oauth::capable(&entry) {
        let url = entry["url"].as_str().unwrap_or_default();
        core.db()
            .remove_integration_credentials(&oauth::integration_id(&name, url))
            .map_err(text)?;
        Mcp::connect(&*runtime, &name);
    }
    list(&app, &core, &runtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_paths() {
        assert_eq!(
            target(&json!({"type": "local", "command": ["npx", "-y", "my server"]})),
            "npx -y \"my server\""
        );
        assert_eq!(
            target(&json!({"type": "remote", "url": "https://h/mcp"})),
            "https://h/mcp"
        );
        let native = json!({"mcp": {"servers": {"a": {"type": "remote", "url": "u"}}}});
        assert_eq!(
            entry_path(native.as_object().unwrap(), "a"),
            ["mcp", "servers", "a"]
        );
        let legacy = json!({"mcp": {"a": {"type": "local", "command": ["x"]}}});
        assert_eq!(entry_path(legacy.as_object().unwrap(), "a"), ["mcp", "a"]);
        assert_eq!(entry_path(&Map::new(), "new"), ["mcp", "servers", "new"]);
    }

    #[test]
    fn drafts_parse() {
        let remote: Draft = serde_json::from_value(
            json!({"name": "gh", "kind": "remote", "url": "https://h", "headers": {"A": "b"}}),
        )
        .unwrap();
        assert!(matches!(remote, Draft::Remote { ref headers, .. } if headers["A"] == "b"));
        let local: Draft = serde_json::from_value(
            json!({"name": "fs", "kind": "local", "command": ["x"], "environment": {}}),
        )
        .unwrap();
        assert!(matches!(local, Draft::Local { ref command, .. } if command == &["x"]));
    }
}
