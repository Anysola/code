//! Jupyter notebooks (nbformat 4): the cell view `read` shows for an `.ipynb` file, and the
//! `notebook` tool (Claude Code's `NotebookEdit`) that replaces, inserts or deletes one cell.
//!
//! The tool writes a file, so it goes through every gate `edit` does: the path is resolved and
//! checked against the session's checkout (`refuse_foreign_write`, `authorize_external`), the
//! change is asked as an `edit` with a diff, it runs under the file's lock, and the step around
//! it treats it as a file write (protected files ask, hooks see `file_path`, snapshots capture
//! it). Everything it does not change is written back as it was read, as far as JSON allows:
//! key order, the indentation, the final newline, the form of each `source`.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::access::{self, Kind};
use super::edit::diff::{FileDiff, Status};
use super::edit::mutation::{self, ReadFailure};
use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult,
    parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;

embedded_def!(def, "notebook");

/// Characters of one cell's outputs the view shows.
const OUTPUT_CHARS: usize = 2000;
/// Notebooks larger than this are read as text (their JSON), not parsed for the view.
pub(crate) const VIEW_BYTES: u64 = 64 * 1024 * 1024;
/// JSON values a notebook may hold to be parsed (see `storage::import::within`); a larger one is
/// read as text and can't be changed with the tool.
const MAX_VALUES: usize = 2_000_000;

pub struct NotebookTool;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    path: String,
    cell_id: Option<String>,
    index: Option<i64>,
    mode: Option<String>,
    source: Option<String>,
    cell_type: Option<String>,
}

impl Tool for NotebookTool {
    fn name(&self) -> &'static str {
        "notebook"
    }

    fn writes_files(&self) -> bool {
        true
    }

    fn permission(&self) -> &'static str {
        "edit"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            edit(&input, ctx).await
        }
        .boxed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Replace,
    Insert,
    Delete,
}

/// A notebook as read: its JSON and how it was written.
#[derive(Debug)]
pub(crate) struct Notebook {
    value: Value,
    /// The indentation unit (`None`: one line).
    indent: Option<String>,
    crlf: bool,
    newline_at_end: bool,
}

impl Notebook {
    /// The notebook, when `text` is one in nbformat 4.
    pub(crate) fn parse(text: &str) -> Option<Notebook> {
        // Bounded before parsing: what `read` takes in is bounded by more than the file's size.
        crate::storage::import::within(text.as_bytes(), MAX_VALUES, 64).ok()?;
        // A key there twice: the view and an edit could disagree on the cells.
        if duplicate_key(text).is_some() {
            return None;
        }
        let value: Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        if object.get("nbformat").and_then(Value::as_u64) != Some(4) {
            return None;
        }
        let cells = object.get("cells")?.as_array()?;
        if !cells
            .iter()
            .all(|c| c.get("cell_type").is_some_and(Value::is_string))
        {
            return None;
        }
        let crlf = text.contains("\r\n");
        let indent = text.split('\n').nth(1).map(|line| {
            line.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>()
        });
        let indent = indent.filter(|unit| !unit.is_empty());
        Some(Notebook {
            value,
            indent,
            crlf,
            newline_at_end: text.ends_with('\n'),
        })
    }

    fn cells(&self) -> &Vec<Value> {
        static NONE: Vec<Value> = Vec::new();
        self.value["cells"].as_array().unwrap_or(&NONE)
    }

    fn cells_mut(&mut self) -> &mut Vec<Value> {
        self.value["cells"]
            .as_array_mut()
            .expect("parse checked the cells")
    }

    fn minor(&self) -> u64 {
        self.value["nbformat_minor"].as_u64().unwrap_or(0)
    }

    /// Cells carry ids from nbformat 4.5 on (or when this one's already do).
    fn uses_ids(&self) -> bool {
        self.minor() >= 5 || self.cells().iter().any(|c| c.get("id").is_some())
    }

    /// One cell in the notebook's style, its lines after the first indented by `prefix`.
    fn cell_text(&self, cell: &Value, prefix: &str) -> String {
        use serde::Serialize;
        let mut out = Vec::new();
        match &self.indent {
            Some(unit) => {
                let formatter = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
                let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
                let _ = cell.serialize(&mut serializer);
            }
            None => {
                let _ = serde_json::to_writer(&mut out, cell);
            }
        }
        let text = String::from_utf8(out).unwrap_or_default();
        let newline = if self.crlf { "\r\n" } else { "\n" };
        text.replace('\n', &format!("{newline}{prefix}"))
    }

