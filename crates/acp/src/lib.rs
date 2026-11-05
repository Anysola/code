//! `anysola acp`: the Agent Client Protocol (version 1) over newline-delimited JSON-RPC on
//! stdin and stdout, with the agent running in this process.

mod agent;
mod rpc;
mod tool;
mod turn;

use std::sync::Arc;

use anysola_core::paths::Paths;
use anysola_core::storage::Db;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use agent::Agent;
use rpc::{Connection, Failure};

/// The TypeScript SDK's default limit for one incoming message.
const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

/// Serves until the client closes stdin (0) or a fatal error, reported on stderr (1).
pub async fn serve() -> i32 {
    anysola_core::version::set_acp_client();
    let db = match Db::open(&Paths::from_env().database()) {
        Ok(db) => Arc::new(db),
        Err(error) => {
            eprintln!("anysola acp: {error}");
            return 1;
        }
    };
    let (out, lines) = mpsc::unbounded_channel();
    let writer = tokio::spawn(write(lines));
    let connection = Arc::new(Connection::new(out));
    let agent = Arc::new(Agent::new(connection.clone(), db));
    let failure = read(&agent).await;
    connection.close();
    let open: Vec<String> = agent.sessions().keys().cloned().collect();
    for id in open {
        agent.close(&id);
    }
    drop(agent);
    drop(connection);
    let _ = writer.await;
    match failure {
        Some(failure) => {
            eprintln!("anysola acp: {failure}");
            1
        }
        None => 0,
    }
}

async fn write(mut lines: mpsc::UnboundedReceiver<String>) {
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.recv().await {
        if stdout
            .write_all(format!("{line}\n").as_bytes())
            .await
            .is_err()
            || stdout.flush().await.is_err()
        {
            return;
        }
    }
}

/// Reads messages until end of input; `Some` names a fatal failure.
async fn read(agent: &Arc<Agent>) -> Option<String> {
    let mut input = BufReader::new(tokio::io::stdin());
    let mut line = Vec::new();
    loop {
        let buffer = match input.fill_buf().await {
            Ok(buffer) => buffer,
            Err(error) => return Some(error.to_string()),
        };
        if buffer.is_empty() {
            if !line.is_empty() {
                dispatch(agent, &std::mem::take(&mut line)).await;
            }
            return None;
        }
        let (chunk, complete) = match buffer.iter().position(|b| *b == b'\n') {
            Some(at) => (&buffer[..=at], true),
            None => (buffer, false),
        };
        line.extend_from_slice(chunk);
        let used = chunk.len();
        input.consume(used);
        if line.len() > MAX_MESSAGE_BYTES {
            return Some(format!(
                "incoming message exceeded the {} MiB limit",
                MAX_MESSAGE_BYTES / 1024 / 1024
            ));
        }
        if complete {
            dispatch(agent, &std::mem::take(&mut line)).await;
        }
    }
}

async fn dispatch(agent: &Arc<Agent>, line: &[u8]) {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let connection = &agent.connection;
    let message: Value = match serde_json::from_str(text) {
        Ok(message) => message,
        Err(_) => return connection.respond(Value::Null, Err(Failure::parse_error())),
    };
    if !message.is_object() {
        return connection.respond(Value::Null, Err(Failure::invalid_request(message)));
    }
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str);
    let params = message.get("params").cloned().unwrap_or(json!({}));
    match (method, id) {
        (Some("session/cancel"), None) => turn::cancel(agent, &params),
        (Some(_), None) => {}
        // Turns run alongside further requests: cancels and permission answers arrive meanwhile.
        (Some("session/prompt"), Some(id)) => {
            let agent = agent.clone();
            tokio::spawn(async move {
                let result = turn::prompt(agent.clone(), params).await;
                agent.connection.respond(id, result);
            });
        }
        (Some(method), Some(id)) => {
            let result = request(agent, method, &params).await;
            let session = result
                .as_ref()
                .ok()
                .and_then(|r| r["sessionId"].as_str().or(params["sessionId"].as_str()))
                .map(str::to_string);
            connection.respond(id, result);
            // The client learns the session before its commands.
            if let Some(session) = session
                && matches!(
                    method,
                    "session/new" | "session/load" | "session/resume" | "session/fork"
                )
            {
                agent.announce(&session);
            }
        }
        (None, Some(id)) => match message.get("error") {
            Some(error) => connection.resolve(&id, Err(error.clone())),
            None => connection.resolve(&id, Ok(message["result"].clone())),
        },
        (None, None) => connection.respond(Value::Null, Err(Failure::invalid_request(message))),
    }
}

async fn request(agent: &Arc<Agent>, method: &str, params: &Value) -> Result<Value, Failure> {
    match method {
        "initialize" => Ok(agent.initialize(params)),
        "authenticate" => agent.authenticate(params),
        "session/new" => agent.new_session(params).await,
        "session/load" => agent.load_session(params, true).await,
        "session/resume" => agent.load_session(params, false).await,
        "session/fork" => agent.fork_session(params).await,
        "session/list" => agent.list_sessions(params),
        "session/delete" => agent.delete_session(params),
        "session/close" => {
            agent.close(params["sessionId"].as_str().unwrap_or_default());
            Ok(json!({}))
        }
        "session/set_config_option" => agent.set_config_option(params),
        "session/set_mode" => agent.set_mode(params),
        _ => Err(Failure::method_not_found(method)),
    }
}
