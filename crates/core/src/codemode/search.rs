//! The program-side `search(...)` function and the "Did you mean" hint for unknown tools: the same
//! word tokenizing and scoring as 0.5.1.

use serde_json::{Value, json};

use super::Inventory;
use super::typescript::{signature, tool_expression};

const DEFAULT_LIMIT: u64 = 10;

struct Entry {
    path: String,
    description: String,
    signature: String,
    path_words: Vec<String>,
    search_text: String,
}

/// Every tool of an inventory, sorted by path.
pub struct Index {
    entries: Vec<Entry>,
}

/// Words of a query or path: camelCase split, lowercased, cut at anything but ASCII letters and
/// digits.
fn tokenize(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut spaced = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase()
            && i > 0
            && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit())
        {
            spaced.push(' ');
        }
        spaced.push(c);
    }
    spaced
        .to_lowercase()
        .split(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit())
        .filter(|t| !t.is_empty() && *t != "*")
        .map(str::to_string)
        .collect()
}

/// A term and its plural-stripped forms.
fn forms(term: &str) -> Vec<String> {
    let mut forms = vec![term.to_string()];
    if term.ends_with("es") && term.len() > 3 {
        forms.push(term[..term.len() - 2].to_string());
    }
    if term.ends_with('s') && term.len() > 2 {
        forms.push(term[..term.len() - 1].to_string());
    }
    forms
}

impl Index {
    pub fn new(inventory: &Inventory) -> Self {
        let mut entries: Vec<Entry> = inventory
            .tools()
            .map(|(namespace, tool)| {
                let path = super::path_of(&namespace.name, tool.name());
                let mut text = vec![path.clone(), tool.description().to_string()];
                text.extend(namespace.description.clone());
                if let Some(properties) = tool
                    .input_schema()
                    .get("properties")
                    .and_then(Value::as_object)
                {
                    for (name, property) in properties {
                        text.push(name.clone());
                        if let Some(description) =
                            property.get("description").and_then(Value::as_str)
                        {
                            text.push(description.to_string());
                        }
                    }
                }
                Entry {
                    signature: signature(&path, tool.input_schema(), tool.output_schema()),
                    description: tool.description().to_string(),
                    path_words: tokenize(&path),
                    search_text: text.join("\n").to_lowercase(),
                    path,
                }
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Index { entries }
    }

    fn rank<'a>(&self, entries: impl Iterator<Item = &'a Entry>, query: &str) -> Vec<&'a Entry> {
        let terms: Vec<Vec<String>> = tokenize(query).iter().map(|t| forms(t)).collect();
        let mut scored: Vec<(&Entry, u32)> = entries
            .map(|entry| {
                let path = entry.path.to_lowercase();
                let description = entry.description.to_lowercase();
                let score = terms
                    .iter()
                    .map(|forms| {
                        let any = |test: &dyn Fn(&str) -> bool| forms.iter().any(|f| test(f));
                        let mut score = 0;
                        if any(&|f| path == f || path.ends_with(&format!(".{f}"))) {
                            score += 20;
                        }
                        if any(&|f| entry.path_words.iter().any(|w| w == f)) {
                            score += 12;
                        }
                        if any(&|f| path.contains(f)) {
                            score += 8;
                        }
                        if any(&|f| description.contains(f)) {
                            score += 4;
                        }
                        if any(&|f| entry.search_text.contains(f)) {
                            score += 2;
                        }
                        score
                    })
                    .sum();
                (entry, score)
            })
            .filter(|(_, score)| terms.is_empty() || *score > 0)
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
        scored.into_iter().map(|(entry, _)| entry).collect()
    }

    /// The nearest tool to an unknown path, ranked within its namespace when that exists.
    pub fn closest(&self, segments: &[String], namespace_exists: bool) -> Option<String> {
        let name = segments.join(".");
        let ranked = if segments.len() > 1 && namespace_exists {
            let prefix = format!("{}.", segments[0]);
            self.rank(
                self.entries.iter().filter(|e| e.path.starts_with(&prefix)),
                &segments[1..].join(" "),
            )
        } else {
            self.rank(self.entries.iter(), &name)
        };
        ranked.first().map(|e| tool_expression(&e.path))
    }

    /// Runs `search(input)`. The error is the program-visible message.
    pub fn search(&self, input: &Value) -> Result<Value, String> {
        let request = decode(input).map_err(|e| format!("Invalid input for tool 'search': {e}"))?;
        let mut namespace = request.namespace;
        if let Some(ns) = &namespace
            && !self.entries.iter().any(|e| e.path.starts_with("tools."))
        {
            if ns == "tools" {
                namespace = None;
            } else if let Some(rest) = ns.strip_prefix("tools.") {
                namespace = Some(rest.to_string());
            }
        }
        let scoped: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| {
                namespace
                    .as_ref()
                    .is_none_or(|ns| e.path == *ns || e.path.starts_with(&format!("{ns}.")))
            })
            .collect();
        let query = request.query.unwrap_or_default();
        let trimmed = query.trim();
        let path_query = trimmed.strip_prefix("tools.").unwrap_or(trimmed);
        let exact = (!path_query.is_empty())
            .then(|| {
                scoped
                    .iter()
                    .find(|e| e.path == path_query || tool_expression(&e.path) == trimmed)
            })
            .flatten();
        let ranked = match exact {
            Some(entry) => vec![*entry],
            None => self.rank(scoped.into_iter(), &query),
        };
        let offset = request.offset.unwrap_or(0) as usize;
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT) as usize;
        let items: Vec<Value> = ranked
            .iter()
            .skip(offset)
            .take(limit)
            .map(|e| {
                json!({
                    "path": tool_expression(&e.path),
                    "description": e.description,
                    "signature": e.signature,
                })
            })
            .collect();
        let remaining = ranked.len().saturating_sub(offset + items.len());
        let next = if remaining > 0 {
            json!({"offset": offset + items.len()})
        } else {
            Value::Null
        };
        Ok(json!({"items": items, "remaining": remaining, "next": next}))
    }
}

