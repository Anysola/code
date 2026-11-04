//! The `/v1/code/messages` request body and its request-specific headers, byte for byte as 0.5.1
//! built them: keys in the order `model, system, messages, tools, [tool_choice], stream,
//! max_tokens, thinking, [output_config]`, four ephemeral cache breakpoints (last tool, first and
//! last system block, tail message).

use serde_json::{Map, Value, json};

use super::lower::{Entry, mark_tail};
use crate::model::{Model, Variant};
use crate::tool::ToolDefinition;

const OUTPUT_CAP: u64 = 256_000;
const DEFAULT_OUTPUT: u64 = 32_000;
const BETA_INTERLEAVED: &str = "interleaved-thinking-2025-05-14";
const BETA_BINDING: &str = "thinking-binding-controls-2026-08-01";
const BETA_EFFORT_UPDATES: &str = "mid-conversation-output-config-2026-07-01";
const DEFAULT_EFFORT: &str = "high";

pub struct Request<'a> {
    pub model: &'a Model,
    pub variant: Option<&'a Variant>,
    pub system: Vec<String>,
    pub tools: Vec<ToolDefinition>,
    /// Lowered history; the tail breakpoint, effort changes and instruction updates are placed
    /// here.
    pub messages: Vec<Entry>,
    pub max_tokens: u64,
    /// The step-capped final step keeps the tools but forbids calling them.
    pub tools_disabled: bool,
}

