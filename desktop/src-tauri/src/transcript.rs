//! A session's stored messages as the window shows them: user turns, the model's text and
//! reasoning, and tool calls with the diffs edits attach. Read from SQLite on demand, so a
//! missed event never leaves the window wrong for longer than the next refresh.

use anysola_core::api::Core;
use anysola_core::model::ModelRef;
use anysola_core::session::message::{Message, Part, ToolState};
use anysola_core::tool::ToolContent;
use serde_json::{Map, Value, json};

/// A stored message decoded once; `idle` keeps the data of the row a finished turn appends.
pub struct Entry {
    pub id: String,
    pub message: Message,
    pub idle: Option<Value>,
}

/// The session's messages, each decoded once.
pub fn entries(core: &Core, session_id: &str) -> Result<Vec<Entry>, String> {
    Ok(core
        .db()
        .messages(session_id)
        .map_err(crate::text)?
        .into_iter()
        .map(|row| Entry {
            message: Message::decode(&row.kind, &row.data),
            idle: (row.kind == "idle").then_some(row.data),
            id: row.id,
        })
        .collect())
}

/// What the window shows of a session: its items up to a staged revert, the revert itself
/// (turns hidden, files put back), the latest todo list, what waits in its inbox, how full the
/// model's context is and the background commands still running.
pub fn view(core: &Core, session_id: &str, directory: &str) -> Result<Value, String> {
    let messages = entries(core, session_id)?;
    let boundary = core.revert_boundary(session_id).map_err(crate::text)?;
    let (shown, hidden) = match &boundary {
        Some(b) => match messages.iter().position(|e| e.id == *b) {
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
            .filter(|e| matches!(e.message, Message::User(_)))
            .count();
        json!({"turns": turns, "files": core.revert_files(session_id).unwrap_or_default()})
    });
    // The agent of the last shown reply: a plan offers to be built only when Plan wrote it.
    let agent = shown.iter().rev().find_map(|e| match &e.message {
        Message::Assistant(data) => Some(data.agent.clone()),
        _ => None,
    });
    let context = context(core, session_id, &shown);
    Ok(json!({
        "agent": agent,
        "todos": todos(&shown),
        "context": context,
        "items": items(shown, directory),
        "revert": revert,
        "queued": queued(core, session_id),
        "background": background(core, session_id),
    }))
}

/// The input of the latest completed `todowrite` call (what `Core::todos` reads, from the
/// messages already decoded).
fn todos(shown: &[Entry]) -> Value {
    shown
        .iter()
        .rev()
        .filter_map(|e| match &e.message {
            Message::Assistant(data) => Some(data.content.iter().rev()),
            _ => None,
        })
        .flatten()
        .find_map(|part| match part {
            Part::Tool {
                name,
                state: ToolState::Completed { input, .. },
                ..
            } if name == "todowrite" => input.get("todos").filter(|t| t.is_array()).cloned(),
            _ => None,
        })
        .unwrap_or_else(|| json!([]))
}

/// Tokens of the latest step that reported them, over the session model's context window;
/// null while either is unknown.
fn context(core: &Core, session_id: &str, shown: &[Entry]) -> Value {
    let loaded = core.loaded();
    let model = core
        .session(session_id)
        .ok()
        .flatten()
        .and_then(|row| row.model)
        .as_ref()
        .and_then(ModelRef::from_json)
        .map(|r| r.id)
        .or_else(|| {
            loaded
                .config
                .model()
                .and_then(ModelRef::parse)
                .map(|r| r.id)
        })
        // As `info` picks it for a session that never chose one.
        .or_else(|| loaded.models.first().map(|m| m.id.clone()));
    let limit = model
        .and_then(|id| loaded.models.iter().find(|m| m.id == id))
        .and_then(|m| m.context)
        .filter(|limit| *limit > 0);
    context_of(shown, limit)
}

