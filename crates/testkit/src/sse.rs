//! Builders for Anthropic SSE streams as the gateway relays them.

use serde_json::{Value, json};

#[derive(Debug, Clone, Default)]
pub struct Stream {
    events: Vec<String>,
    index: u64,
    stop_reason: Option<&'static str>,
    output_tokens: u64,
    usage: Option<Value>,
}

fn event(kind: &str, data: Value) -> String {
    let mut data = data;
    let mut object = serde_json::Map::new();
    object.insert("type".into(), Value::String(kind.into()));
    if let Value::Object(rest) = data.take() {
        object.extend(rest);
    }
    format!("event: {kind}\ndata: {}\n\n", Value::Object(object))
}

impl Stream {
    pub fn new() -> Self {
        Stream {
            output_tokens: 3,
            ..Stream::default()
        }
    }

    /// Usage reported in `message_start` (defaults to 10 input tokens).
    pub fn usage(mut self, usage: Value) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn text(mut self, text: &str) -> Self {
        let i = self.index;
        self.events.push(event(
            "content_block_start",
            json!({"index": i, "content_block": {"type": "text", "text": ""}}),
        ));
        self.events.push(event(
            "content_block_delta",
            json!({"index": i, "delta": {"type": "text_delta", "text": text}}),
        ));
        self.events
            .push(event("content_block_stop", json!({"index": i})));
        self.index += 1;
        self.stop_reason.get_or_insert("end_turn");
        self
    }

    pub fn thinking(mut self, text: &str, signature: &str) -> Self {
        let i = self.index;
        self.events.push(event(
            "content_block_start",
            json!({"index": i, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
        ));
        self.events.push(event(
            "content_block_delta",
            json!({"index": i, "delta": {"type": "thinking_delta", "thinking": text}}),
        ));
        self.events.push(event(
            "content_block_delta",
            json!({"index": i, "delta": {"type": "signature_delta", "signature": signature}}),
        ));
        self.events
            .push(event("content_block_stop", json!({"index": i})));
        self.index += 1;
        self
    }

    pub fn redacted(mut self, data: &str) -> Self {
        let i = self.index;
        self.events.push(event(
            "content_block_start",
            json!({"index": i, "content_block": {"type": "redacted_thinking", "data": data}}),
        ));
        self.events
            .push(event("content_block_stop", json!({"index": i})));
        self.index += 1;
        self
    }

    pub fn tool(mut self, id: &str, name: &str, input: Value) -> Self {
        let i = self.index;
        self.events.push(event(
            "content_block_start",
            json!({"index": i, "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
        ));
        self.events.push(event(
            "content_block_delta",
            json!({"index": i, "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
        ));
        self.events
            .push(event("content_block_stop", json!({"index": i})));
        self.index += 1;
        self.stop_reason = Some("tool_use");
        self.output_tokens = 5;
        self
    }

    pub fn stop(mut self, reason: &'static str) -> Self {
        self.stop_reason = Some(reason);
        self
    }

    /// The complete stream; `credits` appends the gateway's closing credits event.
    pub fn finish(self, model: &str, credits: Option<u64>) -> String {
        let usage = self
            .usage
            .unwrap_or_else(|| json!({"input_tokens": 10, "output_tokens": 1}));
        let mut out = event(
            "message_start",
            json!({"message": {"id": "msg_fake", "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": null, "usage": usage}}),
        );
        out.extend(self.events);
        out.push_str(&event(
            "message_delta",
            json!({"delta": {"stop_reason": self.stop_reason.unwrap_or("end_turn")}, "usage": {"output_tokens": self.output_tokens}}),
        ));
        out.push_str(&event("message_stop", json!({})));
        if let Some(charged) = credits {
            out.push_str(&event("anysola_credits", json!({"charged": charged})));
        }
        out
    }

    /// A stream cut before `message_stop` (no credits event), ending with a gateway error event.
    pub fn interrupted(self, model: &str) -> String {
        let mut out = event(
            "message_start",
            json!({"message": {"id": "msg_fake", "type": "message", "role": "assistant", "model": model, "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        );
        out.extend(self.events);
        out.push_str(&event("error", json!({"error": {"type": "api_error", "message": "The model's stream was interrupted."}})));
        out
    }
}
