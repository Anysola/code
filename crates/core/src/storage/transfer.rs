//! Session listing, deletion and the `session export` / `session import` transfer format:
//! `{info, messages}` with 0.5.1's `Session.Info` and message shapes.

use rusqlite::{OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::{Db, Result, json as compact, parse};
use crate::time::now_ms;

/// One row of `session list`.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub project_id: String,
    pub directory: String,
    pub time_created: u64,
    pub time_updated: u64,
}

/// Why an import was refused.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("the session already exists")]
    Conflict,
    #[error("the parent session does not exist")]
    MissingParent,
    #[error(transparent)]
    Storage(#[from] super::StorageError),
}

/// A JSON number as `JSON.stringify` writes it: integral values without a fraction.
fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// Every number in `value` as `JSON.stringify` writes it.
fn js_numbers(value: &mut Value) {
    match value {
        Value::Number(n) if n.is_f64() => *value = number(n.as_f64().unwrap_or(0.0)),
        Value::Array(items) => items.iter_mut().for_each(js_numbers),
        Value::Object(map) => map.values_mut().for_each(js_numbers),
        _ => {}
    }
}

/// Assistant rows count once completed; shells and compactions once no longer running.
fn settled(kind: &str, data: &Value) -> bool {
    match kind {
        "assistant" => data["time"].get("completed").is_some(),
        "shell" | "compaction" => data["status"] != "running",
        _ => true,
    }
}

/// A stored row as its message object: `id` and `type` back at the places the 0.5.1 schema puts
/// them (after the shared `id`/`metadata`/`time` fields; last for user and synthetic rows; first
/// for compactions).
fn message(id: &str, kind: &str, data: &Value) -> Value {
    let mut out = Map::new();
    if kind == "compaction" {
        out.insert("type".into(), json!(kind));
    }
    out.insert("id".into(), json!(id));
    let after_time = !matches!(kind, "user" | "synthetic" | "compaction");
    for (key, value) in data.as_object().into_iter().flatten() {
        if key == "id" || key == "type" {
            continue;
        }
        out.insert(key.clone(), value.clone());
        if after_time && key == "time" {
            out.insert("type".into(), json!(kind));
        }
    }
    out.entry("type").or_insert_with(|| json!(kind));
    Value::Object(out)
}

fn redact(kind: &str, id: &str, value: &Value) -> Value {
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => json!(format!("[redacted:{kind}:{id}]")),
        _ => value.clone(),
    }
}

/// Replaces `key` in `object` when present.
fn replace(object: &mut Value, key: &str, f: impl FnOnce(&Value) -> Value) {
    if let Some(slot) = object.get_mut(key) {
        *slot = f(slot);
    }
}

/// A non-empty object under `key` becomes `{"redacted": "<kind>:<id>"}`.
fn hide(object: &mut Value, key: &str, kind: &str, id: &str) {
    replace(object, key, |value| match value.as_object() {
        Some(map) if !map.is_empty() => json!({"redacted": format!("{kind}:{id}")}),
        _ => value.clone(),
    });
}

fn redact_mention(item: &mut Value, kind: &str, index: &str) {
    if let Some(mention) = item.get_mut("mention").filter(|m| m.is_object()) {
        replace(mention, "text", |v| redact(kind, index, v));
    }
}

fn each(object: &mut Value, key: &str, mut f: impl FnMut(&mut Value, String)) {
    if let Some(Value::Array(items)) = object.get_mut(key) {
        for (index, item) in items.iter_mut().enumerate() {
            f(item, index.to_string());
        }
    }
}

fn sanitize_tool_content(id: &str, content: &mut Value) {
    if content["type"] == "text" {
        replace(content, "text", |v| redact("tool-output", id, v));
    } else {
        replace(content, "uri", |v| redact("tool-file-uri", id, v));
        replace(content, "name", |v| redact("tool-file-name", id, v));
    }
}

fn sanitize_tool_state(id: &str, state: &mut Value) {
    let hidden = |kind: &str| json!({"redacted": format!("{kind}:{id}")});
    match state["status"].as_str() {
        Some("streaming") => replace(state, "input", |v| redact("tool-input", id, v)),
        Some("running") => {
            state["input"] = hidden("tool-input");
            state["metadata"] = hidden("tool-metadata");
        }
        _ => {
            state["input"] = hidden("tool-input");
            each(state, "content", |item, _| sanitize_tool_content(id, item));
            replace(state, "metadata", |_| hidden("tool-metadata"));
        }
    }
}

