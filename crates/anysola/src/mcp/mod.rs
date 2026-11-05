//! `anysola mcp`: `list` (live statuses), `add`, `auth` (OAuth sign-in) and `logout`.

mod jsonc;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anysola_core::api::Core;
use anysola_core::config::{self, LoadOptions};
use anysola_core::mcp::{Mcp, Status};
use anysola_core::paths::Paths;
use anysola_core::storage::Db;
use anysola_mcp::{Runtime, oauth};
use serde_json::{Map, Value, json};

use crate::cli::{McpAddArgs, McpCommand};
use crate::clack;

/// How long `list` waits for servers that are still connecting: the longest startup timeout.
const STARTUP_DEFAULT: u64 = 30_000;

pub async fn mcp(command: McpCommand) -> i32 {
    let outcome = match command {
        McpCommand::Add(args) => add(args),
        McpCommand::Logout { name } => logout(&name),
        McpCommand::List => list().await,
        McpCommand::Auth { name } => return auth(name).await,
    };
    match outcome {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(message) => crate::fail(&message),
    }
}

fn pairs(items: &[(String, String)]) -> Value {
    Value::Object(
        items
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect::<Map<_, _>>(),
    )
}

/// The first existing config file of `directory`, else its `opencode.json`.
fn project_file(directory: &Path) -> PathBuf {
    let candidates = [
        directory.join("opencode.json"),
        directory.join("opencode.jsonc"),
        directory.join(".opencode").join("opencode.json"),
        directory.join(".opencode").join("opencode.jsonc"),
    ];
    candidates
        .iter()
        .find(|path| path.is_file())
        .unwrap_or(&candidates[0])
        .clone()
}

fn add(args: McpAddArgs) -> Result<String, String> {
    let McpAddArgs {
        name,
        command,
        url,
        header,
        env,
        global,
    } = args;
    let server = match (url, command.is_empty()) {
        (Some(_), false) => {
            return Err("Provide either --url <url> or a command after --, not both".into());
        }
        (None, true) => return Err("Provide either --url <url> or a command after --".into()),
        (Some(url), true) => {
            if url::Url::parse(&url).is_err() {
                return Err(format!("Invalid URL: {url}"));
            }
            if !env.is_empty() {
                return Err("--env is only valid for local MCP servers".into());
            }
            let mut server = json!({"type": "remote", "url": url});
            if !header.is_empty() {
                server["headers"] = pairs(&header);
            }
            server
        }
        (None, false) => {
            if !header.is_empty() {
                return Err("--header is only valid for remote MCP servers".into());
            }
            let mut server = json!({"type": "local", "command": command});
            if !env.is_empty() {
                server["environment"] = pairs(&env);
            }
            server
        }
    };
    let path = if global {
        Paths::from_env().config_file()
    } else {
        project_file(&std::env::current_dir().map_err(|e| e.to_string())?)
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(error) => return Err(error.to_string()),
    };
    let updated = jsonc::set(&text, &["mcp", "servers", &name], server)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, updated).map_err(|e| e.to_string())?;
    Ok(format!("MCP server \"{name}\" added to {}", path.display()))
}

/// The servers as configured for the current directory (merged, defaults and policy applied).
fn servers(directory: &Path) -> Vec<(String, Value)> {
    config::load(&Paths::from_env(), directory, &LoadOptions::default()).mcp_servers()
}

/// Starts every server in-process and waits up to the startup timeout for their statuses.
async fn start() -> Result<(Core, Arc<Runtime>), String> {
    let directory = std::env::current_dir().map_err(|e| e.to_string())?;
    let core = Core::start(&directory).await.map_err(|e| e.to_string())?;
    let runtime = Runtime::attach(&core);
    let limit = core
        .loaded()
        .config
        .mcp_servers()
        .iter()
        .map(|(_, s)| s["timeout"]["startup"].as_u64().unwrap_or(STARTUP_DEFAULT))
        .max()
        .unwrap_or(0);
    runtime.settled(Duration::from_millis(limit)).await;
    Ok((core, runtime))
}

async fn list() -> Result<String, String> {
    let (_core, runtime) = start().await?;
    let servers = runtime.servers();
    runtime.shutdown().await;
    if servers.is_empty() {
        return Ok("No MCP servers configured".into());
    }
    let width = servers.iter().map(|s| s.name.chars().count()).max().unwrap_or(0);
    let lines: Vec<String> = servers
        .iter()
        .map(|server| {
            let (icon, describe) = match &server.status {
                Status::Connected => ("✓", "connected".to_string()),
                Status::NeedsAuth { .. } => ("⚠", "needs authentication".to_string()),
                Status::Failed { error } => ("✗", format!("failed: {error}")),
                Status::Disabled => ("○", "disabled".to_string()),
                Status::Pending => ("○", "pending".to_string()),
            };
            format!("{icon} {:<width$}  {describe}", server.name)
        })
        .collect();
    Ok(lines.join("\n"))
}

