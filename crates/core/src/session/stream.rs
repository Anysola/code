//! Assembles one streamed Anthropic response into assistant parts and reports what happened.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::message::{Cache, Part, PartTime, Tokens, ToolState};
use crate::gateway::error::{ApiError, SessionError};
use crate::gateway::sse::SseEvent;
use crate::time::now_ms;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    TextStarted {
        ordinal: usize,
    },
    /// Text streamed into the block `ordinal`.
    TextDelta {
        ordinal: usize,
        text: String,
    },
    TextEnded {
        ordinal: usize,
        text: String,
    },
    ReasoningStarted {
        ordinal: usize,
    },
    ReasoningDelta {
        ordinal: usize,
        text: String,
    },
    ReasoningEnded {
        ordinal: usize,
        text: String,
        native: Option<Value>,
    },
    ToolStarted {
        call_id: String,
        name: String,
    },
    /// A complete `tool_use` block: execution can start.
    ToolReady {
        call_id: String,
        name: String,
        input: Value,
        raw: String,
        part: usize,
    },
}

enum Block {
    Text {
        part: usize,
        ordinal: usize,
    },
    Reasoning {
        part: usize,
        ordinal: usize,
        signature: String,
    },
    Redacted,
    Tool {
        part: usize,
        raw: String,
    },
}

/// Server tool result blocks, which arrive whole and settle the provider-executed call they name.
const HOSTED_RESULTS: [&str; 3] = [
    "web_search_tool_result",
    "code_execution_tool_result",
    "web_fetch_tool_result",
];

const INPUT_EXCERPT_LIMIT: usize = 2048;

/// Why a client call's arguments never became a call.
enum InputFailure {
    Malformed,
    Incomplete,
}

#[derive(Default)]
pub struct Assembly {
    pub parts: Vec<Part>,
    blocks: HashMap<u64, Block>,
    texts: usize,
    reasonings: usize,
    pub tokens: Tokens,
    pub stop_reason: Option<String>,
    pub finished: bool,
    /// Client calls whose complete arguments were not valid JSON, with the raw arguments.
    malformed: Vec<(usize, String)>,
}

impl Assembly {
    /// True once any content arrived; a failure after that cannot simply be retried.
    pub fn has_output(&self) -> bool {
        !self.parts.is_empty()
    }

