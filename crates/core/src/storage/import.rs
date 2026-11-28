//! `session import`: a `{info, messages}` export (see `transfer.rs`) taken back in as a new root
//! session. The file is untrusted: it may come from anyone. So it is size-checked before it is
//! parsed, every row is rebuilt from the fields this version reads (nothing else is kept), and
//! nothing in it carries authority here:
//!
//! - The session gets a fresh id, each message a fresh id and each tool call a fresh call id;
//!   the only reference between rows (a compaction's `keepID`) is remapped or dropped. Importing
//!   a file twice gives two sessions.
//! - Session-level state is not taken: permissions, metadata (permission floor, granted folders),
//!   parent, fork, revert, project, worktree, goal, budget, agent, model, cost. The session is a
//!   root session of the folder the caller names, as a new session would be.
//! - Rows that the session loop acts on are not taken: `system` rows (instruction updates, sent
//!   with the system role), `location-switched` and `model-switched` rows, unsettled steps, running compactions, and
//!   kinds this version does not know. Tool calls that were still running become interrupted.
//! - Nothing that points at this machine's state is kept: step snapshots (so revert and fork
//!   have nothing to restore and touch no file), signed reasoning (`native`), subagent session
//!   ids, background shell ids, and every tool metadata key but the few a transcript shows.
//! - What a request would be refused for is fixed or dropped: tool names outside the API's
//!   alphabet, non-object tool inputs, media that is not valid base64, steps before the first
//!   user turn.

use std::io::Read;
use std::path::Path;

use base64::Engine;
use rusqlite::params;
use serde_json::{Map, Value, json};

use super::{Db, SessionRow, StorageError, json as compact};
use crate::gateway::error::SessionError;
use crate::session::message::{
    AgentSwitchedData, AssistantData, CompactionData, FileAttachment, InvokedSkill, Part,
    ShellData, ShellOutput, SkillData, SyntheticData, ToolState, UserData, encode,
};
use crate::tool::ToolContent;

/// The largest file taken: sessions with pasted images run to tens of MiB.
pub const MAX_FILE_BYTES: u64 = 64 << 20;
/// The most messages a file may hold.
pub const MAX_MESSAGES: usize = 20_000;
/// JSON values (objects, arrays, strings, members) a file may hold, counted before parsing: what
/// parsing allocates is bounded by this, not only by the file's size.
const MAX_NODES: usize = 2_000_000;
/// How deeply a file's JSON may nest.
const MAX_DEPTH: usize = 64;
/// Characters of the title kept.
const TITLE_CHARS: usize = 200;

/// Why an import was refused. Nothing was written.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("The file is larger than {} MiB, the most a session import takes.", MAX_FILE_BYTES >> 20)]
    TooLarge,
    #[error("This isn't a session export Anysola Code can import: {0}")]
    Invalid(String),
    #[error("Can’t read the file: {0}")]
    Read(String),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

fn invalid(message: impl Into<String>) -> ImportError {
    ImportError::Invalid(message.into())
}

/// The bytes of the file at `path`: a regular file (a symlink to one is followed) of at most
/// [`MAX_FILE_BYTES`], read no further than that.
pub fn read_file(path: &Path) -> Result<Vec<u8>, ImportError> {
    let read = |e: std::io::Error| ImportError::Read(e.to_string());
    let meta = std::fs::metadata(path).map_err(read)?;
    if !meta.is_file() {
        return Err(invalid("it is not a file."));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(read)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(read)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    Ok(bytes)
}

/// Counts the JSON values and the nesting of `bytes` without parsing them.
fn prescan(bytes: &[u8]) -> Result<(), ImportError> {
    within(bytes, MAX_NODES, MAX_DEPTH).map_err(invalid)
}

/// Whether `bytes` holds at most `max_nodes` JSON values (objects, arrays, strings, members)
/// nested at most `max_depth` deep, counted in one pass without parsing: what parsing it would
/// allocate is bounded by that. The error says which limit it passed.
pub(crate) fn within(bytes: &[u8], max_nodes: usize, max_depth: usize) -> Result<(), &'static str> {
    let (mut nodes, mut depth) = (0usize, 0usize);
    let (mut in_string, mut escaped) = (false, false);
    for &byte in bytes {
        if in_string {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                nodes += 1;
            }
            b'{' | b'[' => {
                depth += 1;
                nodes += 1;
                if depth > max_depth {
                    return Err("its JSON is nested too deeply.");
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            b',' => nodes += 1,
            _ => {}
        }
        if nodes > max_nodes {
            return Err("it holds too many values.");
        }
    }
    Ok(())
}

/// What an import stores: the title and the rows, in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub title: String,
    pub rows: Vec<Row>,
}