/// The OAuth sign-in of `mcp auth`: 0 done, 1 failed, 130 cancelled.
async fn auth(name: Option<String>) -> i32 {
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if name.is_none() && !interactive {
        clack::failed("Pass an MCP server name when running without an interactive terminal");
        return 1;
    }
    clack::intro("Authenticate an MCP server");
    let (core, runtime) = match start().await {
        Ok(started) => started,
        Err(error) => {
            clack::failed(&error);
            return 1;
        }
    };
    let code = sign_in(&core, &runtime, name, interactive).await;
    runtime.shutdown().await;
    code
}

async fn sign_in(core: &Core, runtime: &Runtime, name: Option<String>, interactive: bool) -> i32 {
    let servers = runtime.servers();
    let capable: Vec<_> = servers.iter().filter(|s| s.integration.is_some()).collect();
    let server = match name {
        Some(name) => match servers.iter().find(|s| s.name == name) {
            Some(server) if server.integration.is_some() => server,
            Some(_) => {
                clack::failed(&format!(
                    "MCP server \"{name}\" is not an OAuth-capable remote server"
                ));
                return 1;
            }
            None => {
                clack::failed(&format!("MCP server not found: {name}"));
                return 1;
            }
        },
        None if capable.is_empty() => {
            clack::warn("No OAuth-capable MCP servers configured");
            clack::info(&format!(
                "Remote MCP servers support OAuth by default. Add one with `anysola mcp add` or in opencode.json:\n{EXAMPLE}"
            ));
            clack::outro("Done");
            return 0;
        }
        None => {
            let choices: Vec<crate::prompt::Choice> = capable
                .iter()
                .map(|s| crate::prompt::Choice {
                    label: s.name.clone(),
                    connected: stored(core, s).is_some(),
                    hint: match &s.status {
                        Status::NeedsAuth { .. } => Some("needs authentication".into()),
                        Status::Failed { .. } => Some("failed".into()),
                        Status::Disabled => Some("disabled".into()),
                        _ => None,
                    },
                })
                .collect();
            match crate::prompt::select("Select MCP server", "MCP", &choices) {
                Some(index) => capable[index],
                None => {
                    clack::cancel("Cancelled");
                    return 130;
                }
            }
        }
    };
    if stored(core, server).is_some() {
        if matches!(server.status, Status::NeedsAuth { .. }) {
            clack::warn(&format!(
                "{} has expired credentials. Re-authenticating...",
                server.name
            ));
        } else if interactive {
            let question = format!(
                "{} already has valid credentials. Re-authenticate?",
                server.name
            );
            match crate::prompt::confirm(&question, true) {
                Some(true) => {}
                Some(false) => {
                    clack::outro("Cancelled");
                    return 0;
                }
                None => {
                    clack::cancel("Cancelled");
                    return 130;
                }
            }
        }
    }
    let spinner = clack::Spinner::start("Starting authorization...");
    let attempt = match runtime.authorize(&server.name).await {
        Ok(attempt) => attempt,
        Err(error) => {
            spinner.stop("Authentication failed", false);
            clack::failed(&error);
            return 1;
        }
    };
    spinner.stop("Authorization started", true);
    clack::info(&attempt.instructions);
    clack::info(&attempt.url);
    if interactive {
        crate::account::open_browser(&attempt.url);
    }
    // Plain log lines rather than a spinner, so Ctrl+C interrupts the wait.
    clack::step("Waiting for authorization...");
    let result = tokio::select! {
        result = attempt.done => result.unwrap_or_else(|_| Err("Authorization expired".into())),
        _ = tokio::signal::ctrl_c() => {
            clack::cancel("Cancelled");
            return 130;
        }
    };
    match result {
        Ok(()) => {
            clack::success(&format!("Connected to {}", server.name));
            clack::outro("Done");
            0
        }
        Err(error) => {
            clack::failed(&error);
            1
        }
    }
}

fn stored(core: &Core, server: &anysola_core::mcp::Server) -> Option<Value> {
    let integration = server.integration.as_deref()?;
    core.db()
        .active_credential(integration)
        .ok()
        .flatten()
        .map(|(_, v)| v)
}

const EXAMPLE: &str = r#"
  "mcp": {
    "servers": {
      "my-server": {
        "type": "remote",
        "url": "https://example.com/mcp"
      }
    }
  }"#;

fn logout(name: &str) -> Result<String, String> {
    let directory = std::env::current_dir().map_err(|e| e.to_string())?;
    let servers = servers(&directory);
    let (_, server) = servers
        .iter()
        .find(|(n, _)| n == name)
        .ok_or_else(|| format!("MCP server not found: {name}"))?;
    let none = format!("No stored credentials for {name}");
    if !oauth::capable(server) {
        return Ok(none);
    }
    let url = server["url"].as_str().unwrap_or_default();
    let db = Db::open(&Paths::from_env().database()).map_err(|e| e.to_string())?;
    let removed = db
        .remove_integration_credentials(&oauth::integration_id(name, url))
        .map_err(|e| e.to_string())?;
    Ok(if removed == 0 {
        none
    } else {
        format!("Removed OAuth credentials for {name}")
    })
}
