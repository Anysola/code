//! The transcript as the TUI shows it: built from stored messages, then kept current by core
//! events while a turn streams.

use anysola_core::events::Event;
use anysola_core::session::message::{Message, Part as StoredPart, ToolState};
use anysola_core::tool::ToolContent;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    User {
        id: String,
        text: String,
        agent: String,
        files: Vec<String>,
    },
    Assistant(Assistant),
    Shell {
        command: String,
        output: String,
        exit: Option<i64>,
        timed_out: bool,
    },
    /// A one-line notice (`Switched agent to Plan`, `↳ Moved to <dir>`, `→ Skill <name>`).
    Notice(String),
    Compaction {
        running: bool,
        summary: Option<String>,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Assistant {
    pub id: String,
    pub agent: String,
    pub model: String,
    pub created: u64,
    pub completed: Option<u64>,
    /// The step ended the turn (any finish but tool calls).
    pub terminal: bool,
    pub parts: Vec<Part>,
    pub error: Option<String>,
    pub interrupted: bool,
    pub output_tokens: u64,
    /// Every token of the step (input, output, reasoning, cache): the context it used.
    pub usage: u64,
    /// When the user input of this turn was created (start of the footer's duration).
    pub turn_start: u64,
}

// Tools dominate the parts; boxing them buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text {
        text: String,
        done: bool,
    },
    Reasoning {
        text: String,
        start: u64,
        end: Option<u64>,
    },
    Tool(Tool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Streaming,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub call_id: String,
    pub name: String,
    pub input: Value,
    pub status: Status,
    pub output: String,
    pub metadata: Map<String, Value>,
    pub error: Option<String>,
    pub started: u64,
    pub awaiting: bool,
}

impl Tool {
    pub fn str(&self, key: &str) -> Option<&str> {
        self.input.get(key).and_then(Value::as_str)
    }
}

fn content_text(content: &[ToolContent]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            ToolContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn part(stored: StoredPart) -> Option<Part> {
    Some(match stored {
        StoredPart::Text { text, .. } => Part::Text { text, done: true },
        StoredPart::Reasoning { text, time, .. } => {
            let time = time.unwrap_or_default();
            Part::Reasoning {
                text,
                start: time.created,
                end: time.completed.or(Some(time.created)),
            }
        }
        StoredPart::Tool {
            id,
            name,
            state,
            time,
            ..
        } => {
            let mut tool = Tool {
                call_id: id,
                name,
                input: Value::Null,
                status: Status::Streaming,
                output: String::new(),
                metadata: Map::new(),
                error: None,
                started: time.created,
                awaiting: false,
            };
            match state {
                ToolState::Streaming { .. } => {}
                ToolState::Running { input, metadata } => {
                    tool.input = input;
                    tool.metadata = metadata;
                    tool.status = Status::Running;
                }
                ToolState::Completed {
                    input,
                    content,
                    metadata,
                } => {
                    tool.input = input;
                    tool.output = content_text(&content);
                    tool.metadata = metadata;
                    tool.status = Status::Completed;
                }
                ToolState::Error {
                    input,
                    error,
                    content,
                    metadata,
                } => {
                    tool.input = input;
                    tool.output = content_text(&content);
                    tool.metadata = metadata;
                    tool.error = Some(error.message);
                    tool.status = Status::Failed;
                }
            }
            Part::Tool(tool)
        }
    })
}

fn total(t: &anysola_core::session::message::Tokens) -> u64 {
    t.input + t.output + t.reasoning + t.cache.read + t.cache.write
}

/// Model display names by id, for footers.
pub type Names<'a> = &'a dyn Fn(&str) -> String;

/// Builds the items of a session from its stored messages, up to the revert boundary.
pub fn from_messages(
    messages: Vec<(String, Message)>,
    boundary: Option<&str>,
    name: Names,
) -> Vec<Item> {
    let mut items = Vec::new();
    let mut turn_start = 0;
    let mut agent = String::from("build");
    for (id, message) in messages {
        if Some(id.as_str()) == boundary {
            break;
        }
        match message {
            Message::User(data) => {
                turn_start = data.time.created;
                let files = data
                    .files
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f| f.name.unwrap_or_else(|| "attachment".into()))
                    .collect();
                items.push(Item::User {
                    id,
                    text: data.text,
                    agent: agent.clone(),
                    files,
                });
            }
            Message::Assistant(data) => {
                agent = data.agent.clone();
                let terminal = data.finish.as_deref().is_some_and(|f| f != "tool-calls");
                let interrupted = data
                    .error
                    .as_ref()
                    .is_some_and(|e| e.message == "Step interrupted");
                items.push(Item::Assistant(Assistant {
                    id,
                    agent: data.agent,
                    model: name(&data.model.id),
                    created: data.time.created,
                    completed: data.time.completed,
                    terminal: terminal || data.error.is_some(),
                    parts: data.content.into_iter().filter_map(part).collect(),
                    error: data.error.filter(|_| !interrupted).map(|e| e.message),
                    interrupted,
                    output_tokens: data.tokens.map_or(0, |t| t.output),
                    usage: data.tokens.map_or(0, |t| total(&t)),
                    turn_start,
                }));
            }
            Message::Shell(data) => {
                let metadata = data.metadata.unwrap_or(Value::Null);
                items.push(Item::Shell {
                    command: data.command,
                    output: data.output.map(|o| o.output).unwrap_or_default(),
                    exit: metadata.get("exit").and_then(Value::as_i64),
                    timed_out: metadata.get("timedOut").and_then(Value::as_bool) == Some(true),
                });
            }
            Message::AgentSwitched(data) => {
                let to = crate::text::title_case(&data.agent);
                items.push(Item::Notice(match data.previous {
                    Some(from) => format!(
                        "Switched agent from {} to {to}",
                        crate::text::title_case(&from)
                    ),
                    None => format!("Switched agent to {to}"),
                }));
                agent = data.agent;
            }
            Message::ModelSwitched(data) => {
                let model = name(&data.model.id);
                items.push(Item::Notice(match data.model.variant {
                    Some(variant) => format!("Switched model to {model} ({variant})"),
                    None => format!("Switched model to {model}"),
                }));
            }
            Message::LocationSwitched(data) => {
                items.push(Item::Notice(format!(
                    "↳ Moved to {}",
                    data.location.directory
                )));
            }
            Message::Skill(data) => items.push(Item::Notice(format!("→ Skill {}", data.name))),
            Message::Compaction(data) => items.push(Item::Compaction {
                running: data.status == "running" || data.status == "pending",
                summary: data.summary,
                error: data.error.map(|e| e.message),
            }),
            _ => {}
        }
    }
    items
}

