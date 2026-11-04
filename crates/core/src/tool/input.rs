//! Tool input handling: the same repairs 0.5.1 applied before validation (numeric strings, stray
//! keys, null placeholders, stringified objects/arrays), then validation against the tool's schema
//! with Effect Schema's issue wording, then deserialization.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::ToolError;

const MAX_DEPTH: usize = 6;
const MAX_ISSUES: usize = 5;

/// Repairs, validates and decodes a tool's input. The error text is the model-visible
/// `Invalid arguments for tool …` message.
pub fn parse_input<T: DeserializeOwned>(
    tool: &str,
    schema: &Value,
    input: Value,
) -> Result<T, ToolError> {
    let repaired = repair(input, schema, 0);
    let mut issues = Vec::new();
    validate(&repaired, schema, "", &mut issues);
    if issues.is_empty() {
        if let Ok(value) = serde_json::from_value(repaired.clone()) {
            return Ok(value);
        }
        issues.push(("root".to_string(), "Expected object".to_string()));
    }
    Err(ToolError::execution(format_issues(
        tool, &issues, &repaired,
    )))
}

pub(crate) fn format_issues(tool: &str, issues: &[(String, String)], value: &Value) -> String {
    let mut lines: Vec<String> = issues
        .iter()
        .take(MAX_ISSUES)
        .map(|(path, message)| format!("- {path}: {message}"))
        .collect();
    if issues.len() > MAX_ISSUES {
        let more = issues.len() - MAX_ISSUES;
        lines.push(format!(
            "- ...and {more} more {}",
            if more == 1 { "issue" } else { "issues" }
        ));
    }
    format!(
        "Invalid arguments for tool \"{tool}\":\n{}\n\nArguments provided:\n{}\n\nUpdate the arguments and call the tool again.",
        lines.join("\n"),
        crate::json::pretty(value)
    )
}

fn schema_type(schema: &Value) -> Option<&str> {
    schema.get("type").and_then(Value::as_str)
}

fn repair(value: Value, schema: &Value, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return value;
    }
    match schema_type(schema) {
        Some("number") | Some("integer") => match &value {
            Value::String(text) if !text.trim().is_empty() => match text.trim().parse::<f64>() {
                Ok(n)
                    if n.is_finite()
                        && (schema_type(schema) != Some("integer") || is_safe_integer(n)) =>
                {
                    number(n)
                }
                _ => value,
            },
            _ => value,
        },
        Some("boolean") => match value.as_str() {
            Some("true") => Value::Bool(true),
            Some("false") => Value::Bool(false),
            _ => value,
        },
        Some("object") => repair_object(value, schema, depth),
        Some("array") => repair_array(value, schema, depth),
        _ => value,
    }
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

fn is_safe_integer(n: f64) -> bool {
    n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0
}

fn decode_json(value: &Value) -> Option<Value> {
    value
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
}

fn repair_object(value: Value, schema: &Value, depth: usize) -> Value {
    let parsed = if value.is_string() {
        decode_json(&value).unwrap_or(value)
    } else {
        value
    };
    let Value::Object(map) = parsed else {
        return parsed;
    };
    let empty = Map::new();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    let mut out = Map::new();
    for (key, current) in map {
        let Some(property) = properties.get(&key) else {
            if !closed {
                out.insert(key, current);
            }
            continue;
        };
        let plain = matches!(schema_type(property), Some(t) if t != "null")
            && [
                "anyOf", "oneOf", "allOf", "enum", "const", "$ref", "nullable",
            ]
            .iter()
            .all(|k| property.get(k).is_none());
        let placeholder = current.as_object().is_some_and(|o| o.is_empty())
            && schema_type(property) != Some("object");
        if !required.contains(&key.as_str()) && plain && (current.is_null() || placeholder) {
            continue;
        }
        let repaired = repair(current, property, depth + 1);
        out.insert(key, repaired);
    }
    Value::Object(out)
}

