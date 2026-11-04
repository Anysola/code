//! The `/v1/code/messages` request body and its request-specific headers, byte for byte as 0.5.1
//! built them: keys in the order `model, system, messages, tools, [tool_choice], stream,
//! max_tokens, thinking, [output_config]`, four ephemeral cache breakpoints (last tool, first and
//! last system block, tail message).

use serde_json::{Map, Value, json};

use crate::model::{Model, Variant};
use crate::tool::ToolDefinition;

const OUTPUT_CAP: u64 = 256_000;
const DEFAULT_OUTPUT: u64 = 32_000;
const BETA_INTERLEAVED: &str = "interleaved-thinking-2025-05-14";
const BETA_BINDING: &str = "thinking-binding-controls-2026-08-01";

pub struct Request<'a> {
    pub model: &'a Model,
    pub variant: Option<&'a Variant>,
    pub system: Vec<String>,
    pub tools: Vec<ToolDefinition>,
    /// Lowered messages; the tail breakpoint is added here.
    pub messages: Vec<Value>,
    pub max_tokens: u64,
    /// The step-capped final step keeps the tools but forbids calling them.
    pub tools_disabled: bool,
}

/// `min(limit.output ?? 32000, 256000)`, reduced when the context window is nearly full.
pub fn output_limit(model: &Model, input_tokens: u64) -> u64 {
    let limit = model.output.unwrap_or(DEFAULT_OUTPUT).min(OUTPUT_CAP);
    match model.context {
        Some(context) => {
            let room = ((context as f64 * 0.99).floor() as u64).saturating_sub(input_tokens);
            limit.min(room.max(1024))
        }
        None => limit,
    }
}

fn ephemeral() -> Value {
    json!({"type": "ephemeral"})
}

fn thinking(model: &Model, variant: Option<&Variant>) -> Option<Value> {
    let settings = variant.and_then(|v| v.thinking.clone());
    if !model.thinking_binding() {
        return settings;
    }
    let mut thinking = match settings {
        Some(Value::Object(map)) => map,
        _ => Map::from_iter([("type".to_string(), Value::String("adaptive".into()))]),
    };
    let mut binding = Map::from_iter([(
        "prefix_mismatch_behavior".to_string(),
        Value::String("drop_block".into()),
    )]);
    if let Some(Value::Object(extra)) = thinking.get("block_binding") {
        binding.extend(extra.clone());
    }
    thinking.insert("block_binding".into(), Value::Object(binding));
    Some(Value::Object(thinking))
}

/// Serialized body plus the `anthropic-beta` and variant headers.
pub fn build(request: Request) -> (String, Vec<(String, String)>) {
    let mut body = Map::new();
    body.insert("model".into(), Value::String(request.model.id.clone()));
    let system: Vec<&String> = request.system.iter().filter(|s| !s.is_empty()).collect();
    if !system.is_empty() {
        let last = system.len() - 1;
        let blocks = system
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let mut block = json!({"type": "text", "text": text});
                if index == 0 || index == last {
                    block["cache_control"] = ephemeral();
                }
                block
            })
            .collect();
        body.insert("system".into(), Value::Array(blocks));
    }
    let mut messages = request.messages;
    super::lower::mark_tail(&mut messages);
    body.insert("messages".into(), Value::Array(messages));
    if !request.tools.is_empty() {
        let last = request.tools.len() - 1;
        let tools = request
            .tools
            .into_iter()
            .enumerate()
            .map(|(index, tool)| {
                let mut entry = json!({"name": tool.name, "description": tool.description, "input_schema": tool.input_schema});
                if index == last {
                    entry["cache_control"] = ephemeral();
                }
                entry
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        if request.tools_disabled {
            body.insert("tool_choice".into(), json!({"type": "none"}));
        }
    }
    body.insert("stream".into(), Value::Bool(true));
    body.insert("max_tokens".into(), Value::from(request.max_tokens));
    let thinking = thinking(request.model, request.variant);
    let bound = thinking
        .as_ref()
        .is_some_and(|t| t.get("block_binding").is_some());
    if let Some(thinking) = thinking {
        body.insert("thinking".into(), thinking);
    }
    if let Some(effort) = request.variant.and_then(|v| v.effort.as_deref()) {
        body.insert("output_config".into(), json!({"effort": effort}));
    }
    let mut betas = vec![BETA_INTERLEAVED];
    if bound {
        betas.push(BETA_BINDING);
    }
    let mut headers = vec![("anthropic-beta".to_string(), betas.join(","))];
    if let Some(variant) = request.variant {
        headers.extend(variant.headers.iter().cloned());
    }
    (crate::json::compact(&Value::Object(body)), headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            context: Some(1_000_000),
            output: Some(32_000),
            supports_binding: None,
            supports_effort_updates: None,
            variants: vec![],
        }
    }

    #[test]
    fn body_order_and_marks() {
        let m = model("claude-sonnet-5-5");
        let (body, headers) = build(Request {
            model: &m,
            variant: None,
            system: vec!["a".into(), "b".into(), "".into(), "d".into()],
            tools: vec![ToolDefinition {
                name: "t".into(),
                description: "d".into(),
                input_schema: json!({}),
            }],
            messages: vec![json!({"role": "user", "content": [{"type": "text", "text": "hi"}]})],
            max_tokens: 32000,
            tools_disabled: false,
        });
        assert_eq!(
            body,
            r#"{"model":"claude-sonnet-5-5","system":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}},{"type":"text","text":"b"},{"type":"text","text":"d","cache_control":{"type":"ephemeral"}}],"messages":[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}],"tools":[{"name":"t","description":"d","input_schema":{},"cache_control":{"type":"ephemeral"}}],"stream":true,"max_tokens":32000,"thinking":{"type":"adaptive","block_binding":{"prefix_mismatch_behavior":"drop_block"}}}"#
        );
        assert_eq!(
            headers[0].1,
            "interleaved-thinking-2025-05-14,thinking-binding-controls-2026-08-01"
        );
    }

    #[test]
    fn limits() {
        let m = model("x");
        assert_eq!(output_limit(&m, 0), 32_000);
        assert_eq!(output_limit(&m, 985_000), 5_000);
        assert_eq!(output_limit(&m, 999_999), 1024);
    }
}
