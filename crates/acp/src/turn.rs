//! Prompt turns: content blocks in, session updates out, permission and question asks
//! forwarded to the client; and the history replay of `session/load`.

use std::collections::HashMap;
use std::sync::Arc;

use anysola_core::api::{Core, Prompt};
use anysola_core::events::Event;
use anysola_core::session::Outcome;
use anysola_core::session::gate::Reply;
use anysola_core::session::message::{Message, Part, ToolState};
use anysola_core::storage::SessionRow;
use anysola_core::tool::question::{Question, QuestionReply};
use serde_json::{Map, Value, json};
use tokio::sync::{broadcast, oneshot};

use crate::agent::{Agent, current_model, not_found};
use crate::rpc::{Connection, Failure};
use crate::tool::{self, Tool};

const UNSHOWN_QUESTION: &str = "The question couldn't be shown to the user in this client. Continue without an answer: make reasonable assumptions and state them, or ask the user in your reply if you can't proceed.";

fn session_failure(e: impl std::fmt::Display) -> Failure {
    Failure::internal(e, json!({"service": "session"}))
}

pub async fn prompt(agent: Arc<Agent>, params: Value) -> Result<Value, Failure> {
    let id = params["sessionId"].as_str().unwrap_or_default().to_string();
    {
        let mut sessions = agent.sessions();
        let attached = sessions.get_mut(&id).ok_or_else(|| not_found(&id))?;
        if attached.prompting {
            return Err(Failure::invalid_field(
                format!("Session already has an active ACP prompt: {id}"),
                "sessionId",
            ));
        }
        attached.prompting = true;
    }
    let result = run(&agent, &id, &params).await;
    if let Some(attached) = agent.sessions().get_mut(&id) {
        attached.prompting = false;
    }
    result
}

pub fn cancel(agent: &Agent, params: &Value) {
    let id = params["sessionId"].as_str().unwrap_or_default();
    let location = agent.sessions().get(id).map(|a| a.location.clone());
    if let Some(location) = location {
        location.core.interrupt(id);
    }
}

async fn run(agent: &Arc<Agent>, id: &str, params: &Value) -> Result<Value, Failure> {
    let (location, session) = agent.require(id)?;
    let core = &location.core;
    if !core.signed_in() {
        return Err(Failure::auth_required());
    }
    let prompt = blocks(&params["prompt"])?;
    let (done, finished) = oneshot::channel();
    let follower = tokio::spawn(follow(
        agent.clone(),
        core.clone(),
        session.clone(),
        core.subscribe(),
        finished,
    ));
    let trimmed = prompt.text.trim();
    let slash = trimmed.strip_prefix('/').and_then(|rest| {
        let mut words = rest.split_whitespace();
        let name = words.next()?.to_string();
        Some((name, words.collect::<Vec<_>>().join(" ")))
    });
    let options = anysola_core::session::TurnOptions::default();
    let outcome = match slash {
        Some((name, _)) if name == "compact" => core.compact(id).await,
        Some((name, args)) if core.commands().get(&name).is_some() => {
            core.command(id, &name, &args, options).await
        }
        _ => core.prompt(id, prompt, options).await,
    };
    let _ = done.send(());
    let state = follower.await.unwrap_or_default();
    let outcome = outcome.map_err(session_failure)?;
    let stop = match &outcome {
        Outcome::Interrupted(_) => "cancelled",
        Outcome::Failed(error) if error.kind == "aborted" => "cancelled",
        Outcome::Failed(error) if error.kind == "provider.content-filter" => "refusal",
        Outcome::Failed(error) => {
            let message = if error.message.is_empty() {
                "Anysola Code prompt failed"
            } else {
                &error.message
            };
            return Err(Failure::internal(
                message,
                json!({"service": "session", "errorName": error.kind}),
            ));
        }
        Outcome::Succeeded => match state.finish.as_deref() {
            Some("length") => "max_tokens",
            Some("content-filter") => "refusal",
            _ => "end_turn",
        },
    };
    if state.last > 0
        && let Some(size) = agent
            .db
            .session(id)
            .ok()
            .flatten()
            .and_then(|s| current_model(core, &s))
            .and_then(|(_, model)| model.context)
    {
        agent.connection.update(
            id,
            json!({"sessionUpdate": "usage_update", "used": state.last, "size": size,
                "cost": {"amount": 0, "currency": "USD"}}),
        );
    }
    let mut response = json!({"stopReason": stop});
    if let Some(usage) = state.usage() {
        response["usage"] = usage;
    }
    response["_meta"] = json!({});
    Ok(response)
}

