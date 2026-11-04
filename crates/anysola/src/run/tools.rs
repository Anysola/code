//! One-line tool summaries for `run`'s stderr, as 0.5.1's shared tool formatter wrote them.

use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};

pub struct Inline {
    pub icon: &'static str,
    pub title: String,
    pub description: Option<String>,
    /// Block tools print their body under the title, framed by blank lines.
    pub body: Option<String>,
    pub block: bool,
}

/// A value interpolated into a JavaScript template string (`undefined`/`null` read as empty).
fn js(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(js).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
        other => other.to_string(),
    }
}

fn text(input: &Map<String, Value>, key: &str) -> String {
    input.get(key).map(js).unwrap_or_default()
}

/// `[key=value, …]` of the primitive inputs.
fn primitive_summary(input: &Map<String, Value>, omit: &[&str]) -> String {
    let entries: Vec<String> = input
        .iter()
        .filter(|(key, _)| !omit.contains(&key.as_str()))
        .filter_map(|(key, value)| match value {
            Value::String(s) => Some(format!("{key}={s}")),
            Value::Number(n) => Some(format!("{key}={n}")),
            Value::Bool(b) => Some(format!("{key}={b}")),
            _ => None,
        })
        .collect();
    if entries.is_empty() {
        String::new()
    } else {
        format!("[{}]", entries.join(", "))
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The path relative to the session directory when inside it, else absolute; `/` separators.
pub fn display_path(input: &str, directory: &Path) -> String {
    if input.is_empty() {
        return String::new();
    }
    let absolute = normalize(&directory.join(input));
    let formatted = match absolute.strip_prefix(directory) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.display().to_string(),
        Err(_) => absolute.display().to_string(),
    };
    formatted.replace('\\', "/")
}

fn count(n: u64, label: &str) -> String {
    format!("{n} {label}{}", if n == 1 { "" } else { "es" })
}

fn search(
    icon: &'static str,
    kind: &str,
    input: &Map<String, Value>,
    metadata: &Map<String, Value>,
    key: &str,
    dir: &Path,
) -> Inline {
    let root = text(input, "path");
    let suffix = if root.is_empty() {
        String::new()
    } else {
        format!("in {}", display_path(&root, dir))
    };
    let description = match metadata.get(key).and_then(Value::as_u64) {
        None => suffix,
        Some(n) => format!(
            "{suffix}{}{}",
            if suffix.is_empty() { "" } else { " · " },
            count(n, "match")
        ),
    };
    Inline {
        icon,
        title: format!("{kind} \"{}\"", text(input, "pattern")),
        description: Some(description).filter(|d| !d.is_empty()),
        body: None,
        block: false,
    }
}

fn inline(icon: &'static str, title: String) -> Inline {
    Inline {
        icon,
        title,
        description: None,
        body: None,
        block: false,
    }
}

/// `output` is the tool's display text (the first text item for shell, all text items otherwise).
pub fn inline_info(
    name: &str,
    input: &Map<String, Value>,
    metadata: &Map<String, Value>,
    output: Option<&str>,
    directory: &Path,
) -> Inline {
    let path = || {
        let path = input
            .get("path")
            .or_else(|| input.get("filePath"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        display_path(path, directory)
    };
    // A non-string path made 0.5.1's formatter throw, which fell back to the generic line.
    let bad_path = input
        .get("path")
        .is_some_and(|p| !p.is_string() && !p.is_null());
    match name {
        "read" | "write" | "edit" if bad_path => fallback(name, input),
        "shell" | "bash" => Inline {
            icon: "$",
            title: text(input, "command"),
            description: None,
            body: output.map(|o| o.trim().to_string()),
            block: true,
        },
        "read" => Inline {
            icon: "→",
            title: format!("Read {}", path()),
            description: Some(primitive_summary(input, &["path"])).filter(|d| !d.is_empty()),
            body: None,
            block: false,
        },
        "write" => Inline {
            icon: "←",
            title: format!("Write {}", path()),
            description: None,
            body: output.map(str::to_string),
            block: true,
        },
        "edit" => {
            let patch = metadata
                .get("files")
                .and_then(Value::as_array)
                .and_then(|files| files.first())
                .and_then(|file| file.get("patch"))
                .or_else(|| metadata.get("diff"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Inline {
                icon: "←",
                title: format!("Edit {}", path()),
                description: None,
                body: patch,
                block: true,
            }
        }
        "glob" => search("✱", "Glob", input, metadata, "count", directory),
        "grep" => search("✱", "Grep", input, metadata, "matches", directory),
        "webfetch" => {
            let url = text(input, "url");
            inline(
                "%",
                if url.is_empty() {
                    "WebFetch".into()
                } else {
                    format!("WebFetch {url}")
                },
            )
        }
        "websearch" => {
            let query = text(input, "query");
            let label = "Web Search via Anysola";
            inline(
                "◈",
                if query.is_empty() {
                    label.into()
                } else {
                    format!("{label} \"{query}\"")
                },
            )
        }
        "skill" => {
            let name = metadata
                .get("name")
                .map(js)
                .unwrap_or_else(|| text(input, "id"));
            inline("→", format!("Skill \"{name}\""))
        }
        "question" => {
            let total = input
                .get("questions")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            inline(
                "→",
                format!(
                    "Asked {total} question{}",
                    if total == 1 { "" } else { "s" }
                ),
            )
        }
        _ => fallback(name, input),
    }
}

fn fallback(name: &str, input: &Map<String, Value>) -> Inline {
    let title = if input.is_empty() {
        "Unknown".to_string()
    } else {
        crate::json(&Value::Object(input.clone()))
    };
    inline("⚙", format!("{name} {title}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn titles() {
        let dir = Path::new("/p");
        let read = inline_info(
            "read",
            &map(json!({"path": "a.txt", "extra": 1})),
            &Map::new(),
            None,
            dir,
        );
        assert_eq!(
            (read.title.as_str(), read.description.as_deref()),
            ("Read a.txt", Some("[extra=1]"))
        );
        let read = inline_info(
            "read",
            &map(json!({"path": "/p/sub/../b.txt"})),
            &Map::new(),
            None,
            dir,
        );
        assert_eq!(read.title, "Read b.txt");
        let glob = inline_info(
            "glob",
            &map(json!({"pattern": "x", "path": "/q"})),
            &map(json!({"count": 1})),
            None,
            dir,
        );
        assert_eq!(glob.description.as_deref(), Some("in /q · 1 match"));
        let missing = inline_info("read", &Map::new(), &Map::new(), None, dir);
        assert_eq!(missing.title, "Read ");
        let other = inline_info("read", &map(json!({"path": 1})), &Map::new(), None, dir);
        assert_eq!(other.title, "read {\"path\":1}");
        let glob = inline_info(
            "glob",
            &map(json!({"pattern": ["x"]})),
            &Map::new(),
            None,
            dir,
        );
        assert_eq!(glob.title, "Glob \"x\"");
    }
}
