//! Tool calls as ACP sees them: kinds, titles, locations, contents and diffs.

use std::path::Path;

use anysola_core::tool::ToolContent;
use serde_json::{Map, Value, json};

/// A tool call the turn follows from input start to its result.
#[derive(Debug, Clone)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// V1 names stored by migrated sessions.
pub fn canonical(name: &str) -> String {
    let name = name.to_lowercase();
    match name.as_str() {
        "bash" => "shell".into(),
        "task" => "subagent".into(),
        "apply_patch" => "patch".into(),
        _ => name,
    }
}

pub fn kind(name: &str) -> &'static str {
    match canonical(name).as_str() {
        "shell" => "execute",
        "webfetch" => "fetch",
        "edit" | "patch" | "write" => "edit",
        "grep"
        | "glob"
        | "context"
        | "context7_resolve_library_id"
        | "context7_get_library_docs" => "search",
        "read" => "read",
        "subagent" => "think",
        _ => "other",
    }
}

fn text(input: &Value, key: &str) -> Option<String> {
    input.get(key).and_then(Value::as_str).map(str::to_string)
}

/// `path`, or the V1 `filePath` of migrated sessions.
pub fn file_path(input: &Value) -> Option<String> {
    text(input, "path").or_else(|| text(input, "filePath"))
}

pub fn absolute(path: &str, cwd: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        Path::new(cwd).join(path).display().to_string()
    }
}

/// The files a patch's `*** Add/Update/Delete File:` and `*** Move to:` lines name.
fn patch_paths(input: &Value) -> Vec<String> {
    let Some(patch) = text(input, "patchText") else {
        return Vec::new();
    };
    patch
        .lines()
        .filter_map(|line| {
            [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
            ]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))
        })
        .map(|path| path.trim().to_string())
        .collect()
}

fn locations_of(paths: Vec<Option<String>>, cwd: &str) -> Value {
    let mut seen: Vec<String> = Vec::new();
    for path in paths.into_iter().flatten().filter(|p| !p.is_empty()) {
        let path = absolute(&path, cwd);
        if !seen.contains(&path) {
            seen.push(path);
        }
    }
    Value::Array(seen.into_iter().map(|path| json!({"path": path})).collect())
}

pub fn locations(name: &str, input: &Value, cwd: &str) -> Value {
    let paths = match canonical(name).as_str() {
        "shell" => vec![
            text(input, "workdir")
                .or_else(|| text(input, "cwd"))
                .or_else(|| Some(cwd.to_string())),
        ],
        "read" | "edit" | "write" => vec![file_path(input)],
        "patch" => patch_paths(input).into_iter().map(Some).collect(),
        "external_directory" => vec![text(input, "filepath")],
        "grep"
        | "glob"
        | "context"
        | "context7_resolve_library_id"
        | "context7_get_library_docs" => {
            vec![text(input, "path")]
        }
        _ => Vec::new(),
    };
    locations_of(paths, cwd)
}

pub fn title(name: &str, input: &Value, fallback: Option<&str>) -> String {
    if canonical(name) == "shell"
        && let Some(command) = text(input, "command").or_else(|| text(input, "cmd"))
    {
        return command;
    }
    fallback
        .filter(|f| !f.is_empty())
        .unwrap_or(name)
        .to_string()
}

/// A shell call's input names its working directory.
pub fn raw_input(name: &str, input: &Value, cwd: &str) -> Value {
    let mut input = input.clone();
    if canonical(name) == "shell"
        && input.get("cwd").is_none_or(Value::is_null)
        && input.get("workdir").is_none_or(Value::is_null)
        && let Some(map) = input.as_object_mut()
    {
        map.insert("cwd".into(), json!(cwd));
    }
    input
}

pub fn pending(tool: &Tool, cwd: &str, fallback_title: Option<&str>) -> Value {
    json!({
        "toolCallId": tool.id,
        "name": tool.name,
        "title": title(&tool.name, &tool.input, fallback_title),
        "kind": kind(&tool.name),
        "status": "pending",
        "locations": locations(&tool.name, &tool.input, cwd),
        "rawInput": raw_input(&tool.name, &tool.input, cwd),
    })
}

pub fn running(tool: &Tool, cwd: &str) -> Value {
    json!({
        "toolCallId": tool.id,
        "status": "in_progress",
        "kind": kind(&tool.name),
        "title": title(&tool.name, &tool.input, None),
        "locations": locations(&tool.name, &tool.input, cwd),
        "rawInput": raw_input(&tool.name, &tool.input, cwd),
    })
}

fn contents(content: &[ToolContent]) -> Vec<Value> {
    content
        .iter()
        .filter_map(|part| match part {
            ToolContent::Text { text } => {
                Some(json!({"type": "content", "content": {"type": "text", "text": text}}))
            }
            ToolContent::File { uri, .. } => {
                let (head, data) = uri.strip_prefix("data:")?.split_once(',')?;
                let mime = head.split(';').next()?;
                (mime.starts_with("image/") && head.ends_with(";base64")).then(|| {
                    json!({"type": "content", "content": {"type": "image", "mimeType": mime, "data": data}})
                })
            }
        })
        .collect()
}

