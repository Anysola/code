//! Turns core events into `run`'s output: text mode (answer on stdout, the rest on stderr) or one
//! JSON event per stdout line, in the shapes `Anysola/code-action` reads.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anysola_core::events::Event;
use anysola_core::gateway::error::SessionError;
use anysola_core::tool::ToolContent;
use serde_json::{Map, Value, json};

use super::tools::inline_info;
use crate::ui;

pub const PERMISSION_REJECTED: &str = "This non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Text,
    Json,
}

struct Started {
    part_id: String,
    at: u64,
}

struct ToolCall {
    part_id: String,
    at: u64,
    name: String,
    input: Value,
    raw: String,
}

pub struct Renderer {
    pub mode: Mode,
    pub thinking: bool,
    pub session_id: String,
    pub directory: PathBuf,
    /// Set when the user interrupted (SIGINT): the resulting step error is not reported.
    pub user_interrupt: Arc<AtomicBool>,
    pub emitted_error: bool,
    stdout_tty: bool,
    starts: HashMap<(String, &'static str, usize), Started>,
    tools: HashMap<(String, String), ToolCall>,
}

/// `prt_` + the event id without its `evt_` prefix.
fn part_id(event_id: &str) -> String {
    format!("prt_{}", event_id.strip_prefix("evt_").unwrap_or(event_id))
}

fn now() -> u64 {
    anysola_core::time::now_ms()
}

/// What 0.5.1 displayed for a tool's content: the first text item for shell, all text items else.
pub fn output_text(name: &str, content: &[ToolContent]) -> String {
    let texts = content.iter().filter_map(|c| match c {
        ToolContent::Text { text } => Some(text.as_str()),
        ToolContent::File { .. } => None,
    });
    if name == "shell" {
        return texts.take(1).collect();
    }
    texts
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

impl Renderer {
    pub fn new(mode: Mode, thinking: bool, session_id: String, directory: PathBuf) -> Self {
        Renderer {
            mode,
            thinking,
            session_id,
            directory,
            user_interrupt: Arc::default(),
            emitted_error: false,
            stdout_tty: std::io::stdout().is_terminal(),
            starts: HashMap::new(),
            tools: HashMap::new(),
        }
    }

    fn emit(&self, kind: &str, timestamp: u64, extra: Value) {
        let mut line = Map::new();
        line.insert("type".into(), kind.into());
        line.insert("timestamp".into(), timestamp.into());
        line.insert("sessionID".into(), self.session_id.clone().into());
        if let Value::Object(extra) = extra {
            line.extend(extra);
        }
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}", crate::json(&Value::Object(line)));
        let _ = stdout.flush();
    }

    fn json(&self) -> bool {
        self.mode == Mode::Json
    }

    /// Reports an error: an `error` JSON line, or the red `Error:` line.
    pub fn error(&mut self, error: &SessionError) {
        self.emitted_error = true;
        if self.json() {
            self.emit("error", now(), json!({"error": error}));
        } else {
            ui::error(&error.message);
        }
    }

    fn write_answer(&self, text: &str, dim: bool) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if !self.stdout_tty {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{text}");
            return;
        }
        ui::empty();
        if dim {
            ui::println(&format!("{}\x1b[3m{text}\x1b[0m{}", ui::DIM, ui::NORMAL));
        } else {
            ui::println(text);
        }
        ui::empty();
    }

    fn render_tool(
        &self,
        name: &str,
        input: &Map<String, Value>,
        metadata: &Map<String, Value>,
        output: Option<&str>,
    ) {
        let info = inline_info(name, input, metadata, output, &self.directory);
        if info.block {
            ui::empty();
            ui::println(&format!(
                "{}{} {}{}",
                ui::NORMAL,
                info.icon,
                ui::NORMAL,
                info.title
            ));
            if let Some(body) = info.body.filter(|b| !b.trim().is_empty()) {
                ui::println(&body);
            }
            ui::empty();
            return;
        }
        let description = info
            .description
            .map(|d| format!("{}{d}{}", ui::DIM, ui::NORMAL))
            .unwrap_or_default();
        ui::println(&format!(
            "{}{} {}{} {description}",
            ui::NORMAL,
            info.icon,
            ui::NORMAL,
            info.title
        ));
    }

