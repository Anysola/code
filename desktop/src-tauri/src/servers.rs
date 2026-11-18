//! MCP servers in Settings: the folder's merged servers with their live status, and adding,
//! removing, turning off and signing in. Servers of the global config file are changed in that
//! file (as `anysola mcp add --global` does); others only for this run of the app.
//!
//! Nothing here waits for a server to connect: the list comes back as it stands and `mcp` events
//! tell the window when a status changes.

use std::sync::Arc;

use anysola_core::api::{Core, TrustItem, TrustKind};
use anysola_core::config;
use anysola_core::hooks::display;
use anysola_core::mcp::{Mcp, Status};
use anysola_mcp::{Runtime, oauth};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, State};
use tokio::sync::{Notify, oneshot};

use crate::{Answer, App, key, mcp_for, text};

/// A sign-in in progress: finding the server's sign-in address (no `done` yet), then waiting
/// for the browser. A newer one for the same server, or a cancel, takes its place and ends it.
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
                "error": error.map(plain),
                "source": source,
                "editable": source == global,
                "oauth": server.integration.is_some(),
                "signedIn": signed_in,
                "tools": runtime.tool_count(&server.name),
                "prompts": prompts.iter().filter(|p| p.server == server.name).count(),
            })
        })
        .collect();
    let running: Vec<String> = runtime.servers().into_iter().map(|s| s.name).collect();
    let documents: Vec<(&str, &Map<String, Value>)> = loaded.config.documents().collect();
    let mut servers = servers;
    servers.extend(waiting(&core.trust_items(), &documents, &running));
    Ok(Value::Array(servers))
}

/// A project file's server as written: `mcp.servers.<name>`, or the 0.5.x `mcp.<name>`.
fn written<'a>(data: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    let mcp = data.get("mcp")?;
    mcp.get("servers")
        .and_then(|s| s.get(name))
        .or_else(|| mcp.get(name).filter(|s| s.is_object() && name != "servers"))
}

/// The servers project files define whose MCP section isn't allowed (yet): not running, listed
/// so the user sees them and can allow them. A server of the same name from elsewhere runs as
/// defined there and is listed as that one.
fn waiting(
    items: &[TrustItem],
    documents: &[(&str, &Map<String, Value>)],
    running: &[String],
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for item in items
        .iter()
        .filter(|i| i.kind == TrustKind::Mcp && i.decision != Some(true))
    {
        let Some((_, data)) = documents.iter().find(|(path, _)| *path == item.path) else {
            continue;
        };
        for entry in &item.entries {
            let Some(server) = written(data, &entry.label) else {
                // The section's timeouts, not a server.
                continue;
            };
            let listed = running.contains(&entry.label)
                || out.iter().any(|s| s["name"] == entry.label.as_str());
            if listed {
                continue;
            }
            let remote = server["type"] == "remote"
                || (server.get("type").is_none() && server.get("url").is_some());
            out.push(json!({
                "name": entry.label,
                "kind": if remote { "remote" } else { "local" },
                // Repository text: control characters written out.
                "target": display(&entry.command),
                "status": "waiting",
                "error": null,
                "source": item.path,
                "editable": false,
                "oauth": false,
                "signedIn": false,
                "tools": 0,
                "prompts": 0,
            }));
        }
    }
    out
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

/// The program a failed stdio spawn names: `NotFound: ChildProcess.spawn (npx -y foo)`.
fn spawned(error: &str, kind: &str) -> Option<String> {
    let rest = error
        .split_once(&format!("{kind}: ChildProcess.spawn ("))?
        .1;
    let program = rest.trim_end_matches(')').split_whitespace().next()?;
    // `C:\tools\uvx.exe` or `/usr/bin/uvx`: the name says what to install.
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    Some(name.to_string())
}

/// What installs a program MCP servers are commonly started with.
fn installer(program: &str) -> Option<&'static str> {
    let name = program.to_lowercase();
    let name = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".cmd"))
        .unwrap_or(&name);
    Some(match name {
        "npx" | "npm" | "node" | "pnpm" | "pnpx" => "Node.js",
        "uvx" | "uv" => "uv",
        "python" | "python3" | "py" | "pip" | "pipx" => "Python",
        "docker" => "Docker",
        "bun" | "bunx" => "Bun",
        "deno" => "Deno",
        _ => return None,
    })
}

