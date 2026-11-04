//! Markdown definitions (agents, commands, skills) with a YAML frontmatter block, split the way
//! 0.5.1's gray-matter split them. Frontmatter that is not valid YAML because a value holds an
//! unquoted colon (`description: Reviews: code`) is retried with such values as block scalars.

use serde_json::{Map, Number, Value};
use yaml_rust2::{Yaml, YamlLoader};

fn json(yaml: Yaml) -> Value {
    match yaml {
        Yaml::Real(text) => text
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map_or(Value::Null, Value::Number),
        Yaml::Integer(n) => Value::from(n),
        Yaml::String(s) => Value::String(s),
        Yaml::Boolean(b) => Value::Bool(b),
        Yaml::Array(items) => Value::Array(items.into_iter().map(json).collect()),
        Yaml::Hash(hash) => Value::Object(
            hash.into_iter()
                .map(|(key, value)| (key_text(key), json(value)))
                .collect(),
        ),
        Yaml::Alias(_) | Yaml::Null | Yaml::BadValue => Value::Null,
    }
}

fn key_text(key: Yaml) -> String {
    match json(key) {
        Value::String(s) => s,
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// The frontmatter as an object; `None` when it is not valid YAML.
fn frontmatter(text: &str) -> Option<Map<String, Value>> {
    let documents = YamlLoader::load_from_str(text).ok()?;
    match documents.into_iter().next().map(json) {
        Some(Value::Object(map)) => Some(map),
        _ => Some(Map::new()),
    }
}

/// Rewrites top-level `key: value` lines whose value holds an unquoted colon as block scalars.
fn sanitize(block: &str) -> String {
    let entry = regex::Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_]*)\s*:\s*(.*)$").expect("valid regex");
    block
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let trimmed = line.trim();
            if trimmed.starts_with('#')
                || trimmed.is_empty()
                || line.starts_with(char::is_whitespace)
            {
                return line.to_string();
            }
            let Some(captures) = entry.captures(line) else {
                return line.to_string();
            };
            let value = captures[2].trim();
            let keep = value.is_empty()
                || value == ">"
                || value == "|"
                || value.starts_with('"')
                || value.starts_with('\'')
                || !value.contains(':');
            if keep {
                line.to_string()
            } else {
                format!("{}: |-\n  {value}", &captures[1])
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Splits a document into its frontmatter (as a JSON object) and the content after it (not
/// trimmed). `None` when the frontmatter cannot be read even after the colon retry.
pub fn parse(text: &str) -> Option<(Map<String, Value>, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Some((Map::new(), text.to_string()));
    };
    let (block, content) = match rest.find("\n---") {
        Some(end) => {
            let after = &rest[end + 4..];
            let after = after.strip_prefix('\r').unwrap_or(after);
            (&rest[..end], after.strip_prefix('\n').unwrap_or(after))
        }
        None => (rest, ""),
    };
    let block = block.strip_suffix('\r').unwrap_or(block);
    let meta = frontmatter(block).or_else(|| frontmatter(&sanitize(block)))?;
    Some((meta, content.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(text: &str) -> Value {
        Value::Object(parse(text).unwrap().0)
    }

    #[test]
    fn frontmatter_and_content() {
        let (meta, body) = parse(
            "---\ndescription: \"Reviews code\"\nsteps: 3\npermission:\n  bash:\n    \"git *\": allow\n  edit: ask\nhidden: true\n---\nYou are a reviewer.\n",
        )
        .unwrap();
        assert_eq!(
            Value::Object(meta),
            json!({"description": "Reviews code", "steps": 3, "permission": {"bash": {"git *": "allow"}, "edit": "ask"}, "hidden": true})
        );
        assert_eq!(body, "You are a reviewer.\n");
        assert_eq!(parse("plain\n").unwrap(), (Map::new(), "plain\n".into()));
    }

    #[test]
    fn lists_of_rules() {
        assert_eq!(
            meta(
                "---\npermissions:\n  - action: shell\n    resource: \"*\"\n    effect: deny\ntemperature: 0.5\n---\n"
            ),
            json!({"permissions": [{"action": "shell", "resource": "*", "effect": "deny"}], "temperature": 0.5})
        );
    }

    #[test]
    fn unquoted_colons_are_retried_as_block_scalars() {
        assert_eq!(
            meta("---\ndescription: Reviews: code, carefully\nmode: subagent\n---\nBody"),
            json!({"description": "Reviews: code, carefully", "mode": "subagent"})
        );
        assert_eq!(parse("---\na: [unclosed\n---\n"), None);
    }

    #[test]
    fn crlf_and_bom() {
        let (meta, body) = parse("\u{feff}---\r\nname: x\r\n---\r\nbody\r\n").unwrap();
        assert_eq!(Value::Object(meta), json!({"name": "x"}));
        assert_eq!(body, "body\r\n");
    }
}
