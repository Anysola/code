//! The prompt of a run: message arguments, stdin, and `--file` attachments.

use std::io::{IsTerminal, Read};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;

const ATTACH_MAX_BYTES: u64 = 10 * 1024 * 1024;
const STDIN_WAIT: Duration = Duration::from_secs(3);

/// Message arguments joined by single spaces.
pub fn message(args: &[String]) -> Option<String> {
    let joined = args.join(" ");
    (!joined.is_empty()).then_some(joined)
}

pub fn merge(message: Option<String>, piped: Option<String>) -> Option<String> {
    let piped = piped.filter(|p| !p.is_empty());
    match (message, piped) {
        (None, piped) => piped,
        (Some(message), None) => Some(message),
        (Some(message), Some(piped)) => Some(format!("{message}\n{piped}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdinKind {
    Terminal,
    /// A regular file or a character device such as `/dev/null`: readable right away.
    Immediate,
    /// A pipe, FIFO or socket that may never close.
    Stream,
}

fn stdin_kind() -> StdinKind {
    if std::io::stdin().is_terminal() {
        return StdinKind::Terminal;
    }
    #[cfg(unix)]
    {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(0, &mut stat) } == 0 {
            let kind = stat.st_mode & libc::S_IFMT;
            if kind == libc::S_IFREG || kind == libc::S_IFCHR {
                return StdinKind::Immediate;
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_TYPE_CHAR, FILE_TYPE_DISK, GetFileType,
        };
        // SAFETY: GetFileType only inspects the handle, which stdin keeps open.
        let kind = unsafe { GetFileType(std::io::stdin().as_raw_handle()) };
        if kind == FILE_TYPE_DISK || kind == FILE_TYPE_CHAR {
            return StdinKind::Immediate;
        }
    }
    StdinKind::Stream
}

fn read_all() -> String {
    let mut bytes = Vec::new();
    let _ = std::io::stdin().lock().read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// stdin is read only when it is not a terminal: fully when there is no message; with a message,
/// a pipe is read only if its first byte (or EOF) arrives within 3 s.
pub fn stdin(has_message: bool) -> Option<String> {
    match stdin_kind() {
        StdinKind::Terminal => None,
        StdinKind::Immediate => Some(read_all()),
        StdinKind::Stream if !has_message => Some(read_all()),
        StdinKind::Stream => {
            let (first_sender, first) = mpsc::channel::<()>();
            let (done_sender, done) = mpsc::channel::<Vec<u8>>();
            std::thread::spawn(move || {
                let mut stdin = std::io::stdin().lock();
                let mut byte = [0u8; 1];
                let read = stdin.read(&mut byte).unwrap_or(0);
                let _ = first_sender.send(());
                let mut bytes = byte[..read].to_vec();
                if read > 0 {
                    let _ = stdin.read_to_end(&mut bytes);
                }
                let _ = done_sender.send(bytes);
            });
            match first.recv_timeout(STDIN_WAIT) {
                Ok(()) => done
                    .recv()
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                Err(_) => {
                    crate::ui::println("No input on stdin after 3s; continuing without it.");
                    None
                }
            }
        }
    }
}

/// A `--file` attachment: text files are inlined into the message, media go as attachments.
pub enum Attachment {
    Text(String),
    Media {
        mime: String,
        data: String,
        name: String,
    },
}

fn mime_by_extension(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "tif" | "tiff" => "image/tiff",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }
    let control = bytes
        .iter()
        .filter(|b| **b < 9 || (**b > 13 && **b < 32))
        .count();
    control as f64 / bytes.len() as f64 > 0.3
}

pub fn attach(input: &str, directory: &Path) -> Result<Attachment, String> {
    let path = directory.join(input);
    // Metadata first: Windows cannot open a directory as a file.
    let meta = std::fs::metadata(&path).map_err(|_| format!("File not found: {input}"))?;
    if !meta.is_file() || meta.len() > ATTACH_MAX_BYTES {
        return Err(format!(
            "Cannot attach a directory, special file, or file larger than 10 MiB: {input}"
        ));
    }
    let bytes = std::fs::read(&path).map_err(|_| format!("File not found: {input}"))?;
    let detected = mime_by_extension(&path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let media = detected.starts_with("image/") || detected == "application/pdf";
    if !media
        && !binary(&bytes)
        && let Ok(text) = String::from_utf8(bytes.clone())
    {
        return Ok(Attachment::Text(format!(
            "<file name=\"{name}\">\n{text}\n</file>"
        )));
    }
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Attachment::Media {
        mime: detected.to_string(),
        data,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_and_merge() {
        assert_eq!(
            message(&["a".into(), "b c".into()]).as_deref(),
            Some("a b c")
        );
        assert_eq!(
            merge(Some("m".into()), Some("p".into())).as_deref(),
            Some("m\np")
        );
        assert_eq!(merge(None, Some(String::new())), None);
    }

    #[test]
    fn attachments() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hi\n").unwrap();
        std::fs::write(dir.path().join("pixel.png"), [0x89, b'P', b'N', b'G']).unwrap();
        match attach("note.txt", dir.path()).unwrap() {
            Attachment::Text(text) => assert_eq!(text, "<file name=\"note.txt\">\nhi\n\n</file>"),
            Attachment::Media { .. } => panic!("text expected"),
        }
        assert!(matches!(
            attach("pixel.png", dir.path()).unwrap(),
            Attachment::Media { .. }
        ));
        assert_eq!(
            attach("missing", dir.path()).err().unwrap(),
            "File not found: missing"
        );
        assert!(
            attach(".", dir.path())
                .err()
                .unwrap()
                .starts_with("Cannot attach a directory")
        );
    }
}