/// What the turn's steps add up to.
#[derive(Default)]
struct Totals {
    steps: u32,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    /// The last step's total, the context the session now uses.
    last: u64,
    finish: Option<String>,
}

impl Totals {
    fn usage(&self) -> Option<Value> {
        if self.steps == 0 {
            return None;
        }
        let total = self.input + self.output + self.reasoning + self.cache_read + self.cache_write;
        let mut usage =
            json!({"inputTokens": self.input, "outputTokens": self.output, "totalTokens": total});
        for (key, value) in [
            ("thoughtTokens", self.reasoning),
            ("cachedReadTokens", self.cache_read),
            ("cachedWriteTokens", self.cache_write),
        ] {
            if value > 0 {
                usage[key] = json!(value);
            }
        }
        Some(usage)
    }
}

/// Whether `session_id` is `root` or one of its descendants.
fn within(agent: &Agent, session_id: &str, root: &str) -> bool {
    let mut current = session_id.to_string();
    for _ in 0..64 {
        if current == root {
            return true;
        }
        match agent.db.session(&current).ok().flatten().and_then(|s| s.parent_id) {
            Some(parent) => current = parent,
            None => return false,
        }
    }
    false
}

/// Translates the turn's events until the turn returned, then drains what is left.
async fn follow(
    agent: Arc<Agent>,
    core: Core,
    session: SessionRow,
    mut events: broadcast::Receiver<Event>,
    mut finished: oneshot::Receiver<()>,
) -> Totals {
    let mut totals = Totals::default();
    let mut tools: HashMap<(String, String), Tool> = HashMap::new();
    let mut running: Vec<(String, String)> = Vec::new();
    let mut ended = false;
    loop {
        let event = if ended {
            match events.try_recv() {
                Ok(event) => event,
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => return totals,
            }
        } else {
            tokio::select! {
                received = events.recv() => match received {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return totals,
                },
                _ = &mut finished => {
                    ended = true;
                    continue;
                }
            }
        };
        let own = event.session_id() == session.id;
        let cwd = session.directory.as_str();
        let send = |update: Value| agent.connection.update(&session.id, update);
        match event {
            Event::TextDelta {
                message_id, text, ..
            } if own => send(json!({"sessionUpdate": "agent_message_chunk", "messageId": message_id,
                "content": {"type": "text", "text": text}})),
            Event::ReasoningDelta {
                message_id,
                ordinal,
                text,
                ..
            } if own => send(json!({"sessionUpdate": "agent_thought_chunk",
                "messageId": format!("{message_id}:reasoning:{ordinal}"),
                "content": {"type": "text", "text": text}})),
            Event::ToolInputStarted {
                session_id,
                call_id,
                name,
                ..
            } => {
                let call = Tool {
                    id: call_id.clone(),
                    name,
                    input: json!({}),
                };
                if own {
                    let mut update = json!({"sessionUpdate": "tool_call"});
                    merge(&mut update, tool::pending(&call, cwd, None));
                    send(update);
                }
                tools.insert((session_id, call_id), call);
            }
            Event::ToolCalled {
                session_id,
                call_id,
                input,
                ..
            } => {
                let key = (session_id, call_id.clone());
                let call = tools.entry(key.clone()).or_insert_with(|| Tool {
                    id: call_id,
                    name: "tool".into(),
                    input: json!({}),
                });
                call.input = input;
                if own {
                    let mut update = json!({"sessionUpdate": "tool_call_update"});
                    merge(&mut update, tool::running(call, cwd));
                    send(update);
                }
                running.push(key);
            }
            Event::ToolSucceeded {
                session_id,
                call_id,
                content,
                metadata,
                ..
            } => {
                let key = (session_id, call_id.clone());
                running.retain(|k| *k != key);
                let call = tools.remove(&key).unwrap_or_else(|| unknown(&call_id));
                if own {
                    let mut update = json!({"sessionUpdate": "tool_call_update"});
                    merge(&mut update, tool::completed(&call, &content, Some(&metadata), cwd));
                    send(update);
                }
            }
            Event::ToolFailed {
                session_id,
                call_id,
                error,
                content,
                metadata,
                ..
            } => {
                let key = (session_id, call_id.clone());
                running.retain(|k| *k != key);
                let call = tools.remove(&key).unwrap_or_else(|| unknown(&call_id));
                if own {
                    let mut update = json!({"sessionUpdate": "tool_call_update"});
                    merge(
                        &mut update,
                        tool::failed(&call, &content, Some(&metadata), &error.message, cwd),
                    );
                    send(update);
                }
            }
            Event::StepEnded { finish, tokens, .. } if own => {
                totals.steps += 1;
                totals.input += tokens.input;
                totals.output += tokens.output;
                totals.reasoning += tokens.reasoning;
                totals.cache_read += tokens.cache.read;
                totals.cache_write += tokens.cache.write;
                totals.last = tokens.input
                    + tokens.output
                    + tokens.reasoning
                    + tokens.cache.read
                    + tokens.cache.write;
                totals.finish = Some(finish);
            }
            Event::PermissionAsked {
                session_id,
                request_id,
                action,
                resources,
                metadata,
                ..
            } if within(&agent, &session_id, &session.id) => {
                let wanted = tool::canonical(&action);
                let call = running
                    .iter()
                    .rev()
                    .filter(|(s, _)| *s == session_id)
                    .filter_map(|key| tools.get(key))
                    .find(|t| tool::canonical(&t.name) == wanted)
                    .or_else(|| {
                        running
                            .iter()
                            .rev()
                            .find(|(s, _)| *s == session_id)
                            .and_then(|key| tools.get(key))
                    })
                    .cloned();
                let ask = Ask {
                    request_id,
                    action,
                    resources,
                    metadata,
                    tool: call,
                };
                tokio::spawn(ask_permission(
                    agent.clone(),
                    core.clone(),
                    session.clone(),
                    ask,
                ));
            }
            Event::QuestionAsked {
                session_id,
                request_id,
                questions,
            } if within(&agent, &session_id, &session.id) => {
                tokio::spawn(ask_questions(
                    agent.clone(),
                    core.clone(),
                    session.id.clone(),
                    request_id,
                    questions,
                ));
            }
            // MCP servers' forms may collect secrets, which form elicitation must not.
            Event::FormAsked { form } => {
                core.reply_form(&form.id, anysola_core::mcp::FormReply::Cancelled);
            }
            _ => {}
        }
    }
}