fn sanitize_message(message: &mut Value) {
    let id = message["id"].as_str().unwrap_or_default().to_string();
    let id = id.as_str();
    hide(message, "metadata", "message-metadata", id);
    let kind = message["type"].as_str().unwrap_or_default().to_string();
    match kind.as_str() {
        "user" => {
            replace(message, "text", |v| redact("text", id, v));
            each(message, "files", |file, index| {
                file["data"] = json!("");
                file["source"] = json!({"type": "inline"});
                replace(file, "name", |v| redact("file-name", &index, v));
                replace(file, "description", |v| {
                    redact("file-description", &index, v)
                });
                redact_mention(file, "file-mention", &index);
            });
            each(message, "agents", |agent, index| {
                replace(agent, "name", |v| redact("agent-name", &index, v));
                redact_mention(agent, "agent-mention", &index);
            });
            each(message, "skills", |skill, index| {
                replace(skill, "name", |v| redact("skill-name", &index, v));
                replace(skill, "text", |v| redact("skill", &index, v));
                redact_mention(skill, "skill-mention", &index);
            });
        }
        "synthetic" => {
            replace(message, "text", |v| redact("synthetic", id, v));
            replace(message, "description", |v| {
                redact("synthetic-description", id, v)
            });
        }
        "system" | "skill" => replace(message, "text", |v| redact(&kind, id, v)),
        "shell" => {
            replace(message, "command", |v| redact("shell-command", id, v));
            if let Some(output) = message.get_mut("output").filter(|o| o.is_object()) {
                replace(output, "output", |v| redact("shell-output", id, v));
            }
        }
        "assistant" => each(message, "content", |content, _| {
            let kind = content["type"].as_str().unwrap_or_default().to_string();
            if kind == "text" || kind == "reasoning" {
                replace(content, "text", |v| redact(&kind, id, v));
            } else if let Some(state) = content.get_mut("state") {
                sanitize_tool_state(id, state);
            }
            let native = if kind == "text" || kind == "reasoning" {
                kind.as_str()
            } else {
                "tool"
            };
            replace(
                content,
                "native",
                |_| json!({"redacted": format!("{native}-native:{id}")}),
            );
        }),
        "compaction" if message["status"] != "failed" => {
            replace(message, "summary", |v| redact("compaction-summary", id, v));
            if message["status"] == "completed" {
                hide(message, "native", "compaction-native", id);
            }
        }
        _ => {}
    }
}

/// `session export --sanitize`: transcript text, paths and file data replaced by
/// `[redacted:<kind>:<id>]` markers; the shape stays importable.
pub fn sanitize(data: &mut Value) {
    let info = &mut data["info"];
    let id = info["id"].as_str().unwrap_or_default().to_string();
    replace(info, "title", |v| redact("session-title", &id, v));
    hide(info, "metadata", "session-metadata", &id);
    if let Some(location) = info.get_mut("location") {
        replace(location, "directory", |v| {
            json!(format!(
                "/{}",
                redact("session-directory", &id, v)
                    .as_str()
                    .unwrap_or_default()
            ))
        });
    }
    if let Some(revert) = info.get_mut("revert").filter(|r| r.is_object()) {
        each(revert, "files", |file, index| {
            replace(file, "file", |v| redact("revert-file", &index, v));
            replace(file, "patch", |v| redact("revert-patch", &index, v));
        });
    }
    each(data, "messages", |message, _| sanitize_message(message));
}