/// One message as it is stored: a fresh id, its kind and its rebuilt data.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub kind: String,
    pub data: Value,
}

fn decode<T: serde::de::DeserializeOwned>(
    value: Value,
    n: usize,
    kind: &str,
) -> Result<T, ImportError> {
    serde_json::from_value(value)
        .map_err(|e| invalid(format!("message {} ({kind}) is not valid: {e}", n + 1)))
}

fn base64_ok(data: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .is_ok()
}

/// A tool name the API takes.
fn tool_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `data` is base64 of a PDF.
fn is_pdf(data: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .is_ok_and(|bytes| bytes.starts_with(b"%PDF-"))
}

/// A prompt's attachments as the model may get them: images through the same checks a prompt's
/// images pass ([`crate::media::normalize`]: decodable, of a type the API takes, within size,
/// converted or resized as there), PDFs only when they are one; what fails is dropped. Other
/// types are kept (they are not sent).
fn files(files: Option<Vec<FileAttachment>>) -> Option<Vec<FileAttachment>> {
    let files = files?;
    let limits = crate::media::Limits::from_config(None);
    Some(
        files
            .into_iter()
            .filter_map(|f| {
                let mime = f.mime.to_ascii_lowercase();
                let (mime, data) = if mime.starts_with("image/") {
                    match crate::media::normalize(&f.mime, &f.data, &limits) {
                        Ok(None) => (f.mime, f.data),
                        Ok(Some(normalized)) => normalized,
                        Err(_) => return None,
                    }
                } else if mime == "application/pdf" {
                    if !is_pdf(&f.data) {
                        return None;
                    }
                    (f.mime, f.data)
                } else if mime.is_empty() || !base64_ok(&f.data) {
                    return None;
                } else {
                    (f.mime, f.data)
                };
                Some(FileAttachment {
                    data,
                    mime,
                    source: json!({"type": "inline"}),
                    name: f.name,
                    extra: Map::new(),
                })
            })
            .collect(),
    )
}

fn user(data: UserData) -> UserData {
    let mut extra = Map::new();
    if let Some(skills) = data.extra.get("skills")
        && let Ok(skills) = serde_json::from_value::<Vec<InvokedSkill>>(skills.clone())
        && !skills.is_empty()
    {
        extra.insert("skills".into(), json!(skills));
    }
    if let Some(Value::String(command)) = data.extra.get("command") {
        extra.insert("command".into(), json!(command));
    }
    UserData {
        time: data.time,
        text: data.text,
        files: files(data.files),
        extra,
    }
}

/// A tool result's content as the model may get it: images as a tool's own images are
/// ([`crate::media::normalize_content`]: what can't be sent is dropped with a note), PDFs only
/// when they are one, any other file as a note (the request would carry it as an image).
fn content(items: Vec<ToolContent>) -> Vec<ToolContent> {
    let items: Vec<ToolContent> = items
        .into_iter()
        .map(|item| match item {
            ToolContent::File { uri, mime, name } => {
                let data = uri
                    .strip_prefix("data:")
                    .and_then(|rest| rest.split_once(";base64,"))
                    .map(|(_, data)| data.to_string());
                let image = mime.to_ascii_lowercase().starts_with("image/");
                let pdf = mime.eq_ignore_ascii_case("application/pdf");
                match data {
                    Some(data) if image && base64_ok(&data) => {
                        ToolContent::File { uri, mime, name }
                    }
                    Some(data) if pdf && is_pdf(&data) => ToolContent::File { uri, mime, name },
                    _ => ToolContent::text(format!("[{mime} omitted]")),
                }
            }
            text => text,
        })
        .collect();
    crate::media::normalize_content(items, &crate::media::Limits::from_config(None))
}

