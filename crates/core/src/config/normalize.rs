//! Normalizes one config document to the V2 shape: V1 keys (`agent`, `mode`, `permission`, `tools`,
//! `provider`, `command`, `small_model`, `snapshot`, `plugin`) are migrated, V2 keys win ties.

use serde_json::{Map, Value, json};

/// V1 permission/tool names renamed in V2.
pub fn normalize_action(action: &str) -> &str {
    match action {
        "write" | "patch" | "apply_patch" => "edit",
        "task" => "subagent",
        "bash" => "shell",
        other => other,
    }
}

/// `Object.entries` order, which decides last-match-wins rules: integer-like keys first,
/// ascending, then the rest as written.
fn js_entries(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let index = |key: &str| {
        key.parse::<u32>()
            .ok()
            .filter(|n| *n != u32::MAX && n.to_string() == key)
    };
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(key, _)| index(key).map_or((1, 0), |n| (0, n)));
    entries
}

fn effect(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|e| matches!(*e, "allow" | "ask" | "deny"))
}

/// `"permission": "allow"` or `{"tool": "deny", "shell": {"git *": "allow"}}` as V2 rules.
fn migrate_permission(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::String(_)) => effect(value.unwrap())
            .map(|e| vec![rule("*", "*", e)])
            .unwrap_or_default(),
        Some(Value::Object(map)) => js_entries(map)
            .into_iter()
            .flat_map(|(tool, setting)| {
                let action = normalize_action(tool);
                match setting {
                    Value::Object(patterns) => js_entries(patterns)
                        .into_iter()
                        .filter_map(|(resource, e)| effect(e).map(|e| rule(action, resource, e)))
                        .collect(),
                    other => effect(other)
                        .map(|e| vec![rule(action, "*", e)])
                        .unwrap_or_default(),
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `"tools": {"webfetch": false}` as allow/deny rules.
fn migrate_tools(value: Option<&Value>) -> Vec<Value> {
    let Some(Value::Object(map)) = value else {
        return Vec::new();
    };
    js_entries(map)
        .into_iter()
        .filter_map(|(tool, on)| {
            on.as_bool().map(|on| {
                rule(
                    normalize_action(tool),
                    "*",
                    if on { "allow" } else { "deny" },
                )
            })
        })
        .collect()
}

fn rule(action: &str, resource: &str, effect: &str) -> Value {
    json!({"action": action, "resource": resource, "effect": effect})
}

/// `provider/model` plus an optional variant as the `provider/model#variant` reference.
pub fn model_ref(value: &Value, variant: Option<&Value>) -> Option<String> {
    let base = match value {
        Value::String(text) => text.clone(),
        Value::Object(map) => {
            let provider = map.get("providerID")?.as_str()?;
            let model = map.get("model").or_else(|| map.get("id"))?.as_str()?;
            let variant = map.get("variant").and_then(Value::as_str);
            return Some(match variant {
                Some(v) if !v.is_empty() => format!("{provider}/{model}#{v}"),
                _ => format!("{provider}/{model}"),
            });
        }
        _ => return None,
    };
    match variant.and_then(Value::as_str) {
        Some(v) if !v.is_empty() && !v.contains('#') && !base.contains('#') => {
            Some(format!("{base}#{v}"))
        }
        _ => Some(base),
    }
}

const V1_THEME_COLORS: [&str; 7] = [
    "primary",
    "secondary",
    "accent",
    "success",
    "warning",
    "error",
    "info",
];

fn hex_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn positive_int(value: &Value) -> bool {
    value.as_u64().is_some_and(|n| n > 0)
}

/// Whether a V1 agent decodes: every known field has its type, `color` is a hex colour or a
/// theme colour name.
fn valid_v1(v1: &Map<String, Value>) -> bool {
    let typed = |key: &str, check: fn(&Value) -> bool| v1.get(key).is_none_or(check);
    typed("model", Value::is_string)
        && typed("variant", Value::is_string)
        && typed("prompt", Value::is_string)
        && typed("description", Value::is_string)
        && typed("temperature", Value::is_number)
        && typed("top_p", Value::is_number)
        && typed("disable", Value::is_boolean)
        && typed("hidden", Value::is_boolean)
        && typed("steps", positive_int)
        && typed("maxSteps", positive_int)
        && typed("tools", Value::is_object)
        && typed("mode", |m| {
            matches!(m.as_str(), Some("subagent" | "primary" | "all"))
        })
        && typed("color", |c| {
            c.as_str()
                .is_some_and(|c| hex_color(c) || V1_THEME_COLORS.contains(&c))
        })
}

/// A V1 agent as a V2 agent, `None` when it does not decode as V1. `tools` and `permission`
/// share one map per tool (a key keeps its first position, its last value), as V1 merged them.
pub fn migrate_agent(v1: &Map<String, Value>) -> Option<Value> {
    if !valid_v1(v1) {
        return None;
    }
    let mut agent = Map::new();
    if let Some(model) = v1
        .get("model")
        .and_then(|m| model_ref(m, v1.get("variant")))
    {
        agent.insert("model".into(), Value::String(model));
    }
    let copy = [
        ("prompt", "system"),
        ("description", "description"),
        ("mode", "mode"),
        ("hidden", "hidden"),
    ];
    for (from, to) in copy {
        if let Some(value) = v1.get(from) {
            agent.insert(to.into(), value.clone());
        }
    }
    if let Some(color) = v1.get("color").and_then(Value::as_str) {
        let color = if color.starts_with('#') {
            color.to_string()
        } else {
            "#aaaaaa".to_string()
        };
        agent.insert("color".into(), Value::String(color));
    }
    if let Some(steps) = v1.get("steps").or_else(|| v1.get("maxSteps")) {
        agent.insert("steps".into(), steps.clone());
    }
    if let Some(disable) = v1.get("disable") {
        agent.insert("disabled".into(), disable.clone());
    }
    let mut merged = Map::new();
    if let Some(Value::Object(tools)) = v1.get("tools") {
        for (tool, on) in js_entries(tools) {
            let Some(on) = on.as_bool() else { continue };
            let key = if tool == "write" || tool == "patch" {
                "edit"
            } else {
                tool
            };
            merged.insert(key.to_string(), json!(if on { "allow" } else { "deny" }));
        }
    }
    match v1.get("permission") {
        Some(Value::Object(permission)) => {
            for (key, rule) in js_entries(permission) {
                merged.insert(key.clone(), rule.clone());
            }
        }
        Some(other) => {
            merged.insert("*".into(), other.clone());
        }
        None => {}
    }
    let permissions = migrate_permission(Some(&Value::Object(merged)));
    if !permissions.is_empty() {
        agent.insert("permissions".into(), Value::Array(permissions));
    }
    Some(Value::Object(agent))
}

pub fn native_agent(agent: &Value) -> Value {
    let Value::Object(map) = agent else {
        return agent.clone();
    };
    let mut out = map.clone();
    if let Some(model) = map
        .get("model")
        .and_then(|m| model_ref(m, map.get("variant")))
    {
        out.insert("model".into(), Value::String(model));
    }
    out.remove("variant");
    Value::Object(out)
}

/// Merges `next` into `base` entry by entry (`next` wins whole entries).
fn merge_entries(base: &mut Map<String, Value>, next: Map<String, Value>) {
    for (key, value) in next {
        base.insert(key, value);
    }
}

pub fn normalize(input: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let mut agents = Map::new();
    for key in ["agent", "mode"] {
        if let Some(Value::Object(map)) = input.get(key) {
            for (id, v1) in map {
                let Value::Object(v1) = v1 else { continue };
                let Some(mut migrated) = migrate_agent(v1) else {
                    continue;
                };
                if key == "mode" {
                    migrated["mode"] = Value::String("primary".into());
                }
                agents.insert(id.clone(), migrated);
            }
        }
    }
    if let Some(small) = input.get("small_model").and_then(|m| model_ref(m, None)) {
        let title = agents.entry("title").or_insert_with(|| json!({}));
        if let Value::Object(title) = title {
            title.entry("model").or_insert(Value::String(small));
        }
    }
    if let Some(Value::Object(native)) = input.get("agents") {
        merge_entries(
            &mut agents,
            native
                .iter()
                .map(|(k, v)| (k.clone(), native_agent(v)))
                .collect(),
        );
    }
    if !agents.is_empty() || input.contains_key("agents") {
        out.insert("agents".into(), Value::Object(agents));
    }

    let permissions: Vec<Value> = migrate_tools(input.get("tools"))
        .into_iter()
        .chain(migrate_permission(input.get("permission")))
        .chain(
            input
                .get("permissions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        )
        .collect();
    if !permissions.is_empty() {
        out.insert("permissions".into(), Value::Array(permissions));
    }

    for (legacy, native) in [("provider", "providers"), ("command", "commands")] {
        let mut merged = input
            .get(legacy)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(Value::Object(map)) = input.get(native) {
            merge_entries(&mut merged, map.clone());
        }
        if !merged.is_empty() {
            out.insert(native.into(), Value::Object(merged));
        }
    }
    if let Some(model) = input.get("model").and_then(|m| model_ref(m, None)) {
        out.insert("model".into(), Value::String(model));
    }
    if let Some(snapshot) = input
        .get("snapshot")
        .filter(|_| !input.contains_key("snapshots"))
    {
        out.insert("snapshots".into(), snapshot.clone());
    }
    if let Some(plugin) = input
        .get("plugin")
        .filter(|_| !input.contains_key("plugins"))
    {
        out.insert("plugins".into(), plugin.clone());
    }
    const HANDLED: &[&str] = &[
        "agent",
        "mode",
        "agents",
        "small_model",
        "tools",
        "permission",
        "permissions",
        "provider",
        "providers",
        "command",
        "commands",
        "model",
        "snapshot",
        "plugin",
    ];
    for (key, value) in input {
        if !HANDLED.contains(&key.as_str()) {
            out.insert(key, value);
        }
    }
    out
}

const V2_AGENT_KEYS: &[&str] = &[
    "model",
    "variant",
    "request",
    "system",
    "description",
    "mode",
    "hidden",
    "color",
    "steps",
    "disabled",
    "permissions",
];

/// Whether a V2 agent decodes; a bad `color` alone is dropped, not the agent.
fn valid_v2(agent: &Map<String, Value>) -> bool {
    let typed = |key: &str, check: fn(&Value) -> bool| agent.get(key).is_none_or(check);
    typed("model", |m| m.is_string() || m.is_object())
        && typed("request", Value::is_object)
        && typed("system", Value::is_string)
        && typed("description", Value::is_string)
        && typed("hidden", Value::is_boolean)
        && typed("disabled", Value::is_boolean)
        && typed("steps", positive_int)
        && typed("permissions", Value::is_array)
        && typed("mode", |m| {
            matches!(m.as_str(), Some("subagent" | "primary" | "all"))
        })
}

/// A markdown agent as a V2 agent: its frontmatter and `body` (the system prompt). Read as V1 and
/// migrated when it has keys V2 does not know and none only V2 knows. `None` when it does not
/// decode.
pub fn markdown_agent(meta: &Map<String, Value>, body: &str) -> Option<Value> {
    let native = ["permissions", "request", "system", "disabled"]
        .iter()
        .any(|k| meta.contains_key(*k));
    let unsupported = meta.keys().any(|k| !V2_AGENT_KEYS.contains(&k.as_str()));
    if unsupported && !native {
        let mut v1 = meta.clone();
        v1.insert("prompt".into(), Value::String(body.into()));
        return migrate_agent(&v1);
    }
    let mut v2 = meta.clone();
    v2.insert("system".into(), Value::String(body.into()));
    if !valid_v2(&v2) {
        return None;
    }
    if v2
        .get("color")
        .is_some_and(|c| !c.as_str().is_some_and(hex_color))
    {
        v2.remove("color");
    }
    v2.retain(|k, _| V2_AGENT_KEYS.contains(&k.as_str()));
    Some(native_agent(&Value::Object(v2)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(value: Value) -> Value {
        Value::Object(normalize(value.as_object().unwrap().clone()))
    }

    #[test]
    fn v1_keys() {
        let out = norm(json!({
            "small_model": "anysola/claude-haiku-5-5",
            "permission": {"bash": {"git *": "allow"}, "webfetch": "deny"},
            "tools": {"write": false},
            "agent": {"rev": {"prompt": "Review.", "maxSteps": 3, "color": "primary", "model": "anysola/m", "variant": "high"}},
        }));
        assert_eq!(out["agents"]["title"]["model"], "anysola/claude-haiku-5-5");
        let ordered =
            norm(json!({"permission": {"read": {"*.md": "deny", "10": "allow", "2": "ask"}}}));
        let resources: Vec<&str> = ordered["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["resource"].as_str().unwrap())
            .collect();
        assert_eq!(resources, ["2", "10", "*.md"]);
        assert_eq!(
            out["agents"]["rev"],
            json!({"model": "anysola/m#high", "system": "Review.", "color": "#aaaaaa", "steps": 3})
        );
        assert_eq!(
            out["permissions"],
            json!([
                {"action": "edit", "resource": "*", "effect": "deny"},
                {"action": "shell", "resource": "git *", "effect": "allow"},
                {"action": "webfetch", "resource": "*", "effect": "deny"},
            ])
        );
    }
}