fn assistant<'a>(items: &'a mut [Item], message_id: &str) -> Option<&'a mut Assistant> {
    items.iter_mut().rev().find_map(|item| match item {
        Item::Assistant(a) if a.id == message_id => Some(a),
        _ => None,
    })
}

fn nth_text(parts: &mut [Part], ordinal: usize, reasoning: bool) -> Option<&mut String> {
    parts
        .iter_mut()
        .filter_map(|p| match p {
            Part::Text { text, .. } if !reasoning => Some(text),
            Part::Reasoning { text, .. } if reasoning => Some(text),
            _ => None,
        })
        .nth(ordinal)
}

fn tool<'a>(items: &'a mut [Item], call_id: &str) -> Option<&'a mut Tool> {
    items.iter_mut().rev().find_map(|item| match item {
        Item::Assistant(a) => a.parts.iter_mut().find_map(|p| match p {
            Part::Tool(t) if t.call_id == call_id => Some(t),
            _ => None,
        }),
        _ => None,
    })
}

/// Applies a streaming event of this session; `now` stamps live timings.
pub fn apply(items: &mut Vec<Item>, event: &Event, now: u64, name: Names) {
    match event {
        Event::StepStarted {
            message_id,
            agent,
            model_id,
            ..
        } => {
            if assistant(items, message_id).is_none() {
                let turn_start = items
                    .iter()
                    .rev()
                    .find_map(|i| match i {
                        Item::Assistant(a) => Some(a.turn_start),
                        Item::User { .. } => Some(now),
                        _ => None,
                    })
                    .unwrap_or(now);
                items.push(Item::Assistant(Assistant {
                    id: message_id.clone(),
                    agent: agent.clone(),
                    model: name(model_id),
                    created: now,
                    turn_start,
                    ..Assistant::default()
                }));
            }
        }
        Event::TextStarted { message_id, .. } => {
            if let Some(a) = assistant(items, message_id) {
                a.parts.push(Part::Text {
                    text: String::new(),
                    done: false,
                });
            }
        }
        Event::TextDelta {
            message_id,
            ordinal,
            text,
            ..
        }
        | Event::ReasoningDelta {
            message_id,
            ordinal,
            text,
            ..
        } => {
            let reasoning = matches!(event, Event::ReasoningDelta { .. });
            if let Some(a) = assistant(items, message_id)
                && let Some(current) = nth_text(&mut a.parts, *ordinal, reasoning)
            {
                current.push_str(text);
            }
        }
        Event::TextEnded {
            message_id,
            ordinal,
            text,
            ..
        } => {
            if let Some(a) = assistant(items, message_id) {
                let mut seen = 0;
                for p in &mut a.parts {
                    if let Part::Text { text: t, done } = p {
                        if seen == *ordinal {
                            *t = text.clone();
                            *done = true;
                        }
                        seen += 1;
                    }
                }
            }
        }
        Event::ReasoningStarted { message_id, .. } => {
            if let Some(a) = assistant(items, message_id) {
                a.parts.push(Part::Reasoning {
                    text: String::new(),
                    start: now,
                    end: None,
                });
            }
        }
        Event::ReasoningEnded {
            message_id,
            ordinal,
            text,
            ..
        } => {
            if let Some(a) = assistant(items, message_id) {
                let mut seen = 0;
                for p in &mut a.parts {
                    if let Part::Reasoning { text: t, end, .. } = p {
                        if seen == *ordinal {
                            *t = text.clone();
                            *end = Some(now);
                        }
                        seen += 1;
                    }
                }
            }
        }
        Event::ToolInputStarted {
            message_id,
            call_id,
            name: tool_name,
            ..
        } => {
            if let Some(a) = assistant(items, message_id) {
                a.parts.push(Part::Tool(Tool {
                    call_id: call_id.clone(),
                    name: tool_name.clone(),
                    input: Value::Null,
                    status: Status::Streaming,
                    output: String::new(),
                    metadata: Map::new(),
                    error: None,
                    started: now,
                    awaiting: false,
                }));
            }
        }
        Event::ToolCalled { call_id, input, .. } => {
            if let Some(t) = tool(items, call_id) {
                t.input = input.clone();
                t.status = Status::Running;
                t.started = now;
            }
        }
        Event::ToolSucceeded {
            call_id,
            content,
            metadata,
            ..
        } => {
            if let Some(t) = tool(items, call_id) {
                t.output = content_text(content);
                t.metadata = metadata.clone();
                t.status = Status::Completed;
                t.awaiting = false;
            }
        }
        Event::ToolFailed {
            call_id,
            error,
            content,
            metadata,
            ..
        } => {
            if let Some(t) = tool(items, call_id) {
                t.output = content_text(content);
                t.metadata = metadata.clone();
                t.error = Some(error.message.clone());
                t.status = Status::Failed;
                t.awaiting = false;
            }
        }
        Event::StepEnded {
            message_id,
            finish,
            tokens,
            last,
            ..
        } => {
            if let Some(a) = assistant(items, message_id) {
                a.completed = Some(now);
                a.terminal = *last || finish != "tool-calls";
                a.output_tokens = tokens.output;
                a.usage = total(tokens);
            }
        }
        Event::StepFailed {
            message_id, error, ..
        } => {
            if let Some(a) = assistant(items, message_id) {
                a.completed = Some(now);
                a.terminal = true;
                if error.message == "Step interrupted" {
                    a.interrupted = true;
                } else {
                    a.error = Some(error.message.clone());
                }
            }
        }
        _ => {}
    }
}

/// The running tool a permission ask belongs to: the latest running call of `action`'s tool.
pub fn mark_awaiting(items: &mut [Item], action: &str) -> Option<Tool> {
    fn tool_name(name: &str) -> &str {
        match name {
            "bash" => "shell",
            "write" | "patch" | "apply_patch" => "edit",
            other => other,
        }
    }
    for item in items.iter_mut().rev() {
        if let Item::Assistant(a) = item {
            for part in a.parts.iter_mut().rev() {
                if let Part::Tool(t) = part
                    && t.status == Status::Running
                    && tool_name(&t.name) == action
                {
                    t.awaiting = true;
                    return Some(t.clone());
                }
            }
        }
    }
    None
}

pub fn clear_awaiting(items: &mut [Item]) {
    for item in items {
        if let Item::Assistant(a) = item {
            for part in &mut a.parts {
                if let Part::Tool(t) = part {
                    t.awaiting = false;
                }
            }
        }
    }
}
