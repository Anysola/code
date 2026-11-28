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
//! - Everything a later request sends is rebuilt from checked values: tool names in the API's
//!   alphabet, object inputs, images through the same checks as a prompt's (size read from the
//!   header first, a per-import image budget) with their type taken from the bytes and the data
//!   URI written anew, PDFs only when they are one, no blank text block, no row that lowers to
//!   nothing, the first request turn a user one, token counts and times clamped.
//! - The work is bounded (file size, values, depth, messages, images and pixels) and can be
//!   cancelled; it blocks, so callers run it off their async threads.

use std::io::Read;
use std::path::Path;

use base64::Engine;
use rusqlite::params;
use serde_json::{Map, Value, json};

use super::{Db, SessionRow, StorageError, json as compact};
use tokio_util::sync::CancellationToken;

use crate::gateway::error::SessionError;
use crate::session::message::{
    AgentSwitchedData, AssistantData, AssistantTime, Cache, CompactionData, Created,
    FileAttachment, InvokedSkill, Part, PartTime, ShellData, ShellOutput, SkillData, SyntheticData,
    Tokens, ToolState, UserData, encode,
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
    #[error("The import was cancelled.")]
    Cancelled,
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
    // serde's words quote the file (any length, any wording): a fixed sentence instead.
    serde_json::from_value(value).map_err(|_| {
        invalid(format!(
            "message {} ({}) has a field that is missing or of the wrong type.",
            n + 1,
            shown_kind(kind)
        ))
    })
}

/// A row kind as a refusal may quote it: known kinds only.
fn shown_kind(kind: &str) -> &str {
    match kind {
        "user" | "assistant" | "synthetic" | "skill" | "shell" | "compaction"
        | "agent-switched" | "idle" => kind,
        _ => "unknown",
    }
}

/// The description every imported synthetic row gets: what the file says a row is (a hook's
/// note, a subagent's result) is not shown as the app's own words; the window shows these as
/// imported text.
pub const IMPORTED: &str = "Imported";

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

/// Characters a title keeps: none that draw nothing or reorder text (controls, bidirectional
/// overrides, zero-width characters), as hook commands are shown.
fn visible(c: char) -> bool {
    crate::hooks::display(c.encode_utf8(&mut [0; 4]))
        .chars()
        .count()
        == 1
}

/// Token counts a row may claim.
const MAX_TOKENS: u64 = 1_000_000_000;
/// Images per import, the pixels of one and of all of them: each image is decoded (and maybe
/// resized) to be checked, so this bounds the import's time and memory. An image the app stores
/// is at most 2000 x 2000 by default.
const MAX_IMAGES: usize = 64;
const MAX_IMAGE_PIXELS: u64 = 4096 * 4096;
const MAX_TOTAL_PIXELS: u64 = 64_000_000;

/// The state of one import: when it started (no time may be later), its image budget, and the
/// caller's cancel.
struct Rebuild<'a> {
    now: u64,
    images: usize,
    pixels: u64,
    limits: crate::media::Limits,
    cancel: &'a CancellationToken,
}

