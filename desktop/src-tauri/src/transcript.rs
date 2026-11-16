//! A session's stored messages as the window shows them: user turns, the model's text and
//! reasoning, and tool calls with the diffs edits attach. Read from SQLite on demand, so a
//! missed event never leaves the window wrong for longer than the next refresh.

use anysola_core::api::Core;
use anysola_core::session::message::{Message, Part, ToolState};
use anysola_core::tool::ToolContent;
use serde_json::{Map, Value, json};

/// What the window shows of a session: its items up to a staged revert, the revert itself
/// (turns hidden, files put back) and the latest todo list.
pub fn view(core: &Core, session_id: &str, directory: &str) -> Result<Value, String> {
    let messages = core.messages(session_id).map_err(crate::text)?;
    let boundary = core.revert_boundary(session_id).map_err(crate::text)?;
    let (shown, hidden) = match &boundary {
        Some(b) => match messages.iter().position(|(id, _)| id == b) {
            Some(at) => {
                let mut shown = messages;
                let hidden = shown.split_off(at);
                (shown, hidden)
            }
            None => (messages, Vec::new()),
        },
        None => (messages, Vec::new()),
    };
    let revert = boundary.map(|_| {
        let turns = hidden
            .iter()
            .filter(|(_, m)| matches!(m, Message::User(_)))
            .count();
        json!({"turns": turns, "files": core.revert_files(session_id).unwrap_or_default()})
    });
    Ok(json!({
        "items": items(shown, directory),
        "revert": revert,
        "todos": core.todos(session_id).unwrap_or_default(),
    }))
}

/// Tool output kept for display; a command's full log stays in the session.
const OUTPUT_CHARS: usize = 4_000;

pub fn items(messages: Vec<(String, Message)>, directory: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for (id, message) in messages {
        match message {
            Message::User(data) => {
                let images = data.files.as_ref().map_or(0, |f| {
                    f.iter().filter(|f| f.mime.starts_with("image/")).count()
                });
                if !data.text.is_empty() || images > 0 {
                    out.push(
                        json!({"kind": "user", "id": id, "text": data.text, "images": images}),
                    );
                }
            }
            Message::Assistant(data) => {
                for (n, part) in data.content.into_iter().enumerate() {
                    let key = format!("{id}:{n}");
                    match part {
                        Part::Text { text, .. } if !text.trim().is_empty() => {
                            out.push(json!({"kind": "text", "id": key, "text": text}));
                        }
                        Part::Reasoning { text, .. } if !text.trim().is_empty() => {
                            out.push(json!({"kind": "reasoning", "id": key, "text": text}));
                        }
                        Part::Tool {
                            id: call,
                            name,
                            state,
                            ..
                        } => out.push(tool(&call, &name, state, directory)),
                        _ => {}
                    }
                }
                if let Some(error) = data.error {
                    out.push(json!({"kind": "error", "id": format!("{id}:error"), "message": error.message}));
                }
            }
            _ => {}
        }
    }
    out
}

fn tool(call: &str, name: &str, state: ToolState, directory: &str) -> Value {
    let (status, input, content, metadata, error) = match state {
        ToolState::Streaming { .. } => ("streaming", Value::Null, Vec::new(), Map::new(), None),
        ToolState::Running { input, metadata } => ("running", input, Vec::new(), metadata, None),
        ToolState::Completed {
            input,
            content,
            metadata,
        } => ("completed", input, content, metadata, None),
        ToolState::Error {
            input,
            error,
            content,
            metadata,
        } => ("error", input, content, metadata, Some(error.message)),
    };
    let output: String = content
        .iter()
        .filter_map(|c| match c {
            ToolContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "kind": "tool",
        "id": call,
        "name": name,
        "status": status,
        "target": target(name, &input, directory),
        "files": metadata.get("files").cloned().unwrap_or(Value::Null),
        // A subagent's own session, to read its transcript.
        "child": metadata.get("sessionID").cloned().unwrap_or(Value::Null),
        "output": clip(&output),
        "error": error,
    })
}

/// What a call is about, in one line: a path relative to the project, a pattern, a command.
pub fn target(name: &str, input: &Value, directory: &str) -> String {
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| input.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };
    let text = match name {
        "read" | "edit" | "write" | "list" => field(&["filePath", "path"]),
        "grep" | "glob" => field(&["pattern"]),
        "shell" | "bash" => field(&["command"]),
        "webfetch" => field(&["url"]),
        "websearch" => field(&["query"]),
        "task" | "subagent" => field(&["description", "prompt"]),
        "question" => input
            .pointer("/questions/0/question")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => field(&[
            "filePath",
            "path",
            "pattern",
            "command",
            "url",
            "query",
            "description",
        ]),
    };
    relative(&text, directory)
}

fn relative(path: &str, directory: &str) -> String {
    let dir = directory.trim_end_matches(['/', '\\']);
    path.strip_prefix(dir)
        .and_then(|rest| rest.strip_prefix(['/', '\\']))
        .unwrap_or(path)
        .to_string()
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(OUTPUT_CHARS) {
        Some((at, _)) => format!("{}\n…", &text[..at]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_relative_to_the_project() {
        let input = json!({"filePath": "/p/web/src/a.tsx"});
        assert_eq!(target("edit", &input, "/p/web"), "src/a.tsx");
        assert_eq!(target("edit", &input, "/p/web/"), "src/a.tsx");
        assert_eq!(target("edit", &input, "/q"), "/p/web/src/a.tsx");
        assert_eq!(
            target("shell", &json!({"command": "pnpm test"}), "/p"),
            "pnpm test"
        );
    }

    #[test]
    fn long_output_is_clipped() {
        let long = "x".repeat(OUTPUT_CHARS + 10);
        assert_eq!(clip(&long).chars().count(), OUTPUT_CHARS + 2);
        assert_eq!(clip("short"), "short");
    }
}