    pub fn on_event(&mut self, event: &SseEvent) -> Result<Vec<Action>, ApiError> {
        if event.event.as_deref() == Some("error") {
            return Err(ApiError::stream(&event.data));
        }
        let Ok(data) = serde_json::from_str::<Value>(&event.data) else {
            return Ok(Vec::new());
        };
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or_default();
        Ok(match kind {
            "message_start" => {
                self.usage(data.get("message").and_then(|m| m.get("usage")));
                Vec::new()
            }
            "content_block_start" => {
                self.start(index, data.get("content_block").unwrap_or(&Value::Null))
            }
            "content_block_delta" => self.delta(index, data.get("delta").unwrap_or(&Value::Null)),
            "content_block_stop" => self.stop(index),
            "message_delta" => {
                if let Some(reason) = data
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop_reason = Some(reason.to_string());
                }
                self.usage(data.get("usage"));
                Vec::new()
            }
            "message_stop" => {
                self.finished = true;
                Vec::new()
            }
            "error" => return Err(ApiError::stream(&event.data)),
            _ => Vec::new(),
        })
    }

    fn usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage else { return };
        let field = |key: &str| usage.get(key).and_then(Value::as_u64);
        if let Some(input) = field("input_tokens") {
            self.tokens.input = input;
        }
        if let Some(output) = field("output_tokens") {
            self.tokens.output = output;
        }
        let read = field("cache_read_input_tokens");
        let write = field("cache_creation_input_tokens");
        if read.is_some() || write.is_some() {
            self.tokens.cache = Cache {
                read: read.unwrap_or(self.tokens.cache.read),
                write: write.unwrap_or(self.tokens.cache.write),
            };
        }
    }

    fn start(&mut self, index: u64, block: &Value) -> Vec<Action> {
        let part = self.parts.len();
        let created = Some(PartTime {
            created: now_ms(),
            ..PartTime::default()
        });
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let ordinal = self.texts;
                self.texts += 1;
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.parts.push(Part::Text { text, native: None });
                self.blocks.insert(index, Block::Text { part, ordinal });
                vec![Action::TextStarted { ordinal }]
            }
            Some("thinking") => {
                let ordinal = self.reasonings;
                self.reasonings += 1;
                let text = block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let signature = block
                    .get("signature")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.parts.push(Part::Reasoning {
                    text,
                    native: None,
                    time: created,
                });
                self.blocks.insert(
                    index,
                    Block::Reasoning {
                        part,
                        ordinal,
                        signature,
                    },
                );
                vec![Action::ReasoningStarted { ordinal }]
            }
            Some("redacted_thinking") => {
                let data = block
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut time = PartTime {
                    created: now_ms(),
                    ..PartTime::default()
                };
                time.completed = Some(now_ms());
                let native =
                    Map::from_iter([("redactedData".to_string(), Value::String(data.into()))]);
                self.parts.push(Part::Reasoning {
                    text: String::new(),
                    native: Some(Value::Object(native)),
                    time: Some(time),
                });
                self.blocks.insert(index, Block::Redacted);
                Vec::new()
            }
            Some(kind @ ("tool_use" | "server_tool_use")) => {
                let hosted = kind == "server_tool_use";
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                // Server tools may carry their whole input on the start block.
                let raw = block
                    .get("input")
                    .filter(|i| i.as_object().is_none_or(|o| !o.is_empty()))
                    .map(crate::json::compact)
                    .unwrap_or_default();
                self.parts.push(Part::Tool {
                    id: call_id.clone(),
                    name: name.clone(),
                    executed: Some(hosted),
                    native: None,
                    state: ToolState::Streaming { input: raw.clone() },
                    time: PartTime {
                        created: now_ms(),
                        ..PartTime::default()
                    },
                });
                self.blocks.insert(index, Block::Tool { part, raw });
                vec![Action::ToolStarted { call_id, name }]
            }
            Some(kind) if HOSTED_RESULTS.contains(&kind) => {
                self.hosted_result(kind, block);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn delta(&mut self, index: u64, delta: &Value) -> Vec<Action> {
        let text = |key: &str| delta.get(key).and_then(Value::as_str).unwrap_or_default();
        match (
            self.blocks.get_mut(&index),
            delta.get("type").and_then(Value::as_str),
        ) {
            (Some(Block::Text { part, ordinal }), Some("text_delta")) => {
                if let Some(Part::Text { text: current, .. }) = self.parts.get_mut(*part) {
                    current.push_str(text("text"));
                }
                return vec![Action::TextDelta {
                    ordinal: *ordinal,
                    text: text("text").to_string(),
                }];
            }
            (Some(Block::Reasoning { part, ordinal, .. }), Some("thinking_delta")) => {
                if let Some(Part::Reasoning { text: current, .. }) = self.parts.get_mut(*part) {
                    current.push_str(text("thinking"));
                }
                return vec![Action::ReasoningDelta {
                    ordinal: *ordinal,
                    text: text("thinking").to_string(),
                }];
            }
            (Some(Block::Reasoning { signature, .. }), Some("signature_delta")) => {
                signature.push_str(text("signature"))
            }
            (Some(Block::Tool { part, raw }), Some("input_json_delta")) => {
                raw.push_str(text("partial_json"));
                if let Some(Part::Tool { state, .. }) = self.parts.get_mut(*part) {
                    *state = ToolState::Streaming { input: raw.clone() };
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn stop(&mut self, index: u64) -> Vec<Action> {
        match self.blocks.remove(&index) {
            Some(Block::Text { part, ordinal }) => match self.parts.get(part) {
                Some(Part::Text { text, .. }) => vec![Action::TextEnded {
                    ordinal,
                    text: text.clone(),
                }],
                _ => Vec::new(),
            },
            Some(Block::Reasoning {
                part,
                ordinal,
                signature,
            }) => {
                let native = (!signature.is_empty()).then(|| {
                    Value::Object(Map::from_iter([(
                        "signature".to_string(),
                        Value::String(signature),
                    )]))
                });
                match self.parts.get_mut(part) {
                    Some(Part::Reasoning {
                        text,
                        native: slot,
                        time,
                    }) => {
                        *slot = native.clone();
                        if let Some(time) = time {
                            time.completed = Some(now_ms());
                        }
                        vec![Action::ReasoningEnded {
                            ordinal,
                            text: text.clone(),
                            native,
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            Some(Block::Tool { part, raw }) => {
                let input = if raw.trim().is_empty() {
                    Ok(Value::Object(Map::new()))
                } else {
                    serde_json::from_str(&raw)
                };
                match self.parts.get_mut(part) {
                    Some(Part::Tool {
                        id,
                        name,
                        state,
                        time,
                        executed,
                        ..
                    }) => {
                        let Ok(input) = input else {
                            if *executed != Some(true) {
                                self.malformed.push((part, raw));
                            }
                            return Vec::new();
                        };
                        *state = ToolState::Running {
                            input: input.clone(),
                            metadata: Map::new(),
                        };
                        time.ran = Some(now_ms());
                        if *executed == Some(true) {
                            return Vec::new();
                        }
                        vec![Action::ToolReady {
                            call_id: id.clone(),
                            name: name.clone(),
                            input,
                            raw,
                            part,
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            Some(Block::Redacted) | None => Vec::new(),
        }
    }

    /// Settles the provider-executed call a server tool result block answers.
    fn hosted_result(&mut self, kind: &str, block: &Value) {
        let id = block.get("tool_use_id").and_then(Value::as_str);
        let content = block.get("content").cloned().unwrap_or(Value::Null);
        let Some(Part::Tool {
            state,
            native,
            time,
            ..
        }) = self.parts.iter_mut().find(|p| {
            matches!(p, Part::Tool { id: call, executed: Some(true), .. } if Some(call.as_str()) == id)
        })
        else {
            return;
        };
        let input = match state {
            ToolState::Running { input, .. } => input.clone(),
            _ => return,
        };
        let text = match &content {
            Value::String(text) => text.clone(),
            other => crate::json::compact(other),
        };
        let failed = content
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t.ends_with("_tool_result_error"));
        *state = if failed {
            ToolState::Error {
                input,
                error: SessionError::new("tool.execution", text),
                content: Vec::new(),
                metadata: Map::new(),
            }
        } else {
            ToolState::Completed {
                input,
                content: vec![crate::tool::ToolContent::text(text)],
                metadata: Map::new(),
            }
        };
        *native = Some(serde_json::json!({"blockType": kind, "result": content}));
        time.completed = Some(now_ms());
    }

    /// Calls the response left without an outcome, with the error each one gets: client calls
    /// whose arguments were invalid or never completed, and provider-executed calls with no
    /// result block.
    pub fn unfinished(&self) -> Vec<(usize, SessionError)> {
        let length = self.stop_reason.as_deref() == Some("max_tokens");
        let mut out: Vec<(usize, SessionError)> = self
            .parts
            .iter()
            .enumerate()
            .filter_map(|(index, part)| match part {
                Part::Tool {
                    executed: Some(true),
                    state: ToolState::Streaming { .. } | ToolState::Running { .. },
                    ..
                } => Some((
                    index,
                    SessionError::new(
                        "tool.result-missing",
                        "Provider did not return a tool result",
                    ),
                )),
                _ => None,
            })
            .collect();
        for (index, part) in self.parts.iter().enumerate() {
            let Part::Tool {
                executed: Some(false) | None,
                state: ToolState::Streaming { input },
                ..
            } = part
            else {
                continue;
            };
            let malformed = self.malformed.iter().find(|(p, _)| *p == index);
            let (reason, raw) = match malformed {
                Some((_, raw)) => (InputFailure::Malformed, raw.as_str()),
                None => (InputFailure::Incomplete, input.as_str()),
            };
            out.push((index, input_failure(reason, length, raw)));
        }
        out
    }

    /// The step's finish reason as stored and reported.
    pub fn finish(&self) -> &'static str {
        match self.stop_reason.as_deref() {
            Some("end_turn" | "stop_sequence") => "stop",
            Some("tool_use") => "tool-calls",
            Some("max_tokens") => "length",
            Some("refusal") => "content-filter",
            _ => "unknown",
        }
    }
}

/// The error of a client call that was not executed, with an excerpt of its arguments.
fn input_failure(reason: InputFailure, length: bool, raw: &str) -> SessionError {
    let (kind, message) = match (reason, length) {
        (_, true) => (
            "tool.input-incomplete",
            "The tool call was not executed because the output token limit (stop reason: length) was reached before you could complete the arguments. Break large payloads into smaller tool calls and reissue the call with complete arguments.",
        ),
        (InputFailure::Malformed, false) => (
            "tool.input-json",
            "The tool input could not be parsed as JSON, so the call was not executed. Reissue the call with valid JSON arguments.",
        ),
        (InputFailure::Incomplete, false) => (
            "tool.input-incomplete",
            "Tool call arguments were not completed and were not executed. Reissue the call with complete arguments.",
        ),
    };
    let total = crate::text::utf16_len(raw);
    if total == 0 {
        return SessionError::new(kind, message);
    }
    let excerpt = crate::text::utf16_prefix(raw, INPUT_EXCERPT_LIMIT);
    let shown = crate::text::utf16_len(excerpt);
    let truncated = if total > shown { "\n[truncated]" } else { "" };
    SessionError::new(
        kind,
        format!(
            "{message}\n\nInput excerpt (first {shown} of {total} characters):\n{excerpt}{truncated}"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(assembly: &mut Assembly, kind: &str, data: Value) {
        let event = SseEvent {
            event: Some(kind.into()),
            data: data.to_string(),
        };
        assembly.on_event(&event).unwrap();
    }

    fn cut_call(raw: &str) -> Assembly {
        let mut assembly = Assembly::default();
        feed(
            &mut assembly,
            "content_block_start",
            serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "t", "name": "write", "input": {}}}),
        );
        feed(
            &mut assembly,
            "content_block_delta",
            serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": raw}}),
        );
        assembly
    }

    #[test]
    fn cut_off_arguments() {
        let assembly = cut_call("");
        assert_eq!(
            assembly.unfinished(),
            vec![(
                0,
                SessionError::new(
                    "tool.input-incomplete",
                    "Tool call arguments were not completed and were not executed. Reissue the call with complete arguments."
                )
            )]
        );
        let mut assembly = cut_call(&format!("{{\"content\": \"{}", "é".repeat(3000)));
        assembly.stop_reason = Some("max_tokens".into());
        let (_, error) = &assembly.unfinished()[0];
        assert_eq!(error.kind, "tool.input-incomplete");
        assert!(error.message.starts_with("The tool call was not executed because the output token limit (stop reason: length) was reached"));
        assert!(
            error
                .message
                .contains("\n\nInput excerpt (first 2048 of 3013 characters):\n{\"content\": \"é")
        );
        assert!(error.message.ends_with("é\n[truncated]"));
    }
}
