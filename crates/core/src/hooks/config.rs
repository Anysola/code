//! A document's `hooks` value: Claude Code's `settings.json` shape, read leniently so a block
//! pasted from Claude Code loads (what 1.0.9 cannot run is skipped with one warning line).

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::Event;
use super::matcher::Matcher;

/// One command to run for one event.
#[derive(Debug, Clone)]
pub struct Handler {
    pub event: Event,
    /// As configured; `None` when absent.
    pub matcher: Option<String>,
    pub compiled: Matcher,
    pub command: String,
    /// Seconds.
    pub timeout: u64,
}

/// The handlers of a document's `hooks` value, in config order. Problems become warnings.
pub fn parse(value: &Value, source: &str, warnings: &mut Vec<String>) -> Vec<Handler> {
    let Some(events) = value.as_object() else {
        warnings.push(format!(
            "Ignoring \"hooks\" in {source}: it is not an object."
        ));
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, groups) in events {
        let Some(event) = Event::parse(name) else {
            warnings.push(format!(
                "Ignoring {name} hooks in {source}: this version runs PreToolUse, PostToolUse, UserPromptSubmit, Stop and SessionStart hooks."
            ));
            continue;
        };
        let Some(groups) = groups.as_array() else {
            warnings.push(format!(
                "Ignoring {name} hooks in {source}: expected a list."
            ));
            continue;
        };
        for group in groups {
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);
            let (compiled, warning) = if event.uses_matcher() {
                Matcher::parse(matcher.as_deref())
            } else {
                (Matcher::All, None)
            };
            if let Some(warning) = warning {
                warnings.push(format!("{warning} ({source})"));
            }
            let handlers = group.get("hooks").and_then(Value::as_array);
            for handler in handlers.into_iter().flatten() {
                if let Some(command) = handler_command(handler, event, source, warnings) {
                    out.push(Handler {
                        event,
                        matcher: matcher.clone(),
                        compiled: compiled.clone(),
                        command,
                        timeout: timeout(handler, event),
                    });
                }
            }
        }
    }
    out
}

fn handler_command(
    handler: &Value,
    event: Event,
    source: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let name = event.as_str();
    let kind = handler.get("type").and_then(Value::as_str).unwrap_or("");
    if kind != "command" {
        let kind = if kind.is_empty() { "untyped" } else { kind };
        warnings.push(format!(
            "Ignoring a {kind} {name} hook in {source}: only \"command\" hooks run."
        ));
        return None;
    }
    let command = handler
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let Some(command) = command else {
        warnings.push(format!(
            "Ignoring a {name} hook in {source}: it has no \"command\"."
        ));
        return None;
    };
    // Run in shell form and blocking, these would do something else than configured.
    let unsupported = if handler.get("args").is_some() {
        Some("\"args\"")
    } else if handler.get("async") == Some(&Value::Bool(true)) {
        Some("\"async\": true")
    } else {
        None
    };
    if let Some(field) = unsupported {
        warnings.push(format!(
            "Ignoring the {name} hook \"{command}\" in {source}: hooks with {field} are not supported yet."
        ));
        return None;
    }
    Some(command.to_string())
}

fn timeout(handler: &Value, event: Event) -> u64 {
    let configured = handler.get("timeout").and_then(|t| {
        t.as_u64()
            .or_else(|| t.as_f64().filter(|f| *f >= 1.0).map(|f| f as u64))
    });
    match configured {
        Some(seconds) if seconds > 0 => seconds,
        _ if event == Event::UserPromptSubmit => 30,
        _ => 600,
    }
}

/// The value with every object's keys sorted, as compact JSON.
pub fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for key in keys {
                    out.insert(key.clone(), sorted(&map[key]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sorted(value)).unwrap_or_default()
}

/// SHA-256 (hex) of a document's `hooks` value as it will run: after variable substitution,
/// independent of key order and whitespace.
pub fn digest(value: &Value) -> String {
    hex::encode(Sha256::digest(canonical(value).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_claude_code_block_loads_with_warnings_for_what_does_not_run() {
        let value = json!({
            "PreToolUse": [{"matcher": "Bash", "hooks": [
                {"type": "command", "command": "./guard.sh", "timeout": 30, "statusMessage": "x"},
                {"type": "prompt", "prompt": "is this ok?"},
                {"type": "command", "command": "a", "async": true},
                {"type": "command", "command": "b", "args": ["x"]},
                {"type": "command"},
            ]}],
            "UserPromptSubmit": [{"matcher": "ignored(", "hooks": [{"type": "command", "command": "u"}]}],
            "Notification": [{"hooks": [{"type": "command", "command": "n"}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "s", "timeout": 0}]}],
        });
        let mut warnings = Vec::new();
        let handlers = parse(&value, "a.json", &mut warnings);
        let summary: Vec<(Event, &str, u64)> = handlers
            .iter()
            .map(|h| (h.event, h.command.as_str(), h.timeout))
            .collect();
        assert_eq!(
            summary,
            [
                (Event::PreToolUse, "./guard.sh", 30),
                (Event::UserPromptSubmit, "u", 30),
                (Event::Stop, "s", 600),
            ]
        );
        assert_eq!(warnings.len(), 5, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("Notification")));
        assert!(warnings.iter().any(|w| w.contains("prompt")));
        assert!(warnings.iter().any(|w| w.contains("\"async\"")));
        assert!(warnings.iter().any(|w| w.contains("\"args\"")));
        // UserPromptSubmit ignores its matcher: no regex warning, matches anything.
        assert!(handlers[1].compiled.matches(&["whatever"]));
        let mut warnings = Vec::new();
        assert!(parse(&json!([1]), "b.json", &mut warnings).is_empty());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn digest_ignores_key_order_and_whitespace_but_not_content() {
        let a: Value = serde_json::from_str(
            r#"{"Stop": [{"hooks": [{"type": "command", "command": "x"}]}], "PreToolUse": []}"#,
        )
        .unwrap();
        let b: Value = serde_json::from_str(
            "{\"PreToolUse\":[],\n  \"Stop\":[{\"hooks\":[{\"command\":\"x\",\"type\":\"command\"}]}]}",
        )
        .unwrap();
        assert_eq!(digest(&a), digest(&b));
        assert_eq!(digest(&a).len(), 64);
        let c =
            json!({"Stop": [{"hooks": [{"type": "command", "command": "y"}]}], "PreToolUse": []});
        assert_ne!(digest(&a), digest(&c));
    }
}