/// An MCP error as a sentence for the window: what happened and what to do.
fn plain(error: &str) -> String {
    if let Some(program) = spawned(error, "NotFound") {
        return match installer(&program) {
            Some(what) => format!(
                "Couldn’t find `{program}`. Install {what}, or use the full path in the command."
            ),
            None => format!(
                "Couldn’t find `{program}`. Install it, or use the full path in the command."
            ),
        };
    }
    if let Some(program) = spawned(error, "PermissionDenied") {
        return format!(
            "`{program}` isn’t allowed to run. Check that the command names a program you can run."
        );
    }
    let lower = error.to_lowercase();
    if lower.contains("address already in use")
        || lower.contains("only one usage of each socket address")
        || lower.contains("os error 98")
        || lower.contains("os error 10048")
    {
        return "Another app is using the port this server’s sign-in listens on. Close it, or \
                wait a moment, then try again."
            .into();
    }
    if lower.contains("oauth callback timeout") {
        return "The sign-in took too long and was stopped. Try again.".into();
    }
    if lower == "authorization expired" {
        return "The sign-in was interrupted. Try again.".into();
    }
    error.to_string()
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
    // An earlier attempt ends first: with a fixed callback port, its listener holds the port.
    // This one holds the slot while it finds the address, so a cancel meanwhile is seen.
    let slot = (key(&directory), name.clone());
    let this = Arc::new(Notify::new());
    let discovering = Waiting {
        done: None,
        replaced: this.clone(),
    };
    if let Some(earlier) = app.sign_ins.lock().await.insert(slot.clone(), discovering) {
        earlier.replaced.notify_one();
    }
    let mut tries = 0;
    let attempt = loop {
        match runtime.authorize(&name).await {
            Ok(attempt) => break attempt,
            // The earlier listener closes as its wait notices; give it a moment.
            Err(error)
                if tries < 5 && plain(&error).starts_with("Another app is using the port") =>
            {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(error) => {
                let mut sign_ins = app.sign_ins.lock().await;
                if sign_ins
                    .get(&slot)
                    .is_some_and(|w| Arc::ptr_eq(&w.replaced, &this))
                {
                    sign_ins.remove(&slot);
                }
                return Err(plain(&error));
            }
        }
    };
    {
        let mut sign_ins = app.sign_ins.lock().await;
        match sign_ins.get_mut(&slot) {
            Some(waiting) if Arc::ptr_eq(&waiting.replaced, &this) => {
                waiting.done = Some(attempt.done);
            }
            // Cancelled, or a newer attempt took the slot, while the address was found: this
            // attempt is dropped, and its listener frees the callback port.
            _ => return Err(SIGN_IN_CANCELLED.into()),
        }
    }
    // The core's words are for the terminal, where the browser tab closes on its own.
    Ok(json!({
        "url": attempt.url,
        "instructions": format!("Finish signing in to {name} in your browser, then come back here."),
    }))
}

/// What a sign-in cancelled before its address was found answers.
const SIGN_IN_CANCELLED: &str = "The sign-in was cancelled.";

/// Gives up a sign-in, still finding its address or waiting for the browser: its wait returns
/// and its callback port is freed.
#[tauri::command]
pub async fn mcp_sign_in_cancel(app: State<'_, App>, directory: String, name: String) -> Answer {
    if let Some(waiting) = app.sign_ins.lock().await.remove(&(key(&directory), name)) {
        // A wait holding the attempt drops it as it returns; one not started yet drops it here.
        waiting.replaced.notify_one();
    }
    Ok(Value::Null)
}

/// Waits for the browser to finish the sign-in, then lists the servers (the server reconnects
/// with the new credential). A newer sign-in for the same server, or a cancel, ends this wait.
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
            result = done => Some(result.unwrap_or_else(|_| Err("The sign-in expired. Try again.".into())).map_err(|e| plain(&e))),
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
    fn errors_read_as_sentences() {
        assert_eq!(
            plain("NotFound: ChildProcess.spawn (npx -y @modelcontextprotocol/server-x)"),
            "Couldn’t find `npx`. Install Node.js, or use the full path in the command."
        );
        assert_eq!(
            plain("NotFound: ChildProcess.spawn (C:\\tools\\uvx.exe mcp-server)"),
            "Couldn’t find `uvx.exe`. Install uv, or use the full path in the command."
        );
        assert_eq!(
            plain("NotFound: ChildProcess.spawn (my-server)"),
            "Couldn’t find `my-server`. Install it, or use the full path in the command."
        );
        assert!(plain("PermissionDenied: ChildProcess.spawn (./x --y)").starts_with("`x` isn’t"));
        for bind in [
            "Address already in use (os error 98)",
            "Only one usage of each socket address (protocol/network address/port) is normally permitted. (os error 10048)",
        ] {
            assert!(
                plain(bind).starts_with("Another app is using the port"),
                "{bind}"
            );
        }
        assert_eq!(
            plain("OAuth callback timeout - authorization took too long"),
            "The sign-in took too long and was stopped. Try again."
        );
        assert_eq!(plain("Something else"), "Something else");
    }

    #[test]
    fn servers_waiting_for_approval_are_listed() {
        let item = |path: &str, decision, entries: Vec<(&str, &str)>| TrustItem {
            path: path.into(),
            kind: TrustKind::Mcp,
            digest: "d".into(),
            decision,
            standing: false,
            entries: entries
                .into_iter()
                .map(|(label, command)| anysola_core::api::TrustEntry {
                    label: label.into(),
                    command: command.into(),
                    detail: None,
                })
                .collect(),
        };
        let project = json!({"mcp": {
            "servers": {"docs": {"type": "remote", "url": "https://h/mcp"},
                        "fs": {"type": "local", "command": ["npx", "fs"]},
                        "mine": {"type": "local", "command": ["x"]}},
            "old": {"type": "local", "command": ["old\u{202e}"]},
            "timeout": 5}});
        let project = project.as_object().unwrap();
        let allowed = json!({"mcp": {"servers": {"ok": {"type": "local", "command": ["y"]}}}});
        let allowed = allowed.as_object().unwrap();
        let documents = [
            ("/p/anysola.json", project),
            ("/p/.anysola/anysola.json", allowed),
        ];
        let items = [
            item(
                "/p/anysola.json",
                None,
                vec![
                    ("docs", "https://h/mcp"),
                    ("fs", "npx fs"),
                    ("mine", "x"),
                    ("old", "old\u{202e}"),
                    ("MCP timeouts", "{\"catalog\":5}"),
                ],
            ),
            item("/p/.anysola/anysola.json", Some(true), vec![("ok", "y")]),
        ];
        let rows = waiting(&items, &documents, &["mine".to_string()]);
        let names: Vec<_> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["docs", "fs", "old"],
            "the user's own `mine` runs as theirs"
        );
        assert_eq!(
            rows[0],
            json!({"name": "docs", "kind": "remote", "target": "https://h/mcp", "status": "waiting",
                   "error": null, "source": "/p/anysola.json", "editable": false, "oauth": false,
                   "signedIn": false, "tools": 0, "prompts": 0})
        );
        assert_eq!(rows[1]["kind"], "local");
        assert!(!rows[2]["target"].as_str().unwrap().contains('\u{202e}'));
        let refused = [item("/p/anysola.json", Some(false), vec![("fs", "npx fs")])];
        assert_eq!(waiting(&refused, &documents, &[]).len(), 1);
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