/// `{tokens, limit}` from the latest step that reported tokens; null without one or a limit.
fn context_of(shown: &[Entry], limit: Option<u64>) -> Value {
    let tokens = shown.iter().rev().find_map(|e| match &e.message {
        Message::Assistant(data) => data
            .tokens
            .map(|t| t.input + t.output + t.reasoning + t.cache.read + t.cache.write)
            .filter(|total| *total > 0),
        _ => None,
    });
    match (tokens, limit) {
        (Some(tokens), Some(limit)) => json!({"tokens": tokens, "limit": limit}),
        _ => Value::Null,
    }
}

/// The user's messages waiting in the session's inbox, oldest first.
pub fn queued(core: &Core, session_id: &str) -> Value {
    queued_of(core.pending(session_id).unwrap_or_default())
}

/// Inbox rows as `Queued` items: the user's messages only (not background notices or a
/// waiting compaction).
fn queued_of(rows: Vec<anysola_core::storage::InboxRow>) -> Value {
    Value::Array(
        rows.into_iter()
            .filter(|row| row.kind == "user")
            .map(|row| {
                let images = row.payload["files"].as_array().map_or(0, |files| {
                    files
                        .iter()
                        .filter(|f| f["mime"].as_str().is_some_and(|m| m.starts_with("image/")))
                        .count()
                });
                json!({
                    "id": row.id,
                    "text": row.payload["text"].as_str().unwrap_or_default(),
                    "images": images,
                    "delivery": if row.delivery == "steer" { "steer" } else { "queue" },
                })
            })
            .collect(),
    )
}

/// The session and its subagents' sessions, the session first.
pub fn family(core: &Core, session_id: &str) -> Vec<String> {
    let mut out = vec![session_id.to_string()];
    let mut at = 0;
    // Subagents nest a few levels at most; the bound only guards a broken chain.
    while at < out.len() && out.len() < 256 {
        let children = core.children(&out[at]).unwrap_or_default();
        out.extend(children.into_iter().map(|(id, _, _)| id));
        at += 1;
    }
    out
}

/// Background commands of the session and its subagents that still run.
fn background(core: &Core, session_id: &str) -> Value {
    Value::Array(
        family(core, session_id)
            .iter()
            .flat_map(|id| core.background_shells(id))
            .map(|shell| {
                json!({
                    "id": shell.id,
                    "sessionId": shell.session_id,
                    "command": shell.command,
                    "started": shell.started,
                })
            })
            .collect(),
    )
}

/// Tool output kept for display; a command's full log stays in the session.
const OUTPUT_CHARS: usize = 4_000;

/// When the turn a user message started ended, from the `idle` row that closed it: `None` while
/// it runs. A turn the app's closing cut short ends at its last step instead.
fn turn_ends(entries: &[Entry]) -> Vec<Option<u64>> {
    let mut ends = vec![None; entries.len()];
    let mut open: Vec<usize> = Vec::new();
    let mut last_step: Option<u64> = None;
    for (at, entry) in entries.iter().enumerate() {
        match &entry.message {
            Message::User(_) => {
                // A new turn: one cut off before its first step has no step time, not the
                // previous turn's.
                if open.is_empty() {
                    last_step = None;
                }
                open.push(at);
            }
            Message::Assistant(data) => {
                last_step = Some(data.time.completed.unwrap_or(data.time.created));
            }
            _ => {}
        }
        if let Some(idle) = &entry.idle {
            let end = if idle["reason"] == "closed" {
                last_step
            } else {
                idle.pointer("/time/created").and_then(Value::as_u64)
            };
            for user in open.drain(..) {
                ends[user] = end;
            }
        }
    }
    ends
}

/// The note an interrupted turn leaves, by the reason its `idle` row keeps.
fn interrupted_note(idle: &Value) -> Option<&'static str> {
    if idle["outcome"] != "interrupted" {
        return None;
    }
    Some(match idle["reason"].as_str() {
        Some("closed") => "Interrupted when the app closed.",
        Some("budget") => "Stopped at the session’s budget.",
        _ => "Stopped.",
    })
}