/// `min(limit.output ?? 32000, 256000)`, reduced to the room the prompt leaves in the window
/// (`headroom` of it kept free, the estimated part counted 15% high).
pub fn output_limit(model: &Model, measured: u64, estimated: u64, headroom: f64) -> u64 {
    let limit = model.output.unwrap_or(DEFAULT_OUTPUT).min(OUTPUT_CAP);
    match model.context.filter(|c| *c > 0) {
        Some(context) => {
            let room = (context as f64 * (1.0 - headroom)).floor() as i64
                - measured as i64
                - (estimated as f64 * 1.15).ceil() as i64;
            limit.min(room.max(1024) as u64)
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

/// Keeps effort-change markers only when the model takes them and the last one still matches the
/// requested effort (a revert or fork can leave them disagreeing); then the top-level effort stays
/// the one the conversation started with, so the prompt cache survives the switch.
fn resolve_effort(
    entries: Vec<Entry>,
    requested: Option<&str>,
    supported: bool,
) -> (Vec<Entry>, Option<String>) {
    let strip = |entries: Vec<Entry>| {
        entries
            .into_iter()
            .filter(|e| !matches!(e, Entry::Effort { .. }))
            .collect()
    };
    let requested = requested.map(str::to_string);
    let updates: Vec<(&Option<String>, &Option<String>)> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::Effort { to, from } => Some((to, from)),
            _ => None,
        })
        .collect();
    let (Some(first), Some(last)) = (updates.first(), updates.last()) else {
        return (entries, requested);
    };
    if !supported || *last.0 != requested {
        return (strip(entries), requested);
    }
    let top = first.1.clone();
    (entries, top)
}

fn wrap_update(text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<system-update>\n{escaped}\n</system-update>")
}

fn text_with_cache(text: String, cache: bool) -> Value {
    let mut block = json!({"type": "text", "text": text});
    if cache {
        block["cache_control"] = ephemeral();
    }
    block
}

fn append_to_user_turn(messages: &mut Vec<Value>, block: Value) {
    match messages.last_mut() {
        Some(last) if last["role"] == "user" => last["content"]
            .as_array_mut()
            .expect("user turns have content")
            .push(block),
        _ => messages.push(json!({"role": "user", "content": [block]})),
    }
}

/// Places entries as Anthropic messages. Models with native system updates get them as `system`
/// messages after the latest user turn (the nearest spot Anthropic accepts one); others see them as
/// escaped `<system-update>` user text where they happened.
fn place(entries: Vec<Entry>, native: bool) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    let mut held: Vec<(String, bool)> = Vec::new();
    let release = |messages: &mut Vec<Value>, held: &mut Vec<(String, bool)>| {
        let after_user = messages
            .iter()
            .rev()
            .find(|m| m["role"] != "system")
            .is_some_and(|m| m["role"] == "user");
        for (text, cache) in held.drain(..) {
            if after_user {
                messages.push(json!({"role": "system", "content": [text_with_cache(text, cache)]}));
            } else {
                append_to_user_turn(messages, text_with_cache(wrap_update(&text), cache));
            }
        }
    };
    for entry in entries {
        match entry {
            Entry::Effort { to, .. } => messages.push(json!({
                "role": "system",
                "content": [],
                "output_config": {"effort": to.as_deref().unwrap_or(DEFAULT_EFFORT)},
            })),
            Entry::Update { text, cache } if native => held.push((text, cache)),
            Entry::Update { text, cache } => {
                append_to_user_turn(&mut messages, text_with_cache(wrap_update(&text), cache))
            }
            Entry::Turn(message) => {
                if message["role"] == "assistant" {
                    release(&mut messages, &mut held);
                }
                messages.push(message);
            }
        }
    }
    release(&mut messages, &mut held);
    messages
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
    let requested = request.variant.and_then(|v| v.effort.as_deref());
    let (mut entries, effort) =
        resolve_effort(request.messages, requested, request.model.effort_updates());
    let effort_markers = entries.iter().any(|e| matches!(e, Entry::Effort { .. }));
    mark_tail(&mut entries);
    let messages = place(entries, request.model.native_system_updates());
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
    if let Some(effort) = effort {
        body.insert("output_config".into(), json!({"effort": effort}));
    }
    let mut betas = vec![BETA_INTERLEAVED];
    if bound {
        betas.push(BETA_BINDING);
    }
    if effort_markers {
        betas.push(BETA_EFFORT_UPDATES);
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
            input: vec!["text".into(), "image".into()],
            note: None,
            credits_per_turn: None,
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
            messages: vec![Entry::Turn(
                json!({"role": "user", "content": [{"type": "text", "text": "hi"}]}),
            )],
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

    fn user(text: &str) -> Entry {
        Entry::Turn(json!({"role": "user", "content": [{"type": "text", "text": text}]}))
    }

    fn assistant(text: &str) -> Entry {
        Entry::Turn(json!({"role": "assistant", "content": [{"type": "text", "text": text}]}))
    }

    fn update(text: &str) -> Entry {
        Entry::Update {
            text: text.into(),
            cache: false,
        }
    }

    fn effort(to: Option<&str>, from: Option<&str>) -> Entry {
        Entry::Effort {
            to: to.map(Into::into),
            from: from.map(Into::into),
        }
    }

    fn variant(id: &str, effort: Option<&str>) -> Variant {
        Variant {
            id: id.into(),
            thinking: Some(json!({"type": "adaptive", "display": "summarized"})),
            effort: effort.map(Into::into),
            headers: vec![],
        }
    }

    fn messages_of(
        model: &Model,
        variant: Option<&Variant>,
        entries: Vec<Entry>,
    ) -> (Value, String) {
        let (body, headers) = build(Request {
            model,
            variant,
            system: vec![],
            tools: vec![],
            messages: entries,
            max_tokens: 1,
            tools_disabled: false,
        });
        let body: Value = serde_json::from_str(&body).unwrap();
        (body, headers[0].1.clone())
    }

    #[test]
    fn native_update_follows_the_latest_user_turn_and_leaves_it_the_tail() {
        let m = model("claude-sonnet-5-5");
        let (body, _) = messages_of(
            &m,
            None,
            vec![
                user("hello"),
                assistant("hi"),
                update("changed"),
                user("next"),
            ],
        );
        assert_eq!(
            body["messages"],
            json!([
                {"role": "user", "content": [{"type": "text", "text": "hello"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "hi"}]},
                {"role": "user", "content": [{"type": "text", "text": "next", "cache_control": {"type": "ephemeral"}}]},
                {"role": "system", "content": [{"type": "text", "text": "changed"}]},
            ])
        );
    }

    #[test]
    fn wrapped_update_stays_in_place_escaped() {
        let m = model("auto");
        let (body, _) = messages_of(
            &m,
            None,
            vec![
                user("first"),
                assistant("hi"),
                update("a <b> & c\n"),
                user("second"),
            ],
        );
        assert_eq!(
            body["messages"][2],
            json!({"role": "user", "content": [{"type": "text", "text": "<system-update>\na &lt;b&gt; &amp; c\n\n</system-update>"}]})
        );
        assert_eq!(
            body["messages"][3]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        let (body, _) = messages_of(&m, None, vec![user("first"), update("tail")]);
        assert_eq!(
            body["messages"],
            json!([{"role": "user", "content": [
                {"type": "text", "text": "first"},
                {"type": "text", "text": "<system-update>\ntail\n</system-update>", "cache_control": {"type": "ephemeral"}},
            ]}])
        );
    }

    #[test]
    fn effort_markers_keep_the_first_effort_on_top() {
        let m = model("claude-sonnet-5-5");
        let medium = variant("medium", Some("medium"));
        let (body, beta) = messages_of(
            &m,
            Some(&medium),
            vec![
                user("a"),
                assistant("b"),
                effort(Some("medium"), None),
                user("c"),
            ],
        );
        assert!(body.get("output_config").is_none());
        assert_eq!(
            body["messages"][2],
            json!({"role": "system", "content": [], "output_config": {"effort": "medium"}})
        );
        assert_eq!(
            body["messages"][3]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert!(beta.ends_with(",mid-conversation-output-config-2026-07-01"));

        let (body, _) = messages_of(
            &m,
            None,
            vec![
                user("a"),
                effort(Some("max"), Some("low")),
                effort(None, Some("max")),
                user("c"),
            ],
        );
        assert_eq!(body["output_config"], json!({"effort": "low"}));
        assert_eq!(
            body["messages"][2]["output_config"],
            json!({"effort": "high"})
        );
    }

    #[test]
    fn effort_markers_dropped_when_stale_or_unsupported() {
        let economy = variant("economy", Some("low"));
        let entries = vec![user("a"), effort(Some("high"), None), user("c")];
        let (body, beta) =
            messages_of(&model("claude-sonnet-5-5"), Some(&economy), entries.clone());
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["output_config"], json!({"effort": "low"}));
        assert!(!beta.contains("mid-conversation"));
        let high = variant("high", Some("high"));
        let (body, _) = messages_of(&model("claude-sonnet-4-5"), Some(&high), entries);
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["output_config"], json!({"effort": "high"}));
    }

    #[test]
    fn limits() {
        let m = model("x");
        assert_eq!(output_limit(&m, 0, 0, 0.01), 32_000);
        assert_eq!(output_limit(&m, 985_000, 0, 0.01), 5_000);
        assert_eq!(output_limit(&m, 900_000, 70_000, 0.01), 9_500);
        assert_eq!(output_limit(&m, 999_999, 0, 0.01), 1024);
    }
}
