//! Markdown definitions with a YAML frontmatter block (agents, later commands). The frontmatter
//! dialect is the flat one these files use: `key: value` scalars and one level of nested maps.

use serde_json::{Map, Number, Value};

fn scalar(text: &str) -> Value {
    let text = text.trim();
    let unquoted = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')));
    if let Some(inner) = unquoted {
        return Value::String(inner.to_string());
    }
    match text {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" | "~" | "" => Value::Null,
        _ => text
            .parse::<i64>()
            .map(Value::from)
            .ok()
            .or_else(|| {
                text.parse::<f64>()
                    .ok()
                    .and_then(Number::from_f64)
                    .map(Value::Number)
            })
            .unwrap_or_else(|| Value::String(text.to_string())),
    }
}

/// Splits a document into its frontmatter (as a JSON object) and its trimmed body.
pub fn parse(text: &str) -> (Map<String, Value>, String) {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let Some(rest) = text.strip_prefix("---\n") else {
        return (Map::new(), text.trim().to_string());
    };
    let Some(end) = rest.find("\n---") else {
        return (Map::new(), text.trim().to_string());
    };
    let body = rest[end + 4..]
        .trim_start_matches(|c| c != '\n')
        .trim()
        .to_string();
    let mut out = Map::new();
    let mut parent: Option<String> = None;
    for line in rest[..end].lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let nested = line.starts_with(' ') || line.starts_with('\t');
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let key = scalar(key)
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| key.trim().to_string());
        match (&parent, nested) {
            (Some(name), true) => {
                if let Some(Value::Object(map)) = out.get_mut(name) {
                    map.insert(key, scalar(value));
                }
            }
            _ if value.trim().is_empty() => {
                out.insert(key.clone(), Value::Object(Map::new()));
                parent = Some(key);
            }
            _ => {
                out.insert(key, scalar(value));
                parent = None;
            }
        }
    }
    (out, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frontmatter() {
        let (meta, body) = parse(
            "---\ndescription: Reviews: code\nsteps: 3\npermission:\n  bash: deny\n  \"edit\": ask\nhidden: true\n---\nYou are a reviewer.\n",
        );
        assert_eq!(
            Value::Object(meta),
            json!({"description": "Reviews: code", "steps": 3, "permission": {"bash": "deny", "edit": "ask"}, "hidden": true})
        );
        assert_eq!(body, "You are a reviewer.");
    }
}