    /// The notebook after `change`, written into `original` (the text it was read from) so that
    /// every cell but the changed one keeps its exact bytes: JSON numbers (`1e-05`), escapes
    /// (`\u00e9`) and whatever else a reserialization would rewrite. Falls back to
    /// [`Notebook::text`] when the text can't be read that way.
    fn spliced(&self, original: &str, change: Change) -> String {
        let Some(layout) = layout(original) else {
            return self.text();
        };
        let cells = self.cells();
        let before = layout.cells.len();
        let expected = match change {
            Change::Replace(_) => before,
            Change::Insert(_) => before + 1,
            Change::Delete(_) => before.saturating_sub(1),
        };
        if cells.len() != expected {
            return self.text();
        }
        let prefix_of = |start: usize| -> String {
            let line = original[..start].rfind('\n').map_or(0, |i| i + 1);
            let prefix = &original[line..start];
            if prefix.chars().all(|c| c == ' ' || c == '\t') {
                prefix.to_string()
            } else {
                String::new()
            }
        };
        let newline = if self.crlf { "\r\n" } else { "\n" };
        let at = |i: usize| layout.cells[i].clone();
        match change {
            Change::Replace(i) => {
                let range = at(i);
                let cell = self.cell_text(&cells[i], &prefix_of(range.start));
                format!(
                    "{}{cell}{}",
                    &original[..range.start],
                    &original[range.end..]
                )
            }
            Change::Delete(i) => {
                if before == 1 {
                    return format!("{}{}", &original[..=layout.open], &original[layout.close..]);
                }
                let cut = if i + 1 < before {
                    at(i).start..at(i + 1).start
                } else {
                    at(i - 1).end..at(i).end
                };
                format!("{}{}", &original[..cut.start], &original[cut.end..])
            }
            Change::Insert(_) if before == 0 => self.text(),
            Change::Insert(position) if position < before => {
                let start = at(position).start;
                let prefix = prefix_of(start);
                let separator = if self.indent.is_some() {
                    format!(",{newline}{prefix}")
                } else {
                    ",".to_string()
                };
                let cell = self.cell_text(&cells[position], &prefix);
                format!(
                    "{}{cell}{separator}{}",
                    &original[..start],
                    &original[start..]
                )
            }
            Change::Insert(position) => {
                let end = at(before - 1).end;
                let prefix = prefix_of(at(before - 1).start);
                let separator = if self.indent.is_some() {
                    format!(",{newline}{prefix}")
                } else {
                    ",".to_string()
                };
                let cell = self.cell_text(&cells[position], &prefix);
                format!("{}{separator}{cell}{}", &original[..end], &original[end..])
            }
        }
    }

    /// The notebook written back in its own style.
    pub(crate) fn text(&self) -> String {
        use serde::Serialize;
        let mut out = Vec::new();
        match &self.indent {
            Some(unit) => {
                let formatter = serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes());
                let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
                let _ = self.value.serialize(&mut serializer);
            }
            None => {
                let _ = serde_json::to_writer(&mut out, &self.value);
            }
        }
        let mut text = String::from_utf8(out).unwrap_or_default();
        if self.newline_at_end {
            text.push('\n');
        }
        if self.crlf {
            text = text.replace('\n', "\r\n");
        }
        text
    }
}

/// Where each cell of a notebook's text is: byte ranges of the elements of the top-level
/// `cells` array, and the array's brackets.
#[derive(Debug, PartialEq)]
struct Layout {
    cells: Vec<std::ops::Range<usize>>,
    /// The `[` and the `]` of the array.
    open: usize,
    close: usize,
}

