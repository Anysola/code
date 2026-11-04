//! Transcript → Anthropic `messages`, as 0.5.1 lowered its history: user turns as text (+ media)
//! blocks, assistant turns with thinking/text/tool_use, tool results in the following user turn
//! (consecutive result-only turns merge), failures as JSON error strings with `is_error`.

use serde_json::{Map, Value, json};

use super::message::{AssistantData, Message, Part, ToolState, UserData};
use crate::tool::ToolContent;

/// The model a request is for, to decide what reasoning metadata may be replayed.
pub struct Target<'a> {
    pub provider: &'a str,
    pub id: &'a str,
}

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn media_block(mime: &str, data: &str) -> Value {
    let kind = if mime == "application/pdf" {
        "document"
    } else {
        "image"
    };
    json!({"type": kind, "source": {"type": "base64", "media_type": mime, "data": data}})
}

/// `data:<mime>;base64,<data>` split into its parts.
fn data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn user(data: &UserData) -> Option<Value> {
    let mut content = Vec::new();
    if !data.text.trim().is_empty() {
        content.push(text_block(&data.text));
    }
    for file in &data.files {
        let mime = file.mime.to_lowercase();
        if mime.starts_with("image/") || mime == "application/pdf" {
            content.push(media_block(&mime, &file.data));
        }
    }
    (!content.is_empty()).then(|| json!({"role": "user", "content": content}))
}

fn reasoning_replayable(data: &AssistantData) -> bool {
    data.error.is_none()
        && data.content.iter().all(|part| match part {
            Part::Reasoning { time, .. } => time.as_ref().is_some_and(|t| t.completed.is_some()),
            _ => true,
        })
}

fn tool_result_content(content: &[ToolContent]) -> Value {
    if let [ToolContent::Text { text }] = content {
        return Value::String(text.clone());
    }
    Value::Array(
        content
            .iter()
            .map(|item| match item {
                ToolContent::Text { text } => text_block(text),
                ToolContent::File { uri, mime, .. } => match data_uri(uri) {
                    Some((mime, data)) => media_block(mime, data),
                    None => text_block(&format!("[{mime} omitted]")),
                },
            })
            .collect(),
    )
}

fn scrub(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn tool_result(id: &str, state: &ToolState) -> Option<Value> {
    let mut block = Map::new();
    block.insert("type".into(), "tool_result".into());
    block.insert("tool_use_id".into(), scrub(id).into());
    match state {
        ToolState::Completed { content, .. } => {
            block.insert("content".into(), tool_result_content(content));
        }
        ToolState::Error { error, content, .. } => {
            let items: Vec<Value> = content
                .iter()
                .map(|c| serde_json::to_value(c).unwrap_or_default())
                .collect();
            let envelope =
                json!({"error": {"type": error.kind, "message": error.message}, "content": items});
            block.insert("content".into(), crate::json::compact(&envelope).into());
            block.insert("is_error".into(), true.into());
        }
        ToolState::Streaming { .. } | ToolState::Running { .. } => return None,
    }
    Some(Value::Object(block))
}

/// The assistant turn and the tool results that follow it.
fn assistant(data: &AssistantData, target: &Target) -> (Option<Value>, Vec<Value>) {
    let same_model = data.model.provider_id == target.provider && data.model.id == target.id;
    let reasoning = reasoning_replayable(data);
    let mut content = Vec::new();
    let mut results = Vec::new();
    for part in &data.content {
        match part {
            Part::Text { text, .. } if !text.trim().is_empty() => content.push(text_block(text)),
            Part::Text { .. } => {}
            Part::Reasoning { text, native, .. } if reasoning => {
                let native = native.as_ref().filter(|_| same_model);
                let signature = native
                    .and_then(|n| n.get("signature"))
                    .and_then(Value::as_str);
                let redacted = native
                    .and_then(|n| n.get("redactedData"))
                    .and_then(Value::as_str);
                match (signature, redacted) {
                    (Some(sig), _) if !sig.trim().is_empty() => content
                        .push(json!({"type": "thinking", "thinking": text, "signature": sig})),
                    (None, Some(data)) => {
                        content.push(json!({"type": "redacted_thinking", "data": data}))
                    }
                    // Anthropic requires signatures on thinking blocks, so unsigned reasoning is dropped.
                    _ => {}
                }
            }
            Part::Reasoning { .. } => {}
            Part::Tool {
                id,
                name,
                state,
                executed,
                ..
            } => {
                if *executed == Some(true) {
                    continue;
                }
                let input = match state {
                    ToolState::Streaming { input } => {
                        serde_json::from_str(input).unwrap_or(json!({}))
                    }
                    ToolState::Running { input, .. }
                    | ToolState::Completed { input, .. }
                    | ToolState::Error { input, .. } => input.clone(),
                };
                content.push(
                    json!({"type": "tool_use", "id": scrub(id), "name": name, "input": input}),
                );
                results.extend(tool_result(id, state));
            }
        }
    }
    let turn = (!content.is_empty()).then(|| json!({"role": "assistant", "content": content}));
    (turn, results)
}

fn is_result_turn(message: &Value) -> bool {
    message["role"] == "user"
        && message["content"]
            .as_array()
            .is_some_and(|c| c.iter().all(|b| b["type"] == "tool_result"))
}

pub fn lower(history: &[Message], target: &Target) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in history {
        match message {
            Message::User(data) => out.extend(user(data)),
            Message::Synthetic(data) if !data.text.trim().is_empty() => {
                out.push(json!({"role": "user", "content": [text_block(&data.text)]}))
            }
            Message::Assistant(data) => {
                let (turn, results) = assistant(data, target);
                out.extend(turn);
                if results.is_empty() {
                    continue;
                }
                match out.last_mut() {
                    Some(last) if is_result_turn(last) => last["content"]
                        .as_array_mut()
                        .expect("result turn has content")
                        .extend(results),
                    _ => out.push(json!({"role": "user", "content": results})),
                }
            }
            Message::Synthetic(_) | Message::Other => {}
        }
    }
    out
}

/// Marks the tail message's last text block (else its last block) as a cache breakpoint.
pub fn mark_tail(messages: &mut [Value]) {
    let Some(content) = messages
        .last_mut()
        .and_then(|m| m["content"].as_array_mut())
    else {
        return;
    };
    let index = content
        .iter()
        .rposition(|b| b["type"] == "text")
        .or_else(|| content.len().checked_sub(1));
    if let Some(Value::Object(block)) = index.and_then(|i| content.get_mut(i)) {
        block.insert("cache_control".into(), json!({"type": "ephemeral"}));
    }
}
