//! The `read` tool: files (text, images, PDFs) and directories, with filename recovery, not-found
//! suggestions and discovery of nested AGENTS.md files.

mod file;

use std::path::{Path, PathBuf};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;

use super::access::{self, Kind, Target};
use super::edit::mutation::is_missing;
use super::{
    DefinitionEnv, Tool, ToolContent, ToolContext, ToolDefinition, ToolError, ToolOutput,
    ToolResult, parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;
use file::{Content, Page, ReadError};

embedded_def!(def, "read");

const INSTRUCTIONS_FILE: &str = "AGENTS.md";

pub struct ReadTool;

#[derive(Deserialize)]
struct Input {
    path: String,
    offset: Option<u64>,
    limit: Option<u64>,
}

impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
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
            read(&input, ctx).await
        }
        .boxed()
    }
}

async fn read(input: &Input, ctx: &ToolContext) -> ToolResult {
    let page = Page {
        offset: input.offset,
        limit: input.limit,
    };
    let requested = access::authorize_read(ctx, &input.path).await?;
    let (content, target, shown) = match read_blocking(&requested.absolute, page).await {
        Err(ReadError::NotFound) => recover(ctx, &input.path, &requested, page).await?,
        result => {
            let content = result.map_err(|e| read_error(e, &requested.resource, &input.path))?;
            (content, requested, input.path.clone())
        }
    };
    let mut output = ToolOutput {
        content: model_content(&shown, page, &content),
        metadata: Default::default(),
    }
    .with_metadata("truncated", content.truncated());
    if target.external.is_none() {
        let listing = matches!(content, Content::Listing { .. });
        let found = instructions(&ctx.directory, &target.absolute, listing).await;
        if !found.is_empty() {
            output = output.with_metadata("instructions", found);
        }
    }
    Ok(output)
}

async fn read_blocking(path: &Path, page: Page) -> Result<Content, ReadError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || file::read(&path, page))
        .await
        .unwrap_or(Err(ReadError::Failed))
}

fn read_error(error: ReadError, resource: &str, input: &str) -> ToolError {
    match error {
        ReadError::NotFound | ReadError::Failed => {
            ToolError::execution(format!("Unable to read {input}"))
        }
        ReadError::Binary => ToolError::unknown(format!("Cannot read binary file: {resource}")),
        ReadError::MediaLimit => ToolError::unknown(format!(
            "Media exceeds {} byte ingestion limit: {resource}",
            file::MAX_MEDIA_BYTES
        )),
        ReadError::OffsetOutOfRange { offset, lines } => ToolError::unknown(format!(
            "Offset {offset} is out of range for this file ({lines} {})",
            if lines == 1 { "line" } else { "lines" }
        )),
        ReadError::NotFileOrDirectory => {
            ToolError::unknown(format!("Path is not a file or directory: {resource}"))
        }
    }
}

/// Reads the one sibling whose name matches after folding the space and quote variants that
/// screenshots and pasted names carry. It shares the directory approval but asks `read` itself.
async fn recover(
    ctx: &ToolContext,
    input: &str,
    requested: &Target,
    page: Page,
) -> Result<(Content, Target, String), ToolError> {
    let Some(name) = alternate_name(&requested.absolute).await else {
        return Err(missing(input, &requested.absolute).await);
    };
    let alternate = requested.absolute.with_file_name(&name);
    let target = access::resolve(ctx, &alternate.to_string_lossy(), Some(Kind::File));
    ctx.ask(PermissionRequest {
        action: "read".into(),
        resources: vec![target.resource.clone()],
        save: vec!["*".into()],
        ..Default::default()
    })
    .await?;
    match read_blocking(&target.absolute, page).await {
        Err(ReadError::NotFound) | Ok(Content::Listing { .. }) => {
            Err(missing(input, &requested.absolute).await)
        }
        Err(error) => Err(read_error(error, &target.resource, input)),
        Ok(content) => Ok((content, target, js_join(&js_dirname(input), &name))),
    }
}

