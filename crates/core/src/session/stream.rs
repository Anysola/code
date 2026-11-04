//! Assembles one streamed Anthropic response into assistant parts and reports what happened.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::message::{Cache, Part, PartTime, Tokens, ToolState};
use crate::gateway::error::ApiError;
use crate::gateway::sse::SseEvent;
use crate::time::now_ms;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    TextStarted {
        ordinal: usize,
    },
    TextEnded {
        ordinal: usize,
        text: String,
    },
    ReasoningStarted {
        ordinal: usize,
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

#[derive(Default)]
pub struct Assembly {
    pub parts: Vec<Part>,
    blocks: HashMap<u64, Block>,
    texts: usize,
    reasonings: usize,
    pub tokens: Tokens,
    pub stop_reason: Option<String>,
    pub finished: bool,
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
            "content_block_delta" => {
                self.delta(index, data.get("delta").unwrap_or(&Value::Null));
                Vec::new()
            }
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
                let mut time = created.unwrap_or_default();
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
            Some("tool_use") => {
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
                self.parts.push(Part::Tool {
                    id: call_id.clone(),
                    name: name.clone(),
                    executed: Some(false),
                    native: None,
                    state: ToolState::Streaming {
                        input: String::new(),
                    },
                    time: PartTime {
                        created: now_ms(),
                        ..PartTime::default()
                    },
                });
                self.blocks.insert(
                    index,
                    Block::Tool {
                        part,
                        raw: String::new(),
                    },
                );
                vec![Action::ToolStarted { call_id, name }]
            }
            _ => Vec::new(),
        }
    }

    fn delta(&mut self, index: u64, delta: &Value) {
        let text = |key: &str| delta.get(key).and_then(Value::as_str).unwrap_or_default();
        match (
            self.blocks.get_mut(&index),
            delta.get("type").and_then(Value::as_str),
        ) {
            (Some(Block::Text { part, .. }), Some("text_delta")) => {
                if let Some(Part::Text { text: current, .. }) = self.parts.get_mut(*part) {
                    current.push_str(text("text"));
                }
            }
            (Some(Block::Reasoning { part, .. }), Some("thinking_delta")) => {
                if let Some(Part::Reasoning { text: current, .. }) = self.parts.get_mut(*part) {
                    current.push_str(text("thinking"));
                }
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
                    Value::Object(Map::new())
                } else {
                    serde_json::from_str(&raw).unwrap_or(Value::String(raw.clone()))
                };
                match self.parts.get_mut(part) {
                    Some(Part::Tool {
                        id,
                        name,
                        state,
                        time,
                        ..
                    }) => {
                        *state = ToolState::Running {
                            input: input.clone(),
                            metadata: Map::new(),
                        };
                        time.ran = Some(now_ms());
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
