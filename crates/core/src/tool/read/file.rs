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
/// Bytes of one line a page keeps: far past what [`clamp`] shows of it (2000 UTF-16 units take at
/// most 6000 bytes), so the output is the same while a single-line file of hundreds of megabytes
/// is never held whole.
const MAX_LINE_BYTES: usize = MAX_BYTES;
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
        let (read, dropped_nul) = read_line(&mut reader, &mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        binary |= dropped_nul;
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

/// One line into `buffer` like `read_until(b'\n')`, keeping at most [`MAX_LINE_BYTES`] of it; the
/// rest is read past. The bytes read, and whether the part not kept held a NUL.
fn read_line(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> std::io::Result<(usize, bool)> {
    let (mut read, mut nul) = (0, false);
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok((read, nul));
        }
        let (line, ended) = match available.iter().position(|&b| b == b'\n') {
            Some(end) => (&available[..=end], true),
            None => (available, false),
        };
        let kept = line.len().min(MAX_LINE_BYTES.saturating_sub(buffer.len()));
        buffer.extend_from_slice(&line[..kept]);
        nul |= line[kept..].contains(&0);
        let used = line.len();
        reader.consume(used);
        read += used;
        if ended {
            return Ok((read, nul));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_line_keeps_a_capped_prefix() {
        let mut input = vec![b'x'; 4 * MAX_LINE_BYTES];
        input.push(0);
        input.extend_from_slice(b"\nnext\n");
        let mut reader = BufReader::with_capacity(1024, Cursor::new(input));
        let mut buffer = Vec::new();
        assert_eq!(
            read_line(&mut reader, &mut buffer).unwrap(),
            (4 * MAX_LINE_BYTES + 2, true)
        );
        assert_eq!(buffer.len(), MAX_LINE_BYTES);
        buffer.clear();
        assert_eq!(read_line(&mut reader, &mut buffer).unwrap(), (5, false));
        assert_eq!(buffer, b"next\n");
        buffer.clear();
        assert_eq!(read_line(&mut reader, &mut buffer).unwrap(), (0, false));
    }
}