/// Unicode NFC is not applied (no normalization crate); the space and quote folds are.
fn canonical_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\u{A0}' | '\u{202F}' => ' ',
            '\u{2018}' | '\u{2019}' => '\'',
            c => c,
        })
        .collect()
}

async fn alternate_name(absolute: &Path) -> Option<String> {
    let wanted = canonical_name(&absolute.file_name()?.to_string_lossy());
    let mut entries = tokio::fs::read_dir(absolute.parent()?).await.ok()?;
    let mut matches = Vec::new();
    while let Some(entry) = entries.next_entry().await.ok()? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().await.ok()?.is_file() && canonical_name(&name) == wanted {
            matches.push(name);
        }
    }
    (matches.len() == 1).then(|| matches.remove(0))
}

enum Listing {
    Found(Vec<String>),
    Missing,
    Unknown,
}

/// Entry names of a directory, sorted bytewise for stable suggestions.
async fn listing(path: &Path) -> Listing {
    let classify = |error: std::io::Error| {
        if is_missing(&error) {
            Listing::Missing
        } else {
            Listing::Unknown
        }
    };
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Listing::Unknown,
        Err(error) => return classify(error),
    }
    let mut entries = match tokio::fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(error) => return classify(error),
    };
    let mut names = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => names.push(entry.file_name().to_string_lossy().into_owned()),
            Ok(None) => break,
            Err(_) => return Listing::Unknown,
        }
    }
    names.sort();
    Listing::Found(names)
}

/// Up to three names containing, or contained in, `name` (case-insensitive).
fn similar<'a>(names: &'a [String], name: &str) -> impl Iterator<Item = &'a String> {
    let base = name.to_lowercase();
    names
        .iter()
        .filter(move |entry| {
            let candidate = entry.to_lowercase();
            candidate.contains(&base) || base.contains(&candidate)
        })
        .take(3)
}

fn suggestions(lines: Vec<String>) -> Option<String> {
    (!lines.is_empty()).then(|| format!("Did you mean one of these?\n{}", lines.join("\n")))
}

async fn missing(input: &str, absolute: &Path) -> ToolError {
    let not_found = format!("File not found: {input}");
    let Some(folder) = absolute.parent() else {
        return ToolError::execution(not_found);
    };
    let names = match listing(folder).await {
        Listing::Missing => return missing_folder(not_found, folder).await,
        Listing::Found(names) => names,
        Listing::Unknown => Vec::new(),
    };
    let dir = js_dirname(input);
    let lines = similar(&names, &js_basename(input))
        .map(|entry| js_join(&dir, entry))
        .collect();
    match suggestions(lines) {
        Some(suggestions) => ToolError::execution(format!("{not_found}\n\n{suggestions}")),
        None => ToolError::execution(not_found),
    }
}

/// Names the first missing folder and its nearest existing parent: usually a removed worktree, a
/// moved directory, or a relative path resolved from the wrong place.
async fn missing_folder(not_found: String, folder: &Path) -> ToolError {
    let mut child = folder;
    while let Some(parent) = child.parent() {
        match listing(parent).await {
            Listing::Unknown => break,
            Listing::Missing => child = parent,
            Listing::Found(names) => {
                let base = child
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let lines = similar(&names, &base)
                    .map(|entry| parent.join(entry).to_string_lossy().into_owned())
                    .collect();
                let mut parts = vec![
                    not_found,
                    format!(
                        "The folder {} does not exist. The nearest folder that does is {}.",
                        child.display(),
                        parent.display()
                    ),
                ];
                parts.extend(suggestions(lines));
                return ToolError::execution(parts.join("\n\n"));
            }
        }
    }
    ToolError::execution(not_found)
}