fn unknown(call_id: &str) -> Tool {
    Tool {
        id: call_id.to_string(),
        name: "tool".into(),
        input: json!({}),
    }
}

fn merge(target: &mut Value, fields: Value) {
    if let (Some(target), Value::Object(fields)) = (target.as_object_mut(), fields) {
        target.extend(fields);
    }
}

struct Ask {
    request_id: String,
    action: String,
    resources: Vec<String>,
    metadata: Value,
    tool: Option<Tool>,
}

fn permission_title(name: &str, input: &Value, previews: &[Value]) -> Option<String> {
    if previews.len() > 1 {
        return Some(format!("{} files", previews.len()));
    }
    let text = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
    match tool::canonical(name).as_str() {
        "external_directory" => text("description")
            .or_else(|| text("command"))
            .or_else(|| text("parentDir")),
        "webfetch" => text("url"),
        "websearch" => text("query"),
        "grep" | "glob" => text("pattern"),
        "read" | "edit" | "write" | "patch" => tool::file_path(input).or_else(|| {
            previews
                .first()
                .and_then(|p| p["path"].as_str())
                .map(str::to_string)
        }),
        _ => None,
    }
}

async fn ask_permission(agent: Arc<Agent>, core: Core, session: SessionRow, ask: Ask) {
    let cwd = session.directory.as_str();
    let name = ask
        .tool
        .as_ref()
        .map_or(ask.action.clone(), |t| t.name.clone());
    let input = ask
        .tool
        .as_ref()
        .map_or(ask.metadata.clone(), |t| t.input.clone());
    let previews = tool::previews(&ask.metadata, cwd);
    let title = permission_title(&name, &input, &previews);
    let call = Tool {
        id: ask
            .tool
            .as_ref()
            .map_or(ask.request_id.clone(), |t| t.id.clone()),
        name: name.clone(),
        input: input.clone(),
    };
    let mut tool_call = tool::pending(&call, cwd, title.as_deref());
    if ask.tool.is_none()
        && let Some(map) = tool_call.as_object_mut()
    {
        map.remove("name");
        map.remove("rawInput");
    }
    let mut locations = tool::locations(&name, &input, cwd);
    if locations.as_array().is_some_and(Vec::is_empty)
        && ["read", "edit", "external_directory"].contains(&ask.action.as_str())
    {
        let mut paths: Vec<String> = Vec::new();
        for resource in &ask.resources {
            let path = resource.strip_suffix("/*").unwrap_or(resource);
            if path.is_empty() || path.contains(['*', '?']) {
                continue;
            }
            let path = tool::absolute(path, cwd);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        locations = paths.into_iter().map(|path| json!({"path": path})).collect();
    }
    tool_call["locations"] = locations;
    if !previews.is_empty() {
        tool_call["content"] = Value::Array(previews);
    }
    let options = json!([
        {"optionId": "once", "kind": "allow_once", "name": "Allow once"},
        {"optionId": "always", "kind": "allow_always", "name": "Always allow"},
        {"optionId": "reject", "kind": "reject_once", "name": "Reject"},
    ]);
    let result = agent
        .connection
        .request(
            "session/request_permission",
            json!({"sessionId": session.id, "toolCall": tool_call, "options": options}),
        )
        .await;
    let selected = result.ok().and_then(|r| {
        (r["outcome"]["outcome"] == "selected")
            .then(|| r["outcome"]["optionId"].as_str().map(str::to_string))
            .flatten()
    });
    let reply = match selected.as_deref() {
        Some("once") => Reply::Once,
        Some("always") => Reply::Always,
        _ => Reply::Reject(None),
    };
    core.reply_permission(&ask.request_id, reply);
}

/// Questions become a form elicitation when the client takes one; otherwise the model hears
/// that nobody could answer.
async fn ask_questions(
    agent: Arc<Agent>,
    core: Core,
    session_id: String,
    request_id: String,
    questions: Vec<Question>,
) {
    let elicits = agent
        .capabilities
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .form_elicitation;
    if !elicits {
        core.reply_question(
            &request_id,
            QuestionReply::Dismissed(Some(UNSHOWN_QUESTION.into())),
        );
        return;
    }
    let mut properties = Map::new();
    for (index, question) in questions.iter().enumerate() {
        let options: Vec<Value> = question
            .options
            .iter()
            .map(|o| json!({"const": o.label, "title": o.label, "description": o.description}))
            .collect();
        let property = if question.multiple == Some(true) {
            json!({"type": "array", "title": question.header, "description": question.question,
                "items": {"anyOf": options}, "minItems": 1})
        } else {
            json!({"type": "string", "title": question.header, "description": question.question,
                "oneOf": options})
        };
        properties.insert(format!("q{index}"), property);
    }
    let required: Vec<&String> = properties.keys().collect();
    let schema = json!({"type": "object", "properties": properties, "required": required});
    let message = questions
        .iter()
        .map(|q| q.header.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let response = agent
        .connection
        .request(
            "elicitation/create",
            json!({"mode": "form", "sessionId": session_id, "message": message,
                "requestedSchema": schema}),
        )
        .await;
    let reply = match response {
        Ok(response) if response["action"] == "accept" => {
            let content = &response["content"];
            QuestionReply::Answered(
                (0..questions.len())
                    .map(|i| match &content[format!("q{i}")] {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(items) => items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect(),
                        _ => Vec::new(),
                    })
                    .collect(),
            )
        }
        _ => QuestionReply::Dismissed(None),
    };
    core.reply_question(&request_id, reply);
}

/// ACP content blocks as one prompt: texts joined by newlines, media as attachments.
fn blocks(prompt: &Value) -> Result<Prompt, Failure> {
    let mut texts: Vec<String> = Vec::new();
    let mut files = Vec::new();
    let link = |name: Option<&str>, uri: &str| match name.filter(|n| !n.is_empty()) {
        Some(name) => format!("[{name}]({uri})"),
        None => uri.to_string(),
    };
    for block in prompt.as_array().into_iter().flatten() {
        let text = |key: &str| block.get(key).and_then(Value::as_str);
        match text("type").unwrap_or_default() {
            "text" => {
                if block["annotations"]["audience"] != json!(["user"]) {
                    texts.push(text("text").unwrap_or_default().to_string());
                }
            }
            "image" => {
                let mime = text("mimeType").unwrap_or("image/png").to_string();
                let name = text("uri").and_then(file_name).unwrap_or("image".into());
                match (text("data"), text("uri")) {
                    (Some(data), _) => files.push((mime, data.to_string(), name)),
                    (None, Some(uri)) => texts.push(link(Some(&name), uri)),
                    (None, None) => {
                        return Err(Failure::invalid_field(
                            "image content has no data or uri",
                            "prompt",
                        ));
                    }
                }
            }
            "resource_link" => {
                let uri = text("uri").unwrap_or_default();
                let path = url::Url::parse(uri)
                    .ok()
                    .filter(|u| u.scheme() == "file")
                    .and_then(|u| u.to_file_path().ok());
                match path.and_then(|p| Some((std::fs::read_to_string(&p).ok()?, p))) {
                    Some((content, path)) => {
                        texts.push(format!("[{}]\n{content}", path.display()))
                    }
                    None => texts.push(link(text("name"), uri)),
                }
            }
            "resource" => {
                let resource = &block["resource"];
                let uri = resource["uri"].as_str().unwrap_or_default();
                if let Some(body) = resource["text"].as_str() {
                    let parsed = url::Url::parse(uri).ok();
                    let label = match parsed.as_ref().filter(|u| u.scheme() == "file") {
                        Some(url) => {
                            let path = url
                                .to_file_path()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| url.path().to_string());
                            let line = url
                                .fragment()
                                .and_then(|f| f.strip_prefix('L'))
                                .map(|f| f.chars().take_while(char::is_ascii_digit).collect::<String>())
                                .filter(|l| !l.is_empty());
                            match line {
                                Some(line) => format!("{path}:{line}"),
                                None => path,
                            }
                        }
                        None => uri.to_string(),
                    };
                    texts.push(format!("[{label}]\n{body}"));
                } else if let Some(mime) = resource["mimeType"].as_str() {
                    let data = match uri.strip_prefix("data:").and_then(|d| d.split_once(",")) {
                        Some((_, data)) => data.to_string(),
                        None => resource["blob"].as_str().unwrap_or_default().to_string(),
                    };
                    let name = file_name(uri).unwrap_or("file".into());
                    files.push((mime.to_string(), data, name));
                }
            }
            _ => {}
        }
    }
    Ok(Prompt {
        text: texts.join("\n"),
        files,
    })
}

fn file_name(uri: &str) -> Option<String> {
    if uri.starts_with("data:") {
        return None;
    }
    let path = url::Url::parse(uri)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| uri.to_string());
    std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// Replays the stored conversation as session updates.
pub fn replay(connection: &Connection, core: &Core, session: &SessionRow) {
    let cwd = session.directory.as_str();
    let send = |update: Value| connection.update(&session.id, update);
    for (id, message) in core.messages(&session.id).unwrap_or_default() {
        match message {
            Message::User(data) => {
                if !data.text.is_empty() {
                    send(json!({"sessionUpdate": "user_message_chunk", "messageId": id,
                        "content": {"type": "text", "text": data.text}}));
                }
                for file in data.files.unwrap_or_default() {
                    if file.mime.starts_with("image/") {
                        send(json!({"sessionUpdate": "user_message_chunk", "messageId": id,
                            "content": {"type": "image", "mimeType": file.mime, "data": file.data}}));
                    }
                }
            }
            Message::Assistant(data) => {
                let mut reasoning = 0;
                for part in data.content {
                    match part {
                        Part::Text { text, .. } if !text.is_empty() => {
                            send(json!({"sessionUpdate": "agent_message_chunk", "messageId": id,
                                "content": {"type": "text", "text": text}}))
                        }
                        Part::Reasoning { text, .. } => {
                            if !text.is_empty() {
                                send(json!({"sessionUpdate": "agent_thought_chunk",
                                    "messageId": format!("{id}:reasoning:{reasoning}"),
                                    "content": {"type": "text", "text": text}}));
                            }
                            reasoning += 1;
                        }
                        Part::Tool {
                            id: call_id,
                            name,
                            state,
                            ..
                        } => replay_tool(&send, call_id, name, state, cwd),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn replay_tool(send: &impl Fn(Value), id: String, name: String, state: ToolState, cwd: &str) {
    let input = match &state {
        ToolState::Streaming { .. } => json!({}),
        ToolState::Running { input, .. }
        | ToolState::Completed { input, .. }
        | ToolState::Error { input, .. } => input.clone(),
    };
    let call = Tool { id, name, input };
    let mut update = json!({"sessionUpdate": "tool_call"});
    merge(&mut update, tool::pending(&call, cwd, None));
    send(update);
    let stored = |metadata: &Map<String, Value>| Some(metadata.clone()).filter(|m| !m.is_empty());
    let result = match &state {
        ToolState::Completed {
            content, metadata, ..
        } => tool::completed(&call, content, stored(metadata).as_ref(), cwd),
        ToolState::Error {
            error,
            content,
            metadata,
            ..
        } => tool::failed(&call, content, stored(metadata).as_ref(), &error.message, cwd),
        _ => return,
    };
    let mut update = json!({"sessionUpdate": "tool_call_update"});
    merge(&mut update, result);
    send(update);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_blocks_become_one_prompt() {
        let prompt = blocks(&json!([
            {"type": "text", "text": "look"},
            {"type": "text", "text": "hidden", "annotations": {"audience": ["user"]}},
            {"type": "resource", "resource": {"uri": "file:///p/a.rs#L3", "text": "fn a() {}"}},
            {"type": "image", "mimeType": "image/png", "data": "AAAA"},
            {"type": "resource_link", "uri": "https://x.dev/a", "name": "a"},
        ]))
        .unwrap();
        assert_eq!(prompt.text, "look\n[/p/a.rs:3]\nfn a() {}\n[a](https://x.dev/a)");
        assert_eq!(prompt.files, [("image/png".into(), "AAAA".into(), "image".into())]);
        let error = blocks(&json!([{"type": "image", "mimeType": "image/png"}])).unwrap_err();
        assert_eq!(error.message, "Invalid params: image content has no data or uri");
    }
}