fn repair_array(value: Value, schema: &Value, depth: usize) -> Value {
    let parsed = if value.is_string() {
        decode_json(&value).unwrap_or(value)
    } else {
        value
    };
    let Some(items) = schema.get("items").filter(|i| i.is_object()) else {
        return parsed;
    };
    match parsed {
        Value::Array(list) => Value::Array(
            list.into_iter()
                .map(|item| repair(item, items, depth + 1))
                .collect(),
        ),
        single => {
            let repaired = repair(single.clone(), items, depth + 1);
            let compatible = match schema_type(items) {
                Some("object") => repaired.is_object(),
                Some("array") => repaired.is_array(),
                Some("integer") => repaired.as_f64().is_some_and(is_safe_integer),
                Some("number") => repaired.is_number(),
                Some("string") => repaired.is_string(),
                Some("boolean") => repaired.is_boolean(),
                _ => false,
            };
            if compatible {
                Value::Array(vec![repaired])
            } else {
                single
            }
        }
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn validate(value: &Value, schema: &Value, path: &str, issues: &mut Vec<(String, String)>) {
    let at = || {
        if path.is_empty() {
            "root".to_string()
        } else {
            path.to_string()
        }
    };
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(value) {
            let expected: Vec<String> = options.iter().map(Value::to_string).collect();
            issues.push((at(), format!("Expected {}", expected.join(" | "))));
        }
        return;
    }
    match schema_type(schema) {
        Some("object") => {
            let Some(map) = value.as_object() else {
                issues.push((at(), "Expected object".into()));
                return;
            };
            let properties = schema.get("properties").and_then(Value::as_object);
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if let Some(properties) = properties {
                for (key, property) in properties {
                    match map.get(key) {
                        Some(child) => validate(child, property, &join(path, key), issues),
                        None if required.contains(&key.as_str()) => {
                            issues.push((join(path, key), "Missing key".into()))
                        }
                        None => {}
                    }
                }
            }
        }
        Some("array") => {
            let Some(list) = value.as_array() else {
                issues.push((at(), "Expected array".into()));
                return;
            };
            if let Some(items) = schema.get("items") {
                for (index, item) in list.iter().enumerate() {
                    validate(
                        item,
                        items,
                        &format!("{}[{index}]", if path.is_empty() { "" } else { path }),
                        issues,
                    );
                }
            }
        }
        Some("string") => {
            let Some(text) = value.as_str() else {
                issues.push((at(), "Expected string".into()));
                return;
            };
            // Effect renders `startsWith` checks as anchored literal patterns.
            if let Some(prefix) = schema
                .get("pattern")
                .and_then(Value::as_str)
                .and_then(|p| p.strip_prefix('^'))
                .filter(|p| p.chars().all(|c| c.is_alphanumeric() || c == '_'))
                && !text.starts_with(prefix)
            {
                issues.push((
                    at(),
                    format!("Expected a string starting with \"{prefix}\""),
                ));
            }
            let length = text.encode_utf16().count() as u64;
            if let Some(min) = schema.get("minLength").and_then(Value::as_u64)
                && length < min
            {
                issues.push((
                    at(),
                    format!("Expected a value with a length of at least {min}"),
                ));
            }
        }
        Some("boolean") if !value.is_boolean() => issues.push((at(), "Expected boolean".into())),
        Some(kind @ ("integer" | "number")) => {
            let Some(n) = value.as_f64() else {
                issues.push((at(), "Expected number".into()));
                return;
            };
            if kind == "integer" && n.fract() != 0.0 {
                issues.push((at(), "Expected an integer".into()));
                return;
            }
            bounds(n, schema, &at(), issues);
        }
        _ => {}
    }
}

fn bounds(n: f64, schema: &Value, path: &str, issues: &mut Vec<(String, String)>) {
    let show = |v: &Value| v.to_string();
    if let (Some(min), Some(max)) = (schema.get("minimum"), schema.get("maximum"))
        && let (Some(low), Some(high)) = (min.as_f64(), max.as_f64())
    {
        if n < low || n > high {
            issues.push((
                path.into(),
                format!("Expected a value between {} and {}", show(min), show(max)),
            ));
        }
        return;
    }
    if let Some(min) = schema
        .get("minimum")
        .filter(|m| m.as_f64().is_some_and(|m| n < m))
    {
        issues.push((
            path.into(),
            format!("Expected a value greater than or equal to {}", show(min)),
        ));
    }
    if let Some(min) = schema
        .get("exclusiveMinimum")
        .filter(|m| m.as_f64().is_some_and(|m| n <= m))
    {
        issues.push((
            path.into(),
            format!("Expected a value greater than {}", show(min)),
        ));
    }
    if let Some(max) = schema
        .get("maximum")
        .filter(|m| m.as_f64().is_some_and(|m| n > m))
    {
        issues.push((
            path.into(),
            format!("Expected a value less than or equal to {}", show(max)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct ReadInput {
        path: String,
        offset: Option<i64>,
    }

    fn schema() -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0}},"required":["path"],"additionalProperties":false})
    }

    fn message(input: Value) -> String {
        parse_input::<ReadInput>("read", &schema(), input)
            .unwrap_err()
            .message
    }

    #[test]
    fn effect_wording() {
        assert_eq!(
            message(json!({})),
            "Invalid arguments for tool \"read\":\n- path: Missing key\n\nArguments provided:\n{}\n\nUpdate the arguments and call the tool again."
        );
        assert!(message(json!({"path": 1})).contains("- path: Expected string\n"));
        assert!(
            message(json!({"path": "a", "offset": -1}))
                .contains("- offset: Expected a value greater than or equal to 0\n")
        );
        assert!(
            message(json!({"path": "a", "offset": 1.5}))
                .contains("- offset: Expected an integer\n")
        );
        assert!(
            message(json!([1, 2]))
                .contains("- root: Expected object\n\nArguments provided:\n[\n  1,\n  2\n]")
        );
    }

    #[test]
    fn repairs() {
        let ok: ReadInput = parse_input(
            "read",
            &schema(),
            json!({"path": "a", "offset": "3", "extra": 1}),
        )
        .unwrap();
        assert_eq!(ok.offset, Some(3));
        let ok: ReadInput =
            parse_input("read", &schema(), json!({"path": "a", "offset": null})).unwrap();
        assert_eq!(ok.offset, None);
        let ok: ReadInput = parse_input("read", &schema(), json!("{\"path\":\"a\"}")).unwrap();
        assert_eq!(ok.path, "a");
    }
}
