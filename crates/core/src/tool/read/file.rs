//! What read sees on disk: media by magic bytes, whole small text files, line pages of larger ones,
//! and directory listings. Blocking; the tool runs it on the blocking pool.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Cursor, Read};
use std::path::{MAIN_SEPARATOR, Path};

use base64::Engine;

use crate::text::{utf16_len, utf16_prefix};

const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;
pub(super) const MAX_MEDIA_BYTES: u64 = 20 * 1024 * 1024;
const FIRST_CHUNK: u64 = 256 * 1024;
const MAX_LINE_LENGTH: usize = 2000;
const BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Page {
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

impl Page {
    /// 1-based; 0 counts as 1.
    pub fn start(&self) -> usize {
        match self.offset {
            Some(0) | None => 1,
            Some(offset) => usize::try_from(offset).unwrap_or(usize::MAX),
        }
    }

    /// 0 or absent means the cap.
    fn limit(&self) -> usize {
        match self.limit {
            Some(0) | None => MAX_LINES,
            Some(limit) => usize::try_from(limit).map_or(MAX_LINES, |l| l.min(MAX_LINES)),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Content {
    Media {
        mime: &'static str,
        base64: String,
    },
    /// A whole small file: every line clamped, `\r` kept.
    Text(String),
    /// Selected lines joined by `\n`; `next` is the line to continue from.
    TextPage {
        text: String,
        offset: usize,
        next: Option<usize>,
    },
    Listing {
        entries: Vec<String>,
        next: Option<usize>,
    },
}

impl Content {
    pub fn truncated(&self) -> bool {
        match self {
            Content::Media { .. } | Content::Text(_) => false,
            Content::TextPage { next, .. } | Content::Listing { next, .. } => next.is_some(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum ReadError {
    NotFound,
    Failed,
    Binary,
    MediaLimit,
    OffsetOutOfRange { offset: usize, lines: usize },
    NotFileOrDirectory,
}

fn io_error(error: std::io::Error) -> ReadError {
    if crate::tool::edit::mutation::is_missing(&error) {
        ReadError::NotFound
    } else {
        ReadError::Failed
    }
}

pub(super) fn read(path: &Path, page: Page) -> Result<Content, ReadError> {
    let meta = fs::metadata(path).map_err(io_error)?;
    if meta.is_dir() {
        return list(path, page);
    }
    if !meta.is_file() {
        return Err(ReadError::NotFileOrDirectory);
    }
    let mut file = File::open(path).map_err(io_error)?;
    let mut first = Vec::new();
    (&mut file)
        .take(FIRST_CHUNK)
        .read_to_end(&mut first)
        .map_err(io_error)?;
    if let Some(mime) = media_mime(&first) {
        if meta.len() > MAX_MEDIA_BYTES {
            return Err(ReadError::MediaLimit);
        }
        first.clear();
        File::open(path)
            .and_then(|mut f| f.read_to_end(&mut first))
            .map_err(io_error)?;
        let base64 = base64::engine::general_purpose::STANDARD.encode(&first);
        return Ok(Content::Media { mime, base64 });
    }
    let paged = meta.len() > MAX_BYTES as u64 || page.offset.is_some() || page.limit.is_some();
    if !paged {
        if first.contains(&0) {
            return Err(ReadError::Binary);
        }
        let decoded = String::from_utf8_lossy(first.strip_prefix(BOM).unwrap_or(&first));
        let lines: Vec<String> = decoded.split('\n').map(clamp).collect();
        return Ok(Content::Text(lines.join("\n")));
    }
    text_page(BufReader::new(Cursor::new(first).chain(file)), page)
}

/// PNG, JPEG, GIF, WebP and PDF are the readable media; anything else (BMP included) is text.
fn media_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn clamp(line: &str) -> String {
    if utf16_len(line) > MAX_LINE_LENGTH {
        format!(
            "{}... (line truncated to {MAX_LINE_LENGTH} chars)",
            utf16_prefix(line, MAX_LINE_LENGTH)
        )
    } else {
        line.to_string()
    }
}

/// Lines from `page.start()`, stopping at the line cap or before 51200 bytes of output (clamped
/// lines plus separators). Only bytes up to the last selected line count for binary detection.
fn text_page(mut reader: impl BufRead, page: Page) -> Result<Content, ReadError> {
    let (offset, limit) = (page.start(), page.limit());
    let mut entries: Vec<String> = Vec::new();
    let (mut size, mut number, mut binary) = (0, 0, false);
    let mut next = None;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer).map_err(io_error)? == 0 {
            break;
        }
        number += 1;
        if number >= offset {
            if entries.len() >= limit || size >= MAX_BYTES {
                next = Some(number);
                break;
            }
            let text = clamp(&decode_line(&buffer, number == 1));
            let line_size = text.len() + usize::from(!entries.is_empty());
            if size + line_size > MAX_BYTES {
                next = Some(number);
                break;
            }
            entries.push(text);
            size += line_size;
        }
        binary |= buffer.contains(&0);
    }
    if binary {
        return Err(ReadError::Binary);
    }
    if entries.is_empty() && offset != 1 {
        return Err(ReadError::OffsetOutOfRange {
            offset,
            lines: number,
        });
    }
    Ok(Content::TextPage {
        text: entries.join("\n"),
        offset,
        next,
    })
}

/// One line without its `\n` and one trailing `\r`; the file's first line also drops a BOM.
fn decode_line(bytes: &[u8], first: bool) -> String {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = if first {
        bytes.strip_prefix(BOM).unwrap_or(bytes)
    } else {
        bytes
    };
    let text = String::from_utf8_lossy(bytes);
    text.strip_suffix('\r').unwrap_or(&text).to_string()
}

fn list(path: &Path, page: Page) -> Result<Content, ReadError> {
    let mut visible: Vec<(bool, String)> = Vec::new();
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let kind = entry.file_type().map_err(io_error)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() {
            visible.push((true, format!("{name}{MAIN_SEPARATOR}")));
        } else if kind.is_file() || kind.is_symlink() {
            visible.push((false, name));
        }
    }
    visible.sort_by(|(a_dir, a), (b_dir, b)| {
        b_dir
            .cmp(a_dir)
            .then_with(|| crate::text::locale_compare(a, b))
    });
    let (offset, limit) = (page.start(), page.limit());
    let total = visible.len();
    let entries: Vec<String> = visible
        .into_iter()
        .skip(offset - 1)
        .take(limit)
        .map(|(_, name)| name)
        .collect();
    let shown = (offset - 1).saturating_add(entries.len());
    let next = (shown < total).then_some(offset + entries.len());
    Ok(Content::Listing { entries, next })
}