pub fn completed(
    tool: &Tool,
    content: &[ToolContent],
    metadata: Option<&Map<String, Value>>,
    cwd: &str,
) -> Value {
    let all = contents(content);
    let (images, mut primary): (Vec<Value>, Vec<Value>) = all
        .into_iter()
        .partition(|part| part["content"]["type"] == "image");
    primary.extend(images);
    primary.extend(diffs(metadata, cwd));
    let mut raw_output = Map::new();
    if let Some(metadata) = metadata {
        raw_output.insert("metadata".into(), Value::Object(metadata.clone()));
    }
    json!({
        "toolCallId": tool.id,
        "status": "completed",
        "locations": locations(&tool.name, &tool.input, cwd),
        "content": primary,
        "rawOutput": raw_output,
    })
}

pub fn failed(
    tool: &Tool,
    content: &[ToolContent],
    metadata: Option<&Map<String, Value>>,
    error: &str,
    cwd: &str,
) -> Value {
    let mut parts = contents(content);
    parts.push(json!({"type": "content", "content": {"type": "text", "text": error}}));
    let mut raw_output = Map::new();
    if let Some(metadata) = metadata {
        raw_output.insert("metadata".into(), Value::Object(metadata.clone()));
    }
    raw_output.insert("error".into(), json!(error));
    json!({
        "toolCallId": tool.id,
        "status": "failed",
        "kind": kind(&tool.name),
        "title": title(&tool.name, &tool.input, None),
        "locations": locations(&tool.name, &tool.input, cwd),
        "rawInput": raw_input(&tool.name, &tool.input, cwd),
        "content": parts,
        "rawOutput": raw_output,
    })
}

/// The diffs of the files an edit changed (`metadata.files`), rebuilt from the file on disk and
/// the recorded patch.
fn diffs(metadata: Option<&Map<String, Value>>, cwd: &str) -> Vec<Value> {
    let files = metadata
        .and_then(|m| m.get("files"))
        .and_then(Value::as_array);
    files
        .into_iter()
        .flatten()
        .filter_map(|file| {
            let path = absolute(file["file"].as_str()?, cwd);
            let status = file["status"].as_str().unwrap_or("modified");
            let new = if status == "deleted" {
                String::new()
            } else {
                std::fs::read_to_string(&path).ok()?
            };
            let old = match status {
                "added" => Value::Null,
                _ => Value::String(apply(&new, file["patch"].as_str()?, true)?),
            };
            Some(json!({"type": "diff", "path": path, "oldText": old, "newText": new}))
        })
        .collect()
}

/// Previews of the files an edit asks to change (`files` of the ask), from the files as they
/// are now.
pub fn previews(metadata: &Value, cwd: &str) -> Vec<Value> {
    metadata["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|file| {
            let path = absolute(file["file"].as_str()?, cwd);
            let status = file["status"].as_str().unwrap_or("modified");
            let old = if status == "added" {
                String::new()
            } else {
                std::fs::read_to_string(&path).ok()?
            };
            let new = if status == "deleted" {
                String::new()
            } else {
                apply(&old, file["patch"].as_str()?, false)?
            };
            let old = if status == "added" {
                Value::Null
            } else {
                Value::String(old)
            };
            Some(json!({"type": "diff", "path": path, "oldText": old, "newText": new}))
        })
        .collect()
}

/// Applies a unified diff (or undoes it with `reverse`); `None` when a hunk does not fit.
pub fn apply(text: &str, patch: &str, reverse: bool) -> Option<String> {
    let source: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut at = 0usize;
    let mut lines = patch.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(header) = line.strip_prefix("@@ ") else {
            continue;
        };
        let mut ranges = header.split_whitespace();
        let (old, new) = (ranges.next()?, ranges.next()?);
        let start = if reverse { new } else { old };
        let start: usize = start[1..].split(',').next()?.parse().ok()?;
        let start = start.saturating_sub(1).max(at);
        out.extend(source.get(at..start)?.iter().map(|s| s.to_string()));
        at = start;
        let (take, give) = if reverse { ('+', '-') } else { ('-', '+') };
        while let Some(body) = lines.peek() {
            if body.starts_with("@@ ") {
                break;
            }
            let body = lines.next()?;
            let (mark, rest) = body.split_at(body.len().min(1));
            if body.starts_with('\\') {
                // "\ No newline at end of file" applies to the line just written.
                if let Some(last) = out.last_mut()
                    && last.ends_with('\n')
                {
                    last.pop();
                }
                continue;
            }
            let mark = mark.chars().next().unwrap_or(' ');
            if mark == ' ' || mark == take {
                let current = source.get(at)?;
                if current.trim_end_matches('\n') != rest {
                    return None;
                }
                at += 1;
                if mark == ' ' {
                    out.push(current.to_string());
                }
            } else if mark == give {
                out.push(format!("{rest}\n"));
            }
        }
    }
    out.extend(source.get(at..)?.iter().map(|s| s.to_string()));
    Some(out.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_apply_both_ways() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let patch = "Index: f\n===\n--- f\n+++ f\n@@ -1,3 +1,4 @@\n a\n-b\n+B\n c\n+d\n";
        assert_eq!(apply(old, patch, false).as_deref(), Some(new));
        assert_eq!(apply(new, patch, true).as_deref(), Some(old));
        assert_eq!(apply("x\n", patch, false), None);
    }

    #[test]
    fn kinds_and_locations() {
        assert_eq!(kind("bash"), "execute");
        assert_eq!(kind("websearch"), "other");
        assert_eq!(
            locations("read", &json!({"path": "src/a.rs"}), "/p"),
            json!([{"path": "/p/src/a.rs"}])
        );
        assert_eq!(
            raw_input("shell", &json!({"command": "ls"}), "/p"),
            json!({"command": "ls", "cwd": "/p"})
        );
        assert_eq!(title("shell", &json!({"command": "ls"}), None), "ls");
    }
}