/// The tool metadata a transcript shows: the changed files' diffs and a few flags. Everything
/// else (subagent session ids, shell ids, hook entries, instruction paths…) is dropped.
fn tool_metadata(metadata: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(Value::Array(list)) = metadata.get("files") {
        let files: Vec<Value> = list
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|file| {
                let mut kept = Map::new();
                for key in ["file", "patch", "status"] {
                    if let Some(Value::String(text)) = file.get(key) {
                        kept.insert(key.into(), json!(text));
                    }
                }
                for key in ["additions", "deletions"] {
                    if let Some(n) = file.get(key).and_then(Value::as_u64) {
                        kept.insert(key.into(), json!(n));
                    }
                }
                kept.contains_key("file").then_some(Value::Object(kept))
            })
            .collect();
        if !files.is_empty() {
            out.insert("files".into(), Value::Array(files));
        }
    }
    for key in ["truncated", "timedOut"] {
        if let Some(Value::Bool(flag)) = metadata.get(key) {
            out.insert(key.into(), json!(flag));
        }
    }
    if let Some(exit) = metadata.get("exit").filter(|e| e.is_i64() || e.is_null()) {
        out.insert("exit".into(), exit.clone());
    }
    out
}

fn object(input: Value) -> Value {
    if input.is_object() { input } else { json!({}) }
}

fn interrupted(name: &str) -> SessionError {
    SessionError::new("aborted", format!("Tool execution interrupted: {name}"))
}

fn tool_state(name: &str, state: ToolState) -> ToolState {
    match state {
        ToolState::Streaming { .. } => ToolState::Error {
            input: json!({}),
            error: interrupted(name),
            content: Vec::new(),
            metadata: Map::new(),
        },
        ToolState::Running { input, .. } => ToolState::Error {
            input: object(input),
            error: interrupted(name),
            content: Vec::new(),
            metadata: Map::new(),
        },
        ToolState::Completed {
            input,
            content: items,
            metadata,
        } => ToolState::Completed {
            input: object(input),
            content: content(items),
            metadata: tool_metadata(metadata),
        },
        ToolState::Error {
            input,
            error,
            content: items,
            metadata,
        } => ToolState::Error {
            input: object(input),
            error: SessionError::new(&error.kind, error.message),
            content: content(items),
            metadata: tool_metadata(metadata),
        },
    }
}

fn assistant(data: AssistantData) -> AssistantData {
    let parts = data
        .content
        .into_iter()
        .filter_map(|part| match part {
            Part::Text { text, .. } => Some(Part::Text { text, native: None }),
            Part::Reasoning { text, time, .. } => Some(Part::Reasoning {
                text,
                native: None,
                time,
            }),
            Part::Tool {
                name,
                executed,
                state,
                time,
                ..
            } => tool_name_ok(&name).then(|| Part::Tool {
                id: format!("toolu_{}", crate::ids::part().trim_start_matches("prt_")),
                state: tool_state(&name, state),
                name,
                executed,
                native: None,
                time,
            }),
        })
        .collect();
    AssistantData {
        time: data.time,
        agent: data.agent,
        model: data.model,
        content: parts,
        snapshot: None,
        finish: data.finish,
        raw_finish: data.raw_finish,
        cost: None,
        tokens: data.tokens,
        error: data.error.map(|e| SessionError::new(&e.kind, e.message)),
        extra: Map::new(),
    }
}

fn shell(data: ShellData) -> ShellData {
    let metadata = data.metadata.as_ref().and_then(Value::as_object).map(|m| {
        let mut kept = Map::new();
        if let Some(Value::Bool(flag)) = m.get("background") {
            kept.insert("background".into(), json!(flag));
        }
        if let Some(Value::Bool(flag)) = m.get("timedOut") {
            kept.insert("timedOut".into(), json!(flag));
        }
        if let Some(exit) = m.get("exit").filter(|e| e.is_i64() || e.is_null()) {
            kept.insert("exit".into(), exit.clone());
        }
        Value::Object(kept)
    });
    ShellData {
        time: data.time,
        command: data.command,
        output: data.output.map(|o| ShellOutput { output: o.output }),
        metadata,
        extra: Map::new(),
    }
}

fn idle(data: &Map<String, Value>, n: usize) -> Result<Value, ImportError> {
    let created = data
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("message {} (idle) has no time.", n + 1)))?;
    let outcome = match data.get("outcome").and_then(Value::as_str) {
        Some(o @ ("succeeded" | "failed" | "interrupted")) => o,
        _ => return Err(invalid(format!("message {} (idle) has no outcome.", n + 1))),
    };
    let mut out = json!({"time": {"created": created}, "outcome": outcome});
    if let Some(reason @ ("user" | "budget" | "declined" | "closed")) =
        data.get("reason").and_then(Value::as_str)
    {
        out["reason"] = json!(reason);
    }
    if let Some(error) = data.get("error")
        && let Ok(error) = serde_json::from_value::<SessionError>(error.clone())
    {
        out["error"] = encode(&SessionError::new(&error.kind, error.message));
    }
    Ok(out)
}