pub fn items(entries: Vec<Entry>, directory: &str) -> Vec<Value> {
    let ends = turn_ends(&entries);
    let mut out = Vec::new();
    for (entry, end) in entries.into_iter().zip(ends) {
        let Entry { id, message, idle } = entry;
        if let Some(idle) = idle {
            if let Some(text) = interrupted_note(&idle) {
                out.push(json!({"kind": "note", "id": id, "text": text, "failed": false}));
            }
            continue;
        }
        match message {
            Message::User(data) => {
                let images = data.files.as_ref().map_or(0, |f| {
                    f.iter().filter(|f| f.mime.starts_with("image/")).count()
                });
                if !data.text.is_empty() || images > 0 {
                    // A command's message shows as the line the user typed.
                    let command = data.extra.get("command").and_then(Value::as_str);
                    let sent = data.time.created;
                    out.push(json!({
                        "kind": "user",
                        "id": id,
                        "text": data.text,
                        "images": images,
                        "command": command,
                        "time": sent,
                        "elapsed": end.map(|end| end.saturating_sub(sent)),
                    }));
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
                            time,
                            ..
                        } => {
                            let mut item = tool(&call, &name, state, directory);
                            item["elapsed"] = json!(time.completed.map(|end| {
                                end.saturating_sub(time.ran.unwrap_or(time.created))
                            }));
                            out.push(item);
                        }
                        _ => {}
                    }
                }
                // An interrupted step is not a failure: its turn's note says how it ended.
                if let Some(error) = data.error.filter(|e| e.kind != "aborted") {
                    let (kind, retry) = crate::events::error_kind(&error.kind, &error.message);
                    out.push(json!({
                        "kind": "error",
                        "id": format!("{id}:error"),
                        "message": crate::events::plain(kind, &error.message),
                        "errorKind": kind,
                        "retry": retry,
                    }));
                }
            }
            // Other synthetic rows (background notices, a skill's text) are for the model only.
            Message::Synthetic(data) => {
                if let Some(text) = hook_note(data.description.as_deref(), &data.text) {
                    // A hook with several contexts, or several hooks, wrote one row each.
                    let repeated = out
                        .last()
                        .is_some_and(|last| last["kind"] == "note" && last["text"] == text);
                    if !repeated {
                        out.push(json!({"kind": "note", "id": id, "text": text, "failed": false}));
                    }
                }
            }
            Message::Compaction(data) => {
                let (text, failed) = match data.status.as_str() {
                    "completed" => ("Compacted the conversation so far".to_string(), false),
                    "failed" => (
                        match data.error {
                            Some(error) => format!("Couldn’t compact: {}", error.message),
                            None => "Couldn’t compact the conversation".to_string(),
                        },
                        true,
                    ),
                    _ => ("Compacting the conversation…".to_string(), false),
                };
                out.push(json!({"kind": "note", "id": id, "text": text, "failed": failed}));
            }
            _ => {}
        }
    }
    out
}

/// The descriptions the core gives the synthetic rows hooks write (`hooks::fire`, private there).
const HOOK_CONTEXT: &str = "Hook context";
const STOP_HOOK: &str = "Stop hook";

/// The note a synthetic row written by a hook leaves in the conversation.
fn hook_note(description: Option<&str>, text: &str) -> Option<String> {
    match description? {
        HOOK_CONTEXT => Some("A hook added context.".into()),
        STOP_HOOK => Some(match text.lines().map(str::trim).find(|l| !l.is_empty()) {
            Some(line) => format!("A hook kept the agent going: {line}"),
            None => "A hook kept the agent going.".into(),
        }),
        _ => None,
    }
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
    let mut item = json!({
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
    });
    // What the user's hooks did around the call; absent when they said nothing.
    if let Some(hooks) = metadata.get("hooks").and_then(Value::as_array)
        && !hooks.is_empty()
    {
        item["hooks"] = Value::Array(hooks.iter().map(hook_run).collect());
    }
    item
}