fn skip_ws(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// The end of the string starting at `at` (its `"`), past the closing quote.
fn string_end(bytes: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// The end of the JSON value starting at `at`.
fn value_end(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at)? {
        b'"' => string_end(bytes, at),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = at;
            while i < bytes.len() {
                match bytes[i] {
                    b'"' => {
                        i = string_end(bytes, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(i + 1);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            None
        }
        _ => {
            let mut i = at;
            while i < bytes.len()
                && !matches!(bytes[i], b',' | b'}' | b']')
                && !bytes[i].is_ascii_whitespace()
            {
                i += 1;
            }
            Some(i)
        }
    }
}

/// The keys of the top-level object of `text`, decoded (`"cell\u0073"` is `cells`), in order;
/// `None` when it is not an object.
fn top_level_keys(text: &str) -> Option<Vec<String>> {
    let bytes = text.as_bytes();
    let mut at = skip_ws(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at += 1;
    let mut keys = Vec::new();
    loop {
        at = skip_ws(bytes, at);
        match bytes.get(at)? {
            b'}' => return Some(keys),
            b',' => {
                at += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let key_end = string_end(bytes, at)?;
        keys.push(serde_json::from_str::<String>(&text[at..key_end]).ok()?);
        at = skip_ws(bytes, key_end);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        at = value_end(bytes, skip_ws(bytes, at + 1))?;
    }
}

/// A top-level key `text` has twice: JSON readers (and the cell view) take the last one, a
/// splice would not know which to change.
fn duplicate_key(text: &str) -> Option<String> {
    let keys = top_level_keys(text)?;
    keys.iter()
        .enumerate()
        .find(|(i, key)| keys[..*i].contains(key))
        .map(|(_, key)| key.clone())
}

/// The cells' places in `text`, a notebook's JSON; `None` when it can't be read that way (or
/// a top-level key is there twice).
fn layout(text: &str) -> Option<Layout> {
    if duplicate_key(text).is_some() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut at = skip_ws(bytes, 0);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at += 1;
    loop {
        at = skip_ws(bytes, at);
        match bytes.get(at)? {
            b'}' => return None,
            b',' => {
                at += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let key_end = string_end(bytes, at)?;
        let key = serde_json::from_str::<String>(&text[at..key_end]).ok()?;
        at = skip_ws(bytes, key_end);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        at = skip_ws(bytes, at + 1);
        if key != "cells" {
            at = value_end(bytes, at)?;
            continue;
        }
        if bytes.get(at) != Some(&b'[') {
            return None;
        }
        let open = at;
        let mut cells = Vec::new();
        at += 1;
        loop {
            at = skip_ws(bytes, at);
            match bytes.get(at)? {
                b']' => {
                    return Some(Layout {
                        cells,
                        open,
                        close: at,
                    });
                }
                b',' => at += 1,
                _ => {
                    let end = value_end(bytes, at)?;
                    cells.push(at..end);
                    at = end;
                }
            }
        }
    }
}

/// A cell's source as text: a string, or a list of strings joined.
fn source_text(cell: &Value) -> String {
    match &cell["source"] {
        Value::String(text) => text.clone(),
        Value::Array(lines) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// `text` in the form `like` has: a list of lines (each keeping its `\n`) or one string.
fn source_value(text: &str, list: bool) -> Value {
    if list {
        Value::Array(text.split_inclusive('\n').map(|l| json!(l)).collect())
    } else {
        json!(text)
    }
}

fn kind(cell: &Value) -> &str {
    cell["cell_type"].as_str().unwrap_or("")
}

/// ANSI escape sequences (tracebacks are colored) taken out.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn joined(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(lines) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// A code cell's outputs as text: streams, `text/plain` results, error tracebacks; other media
/// as a placeholder. Cut at [`OUTPUT_CHARS`].
fn outputs(cell: &Value) -> Option<String> {
    let list = cell["outputs"].as_array()?;
    if list.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for output in list {
        match output["output_type"].as_str().unwrap_or("") {
            "stream" => parts.push(joined(&output["text"])),
            "error" => {
                let traceback: Vec<String> = output["traceback"]
                    .as_array()
                    .map(|t| t.iter().filter_map(Value::as_str).map(plain).collect())
                    .unwrap_or_default();
                if traceback.is_empty() {
                    parts.push(format!(
                        "{}: {}",
                        output["ename"].as_str().unwrap_or("Error"),
                        output["evalue"].as_str().unwrap_or("")
                    ));
                } else {
                    parts.push(traceback.join("\n"));
                }
            }
            "execute_result" | "display_data" => {
                let data = output["data"].as_object();
                match data.and_then(|d| d.get("text/plain")) {
                    Some(text) => parts.push(joined(text)),
                    None => {
                        let mimes: Vec<&str> = data
                            .map(|d| d.keys().map(String::as_str).collect())
                            .unwrap_or_default();
                        parts.push(format!("[{} output]", mimes.join(", ")));
                    }
                }
            }
            other => parts.push(format!("[{other} output]")),
        }
    }
    let text = parts
        .iter()
        .map(|p| p.trim_end_matches('\n'))
        .collect::<Vec<_>>()
        .join("\n");
    Some(match text.char_indices().nth(OUTPUT_CHARS) {
        Some((at, _)) => format!("{}\n[… output cut]", &text[..at]),
        None => text,
    })
}

/// The cell view `read` shows for a notebook; `None` when `bytes` is not one.
pub(crate) fn view(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).ok()?;
    let notebook = Notebook::parse(text)?;
    let cells = notebook.cells();
    let language = notebook.value["metadata"]["kernelspec"]["language"]
        .as_str()
        .or(notebook.value["metadata"]["language_info"]["name"].as_str());
    let mut lines = vec![format!(
        "Jupyter notebook, nbformat 4.{}{}, {} cell{}. Change cells with the notebook tool, by cellId or index.",
        notebook.minor(),
        language.map(|l| format!(", {l}")).unwrap_or_default(),
        cells.len(),
        if cells.len() == 1 { "" } else { "s" }
    )];
    for (index, cell) in cells.iter().enumerate() {
        let mut head = format!("--- cell {index}");
        if let Some(id) = cell["id"].as_str() {
            head.push_str(&format!(" · id {id}"));
        }
        head.push_str(&format!(" · {}", kind(cell)));
        if let Some(count) = cell["execution_count"].as_u64() {
            head.push_str(&format!(" · executed [{count}]"));
        }
        lines.push(String::new());
        lines.push(head);
        let source = source_text(cell);
        if !source.is_empty() {
            lines.push(source.trim_end_matches('\n').to_string());
        }
        if let Some(output) = outputs(cell) {
            lines.push("--- output".into());
            lines.push(output);
        }
    }
    Some(lines.join("\n"))
}

fn fail(message: impl Into<String>) -> ToolError {
    ToolError::execution(message)
}

/// A new cell id: eight hex characters no other cell has.
fn new_id(cells: &[Value]) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    loop {
        let id: String = (0..8)
            .map(|_| char::from_digit(rng.gen_range(0..16), 16).unwrap_or('0'))
            .collect();
        if !cells.iter().any(|c| c["id"].as_str() == Some(id.as_str())) {
            return id;
        }
    }
}

/// A cell of `kind` with `source`, keys in the order Jupyter writes them.
fn new_cell(kind: &str, source: &str, id: Option<String>, list: bool) -> Value {
    let mut cell = Map::new();
    cell.insert("cell_type".into(), json!(kind));
    if kind == "code" {
        cell.insert("execution_count".into(), Value::Null);
    }
    if let Some(id) = id {
        cell.insert("id".into(), json!(id));
    }
    cell.insert("metadata".into(), json!({}));
    if kind == "code" {
        cell.insert("outputs".into(), json!([]));
    }
    cell.insert("source".into(), source_value(source, list));
    Value::Object(cell)
}

/// `cell` changed to `kind`: a code cell gets empty outputs, a markdown cell loses them. Keys
/// are put back in Jupyter's (sorted) order.
fn retype(cell: &mut Value, kind: &str) {
    let Some(map) = cell.as_object_mut() else {
        return;
    };
    map.insert("cell_type".into(), json!(kind));
    if kind == "code" {
        map.entry("execution_count").or_insert(Value::Null);
        map.entry("outputs").or_insert_with(|| json!([]));
    } else {
        map.shift_remove("execution_count");
        map.shift_remove("outputs");
    }
    map.sort_keys();
}

/// The cell `input` names: by id, else by index.
fn target(notebook: &Notebook, input: &Input) -> Result<Option<usize>, ToolError> {
    let cells = notebook.cells();
    match (&input.cell_id, input.index) {
        (Some(_), Some(_)) => Err(fail("Give cellId or index, not both.")),
        (Some(id), None) => cells
            .iter()
            .position(|c| c["id"].as_str() == Some(id.as_str()))
            .map(Some)
            .ok_or_else(|| {
                fail(format!(
                    "No cell has id \"{id}\". Read the notebook for its cells' ids, or name the cell by index."
                ))
            }),
        (None, Some(index)) => usize::try_from(index)
            .ok()
            .filter(|i| *i < cells.len())
            .map(Some)
            .ok_or_else(|| {
                fail(format!(
                    "Index {index} is out of range: the notebook has {} cells (0 to {}).",
                    cells.len(),
                    cells.len().saturating_sub(1)
                ))
            }),
        (None, None) => Ok(None),
    }
}

/// Which cell an edit changed, for writing only that cell back.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Change {
    Replace(usize),
    Insert(usize),
    Delete(usize),
}

/// Applies the change; returns what the result says and where it was.
fn apply(notebook: &mut Notebook, input: &Input) -> Result<(String, Change), ToolError> {
    let mode = match input.mode.as_deref() {
        None | Some("replace") => Mode::Replace,
        Some("insert") => Mode::Insert,
        Some("delete") => Mode::Delete,
        Some(other) => return Err(fail(format!("Unknown mode \"{other}\"."))),
    };
    let at = target(notebook, input)?;
    let uses_ids = notebook.uses_ids();
    let list = notebook
        .cells()
        .iter()
        .find_map(|c| match &c["source"] {
            Value::Array(_) => Some(true),
            Value::String(_) => Some(false),
            _ => None,
        })
        .unwrap_or(true);
    let name = |index: usize, cell: &Value| match cell["id"].as_str() {
        Some(id) => format!("cell {index} (id {id})"),
        None => format!("cell {index}"),
    };
    match mode {
        Mode::Delete => {
            let at = at.ok_or_else(|| fail("Name the cell to delete with cellId or index."))?;
            let removed = notebook.cells_mut().remove(at);
            Ok((
                format!("Deleted {}", name(at, &removed)),
                Change::Delete(at),
            ))
        }
        Mode::Insert => {
            let source = input
                .source
                .as_deref()
                .ok_or_else(|| fail("Give the new cell's source."))?;
            let kind = input.cell_type.as_deref().unwrap_or("code");
            let id = uses_ids.then(|| new_id(notebook.cells()));
            let cell = new_cell(kind, source, id, list);
            let position = at.map_or(0, |i| i + 1);
            let said = name(position, &cell);
            notebook.cells_mut().insert(position, cell);
            Ok((format!("Inserted {kind} {said}"), Change::Insert(position)))
        }
        Mode::Replace => {
            let at = at.ok_or_else(|| fail("Name the cell to replace with cellId or index."))?;
            let source = input
                .source
                .as_deref()
                .ok_or_else(|| fail("Give the cell's new source."))?;
            let cell = &mut notebook.cells_mut()[at];
            if let Some(kind) = input.cell_type.as_deref()
                && kind != self::kind(cell)
            {
                retype(cell, kind);
            }
            let list = match &cell["source"] {
                Value::Array(_) => true,
                Value::String(_) => false,
                _ => list,
            };
            cell["source"] = source_value(source, list);
            if self::kind(cell) == "code" {
                cell["outputs"] = json!([]);
                cell["execution_count"] = Value::Null;
            }
            Ok((format!("Replaced {}", name(at, cell)), Change::Replace(at)))
        }
    }
}

async fn edit(input: &Input, ctx: &ToolContext) -> ToolResult {
    let path = &input.path;
    let failed = || fail(format!("Unable to edit {path}"));
    let target = access::resolve(ctx, path, Some(Kind::File));
    // As `edit`: the whole read-change-ask-write runs under the file's lock.
    let _guard = ctx
        .locks
        .lock(&mutation::lock_key(&target.absolute).await)
        .await;
    access::refuse_foreign_write(ctx, &target)?;
    access::authorize_external(ctx, &[&target]).await?;
    let original =
        mutation::read_text(&target.absolute)
            .await
            .map_err(|failure| match failure {
                ReadFailure::Missing => fail(format!(
                    "File not found: {path}. Create a notebook with write."
                )),
                ReadFailure::Directory => fail(format!("Path is a directory, not a file: {path}")),
                ReadFailure::NotUtf8 => fail(format!("File is not valid UTF-8: {path}")),
                ReadFailure::Failed => failed(),
            })?;
    if let Some(key) = duplicate_key(&original.text) {
        return Err(fail(format!(
            "{path} has the top-level key \"{key}\" more than once, so it is unclear which one to change. Remove the extra one first."
        )));
    }
    let mut notebook = Notebook::parse(&original.text).ok_or_else(|| {
        fail(format!(
            "{path} is not a Jupyter notebook in nbformat 4 that can be changed here (or it is too large). Use edit for other files."
        ))
    })?;
    let (said, change) = apply(&mut notebook, input)?;
    let after = notebook.spliced(&original.text, change);
    let preview = FileDiff::new(&target.resource, &original.text, &after, Status::Modified);
    ctx.ask(PermissionRequest {
        action: "edit".into(),
        resources: target.resources(),
        save: vec!["*".into()],
        metadata: json!({ "files": [preview.to_value()] }),
    })
    .await?;
    mutation::write_file(&target.absolute, &mutation::join_bom(&after, original.bom))
        .await
        .map_err(|_| failed())?;
    let formatted = mutation::format(ctx, &target.absolute, original.bom).await;
    let after = formatted.as_deref().unwrap_or(&after);
    let file = FileDiff::new(&target.resource, &original.text, after, Status::Modified);
    let mut message = format!("{said} in {}", target.resource);
    if let Some(diag) = mutation::diagnose(ctx, &target.absolute, &target.resource, after).await {
        message.push_str("\n\n");
        message.push_str(&diag);
    }
    Ok(ToolOutput::text(message).with_metadata("files", json!([file.to_value()])))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::tool::ToolContent;
    use crate::tool::test_support::context;

    /// As Jupyter writes a notebook: one-space indent, sorted keys, final newline, non-ASCII as
    /// is, sources as lists of lines.
    const JUPYTER: &str = r##"{
 "cells": [
  {
   "cell_type": "markdown",
   "id": "intro01",
   "metadata": {},
   "source": [
    "# Title ✓\n",
    "\n",
    "Some text"
   ]
  },
  {
   "cell_type": "code",
   "execution_count": 3,
   "id": "code0002",
   "metadata": {
    "tags": [
     "keep"
    ]
   },
   "outputs": [
    {
     "name": "stdout",
     "output_type": "stream",
     "text": [
      "hello\n"
     ]
    },
    {
     "data": {
      "image/png": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
      "text/plain": [
       "<Figure size 640x480 with 1 Axes>"
      ]
     },
     "metadata": {},
     "output_type": "display_data"
    }
   ],
   "source": [
    "print('hello')\n",
    "plot()"
   ]
  },
  {
   "cell_type": "code",
   "execution_count": 4,
   "id": "code0003",
   "metadata": {},
   "outputs": [
    {
     "ename": "ZeroDivisionError",
     "evalue": "division by zero",
     "output_type": "error",
     "traceback": [
      "\u001b[0;31mZeroDivisionError\u001b[0m: division by zero"
     ]
    }
   ],
   "source": [
    "1/0"
   ]
  }
 ],
 "metadata": {
  "kernelspec": {
   "display_name": "Python 3",
   "language": "python",
   "name": "python3"
  },
  "language_info": {
   "name": "python",
   "version": "3.12.1"
  }
 },
 "nbformat": 4,
 "nbformat_minor": 5
}
"##;

    #[test]
    fn a_jupyter_notebook_is_written_back_byte_for_byte() {
        let notebook = Notebook::parse(JUPYTER).unwrap();
        assert_eq!(notebook.text(), JUPYTER);
        let crlf = JUPYTER.replace('\n', "\r\n");
        assert_eq!(Notebook::parse(&crlf).unwrap().text(), crlf);
        let compact =
            serde_json::to_string(&serde_json::from_str::<Value>(JUPYTER).unwrap()).unwrap();
        assert_eq!(Notebook::parse(&compact).unwrap().text(), compact);
        let two =
            serde_json::to_string_pretty(&serde_json::from_str::<Value>(JUPYTER).unwrap()).unwrap();
        assert_eq!(
            Notebook::parse(&two).unwrap().text(),
            two,
            "two spaces, no final newline"
        );
    }

    #[test]
    fn the_view_shows_cells_and_text_outputs_without_image_data() {
        let view = view(JUPYTER.as_bytes()).unwrap();
        assert!(
            view.starts_with("Jupyter notebook, nbformat 4.5, python, 3 cells."),
            "{view}"
        );
        assert!(view.contains("--- cell 0 · id intro01 · markdown\n# Title ✓\n\nSome text"));
        assert!(view.contains(
            "--- cell 1 · id code0002 · code · executed [3]\nprint('hello')\nplot()\n--- output\nhello\n<Figure size 640x480 with 1 Axes>"
        ));
        assert!(
            view.contains("ZeroDivisionError: division by zero"),
            "{view}"
        );
        assert!(!view.contains("iVBOR"), "no base64");
        assert!(!view.contains('\u{1b}'), "no escapes");
        let image_only = JUPYTER.replace("\"text/plain\"", "\"text/x-other\"");
        assert!(view_of(&image_only).contains("[image/png, text/x-other output]"));
        assert!(super::view(b"{\"nbformat\": 3, \"cells\": []}").is_none());
        assert!(super::view(b"not json").is_none());
    }

    fn view_of(text: &str) -> String {
        super::view(text.as_bytes()).unwrap()
    }

    async fn run(dir: &Path, input: Value) -> (ToolResult, Vec<PermissionRequest>) {
        let test = context(dir);
        let result = NotebookTool.execute(input, &test.ctx).await;
        let asked = test.gate.asked.lock().unwrap().clone();
        (result, asked)
    }

    fn setup() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("n.ipynb"), JUPYTER).unwrap();
        dir
    }

    fn cells(dir: &tempfile::TempDir) -> Vec<Value> {
        let text = std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap();
        serde_json::from_str::<Value>(&text).unwrap()["cells"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn text(output: &ToolOutput) -> &str {
        match &output.content[..] {
            [ToolContent::Text { text }] => text,
            other => panic!("unexpected content {other:?}"),
        }
    }

    #[tokio::test]
    async fn replacing_a_code_cell_clears_only_its_outputs() {
        let dir = setup();
        let (result, asked) = run(
            dir.path(),
            json!({"path": "n.ipynb", "cellId": "code0002", "source": "print('bye')\nx = 1\n"}),
        )
        .await;
        let output = result.unwrap();
        assert_eq!(text(&output), "Replaced cell 1 (id code0002) in n.ipynb");
        let written = std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap();
        let expected = JUPYTER
            .replace(
                r#"   "execution_count": 3,"#,
                r#"   "execution_count": null,"#,
            )
            .replace(
                &JUPYTER[JUPYTER
                    .find("   \"outputs\": [\n    {\n     \"name\"")
                    .unwrap()
                    ..JUPYTER.find("   \"source\": [\n    \"print").unwrap()],
                "   \"outputs\": [],\n",
            )
            .replace(
                "    \"print('hello')\\n\",\n    \"plot()\"\n",
                "    \"print('bye')\\n\",\n    \"x = 1\\n\"\n",
            );
        assert_eq!(written, expected, "everything else as it was");
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].action, "edit");
        assert_eq!(asked[0].resources, ["n.ipynb"]);
        assert_eq!(asked[0].metadata["files"][0]["file"], "n.ipynb");
        assert_eq!(output.metadata["files"][0]["status"], "modified");
    }

    #[tokio::test]
    async fn insert_delete_and_retype() {
        let dir = setup();
        let (result, _) = run(
            dir.path(),
            json!({"path": "n.ipynb", "mode": "insert", "source": "## Head", "cellType": "markdown"}),
        )
        .await;
        assert!(text(&result.unwrap()).starts_with("Inserted markdown cell 0 (id "));
        let all = cells(&dir);
        assert_eq!(all.len(), 4);
        assert_eq!(all[0]["source"], json!(["## Head"]));
        assert_eq!(all[0]["id"].as_str().unwrap().len(), 8);
        let keys: Vec<&String> = all[0].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["cell_type", "id", "metadata", "source"]);

        run(
            dir.path(),
            json!({"path": "n.ipynb", "mode": "insert", "index": 3, "source": "y = 2"}),
        )
        .await
        .0
        .unwrap();
        let all = cells(&dir);
        assert_eq!(all[4]["cell_type"], "code");
        assert_eq!(all[4]["outputs"], json!([]));
        assert_eq!(all[4]["execution_count"], Value::Null);

        run(
            dir.path(),
            json!({"path": "n.ipynb", "index": 1, "source": "now code", "cellType": "code"}),
        )
        .await
        .0
        .unwrap();
        let all = cells(&dir);
        assert_eq!(all[1]["cell_type"], "code");
        assert_eq!(all[1]["outputs"], json!([]));
        assert_eq!(all[1]["id"], "intro01");

        let (result, _) = run(
            dir.path(),
            json!({"path": "n.ipynb", "mode": "delete", "cellId": "code0003"}),
        )
        .await;
        assert_eq!(
            text(&result.unwrap()),
            "Deleted cell 3 (id code0003) in n.ipynb"
        );
        let all = cells(&dir);
        assert_eq!(all.len(), 4);
        assert!(all.iter().all(|c| c["id"] != "code0003"));
        let written = std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap();
        assert!(written.ends_with("\"nbformat_minor\": 5\n}\n"));
    }

    #[tokio::test]
    async fn bad_targets_fail_before_asking() {
        let dir = setup();
        for (input, message) in [
            (
                json!({"path": "n.ipynb", "cellId": "nope", "source": "x"}),
                "No cell has id \"nope\"",
            ),
            (
                json!({"path": "n.ipynb", "index": 9, "source": "x"}),
                "Index 9 is out of range: the notebook has 3 cells (0 to 2).",
            ),
            (
                json!({"path": "n.ipynb", "index": 0, "cellId": "intro01", "source": "x"}),
                "Give cellId or index, not both.",
            ),
            (
                json!({"path": "n.ipynb", "mode": "delete"}),
                "Name the cell to delete",
            ),
            (
                json!({"path": "n.ipynb", "index": 0}),
                "Give the cell's new source.",
            ),
        ] {
            let (result, asked) = run(dir.path(), input).await;
            let error = result.unwrap_err().message;
            assert!(error.starts_with(message), "{error}");
            assert!(asked.is_empty());
        }
        std::fs::write(dir.path().join("plain.ipynb"), "{\"a\": 1}").unwrap();
        let (result, _) = run(
            dir.path(),
            json!({"path": "plain.ipynb", "index": 0, "source": "x"}),
        )
        .await;
        assert!(
            result
                .unwrap_err()
                .message
                .contains("not a Jupyter notebook")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap(),
            JUPYTER
        );
    }

    #[tokio::test]
    async fn a_session_pinned_to_its_worktree_cannot_change_the_main_folders_notebook() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        let worktree = dir.path().join("w");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(main.join("n.ipynb"), JUPYTER).unwrap();
        let test = context(&worktree);
        let mut ctx = test.ctx.clone();
        ctx.main = main.clone();
        let path = main.join("n.ipynb").display().to_string();
        let error = NotebookTool
            .execute(json!({"path": path, "index": 0, "source": "x"}), &ctx)
            .await
            .unwrap_err();
        assert_eq!(error.kind, crate::tool::ToolErrorKind::Blocked);
        assert!(
            test.gate.asked.lock().unwrap().is_empty(),
            "refused before asking"
        );
        assert_eq!(
            std::fs::read_to_string(main.join("n.ipynb")).unwrap(),
            JUPYTER
        );
    }

    #[tokio::test]
    async fn untouched_cells_keep_bytes_a_reserialization_would_change() {
        // Plotly writes `1e-05`, some tools escape non-ASCII, big integers exceed u64: serde_json
        // would write `1e-5`, `é` and a float. Only the changed cell is written anew.
        let odd = JUPYTER.replace(
            "   \"metadata\": {\n    \"tags\": [\n     \"keep\"\n    ]\n   },",
            "   \"metadata\": {\n    \"scale\": 1e-05,\n    \"big\": 123456789012345678901234567890,\n    \"name\": \"caf\\u00e9\"\n   },",
        );
        assert_ne!(odd, JUPYTER);
        assert_ne!(
            Notebook::parse(&odd).unwrap().text(),
            odd,
            "a reserialization rewrites"
        );
        for (input, kept) in [
            (
                json!({"path": "n.ipynb", "index": 2, "source": "2/1"}),
                true,
            ),
            (
                json!({"path": "n.ipynb", "mode": "insert", "index": 0, "source": "y"}),
                true,
            ),
            (
                json!({"path": "n.ipynb", "mode": "insert", "index": 2, "source": "y"}),
                true,
            ),
            (
                json!({"path": "n.ipynb", "mode": "delete", "index": 0}),
                true,
            ),
            (
                json!({"path": "n.ipynb", "mode": "delete", "index": 2}),
                true,
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("n.ipynb"), &odd).unwrap();
            run(dir.path(), input.clone()).await.0.unwrap();
            let written = std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap();
            assert_eq!(
                written.contains("\"scale\": 1e-05,\n    \"big\": 123456789012345678901234567890,\n    \"name\": \"caf\\u00e9\""),
                kept,
                "{input}: {written}"
            );
            let value: Value = serde_json::from_str(&written).expect("still JSON");
            assert!(value["cells"].is_array());
            // Written as Jupyter would have written the changed cell.
            let again = Notebook::parse(&written).unwrap();
            let normal: Value = serde_json::from_str(&again.text()).unwrap();
            assert_eq!(normal, value);
        }
        // Inserting at the end and deleting the only cell keep the layout.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("n.ipynb"), JUPYTER).unwrap();
        run(
            dir.path(),
            json!({"path": "n.ipynb", "mode": "insert", "index": 2, "source": "z"}),
        )
        .await
        .0
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap();
        assert_eq!(
            Notebook::parse(&written).unwrap().text(),
            written,
            "same layout as Jupyter"
        );
        for _ in 0..4 {
            run(
                dir.path(),
                json!({"path": "n.ipynb", "mode": "delete", "index": 0}),
            )
            .await
            .0
            .unwrap();
        }
        let written = std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap();
        assert!(
            written.starts_with("{\n \"cells\": [],\n \"metadata\""),
            "{written}"
        );
        let compact =
            serde_json::to_string(&serde_json::from_str::<Value>(JUPYTER).unwrap()).unwrap();
        std::fs::write(dir.path().join("c.ipynb"), &compact).unwrap();
        run(
            dir.path(),
            json!({"path": "c.ipynb", "mode": "insert", "source": "q"}),
        )
        .await
        .0
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join("c.ipynb")).unwrap();
        assert!(!written.contains('\n'), "still one line");
        assert_eq!(
            serde_json::from_str::<Value>(&written).unwrap()["cells"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn the_layout_finds_each_cell() {
        let text = r#"{"metadata": {"a": "x]}\"", "cells": [9]}, "cells" : [ {"s": "}"} , [1,[2]], "c" ], "z": 1}"#;
        let layout = layout(text).unwrap();
        let cells: Vec<&str> = layout.cells.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(cells, [r#"{"s": "}"}"#, "[1,[2]]", "\"c\""]);
        assert_eq!(&text[layout.open..=layout.open], "[");
        assert_eq!(&text[layout.close..=layout.close], "]");
        assert!(super::layout("[]").is_none());
        assert!(super::layout("{\"cells\": 3}").is_none());
    }

    /// Review 1 (F8): two top-level `cells` keys. Every JSON reader takes the last; a splice
    /// would change the first. The view shows the raw text and the tool refuses.
    #[tokio::test]
    async fn a_notebook_with_a_key_twice_is_not_edited() {
        let twice = "{\n \"cells\": [\n  {\"cell_type\": \"code\", \"metadata\": {}, \"outputs\": [], \"source\": [\"dead = 1\"]}\n ],\n \"cell\\u0073\": [\n  {\"cell_type\": \"code\", \"metadata\": {}, \"outputs\": [], \"source\": [\"live = 1\"]}\n ],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 4\n}\n";
        assert_eq!(duplicate_key(twice).as_deref(), Some("cells"));
        assert!(layout(twice).is_none());
        assert!(
            super::view(twice.as_bytes()).is_none(),
            "read shows the text itself"
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("n.ipynb"), twice).unwrap();
        let (result, asked) = run(
            dir.path(),
            json!({"path": "n.ipynb", "index": 0, "source": "live = 2"}),
        )
        .await;
        let error = result.unwrap_err().message;
        assert!(error.contains("\"cells\" more than once"), "{error}");
        assert!(asked.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("n.ipynb")).unwrap(),
            twice
        );
        assert_eq!(duplicate_key(JUPYTER), None);
    }

    #[tokio::test]
    async fn older_notebooks_get_no_cell_ids_and_keep_string_sources() {
        let dir = tempfile::tempdir().unwrap();
        let old = "{\n \"cells\": [\n  {\n   \"cell_type\": \"code\",\n   \"execution_count\": null,\n   \"metadata\": {},\n   \"outputs\": [],\n   \"source\": \"a = 1\"\n  }\n ],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 2\n}";
        std::fs::write(dir.path().join("o.ipynb"), old).unwrap();
        run(
            dir.path(),
            json!({"path": "o.ipynb", "mode": "insert", "index": 0, "source": "b = 2\nc = 3"}),
        )
        .await
        .0
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join("o.ipynb")).unwrap();
        assert!(!written.ends_with('\n'), "no final newline added");
        let value: Value = serde_json::from_str(&written).unwrap();
        assert!(value["cells"][1].get("id").is_none());
        assert_eq!(value["cells"][1]["source"], "b = 2\nc = 3");
        assert_eq!(value["cells"][0]["source"], "a = 1");
    }
}