struct Request {
    query: Option<String>,
    namespace: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
}

/// Effect Schema's first issue, as `SchemaError(<message>[\n  at ["key"]])`.
fn decode(input: &Value) -> Result<Request, String> {
    let issue = |message: &str, key: Option<&str>| {
        let at = key.map(|k| format!("\n  at [\"{k}\"]")).unwrap_or_default();
        format!("SchemaError({message}{at})")
    };
    let Some(map) = input.as_object() else {
        return Err(issue("Expected object", None));
    };
    let text = |key: &str| -> Result<Option<String>, String> {
        match map.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(issue("Expected string", Some(key))),
        }
    };
    let integer = |key: &str, minimum: u64| -> Result<Option<u64>, String> {
        let Some(value) = map.get(key) else {
            return Ok(None);
        };
        let Some(n) = value.as_f64() else {
            return Err(issue("Expected number", Some(key)));
        };
        if n.fract() != 0.0 {
            return Err(issue("Expected an integer", Some(key)));
        }
        if n < minimum as f64 {
            let message = if minimum == 0 {
                "Expected a value greater than or equal to 0"
            } else {
                "Expected a value greater than 0"
            };
            return Err(issue(message, Some(key)));
        }
        Ok(Some(n as u64))
    };
    Ok(Request {
        query: text("query")?,
        namespace: text("namespace")?,
        limit: integer("limit", 1)?,
        offset: integer("offset", 0)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codemode::opencode;

    fn index() -> Index {
        Index::new(&Inventory::new(vec![opencode::namespace()]))
    }

    fn paths(result: &Value) -> Vec<&str> {
        result["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["path"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn ranks_like_0_5_1() {
        let index = index();
        assert_eq!(
            paths(&index.search(&json!({"query": "worktree"})).unwrap()),
            vec![
                "tools.opencode.worktree_create",
                "tools.opencode.worktree_list",
                "tools.opencode.worktree_remove",
                "tools.opencode.list_mcp_resources",
                "tools.opencode.models",
                "tools.opencode.read_mcp_resource",
                "tools.opencode.session_move",
                "tools.opencode.session_rename",
            ]
        );
        let page = index
            .search(&json!({"namespace": "opencode", "limit": 2, "offset": 1}))
            .unwrap();
        assert_eq!(
            paths(&page),
            vec!["tools.opencode.models", "tools.opencode.read_mcp_resource"]
        );
        assert_eq!(page["remaining"], 5);
        assert_eq!(page["next"], json!({"offset": 3}));
        assert_eq!(
            paths(
                &index
                    .search(&json!({"query": "tools.opencode.models"}))
                    .unwrap()
            ),
            vec!["tools.opencode.models"]
        );
    }

    #[test]
    fn closest_tool() {
        let index = index();
        let segments = |p: &str| p.split('.').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            index.closest(&segments("nope.x"), false).as_deref(),
            Some("tools.opencode.list_mcp_resources")
        );
        assert_eq!(index.closest(&segments("opencode.nope"), true), None);
    }

    #[test]
    fn input_errors() {
        let index = index();
        assert_eq!(
            index.search(&json!("worktree")).unwrap_err(),
            "Invalid input for tool 'search': SchemaError(Expected object)"
        );
        assert_eq!(
            index.search(&json!({"query": 1})).unwrap_err(),
            "Invalid input for tool 'search': SchemaError(Expected string\n  at [\"query\"])"
        );
        assert_eq!(
            index.search(&json!({"limit": 0})).unwrap_err(),
            "Invalid input for tool 'search': SchemaError(Expected a value greater than 0\n  at [\"limit\"])"
        );
    }

    #[test]
    fn search_signature_renders_from_its_schema() {
        let input = json!({"type": "object", "properties": {
            "query": {"type": "string"}, "namespace": {"type": "string"},
            "limit": {"type": "integer", "exclusiveMinimum": 0},
            "offset": {"type": "integer", "minimum": 0}}});
        let item = json!({"type": "object", "properties": {"path": {"type": "string"}, "description": {"type": "string"}, "signature": {"type": "string"}}, "required": ["path", "description", "signature"]});
        let output = json!({"type": "object", "properties": {
            "items": {"type": "array", "items": item},
            "remaining": {"type": "integer", "minimum": 0},
            "next": {"anyOf": [{"type": "object", "properties": {"offset": {"type": "integer", "minimum": 0}}, "required": ["offset"]}, {"type": "null"}]}},
            "required": ["items", "remaining", "next"]});
        let rendered = format!(
            "search({}): {}",
            super::super::typescript::render(&input),
            super::super::typescript::render(&output)
        );
        assert_eq!(rendered, include_str!("search-signature.txt"));
    }
}