    /// Handles one event of this run's session; returns true when the turn is over.
    pub fn handle(&mut self, event: &Event) -> bool {
        let at = now();
        match event {
            Event::StepStarted {
                message_id,
                event_id,
                agent,
                model_id,
                ..
            } => {
                if self.json() {
                    let part = json!({"id": part_id(event_id), "sessionID": self.session_id, "messageID": message_id, "type": "step-start"});
                    self.emit("step_start", at, json!({"part": part}));
                } else {
                    ui::empty();
                    ui::println(&format!("> {agent} · {model_id}"));
                    ui::empty();
                }
            }
            Event::TextStarted {
                message_id,
                event_id,
                ordinal,
                ..
            } => {
                self.starts.insert(
                    (message_id.clone(), "text", *ordinal),
                    Started {
                        part_id: part_id(event_id),
                        at,
                    },
                );
            }
            Event::ReasoningStarted {
                message_id,
                event_id,
                ordinal,
                ..
            } => {
                self.starts.insert(
                    (message_id.clone(), "reasoning", *ordinal),
                    Started {
                        part_id: part_id(event_id),
                        at,
                    },
                );
            }
            Event::TextEnded {
                message_id,
                event_id,
                ordinal,
                text,
                ..
            } => {
                let started = self.starts.remove(&(message_id.clone(), "text", *ordinal));
                if self.json() {
                    let (id, start) =
                        started.map_or((part_id(event_id), at), |s| (s.part_id, s.at));
                    let part = json!({"id": id, "sessionID": self.session_id, "messageID": message_id, "type": "text", "text": text, "time": {"start": start, "end": at}});
                    self.emit("text", at, json!({"part": part}));
                } else {
                    self.write_answer(text, false);
                }
            }
            Event::ReasoningEnded {
                message_id,
                event_id,
                ordinal,
                text,
                native,
                ..
            } => {
                let started = self
                    .starts
                    .remove(&(message_id.clone(), "reasoning", *ordinal));
                if !self.thinking {
                    return false;
                }
                if self.json() {
                    let (id, start) =
                        started.map_or((part_id(event_id), at), |s| (s.part_id, s.at));
                    let mut part = json!({"id": id, "sessionID": self.session_id, "messageID": message_id, "type": "reasoning", "text": text});
                    if let Some(native) = native {
                        part["metadata"] = native.clone();
                    }
                    part["time"] = json!({"start": start, "end": at});
                    self.emit("reasoning", at, json!({"part": part}));
                } else if !text.trim().is_empty() {
                    self.write_answer(&format!("Thinking: {}", text.trim()), true);
                }
            }
            Event::ToolInputStarted {
                message_id,
                event_id,
                call_id,
                name,
                ..
            } => {
                self.tools.insert(
                    (message_id.clone(), call_id.clone()),
                    ToolCall {
                        part_id: part_id(event_id),
                        at,
                        name: name.clone(),
                        input: json!({}),
                        raw: String::new(),
                    },
                );
            }
            Event::ToolCalled {
                message_id,
                call_id,
                input,
                raw,
                ..
            } => {
                if let Some(call) = self.tools.get_mut(&(message_id.clone(), call_id.clone())) {
                    call.input = input.clone();
                    call.raw = raw.clone();
                }
            }
            Event::ToolSucceeded {
                message_id,
                call_id,
                content,
                metadata,
                ..
            } => {
                let Some(call) = self.tools.remove(&(message_id.clone(), call_id.clone())) else {
                    return false;
                };
                let output = output_text(&call.name, content);
                if self.json() {
                    let part = json!({
                        "partID": call.part_id, "sessionID": self.session_id, "messageID": message_id, "type": "tool",
                        "id": call_id, "tool": call.name,
                        "state": {
                            "status": "completed", "input": call.input, "output": output, "title": call.name,
                            "metadata": {"metadata": metadata, "content": content, "providerCall": {"executed": false},
                                "providerResult": {"executed": false}, "rawInput": call.raw},
                            "time": {"start": call.at, "end": at},
                        },
                    });
                    self.emit("tool_use", at, json!({"part": part}));
                } else {
                    let input = call.input.as_object().cloned().unwrap_or_default();
                    self.render_tool(&call.name, &input, metadata, Some(&output));
                }
            }
            Event::ToolFailed {
                message_id,
                call_id,
                error,
                content,
                metadata,
                ..
            } => {
                let Some(call) = self.tools.remove(&(message_id.clone(), call_id.clone())) else {
                    return false;
                };
                if self.json() {
                    let part = json!({
                        "partID": call.part_id, "sessionID": self.session_id, "messageID": message_id, "type": "tool",
                        "id": call_id, "tool": call.name,
                        "state": {
                            "status": "error", "input": call.input, "error": error.message,
                            "metadata": {"providerCall": {"executed": false}, "providerResult": {"executed": false}, "rawInput": call.raw},
                            "time": {"start": call.at, "end": at},
                        },
                    });
                    self.emit("tool_use", at, json!({"part": part}));
                } else {
                    let input = call.input.as_object().cloned().unwrap_or_default();
                    let output = output_text(&call.name, content);
                    if !output.trim().is_empty() {
                        self.render_tool(&call.name, &input, metadata, Some(&output));
                    }
                    let info = inline_info(&call.name, &input, metadata, None, &self.directory);
                    ui::println(&format!(
                        "{}✗ {}{} failed",
                        ui::NORMAL,
                        ui::NORMAL,
                        info.title
                    ));
                    ui::error(&error.message);
                }
            }
            Event::StepEnded {
                message_id,
                event_id,
                finish,
                tokens,
                last,
                ..
            } => {
                if self.json() {
                    let id = if *last {
                        format!(
                            "prt_{}_step-finish",
                            message_id.strip_prefix("msg_").unwrap_or(message_id)
                        )
                    } else {
                        part_id(event_id)
                    };
                    let part = json!({"id": id, "sessionID": self.session_id, "messageID": message_id, "type": "step-finish", "reason": finish, "cost": 0, "tokens": tokens});
                    self.emit("step_finish", at, json!({"part": part}));
                }
            }
            Event::StepFailed { error, .. } => {
                if !self.user_interrupt.load(Ordering::Relaxed) {
                    self.error(error);
                }
            }
            Event::Failed { error, .. } => {
                if !self.emitted_error {
                    self.error(error);
                }
                return true;
            }
            Event::Interrupted { .. } | Event::Succeeded { .. } => return true,
            Event::RetryScheduled { .. }
            | Event::PermissionAsked { .. }
            | Event::PermissionReplied { .. }
            | Event::Credits(_)
            | Event::TitleChanged { .. } => {}
        }
        false
    }
}