impl Rebuild<'_> {
    fn at(&self, t: u64) -> u64 {
        t.min(self.now)
    }

    fn created(&self, t: Created) -> Created {
        Created {
            created: self.at(t.created),
        }
    }

    fn assistant_time(&self, t: AssistantTime) -> AssistantTime {
        AssistantTime {
            created: self.at(t.created),
            streamed: t.streamed.map(|t| self.at(t)),
            completed: t.completed.map(|t| self.at(t)),
        }
    }

    fn part_time(&self, t: PartTime) -> PartTime {
        PartTime {
            created: self.at(t.created),
            ran: t.ran.map(|t| self.at(t)),
            completed: t.completed.map(|t| self.at(t)),
        }
    }

    fn tokens(&self, t: Option<Tokens>) -> Option<Tokens> {
        t.map(|t| {
            let n = |v: u64| v.min(MAX_TOKENS);
            Tokens {
                input: n(t.input),
                output: n(t.output),
                reasoning: n(t.reasoning),
                cache: Cache {
                    read: n(t.cache.read),
                    write: n(t.cache.write),
                },
            }
        })
    }

    fn error(&self, e: SessionError) -> SessionError {
        SessionError::new(&e.kind, e.message)
    }

    /// The image `data` as the model may get it: its size read from its header first (nothing
    /// too large is decoded), within this import's budget, then through the checks a prompt's
    /// images pass ([`crate::media::normalize`]: decodable, a type the API takes, resized as
    /// there). `None`: dropped.
    fn image(&mut self, data: &str) -> Option<(String, String)> {
        if self.cancel.is_cancelled() || self.images >= MAX_IMAGES {
            return None;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .ok()?;
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        let pixels = u64::from(width) * u64::from(height);
        if pixels == 0
            || pixels > MAX_IMAGE_PIXELS
            || self.pixels.saturating_add(pixels) > MAX_TOTAL_PIXELS
        {
            return None;
        }
        self.images += 1;
        self.pixels += pixels;
        // The type comes from the bytes: the declared one is not trusted.
        match crate::media::normalize("", data, &self.limits) {
            Ok(Some((mime, data))) => Some((mime, data)),
            Ok(None) => None,
            Err(_) => None,
        }
    }

    /// A prompt's attachments as the model may get them: images as [`Rebuild::image`] gives
    /// them, PDFs only when they are one, other types kept (they are not sent). The type is
    /// lower case; the source is inline.
    fn files(&mut self, files: Option<Vec<FileAttachment>>) -> Option<Vec<FileAttachment>> {
        let files = files?;
        Some(
            files
                .into_iter()
                .filter_map(|f| {
                    let mime = f.mime.trim().to_ascii_lowercase();
                    let (mime, data) = if mime.starts_with("image/") {
                        self.image(&f.data)?
                    } else if mime == "application/pdf" {
                        is_pdf(&f.data).then_some((mime, f.data))?
                    } else if mime.is_empty() || !base64_ok(&f.data) {
                        return None;
                    } else {
                        (mime, f.data)
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

    fn user(&mut self, data: UserData) -> UserData {
        let mut extra = Map::new();
        if let Some(skills) = data.extra.get("skills")
            && let Ok(skills) = serde_json::from_value::<Vec<InvokedSkill>>(skills.clone())
        {
            let skills: Vec<InvokedSkill> = skills
                .into_iter()
                .filter(|s| !s.text.trim().is_empty())
                .collect();
            if !skills.is_empty() {
                extra.insert("skills".into(), json!(skills));
            }
        }
        // `command` is not kept: the window would show the row as a command the user ran here.
        UserData {
            time: self.created(data.time),
            text: data.text,
            files: self.files(data.files),
            extra,
        }
    }

    /// A tool result's content as the model may get it, rebuilt from checked parts: blank text
    /// dropped, images as [`Rebuild::image`] gives them (the data URI written anew from the
    /// checked type, never the one it named), PDFs only when they are one, anything else a note.
    fn content(&mut self, items: Vec<ToolContent>, completed: bool) -> Vec<ToolContent> {
        let mut out: Vec<ToolContent> = items
            .into_iter()
            .filter_map(|item| match item {
                ToolContent::Text { text } if text.trim().is_empty() => None,
                ToolContent::Text { text } => Some(ToolContent::Text { text }),
                ToolContent::File { uri, mime, name } => {
                    let mime = mime.trim().to_ascii_lowercase();
                    let data = uri
                        .strip_prefix("data:")
                        .and_then(|rest| rest.split_once(";base64,"))
                        .map(|(_, data)| data.to_string());
                    let omitted = || ToolContent::text(format!("[{mime} omitted]"));
                    let file = |mime: String, data: String| ToolContent::File {
                        uri: format!("data:{mime};base64,{data}"),
                        mime,
                        name: name.clone(),
                    };
                    Some(match data {
                        Some(data) if mime.starts_with("image/") => match self.image(&data) {
                            Some((mime, data)) => file(mime, data),
                            None => omitted(),
                        },
                        Some(data) if mime == "application/pdf" && is_pdf(&data) => {
                            file(mime.clone(), data)
                        }
                        _ => omitted(),
                    })
                }
            })
            .collect();
        // A result is never empty, as a tool's own never is.
        if out.is_empty() && completed {
            out.push(ToolContent::text("(no output)"));
        }
        out
    }

    fn tool_state(&mut self, name: &str, state: ToolState) -> ToolState {
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
                content: self.content(items, true),
                metadata: tool_metadata(metadata),
            },
            ToolState::Error {
                input,
                error,
                content: items,
                metadata,
            } => ToolState::Error {
                input: object(input),
                error: self.error(error),
                content: self.content(items, false),
                metadata: tool_metadata(metadata),
            },
        }
    }

    fn assistant(&mut self, data: AssistantData) -> AssistantData {
        let mut parts = Vec::new();
        for part in data.content {
            match part {
                Part::Text { text, .. } => parts.push(Part::Text { text, native: None }),
                Part::Reasoning { text, time, .. } => parts.push(Part::Reasoning {
                    text,
                    native: None,
                    time: time.map(|t| self.part_time(t)),
                }),
                Part::Tool {
                    name,
                    executed,
                    state,
                    time,
                    ..
                } if tool_name_ok(&name) => {
                    let state = self.tool_state(&name, state);
                    parts.push(Part::Tool {
                        id: format!("toolu_{}", crate::ids::part().trim_start_matches("prt_")),
                        state,
                        name,
                        executed,
                        native: None,
                        time: self.part_time(time),
                    });
                }
                Part::Tool { .. } => {}
            }
        }
        AssistantData {
            time: self.assistant_time(data.time),
            agent: data.agent,
            model: data.model,
            content: parts,
            snapshot: None,
            finish: data.finish,
            raw_finish: data.raw_finish,
            cost: None,
            tokens: self.tokens(data.tokens),
            // Only an interruption is kept, in the app's own words: an imported error would be
            // shown as the app's, with its retry.
            error: data
                .error
                .filter(|e| e.kind == "aborted")
                .map(|_| SessionError::new("aborted", "Step interrupted")),
            extra: Map::new(),
        }
    }

    fn shell(&self, data: ShellData) -> ShellData {
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
            time: self.assistant_time(data.time),
            command: data.command,
            output: data.output.map(|o| ShellOutput { output: o.output }),
            metadata,
            extra: Map::new(),
        }
    }

    fn idle(&self, data: &Map<String, Value>, n: usize) -> Result<Value, ImportError> {
        let created = data
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid(format!("message {} (idle) has no time.", n + 1)))?;
        let outcome = match data.get("outcome").and_then(Value::as_str) {
            Some(o @ ("succeeded" | "failed" | "interrupted")) => o,
            _ => return Err(invalid(format!("message {} (idle) has no outcome.", n + 1))),
        };
        let mut out = json!({"time": {"created": self.at(created)}, "outcome": outcome});
        if let Some(reason @ ("user" | "budget" | "declined" | "closed")) =
            data.get("reason").and_then(Value::as_str)
        {
            out["reason"] = json!(reason);
        }
        Ok(out)
    }
}

/// Whether a user row lowers to anything the model gets (see `session::lower::user`).
fn user_has_content(data: &UserData) -> bool {
    !data.text.trim().is_empty()
        || data.extra.contains_key("skills")
        || data
            .files
            .iter()
            .flatten()
            .any(|f| f.mime.starts_with("image/") || f.mime == "application/pdf")
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

/// Validates and rebuilds a session export's rows (see the module docs). Nothing is written.
pub fn prepare(bytes: &[u8]) -> Result<Prepared, ImportError> {
    prepare_with(bytes, &CancellationToken::new())
}

/// [`prepare`], ending with [`ImportError::Cancelled`] once `cancel` is cancelled (checked
/// between messages and before each image).
pub fn prepare_with(bytes: &[u8], cancel: &CancellationToken) -> Result<Prepared, ImportError> {
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
                .map(|c| if visible(c) { c } else { ' ' })
                .collect::<String>()
                .trim()
                .chars()
                .take(TITLE_CHARS)
                .collect()
        })
        .filter(|t: &String| !t.is_empty())
        .unwrap_or_else(|| "Imported session".into());

    let mut rebuild = Rebuild {
        now: crate::time::now_ms(),
        images: 0,
        pixels: 0,
        limits: crate::media::Limits::from_config(None),
        cancel,
    };
    // Rows kept so far, with the file's id of each (for `keepID`).
    let mut rows: Vec<Row> = Vec::new();
    let mut ids: Vec<Option<String>> = Vec::new();
    let mut seen_user = false;
    for (n, message) in messages.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(ImportError::Cancelled);
        }
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
        // `user_turn`: the row lowers to user-role content; a row that lowers to nothing is not
        // kept (nor counted as the turn a request must open with).
        let (data, user_turn) = match kind.as_str() {
            "user" => {
                let d = rebuild.user(decode::<UserData>(data, n, &kind)?);
                if !user_has_content(&d) {
                    continue;
                }
                (encode(&d), true)
            }
            "synthetic" => {
                let d: SyntheticData = decode(data, n, &kind)?;
                if d.text.trim().is_empty() {
                    continue;
                }
                let d = SyntheticData {
                    time: rebuild.created(d.time),
                    text: d.text,
                    description: Some(IMPORTED.into()),
                    extra: Map::new(),
                };
                (encode(&d), true)
            }
            "skill" => {
                let d: SkillData = decode(data, n, &kind)?;
                if d.text.trim().is_empty() {
                    continue;
                }
                let d = SkillData {
                    time: rebuild.created(d.time),
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
                (encode(&rebuild.assistant(d)), false)
            }
            "shell" => {
                let d = rebuild.shell(decode::<ShellData>(data, n, &kind)?);
                let background = d.background();
                (encode(&d), !background)
            }
            "compaction" => {
                let d: CompactionData = decode(data, n, &kind)?;
                // A failed one never reaches the model; the window would show its error in the
                // app's words.
                if d.status != "completed" {
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
                    time: rebuild.created(d.time),
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
                    error: None,
                    cost: None,
                    tokens: rebuild.tokens(d.tokens),
                    metadata: keep.map(|id| json!({"keepID": id})),
                };
                (encode(&d), completed)
            }
            "agent-switched" => {
                let d: AgentSwitchedData = decode(data, n, &kind)?;
                let d = AgentSwitchedData {
                    time: rebuild.created(d.time),
                    agent: d.agent,
                    previous: d.previous,
                    extra: Map::new(),
                };
                (encode(&d), false)
            }
            "idle" => match &data {
                Value::Object(map) => (rebuild.idle(map, n)?, false),
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
    if cancel.is_cancelled() {
        return Err(ImportError::Cancelled);
    }
    Ok(Prepared { title, rows })
}

impl Db {
    /// Imports a session export (`bytes`, see [`prepare`]) as a new root session of `project`
    /// in `directory`, in one transaction. Returns the new session.
    /// Blocking (images are checked): call it off an async runtime's threads.
    pub fn import_session(
        &self,
        bytes: &[u8],
        project: &crate::project::Project,
        directory: &str,
        cancel: &CancellationToken,
    ) -> Result<SessionRow, ImportError> {
        let prepared = prepare_with(bytes, cancel)?;
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
/// Blocking: call it off an async runtime's threads (`spawn_blocking`).
pub fn import_file(
    db: &Db,
    path: &Path,
    directory: &Path,
    cancel: &CancellationToken,
) -> Result<SessionRow, ImportError> {
    let bytes = read_file(path)?;
    let project = crate::project::identify(directory);
    db.import_session(&bytes, &project, &directory.display().to_string(), cancel)
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