/// A `metadata.hooks` entry as `HookRun`, each field present.
fn hook_run(entry: &Value) -> Value {
    let field = |key: &str| entry[key].as_str().unwrap_or_default();
    json!({
        "event": field("event"),
        "command": field("command"),
        "outcome": field("outcome"),
        "message": field("message"),
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

    fn entry(id: &str, kind: &str, data: Value) -> Entry {
        Entry {
            id: id.into(),
            message: Message::decode(kind, &data),
            idle: (kind == "idle").then_some(data),
        }
    }

    fn user(id: &str, at: u64) -> Entry {
        entry(id, "user", json!({"time": {"created": at}, "text": id}))
    }

    fn step(id: &str, at: u64, done: u64) -> Entry {
        entry(
            id,
            "assistant",
            json!({"time": {"created": at, "completed": done}, "agent": "build",
                   "model": {"id": "m", "providerID": "anysola"}, "finish": "stop",
                   "content": [{"type": "tool", "id": "call_1", "name": "shell",
                                "state": {"status": "completed", "input": {"command": "ls"}, "content": []},
                                "time": {"created": at, "ran": at + 10, "completed": at + 250}}]}),
        )
    }

    fn idle(id: &str, at: u64, outcome: &str, reason: Option<&str>) -> Entry {
        let mut data = json!({"time": {"created": at}, "outcome": outcome});
        if let Some(reason) = reason {
            data["reason"] = json!(reason);
        }
        entry(id, "idle", data)
    }

    #[test]
    fn turns_carry_their_times_and_stops_leave_a_note() {
        let entries = vec![
            user("u1", 1_000),
            step("a1", 1_100, 1_900),
            idle("i1", 2_000, "succeeded", None),
            user("u2", 3_000),
            idle("i2", 3_500, "interrupted", Some("user")),
            user("u3", 4_000),
            step("a3", 4_100, 4_600),
            // The app closed under it; it was settled when the session opened, much later.
            idle("i3", 90_000, "interrupted", Some("closed")),
            // Closed before its first step.
            user("u5", 95_000),
            idle("i5", 99_000, "interrupted", Some("closed")),
            user("u4", 100_000),
        ];
        let items = items(entries, "/p");
        let user = |id: &str| items.iter().find(|i| i["id"] == id).unwrap().clone();
        assert_eq!(
            (user("u1")["time"].clone(), user("u1")["elapsed"].clone()),
            (json!(1_000), json!(1_000))
        );
        assert_eq!(user("u2")["elapsed"], 500);
        assert_eq!(
            user("u3")["elapsed"],
            600,
            "ends at its last step, not when it was settled"
        );
        assert_eq!(user("u5")["elapsed"], Value::Null, "no time of its own");
        assert_eq!(user("u4")["elapsed"], Value::Null, "still running");
        let tool = items.iter().find(|i| i["kind"] == "tool").unwrap();
        assert_eq!(tool["elapsed"], 240);
        let notes: Vec<_> = items
            .iter()
            .filter(|i| i["kind"] == "note")
            .map(|i| (i["id"].as_str().unwrap(), i["text"].as_str().unwrap()))
            .collect();
        assert_eq!(
            notes,
            [
                ("i2", "Stopped."),
                ("i3", "Interrupted when the app closed."),
                ("i5", "Interrupted when the app closed.")
            ]
        );
    }

    #[test]
    fn context_is_the_latest_step_with_tokens_over_the_window() {
        let step = |tokens: Value| {
            entry(
                "a",
                "assistant",
                json!({"time": {"created": 1}, "agent": "build", "model": {"id": "m", "providerID": "anysola"},
                       "content": [], "tokens": tokens}),
            )
        };
        let used = json!({"input": 100, "output": 20, "reasoning": 5, "cache": {"read": 800, "write": 75}});
        let zero =
            json!({"input": 0, "output": 0, "reasoning": 0, "cache": {"read": 0, "write": 0}});
        let shown = [step(used), step(zero), user("u", 2)];
        assert_eq!(
            context_of(&shown, Some(200_000)),
            json!({"tokens": 1000, "limit": 200_000})
        );
        assert_eq!(context_of(&shown, None), Value::Null);
        assert_eq!(context_of(&[user("u", 1)], Some(200_000)), Value::Null);
    }

    #[test]
    fn queued_lists_the_users_messages_in_order() {
        let row = |id: &str, kind: &str, payload: Value, delivery: &str| {
            anysola_core::storage::InboxRow {
                id: id.into(),
                kind: kind.into(),
                payload,
                delivery: delivery.into(),
            }
        };
        let rows = vec![
            row(
                "m1",
                "user",
                json!({"text": "first", "files": [{"mime": "image/png", "data": "", "source": {}},
                                                  {"mime": "text/plain", "data": "", "source": {}}]}),
                "queue",
            ),
            row("m2", "synthetic", json!({"text": "<shell> done"}), "steer"),
            row("m3", "compaction", json!({}), "steer"),
            row("m4", "user", json!({"text": "second"}), "steer"),
        ];
        assert_eq!(
            queued_of(rows),
            json!([
                {"id": "m1", "text": "first", "images": 1, "delivery": "queue"},
                {"id": "m4", "text": "second", "images": 0, "delivery": "steer"},
            ])
        );
    }

    #[test]
    fn todos_come_from_the_latest_completed_list() {
        let list = |todos: Value| {
            entry(
                "a",
                "assistant",
                json!({"time": {"created": 1}, "agent": "build", "model": {"id": "m", "providerID": "anysola"},
                       "content": [{"type": "tool", "id": "c", "name": "todowrite",
                                    "state": {"status": "completed", "input": {"todos": todos}, "content": []},
                                    "time": {"created": 1}}]}),
            )
        };
        let first = json!([{"content": "a", "status": "pending"}]);
        let second = json!([{"content": "a", "status": "completed"}]);
        assert_eq!(todos(&[list(first), list(second.clone())]), second);
        assert_eq!(todos(&[]), json!([]));
    }

    #[test]
    fn hooks_show_on_tool_items_and_as_notes() {
        let synthetic = |id: &str, description: &str, text: &str| {
            entry(
                id,
                "synthetic",
                json!({"time": {"created": 1}, "text": text, "description": description}),
            )
        };
        let hooks = json!([{"event": "PreToolUse", "command": "./guard.sh", "outcome": "blocked",
                            "message": "No rm -rf"}]);
        let call = |id: &str, metadata: Value| {
            entry(
                id,
                "assistant",
                json!({"time": {"created": 1}, "agent": "build", "model": {"id": "m", "providerID": "anysola"},
                       "content": [{"type": "tool", "id": id, "name": "shell",
                                    "state": {"status": "error", "input": {"command": "rm -rf /"},
                                              "error": {"type": "tool", "message": "No rm -rf"},
                                              "content": [], "metadata": metadata},
                                    "time": {"created": 1}}]}),
            )
        };
        let entries = vec![
            synthetic("s1", "Hook context", "Branch: main"),
            synthetic("s2", "Hook context", "Node 26"),
            user("u1", 2),
            call("c1", json!({"hooks": hooks})),
            call("c2", json!({})),
            synthetic("s3", "Stop hook", "\nTests fail: 3\nmore"),
            synthetic("s4", "Background", "<shell> done"),
        ];
        let items = items(entries, "/p");
        let notes: Vec<_> = items
            .iter()
            .filter(|i| i["kind"] == "note")
            .map(|i| (i["id"].as_str().unwrap(), i["text"].as_str().unwrap()))
            .collect();
        assert_eq!(
            notes,
            [
                ("s1", "A hook added context."),
                ("s3", "A hook kept the agent going: Tests fail: 3")
            ]
        );
        let tool = |id: &str| items.iter().find(|i| i["id"] == id).unwrap().clone();
        assert_eq!(tool("c1")["hooks"], hooks);
        assert!(tool("c2").get("hooks").is_none());
    }

    #[test]
    fn long_output_is_clipped() {
        let long = "x".repeat(OUTPUT_CHARS + 10);
        assert_eq!(clip(&long).chars().count(), OUTPUT_CHARS + 2);
        assert_eq!(clip("short"), "short");
    }
}
