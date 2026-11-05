//! `anysola mcp add` and `anysola mcp logout`. Listing servers and signing in to them need the
//! MCP runtime, which milestone M5 brings; until then they say so.

mod jsonc;

use std::path::{Path, PathBuf};

use anysola_core::config::{self, LoadOptions, Source};
use anysola_core::paths::Paths;
use anysola_core::storage::Db;
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};

use crate::cli::{McpAddArgs, McpCommand};

pub async fn mcp(command: McpCommand) -> i32 {
    let outcome = match command {
        McpCommand::Add(args) => add(args),
        McpCommand::Logout { name } => logout(&name),
        McpCommand::List | McpCommand::Auth { .. } => {
            Err("MCP servers are not available in this build yet.".to_string())
        }
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

/// `mcp.servers` of every config source, later sources replacing whole entries.
fn servers(directory: &Path) -> Map<String, Value> {
    let config = config::load(&Paths::from_env(), directory, &LoadOptions::default());
    let mut servers = Map::new();
    for source in &config.sources {
        if let Source::Document { data, .. } = source
            && let Some(Value::Object(entries)) = data.get("mcp").and_then(|m| m.get("servers"))
        {
            servers.extend(entries.clone());
        }
    }
    servers
}

/// The integration a remote OAuth-capable server's credentials belong to: keyed on name and URL,
/// so two names for one URL keep separate accounts.
fn integration_id(name: &str, url: &str) -> String {
    let digest = Sha1::digest(format!("{name}\0{url}").as_bytes());
    format!("mcp_{}", &hex::encode(digest)[..16])
}

fn logout(name: &str) -> Result<String, String> {
    let directory = std::env::current_dir().map_err(|e| e.to_string())?;
    let servers = servers(&directory);
    let server = servers
        .get(name)
        .ok_or_else(|| format!("MCP server not found: {name}"))?;
    let none = format!("No stored credentials for {name}");
    let url = match (server["type"].as_str(), server["url"].as_str()) {
        (Some("remote"), Some(url)) if server["oauth"] != json!(false) => url,
        _ => return Ok(none),
    };
    let db = Db::open(&Paths::from_env().database()).map_err(|e| e.to_string())?;
    let removed = db
        .remove_integration_credentials(&integration_id(name, url))
        .map_err(|e| e.to_string())?;
    Ok(if removed == 0 {
        none
    } else {
        format!("Removed OAuth credentials for {name}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_ids_hash_name_and_url() {
        let id = integration_id("x", "https://h/mcp");
        assert!(id.starts_with("mcp_") && id.len() == 20);
        assert_ne!(id, integration_id("y", "https://h/mcp"));
    }
}