/// Validates and rebuilds a session export's rows (see the module docs). Nothing is written.
pub fn prepare(bytes: &[u8]) -> Result<Prepared, ImportError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    prescan(bytes)?;
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|e| invalid(format!("it is not valid JSON ({e}).")))?;
    let Some(object) = value.as_object_mut() else {
        return Err(invalid(
            "expected an object with \"info\" and \"messages\".",
        ));
    };
    let info = match object.get("info") {
        Some(Value::Object(info)) => info.clone(),
        _ => return Err(invalid("\"info\" is missing.")),
    };
    let messages = match object.remove("messages") {
        Some(Value::Array(messages)) => messages,
        _ => return Err(invalid("\"messages\" is missing or not a list.")),
    };
    if messages.len() > MAX_MESSAGES {
        return Err(invalid(format!(
            "it has {} messages; at most {MAX_MESSAGES} can be imported.",
            messages.len()
        )));
    }
    let title: String = info
        .get("title")
        .and_then(Value::as_str)
        .map(|t| {
            t.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect::<String>()
                .trim()
                .chars()
                .take(TITLE_CHARS)
                .collect()
        })
        .filter(|t: &String| !t.is_empty())
        .unwrap_or_else(|| "Imported session".into());

    // Rows kept so far, with the file's id of each (for `keepID`).
    let mut rows: Vec<Row> = Vec::new();
    let mut ids: Vec<Option<String>> = Vec::new();
    let mut seen_user = false;
    for (n, message) in messages.into_iter().enumerate() {
        let Value::Object(mut fields) = message else {
            return Err(invalid(format!("message {} is not an object.", n + 1)));
        };
        let original = match fields.shift_remove("id") {
            Some(Value::String(id)) => Some(id),
            _ => None,
        };
        let kind = match fields.shift_remove("type") {
            Some(Value::String(kind)) => kind,
            _ => return Err(invalid(format!("message {} has no type.", n + 1))),
        };
        let data = Value::Object(fields);
        let (data, user_turn) = match kind.as_str() {
            "user" => (encode(&user(decode::<UserData>(data, n, &kind)?)), true),
            "synthetic" => {
                let d: SyntheticData = decode(data, n, &kind)?;
                let d = SyntheticData {
                    time: d.time,
                    text: d.text,
                    description: d.description,
                    extra: Map::new(),
                };
                (encode(&d), true)
            }
            "skill" => {
                let d: SkillData = decode(data, n, &kind)?;
                let d = SkillData {
                    time: d.time,
                    skill: d.skill,
                    name: d.name,
                    text: d.text,
                    extra: Map::new(),
                };
                (encode(&d), true)
            }
            "assistant" => {
                let d: AssistantData = decode(data, n, &kind)?;
                // Unsettled: never finished where it ran (an export leaves these out too). A
                // step before any user turn could not open a request.
                if d.time.completed.is_none() || !seen_user {
                    continue;
                }
                (encode(&assistant(d)), false)
            }
            "shell" => {
                let d = shell(decode::<ShellData>(data, n, &kind)?);
                let background = d.background();
                (encode(&d), !background)
            }
            "compaction" => {
                let d: CompactionData = decode(data, n, &kind)?;
                if d.status != "completed" && d.status != "failed" {
                    continue;
                }
                // `keepID` must name a row kept before this one: history reads the rows from
                // there up to the compaction.
                let keep = d.keep_id().and_then(|keep| {
                    ids.iter()
                        .position(|id| id.as_deref() == Some(keep))
                        .map(|at| rows[at].id.clone())
                });
                let completed = d.completed();
                let d = CompactionData {
                    time: d.time,
                    status: d.status,
                    reason: if d.reason == "manual" {
                        "manual"
                    } else {
                        "auto"
                    }
                    .into(),
                    model: d.model,
                    summary: d.summary,
                    native: None,
                    error: d.error.map(|e| SessionError::new(&e.kind, e.message)),
                    cost: None,
                    tokens: d.tokens,
                    metadata: keep.map(|id| json!({"keepID": id})),
                };
                (encode(&d), completed)
            }
            "agent-switched" => {
                let d: AgentSwitchedData = decode(data, n, &kind)?;
                let d = AgentSwitchedData {
                    time: d.time,
                    agent: d.agent,
                    previous: d.previous,
                    extra: Map::new(),
                };
                (encode(&d), false)
            }
            "idle" => match &data {
                Value::Object(map) => (idle(map, n)?, false),
                _ => continue,
            },
            // `system` (instruction updates, sent as system text), `location-switched` (a folder
            // of another machine), `model-switched` (its effort changes are sent as settings of
            // another session's model) and kinds this version does not know.
            _ => continue,
        };
        seen_user |= user_turn;
        rows.push(Row {
            id: crate::ids::message(),
            kind,
            data,
        });
        ids.push(original);
    }
    Ok(Prepared { title, rows })
}

