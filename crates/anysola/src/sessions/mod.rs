//! `anysola session list|delete|export|import`.

#[cfg(unix)]
mod picker;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anysola_core::paths::Paths;
use anysola_core::storage::{Db, ImportError, SessionSummary};
use serde_json::{Value, json};

use crate::cli::{ListFormat, SessionCommand};

const NO_TERMINAL: &str = "Pass a session ID when running without an interactive terminal";

pub async fn session(command: SessionCommand) -> i32 {
    let db = match Db::open(&Paths::from_env().database()) {
        Ok(db) => db,
        Err(error) => return crate::fail(&error.to_string()),
    };
    let directory = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => return crate::fail(&error.to_string()),
    };
    match command {
        SessionCommand::List { max_count, format } => list(&db, &directory, max_count, format),
        SessionCommand::Delete { session_id } => match db.delete_session(&session_id) {
            Ok(true) => {
                println!("Session {session_id} deleted");
                0
            }
            Ok(false) => bare(&format!("Session not found: {session_id}")),
            Err(error) => crate::fail(&error.to_string()),
        },
        SessionCommand::Export { session, sanitize } => export(&db, &directory, session, sanitize),
        SessionCommand::Import {
            file,
            directory: target,
        } => {
            let target = target.map_or(directory.clone(), |t| {
                anysola_core::paths::normalize(&directory.join(t))
            });
            import(&db, &file, &target).await
        }
    }
}

/// 0.5.1 printed these messages without an `Error:` prefix.
fn bare(message: &str) -> i32 {
    eprintln!("{message}");
    1
}

/// `Date.prototype.toLocaleString()` in en-US: `11/4/2026, 7:59:22 PM`, local time.
fn locale_string(ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map(|t| t.format("%-m/%-d/%Y, %-I:%M:%S %p").to_string())
        .unwrap_or_default()
}

fn roots(db: &Db, directory: &Path, limit: u64) -> Result<Vec<SessionSummary>, String> {
    let project = anysola_core::project::identify(directory);
    db.root_sessions(&project.id, limit)
        .map_err(|e| e.to_string())
}

fn list(db: &Db, directory: &Path, max_count: Option<u64>, format: ListFormat) -> i32 {
    let sessions = match roots(db, directory, max_count.unwrap_or(100)) {
        Ok(sessions) => sessions,
        Err(error) => return crate::fail(&error),
    };
    let output = match format {
        ListFormat::Table if sessions.is_empty() => return 0,
        ListFormat::Table => sessions
            .iter()
            .map(|s| {
                let title = s.title.replace(['\r', '\n', '\t'], " ");
                format!("{}\t{title}\t{}", s.id, locale_string(s.time_updated))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ListFormat::Json => anysola_core::json::pretty(&Value::Array(
            sessions
                .iter()
                .map(|s| {
                    json!({"id": s.id, "title": s.title, "updated": s.time_updated, "created": s.time_created,
                           "projectId": s.project_id, "directory": s.directory})
                })
                .collect(),
        )),
    };
    let output = format!("{output}\n");
    if format == ListFormat::Table && max_count.is_none() && std::io::stdout().is_terminal() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/c", "more"])
        } else {
            ("less", &["-R", "-S"])
        };
        if let Ok(mut pager) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = pager.stdin.take() {
                let _ = stdin.write_all(output.as_bytes());
            }
            let _ = pager.wait();
            return 0;
        }
    }
    print!("{output}");
    0
}

fn export(db: &Db, directory: &Path, session: Option<String>, sanitize: bool) -> i32 {
    let id = match session {
        Some(id) => id,
        None if !std::io::stdin().is_terminal() => return bare(NO_TERMINAL),
        None => match pick(db, directory) {
            Ok(id) => id,
            Err(code) => return code,
        },
    };
    match db.export_session(&id) {
        Ok(Some(mut data)) => {
            if sanitize {
                anysola_core::storage::sanitize(&mut data);
            }
            println!("{}", anysola_core::json::pretty(&data));
            0
        }
        Ok(None) => bare(&format!("Session not found: {id}")),
        Err(error) => crate::fail(&error.to_string()),
    }
}

/// The picked session id, or the exit code when nothing was picked.
#[cfg(unix)]
fn pick(db: &Db, directory: &Path) -> Result<String, i32> {
    let sessions = roots(db, directory, 50).map_err(|error| crate::fail(&error))?;
    match picker::pick_session(&sessions) {
        picker::Picked::Session(id) => Ok(id),
        picker::Picked::None => Err(0),
        picker::Picked::Cancelled => Err(130),
        picker::Picked::NoTerminal => Err(bare(NO_TERMINAL)),
    }
}

/// Raw console input is only wired up on Unix terminals.
#[cfg(not(unix))]
fn pick(_: &Db, _: &Path) -> Result<String, i32> {
    Err(bare(NO_TERMINAL))
}

async fn read(source: &str) -> Result<String, String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let response = reqwest::Client::new()
            .get(source)
            .header("user-agent", anysola_core::version::user_agent())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "Failed to fetch session data: {}",
                status.canonical_reason().unwrap_or_default()
            ));
        }
        return response.text().await.map_err(|e| e.to_string());
    }
    let path = PathBuf::from(source);
    std::fs::read_to_string(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => format!(
            "ENOENT: no such file or directory, open '{}'",
            std::path::absolute(&path).unwrap_or(path).display()
        ),
        _ => error.to_string(),
    })
}

/// The minimum the importer relies on: a session id, numeric times and tokens, and messages
/// with ids and types.
fn well_formed(data: &Value) -> bool {
    let info = &data["info"];
    let number = |v: &Value| v.is_number();
    let messages_ok = data["messages"].as_array().is_some_and(|messages| {
        messages.iter().all(|m| {
            m["id"].as_str().is_some_and(|id| id.starts_with("msg_"))
                && m["type"].is_string()
                && number(&m["time"]["created"])
        })
    });
    info["id"].as_str().is_some_and(|id| id.starts_with("ses_"))
        && number(&info["cost"])
        && number(&info["time"]["created"])
        && ["input", "output", "reasoning"]
            .iter()
            .all(|k| number(&info["tokens"][k]))
        && number(&info["tokens"]["cache"]["read"])
        && number(&info["tokens"]["cache"]["write"])
        && messages_ok
}

async fn import(db: &Db, source: &str, directory: &Path) -> i32 {
    let text = match read(source).await {
        Ok(text) => text,
        Err(error) => return crate::fail(&format!("Failed to read session data: {error}")),
    };
    let Ok(data @ Value::Object(_)) = serde_json::from_str::<Value>(&text) else {
        return crate::fail("Expected a valid JSON string");
    };
    if !well_formed(&data) {
        return crate::fail("Failed to import session: Bad Request");
    }
    let project = anysola_core::project::identify(directory);
    match db.import_session(&data, &project, &directory.display().to_string()) {
        Ok(id) => {
            println!("Imported session: {id}");
            0
        }
        Err(ImportError::Conflict) => {
            eprintln!("Session already exists");
            0
        }
        Err(ImportError::MissingParent) => crate::fail("Failed to import session: Not Found"),
        Err(ImportError::Storage(error)) => crate::fail(&error.to_string()),
    }
}