/// Nested AGENTS.md files between the read path and the session directory (exclusive), nearest
/// first, for the session to load as instructions. A listing starts at the directory itself.
async fn instructions(directory: &Path, absolute: &Path, listing: bool) -> Vec<String> {
    let resolved = canonical(absolute).await;
    let root = canonical(directory).await;
    let start = if listing {
        Some(resolved.as_path())
    } else {
        resolved.parent()
    };
    let mut found = Vec::new();
    let mut current = start;
    while let Some(dir) = current {
        let candidate = dir.join(INSTRUCTIONS_FILE);
        if tokio::fs::metadata(&candidate)
            .await
            .is_ok_and(|meta| meta.is_file())
        {
            found.push(candidate);
        }
        if dir == root {
            break;
        }
        current = dir.parent();
    }
    let mut result = Vec::new();
    for file in found {
        let file = canonical(&file).await;
        // The session directory and its ancestors are covered by the initial instructions.
        let covered = file.parent().is_some_and(|dir| root.starts_with(dir));
        if !covered && file != resolved {
            result.push(file.to_string_lossy().into_owned());
        }
    }
    result
}

async fn canonical(path: &Path) -> PathBuf {
    tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf())
}

fn model_content(path: &str, page: Page, content: &Content) -> Vec<ToolContent> {
    let text = match content {
        Content::Media { mime, base64 } => {
            let label = if *mime == "application/pdf" {
                "PDF read successfully"
            } else {
                "Image read successfully"
            };
            return vec![
                ToolContent::text(label),
                ToolContent::File {
                    uri: format!("data:{mime};base64,{base64}"),
                    mime: (*mime).to_string(),
                    name: Some(path.to_string()),
                },
            ];
        }
        Content::Listing { entries, next } => {
            let start = page.start();
            let mut lines = vec![if entries.is_empty() {
                format!("Read directory {path}, 0 entries")
            } else {
                format!(
                    "Read directory {path}, entries {start}-{}",
                    start + entries.len() - 1
                )
            }];
            lines.extend(entries.iter().cloned());
            lines.extend(next.map(continuation));
            lines.join("\n")
        }
        Content::Text(text) => file_text(path, 1, text, text.strip_suffix('\n'), None),
        Content::TextPage { text, offset, next } => file_text(path, *offset, text, None, *next),
    };
    vec![ToolContent::text(text)]
}

/// Numbered lines. A whole file's final newline does not start another line; a page's text is
/// already its selected lines.
fn file_text(
    path: &str,
    start: usize,
    text: &str,
    trimmed: Option<&str>,
    next: Option<usize>,
) -> String {
    let lines: Vec<&str> = if text.is_empty() {
        Vec::new()
    } else {
        trimmed.unwrap_or(text).split('\n').collect()
    };
    let mut out = vec![if lines.is_empty() {
        format!("Read file {path}, 0 lines")
    } else {
        format!(
            "Read file {path}, lines {start}-{}",
            start + lines.len() - 1
        )
    }];
    out.extend(
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| format!("{}: {line}", start + index)),
    );
    out.extend(next.map(continuation));
    out.join("\n")
}

fn continuation(next: usize) -> String {
    format!("[Output truncated. Continue reading with offset: {next}]")
}

/// Node's `path.dirname` for `/`-separated paths.
fn js_dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "." } else { "/" }.to_string();
    }
    match trimmed.rfind('/') {
        None => ".".to_string(),
        Some(index) => {
            let dir = trimmed[..index].trim_end_matches('/');
            if dir.is_empty() { "/" } else { dir }.to_string()
        }
    }
}

/// Node's `path.basename` for `/`-separated paths.
fn js_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or_default().to_string()
}

/// Node's `path.join(dir, name)`, which also normalizes `.` and `..` segments.
fn js_join(dir: &str, name: &str) -> String {
    let joined = format!("{dir}/{name}");
    let absolute = joined.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            ".." if absolute => {}
            segment => parts.push(segment),
        }
    }
    let body = parts.join("/");
    if absolute {
        format!("/{body}")
    } else if body.is_empty() {
        ".".to_string()
    } else {
        body
    }
}

#[cfg(test)]
mod tests;
