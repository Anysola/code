//! A `session_message` row read the way 0.5.1 reads it before decoding: `session_part` rows replace
//! the inline content, `media:` refs are restored from `blob`, then pre-v2.1 names are upgraded
//! (`SessionLegacy.message`). Everything else in `data`, unknown fields included, is kept.

use base64::Engine;
use serde_json::{Map, Value};

/// Assistant content from its `session_part` rows, in ordinal order, when it has any.
pub fn merge_parts(data: &mut Value, parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    if let Some(content) = data.as_object_mut().and_then(|d| d.get_mut("content")) {
        *content = Value::Array(parts);
    }
}

fn media_hash(value: &Value) -> Option<&str> {
    value.get("uri")?.as_str()?.strip_prefix("media:")
}

fn unavailable(item: &Map<String, Value>) -> String {
    let mime = item.get("mime").and_then(Value::as_str).unwrap_or_default();
    match item.get("name").and_then(Value::as_str) {
        Some(name) => format!("[media unavailable: {name} ({mime})]"),
        None => format!("[media unavailable: {mime}]"),
    }
}

/// Restores inline bytes for `media:<sha256>` refs in user files and tool result files. A ref
/// whose blob is gone reads as 0.5.1's placeholder text.
pub fn hydrate(data: &mut Value, blob: &mut impl FnMut(&str) -> Option<Vec<u8>>) {
    let base64 = |bytes: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(bytes);
    if let Some(files) = data.get_mut("files").and_then(Value::as_array_mut) {
        for file in files {
            let Some(hash) = media_hash(file).map(str::to_string) else {
                continue;
            };
            let Some(map) = file.as_object_mut() else {
                continue;
            };
            map.shift_remove("uri");
            match blob(&hash) {
                Some(bytes) => {
                    map.insert("data".into(), Value::String(base64(bytes)));
                }
                None => {
                    let text = unavailable(map);
                    map.insert("data".into(), Value::String(base64(text.into_bytes())));
                    map.insert("mime".into(), Value::String("text/plain".into()));
                }
            }
        }
    }
    let Some(parts) = data.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    for part in parts {
        if part["type"] != "tool"
            || !matches!(
                part["state"]["status"].as_str(),
                Some("completed" | "error")
            )
        {
            continue;
        }
        let Some(items) = part["state"]
            .get_mut("content")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for item in items {
            if item["type"] != "file" {
                continue;
            }
            let Some(hash) = media_hash(item).map(str::to_string) else {
                continue;
            };
            let Some(map) = item.as_object_mut() else {
                continue;
            };
            match blob(&hash) {
                Some(bytes) => {
                    let mime = map.get("mime").and_then(Value::as_str).unwrap_or_default();
                    let uri = format!("data:{mime};base64,{}", base64(bytes));
                    map.insert("uri".into(), Value::String(uri));
                }
                None => *item = serde_json::json!({"type": "text", "text": unavailable(map)}),
            }
        }
    }
}

/// `SessionLegacy.message`: pre-v2.1 names upgraded by message type.
pub fn upgrade(kind: &str, data: &mut Value) {
    let Some(map) = data.as_object_mut() else {
        return;
    };
    match kind {
        "assistant" => {
            rename(map, "providerState", "native");
            if let Some(parts) = map.get_mut("content").and_then(Value::as_array_mut) {
                parts.iter_mut().for_each(upgrade_part);
            }
        }
        "compaction" => {
            rename(map, "providerState", "native");
            recent(map);
            provider_context(map);
        }
        _ => {}
    }
}

fn upgrade_part(part: &mut Value) {
    let Some(map) = part.as_object_mut() else {
        return;
    };
    match map.get("type").and_then(Value::as_str) {
        Some("text" | "reasoning") => rename(map, "state", "native"),
        // A tool part's own `state` is its lifecycle status, not native data.
        Some("tool") => {
            rename(map, "providerState", "native");
            if let Some(result) = map.shift_remove("providerResultState")
                && let Value::Object(result) = result
            {
                let mut native = match map.get("native") {
                    Some(Value::Object(native)) => native.clone(),
                    _ => Map::new(),
                };
                native.extend(result);
                map.insert("native".into(), Value::Object(native));
            }
        }
        _ => {}
    }
}

/// The current name wins when both are present; the moved value goes last, as a spread put it.
fn rename(map: &mut Map<String, Value>, from: &str, to: &str) {
    if let Some(moved) = map.shift_remove(from)
        && !map.contains_key(to)
    {
        map.insert(to.into(), moved);
    }
}

fn recent(map: &mut Map<String, Value>) {
    let Some(text) = map.shift_remove("recent") else {
        return;
    };
    let Value::String(text) = text else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let mut metadata = Map::new();
    metadata.insert("recent".into(), Value::String(text));
    if let Some(Value::Object(existing)) = map.get("metadata") {
        metadata.extend(existing.clone());
    }
    map.insert("metadata".into(), Value::Object(metadata));
}