impl Db {
    /// Top-level sessions of a project, newest first.
    pub fn root_sessions(&self, project_id: &str, limit: u64) -> Result<Vec<SessionSummary>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT id, title, project_id, directory, time_created, time_updated FROM session
                 WHERE project_id = ?1 AND parent_id IS NULL AND time_archived IS NULL
                 ORDER BY time_updated DESC, id DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![project_id, limit as i64], |r| {
                Ok(SessionSummary {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    project_id: r.get(2)?,
                    directory: r.get(3)?,
                    time_created: r.get::<_, i64>(4)? as u64,
                    time_updated: r.get::<_, i64>(5)? as u64,
                })
            })?;
            rows.collect()
        })
    }

    /// Top-level sessions newest first: of one project or of all, whose title contains `query`.
    pub fn search_sessions(
        &self,
        project_id: Option<&str>,
        query: &str,
        limit: u64,
    ) -> Result<Vec<SessionSummary>> {
        self.with(|c| {
            let mut statement = c.prepare(
                "SELECT id, title, project_id, directory, time_created, time_updated FROM session
                 WHERE (?1 IS NULL OR project_id = ?1) AND parent_id IS NULL AND time_archived IS NULL
                   AND instr(lower(title), lower(?2)) > 0
                 ORDER BY time_updated DESC, id DESC LIMIT ?3",
            )?;
            let rows = statement.query_map(params![project_id, query, limit as i64], |r| {
                Ok(SessionSummary {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    project_id: r.get(2)?,
                    directory: r.get(3)?,
                    time_created: r.get::<_, i64>(4)? as u64,
                    time_updated: r.get::<_, i64>(5)? as u64,
                })
            })?;
            rows.collect()
        })
    }

    /// Deletes a session and its descendants; false when it does not exist.
    pub fn delete_session(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            let tx = c.transaction()?;
            let exists = tx
                .query_row("SELECT 1 FROM session WHERE id = ?1", [id], |_| Ok(()))
                .optional()?
                .is_some();
            if exists {
                tx.execute(
                    "WITH RECURSIVE tree(id) AS (
                        SELECT ?1 UNION ALL SELECT session.id FROM session JOIN tree ON session.parent_id = tree.id)
                     DELETE FROM kv WHERE key IN (SELECT ?2 || id FROM tree)",
                    rusqlite::params![id, super::GOAL_PREFIX],
                )?;
                tx.execute(
                    "WITH RECURSIVE tree(id) AS (
                        SELECT ?1 UNION ALL SELECT session.id FROM session JOIN tree ON session.parent_id = tree.id)
                     DELETE FROM session WHERE id IN (SELECT id FROM tree)",
                    [id],
                )?;
            }
            tx.commit()?;
            Ok(exists)
        })
    }

    /// `{info, messages}` of a session, settled messages only.
    pub fn export_session(&self, id: &str) -> Result<Option<Value>> {
        let info = self.with(|c| {
            c.query_row(
                "SELECT id, parent_id, fork_session_id, fork_boundary, project_id, agent, model, cost,
                        tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write,
                        idle_outcome, time_created, time_updated, time_busy, time_idle, time_viewed, time_archived,
                        title, directory, path, metadata, permission, revert
                 FROM session WHERE id = ?1",
                [id],
                |r| {
                    let mut info = Map::new();
                    info.insert("id".into(), json!(r.get::<_, String>(0)?));
                    if let Some(parent) = r.get::<_, Option<String>>(1)? {
                        info.insert("parentID".into(), json!(parent));
                    }
                    if let (Some(source), Some(boundary)) =
                        (r.get::<_, Option<String>>(2)?, parse(r.get(3)?))
                    {
                        info.insert(
                            "fork".into(),
                            json!({"sessionID": source, "boundary": boundary}),
                        );
                    }
                    info.insert("projectID".into(), json!(r.get::<_, String>(4)?));
                    if let Some(agent) = r.get::<_, Option<String>>(5)? {
                        info.insert("agent".into(), json!(agent));
                    }
                    if let Some(model) = parse(r.get(6)?) {
                        info.insert("model".into(), model);
                    }
                    info.insert("cost".into(), number(r.get(7)?));
                    let count = |i| r.get::<_, i64>(i);
                    info.insert(
                        "tokens".into(),
                        json!({"input": count(8)?, "output": count(9)?, "reasoning": count(10)?,
                               "cache": {"read": count(11)?, "write": count(12)?}}),
                    );
                    if let Some(outcome) = r.get::<_, Option<String>>(13)? {
                        info.insert("outcome".into(), json!(outcome));
                    }
                    let mut time = Map::new();
                    for (index, key) in ["created", "updated", "busy", "idle", "viewed", "archived"]
                        .into_iter()
                        .enumerate()
                    {
                        if let Some(at) = r.get::<_, Option<i64>>(14 + index)? {
                            time.insert(key.into(), json!(at));
                        }
                    }
                    info.insert("time".into(), Value::Object(time));
                    info.insert("title".into(), json!(r.get::<_, String>(20)?));
                    info.insert(
                        "location".into(),
                        json!({"directory": r.get::<_, String>(21)?}),
                    );
                    if let Some(subpath) = r.get::<_, Option<String>>(22)?.filter(|p| !p.is_empty())
                    {
                        info.insert("subpath".into(), json!(subpath));
                    }
                    for (index, key) in [(23, "metadata"), (24, "permissions"), (25, "revert")] {
                        if let Some(value) = parse(r.get(index)?) {
                            info.insert(key.into(), value);
                        }
                    }
                    Ok(Value::Object(info))
                },
            )
            .optional()
        })?;
        let Some(info) = info else {
            return Ok(None);
        };
        let messages: Vec<Value> = self
            .messages(id)?
            .iter()
            .filter(|row| settled(&row.kind, &row.data))
            .map(|row| message(&row.id, &row.kind, &row.data))
            .collect();
        let mut export = json!({"info": info, "messages": messages});
        js_numbers(&mut export);
        Ok(Some(export))
    }

    /// Imports `{info, messages}` into `project`/`directory`; returns the session id. The caller
    /// validated the shape.
    pub fn import_session(
        &self,
        data: &Value,
        project: &crate::project::Project,
        directory: &str,
    ) -> std::result::Result<String, ImportError> {
        let info = &data["info"];
        let id = info["id"].as_str().unwrap_or_default().to_string();
        if self.session(&id)?.is_some() {
            return Err(ImportError::Conflict);
        }
        if let Some(parent) = info["parentID"].as_str()
            && self.session(parent)?.is_none()
        {
            return Err(ImportError::MissingParent);
        }
        self.upsert_project(project)?;
        let subpath = std::path::Path::new(directory)
            .strip_prefix(&project.worktree)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let time = &info["time"];
        let at = |key: &str| time[key].as_i64();
        let idle = at("idle");
        let busy = idle.zip(at("busy")).map(|(i, b)| i.min(b));
        let viewed = idle.zip(at("viewed")).map(|(i, v)| i.min(v));
        let outcome = idle.and(info["outcome"].as_str());
        let tokens = &info["tokens"];
        let token = |v: &Value| v.as_i64().unwrap_or(0);
        let optional = |key: &str| info.get(key).filter(|v| !v.is_null()).map(compact);
        let messages: Vec<&Value> = data["messages"]
            .as_array()
            .map(|m| {
                m.iter()
                    .filter(|m| settled(m["type"].as_str().unwrap_or_default(), m))
                    .collect()
            })
            .unwrap_or_default();
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO session (id, project_id, parent_id, slug, directory, path, title, version, metadata,
                    cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write,
                    permission, agent, model, time_created, time_updated, time_busy, time_idle, idle_outcome,
                    time_viewed, time_archived)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                    ?21, ?22, ?23, ?24, ?25)",
                params![
                    id,
                    project.id,
                    info["parentID"].as_str(),
                    crate::slug::create(),
                    directory,
                    subpath,
                    info["title"].as_str().unwrap_or_default(),
                    crate::version::session_version(),
                    optional("metadata"),
                    info["cost"].as_f64().unwrap_or(0.0),
                    token(&tokens["input"]),
                    token(&tokens["output"]),
                    token(&tokens["reasoning"]),
                    token(&tokens["cache"]["read"]),
                    token(&tokens["cache"]["write"]),
                    optional("permissions"),
                    info["agent"].as_str(),
                    optional("model"),
                    at("created").unwrap_or(0),
                    now_ms() as i64,
                    busy,
                    idle,
                    outcome,
                    viewed,
                    idle.and(at("archived")),
                ],
            )?;
            for (index, message) in messages.iter().enumerate() {
                let mut fields = message.as_object().cloned().unwrap_or_default();
                let message_id = fields.shift_remove("id").unwrap_or_default();
                let kind = fields.shift_remove("type").unwrap_or_default();
                let created = message["time"]["created"].as_i64().unwrap_or(0);
                tx.execute(
                    "INSERT INTO message (id, session_id, seq, type, time_created, time_updated, data)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
                    params![
                        message_id.as_str(),
                        id,
                        index as i64 + 1,
                        kind.as_str(),
                        created,
                        compact(&Value::Object(fields))
                    ],
                )?;
            }
            tx.commit()
        })?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_fields_in_schema_order() {
        let keys = |v: Value| {
            v.as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        };
        let user = json!({"time": {"created": 1}, "text": "hi", "files": []});
        assert_eq!(keys(message("m", "user", &user)), "id,time,text,files,type");
        let assistant = json!({"time": {"created": 1}, "agent": "build", "content": []});
        assert_eq!(
            keys(message("m", "assistant", &assistant)),
            "id,time,type,agent,content"
        );
        let compaction = json!({"time": {"created": 1}, "status": "completed"});
        assert_eq!(
            keys(message("m", "compaction", &compaction)),
            "type,id,time,status"
        );
    }

    #[test]
    fn integral_costs_print_without_a_fraction() {
        assert_eq!(number(0.0).to_string(), "0");
        assert_eq!(number(0.25).to_string(), "0.25");
    }
}