impl Db {
    /// Imports a session export (`bytes`, see [`prepare`]) as a new root session of `project`
    /// in `directory`, in one transaction. Returns the new session.
    pub fn import_session(
        &self,
        bytes: &[u8],
        project: &crate::project::Project,
        directory: &str,
    ) -> Result<SessionRow, ImportError> {
        let prepared = prepare(bytes)?;
        self.upsert_project(project)?;
        let now = crate::time::now_ms();
        let row = SessionRow {
            id: crate::ids::session(),
            project_id: project.id.clone(),
            parent_id: None,
            fork_session_id: None,
            fork_boundary: None,
            slug: crate::slug::create(),
            directory: directory.to_string(),
            title: prepared.title,
            version: crate::version::session_version(),
            agent: None,
            model: None,
            time_created: now,
            time_updated: now,
        };
        self.with(|c| {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO session (id, project_id, slug, directory, title, version, time_created, time_updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    row.id,
                    row.project_id,
                    row.slug,
                    row.directory,
                    row.title,
                    row.version,
                    now as i64
                ],
            )?;
            for (index, message) in prepared.rows.iter().enumerate() {
                let created = message.data["time"]["created"].as_i64().unwrap_or(0);
                tx.execute(
                    "INSERT INTO message (id, session_id, seq, type, time_created, time_updated, data)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
                    params![
                        message.id,
                        row.id,
                        index as i64 + 1,
                        message.kind,
                        created,
                        compact(&message.data)
                    ],
                )?;
            }
            tx.commit()
        })?;
        Ok(row)
    }
}

/// Imports the export at `path` into the database as a new root session of `directory` (its
/// project is identified as a core would): for a front end without a running core there.
pub fn import_file(db: &Db, path: &Path, directory: &Path) -> Result<SessionRow, ImportError> {
    let bytes = read_file(path)?;
    let project = crate::project::identify(directory);
    db.import_session(&bytes, &project, &directory.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prescan_bounds_values_and_depth() {
        assert!(prescan(br#"{"a": [1, 2, {"b": "x,{["}]}"#).is_ok());
        let deep = "[".repeat(MAX_DEPTH + 1);
        assert!(matches!(
            prescan(deep.as_bytes()),
            Err(ImportError::Invalid(_))
        ));
        let wide = format!("[{}]", "[],".repeat(MAX_NODES));
        assert!(matches!(
            prescan(wide.as_bytes()),
            Err(ImportError::Invalid(_))
        ));
        // Brackets and quotes inside strings do not count.
        let quoted = format!("[\"{}\\\"{}\"]", "[".repeat(1000), "{".repeat(1000));
        assert!(prescan(quoted.as_bytes()).is_ok());
    }

    #[test]
    fn tool_names_must_fit_the_api() {
        assert!(tool_name_ok("mcp__github__list_issues"));
        assert!(tool_name_ok("edit"));
        assert!(!tool_name_ok(""));
        assert!(!tool_name_ok("rm -rf"));
        assert!(!tool_name_ok(&"a".repeat(65)));
    }

    #[test]
    fn created_is_kept() {
        let data = json!({"time": {"created": 5}, "text": "hi"});
        let prepared = prepare(
            json!({"info": {"title": "t"}, "messages": [{"id": "m", "type": "user", "time": {"created": 5}, "text": "hi"}]})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(prepared.rows[0].data, data);
        assert_eq!(prepared.title, "t");
    }
}