/// A provider window moves to `native.messages` (replacing `native`); media parts stored before
/// 2.0.15 as `{mediaType, data}` become `{media: {source}}`.
fn provider_context(map: &mut Map<String, Value>) {
    let Some(context) = map.shift_remove("providerContext") else {
        return;
    };
    let Value::Object(context) = context else {
        return;
    };
    let mut native = Map::new();
    match context.get("messages") {
        Some(Value::Array(messages)) => {
            let messages = messages.iter().cloned().map(media).collect();
            native.insert("messages".into(), Value::Array(messages));
        }
        Some(other) => {
            native.insert("messages".into(), other.clone());
        }
        None => {}
    }
    map.insert("native".into(), Value::Object(native));
}

fn media(mut message: Value) -> Value {
    let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) else {
        return message;
    };
    for part in parts {
        let Some(map) = part.as_object_mut() else {
            continue;
        };
        if map.get("type").and_then(Value::as_str) != Some("media")
            || map.contains_key("media")
            || !map.get("data").is_some_and(Value::is_string)
        {
            continue;
        }
        let data = map.shift_remove("data").unwrap_or(Value::Null);
        let media_type = map.shift_remove("mediaType");
        let mut source = Map::new();
        source.insert("type".into(), Value::String("base64".into()));
        source.insert("data".into(), data);
        if let Some(media_type) = media_type {
            source.insert("mediaType".into(), media_type);
        }
        map.insert("media".into(), serde_json::json!({ "source": source }));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parts_replace_inline_content_only_when_present() {
        let mut data = json!({"content": [], "x": 1});
        merge_parts(&mut data, vec![]);
        assert_eq!(data, json!({"content": [], "x": 1}));
        merge_parts(&mut data, vec![json!({"type": "text", "text": "a"})]);
        assert_eq!(
            data,
            json!({"content": [{"type": "text", "text": "a"}], "x": 1})
        );
    }

    #[test]
    fn legacy_assistant_names() {
        let mut data = json!({
            "providerState": {"a": 1},
            "content": [
                {"type": "reasoning", "text": "t", "state": {"signature": "S"}},
                {"type": "text", "text": "x", "state": {"k": 1}, "native": {"k": 2}},
                {"type": "tool", "id": "c", "state": {"status": "completed"}, "providerState": {"p": 1},
                 "providerResultState": {"r": 2}}
            ]
        });
        upgrade("assistant", &mut data);
        assert_eq!(
            data,
            json!({
                "content": [
                    {"type": "reasoning", "text": "t", "native": {"signature": "S"}},
                    {"type": "text", "text": "x", "native": {"k": 2}},
                    {"type": "tool", "id": "c", "state": {"status": "completed"}, "native": {"p": 1, "r": 2}}
                ],
                "native": {"a": 1}
            })
        );
    }

    #[test]
    fn legacy_compaction_window_and_recent() {
        let mut data = json!({
            "status": "completed",
            "recent": "[User]: hi",
            "metadata": {"keepID": "msg_1"},
            "native": {"dropped": true},
            "providerContext": {"messages": [{"role": "user", "content": [
                {"type": "media", "mediaType": "image/png", "data": "AA=="},
                {"type": "text", "text": "x"}
            ]}]}
        });
        upgrade("compaction", &mut data);
        assert_eq!(
            data,
            json!({
                "status": "completed",
                "metadata": {"recent": "[User]: hi", "keepID": "msg_1"},
                "native": {"messages": [{"role": "user", "content": [
                    {"type": "media", "media": {"source": {"type": "base64", "data": "AA==", "mediaType": "image/png"}}},
                    {"type": "text", "text": "x"}
                ]}]}
            })
        );
    }

    #[test]
    fn media_refs_restored_or_marked_unavailable() {
        let mut data = json!({
            "files": [
                {"mime": "image/png", "source": {"type": "inline"}, "name": "a.png", "uri": "media:aa"},
                {"mime": "image/png", "source": {"type": "inline"}, "uri": "media:gone"}
            ],
            "content": [{"type": "tool", "state": {"status": "completed", "content": [
                {"type": "file", "uri": "media:aa", "mime": "image/png"},
                {"type": "file", "uri": "media:gone", "mime": "image/png", "name": "b.png"}
            ]}}]
        });
        hydrate(&mut data, &mut |hash| (hash == "aa").then(|| vec![1, 2]));
        assert_eq!(
            data,
            json!({
                "files": [
                    {"mime": "image/png", "source": {"type": "inline"}, "name": "a.png", "data": "AQI="},
                    {"mime": "text/plain", "source": {"type": "inline"}, "data": "W21lZGlhIHVuYXZhaWxhYmxlOiBpbWFnZS9wbmdd"}
                ],
                "content": [{"type": "tool", "state": {"status": "completed", "content": [
                    {"type": "file", "uri": "data:image/png;base64,AQI=", "mime": "image/png"},
                    {"type": "text", "text": "[media unavailable: b.png (image/png)]"}
                ]}}]
            })
        );
    }
}
