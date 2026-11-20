//! A command's output while it runs. The core names the file a foreground command writes to
//! (`Event::ToolOutputFile`); this reads what was added to it every tick and sends the window the
//! line count and the last few lines of each command that grew. Display only: the call's result
//! is still the stored output.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use serde_json::{Value, json};

/// Lines the window shows under a running command.
const SHOWN: usize = 6;
/// What is kept of the output's end to find those lines in.
const KEPT: usize = 8 * 1024;
/// At most this much new output is read per command per tick; a flood is skipped over.
const READ: u64 = 1024 * 1024;

struct Tail {
    session_id: String,
    file: PathBuf,
    /// Bytes of the file read so far.
    offset: u64,
    newlines: usize,
    /// The end of the output so far (bytes; lossy only when shown).
    end: Vec<u8>,
    /// Grew since the window last heard.
    dirty: bool,
}

#[derive(Default)]
pub struct Tails {
    calls: HashMap<String, Tail>,
}

impl Tails {
    pub fn start(&mut self, session_id: &str, call_id: &str, file: PathBuf) {
        self.calls.insert(
            call_id.to_string(),
            Tail {
                session_id: session_id.to_string(),
                file,
                offset: 0,
                newlines: 0,
                end: Vec::new(),
                dirty: false,
            },
        );
    }

    pub fn end_call(&mut self, call_id: &str) {
        self.calls.remove(call_id);
    }

    pub fn end_session(&mut self, session_id: &str) {
        self.calls.retain(|_, t| t.session_id != session_id);
    }

    pub fn active(&self) -> bool {
        !self.calls.is_empty()
    }

    /// Reads what each file gained; one snapshot per command that grew.
    pub fn poll(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        for (call_id, tail) in &mut self.calls {
            tail.read();
            if tail.dirty {
                tail.dirty = false;
                out.push(json!({
                    "type": "toolOutput",
                    "sessionId": tail.session_id,
                    "callId": call_id,
                    "lines": tail.lines(),
                    "tail": last_lines(&tail.end, SHOWN),
                }));
            }
        }
        out
    }
}

impl Tail {
    fn read(&mut self) {
        let Ok(mut file) = File::open(&self.file) else {
            return;
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return;
        };
        if len <= self.offset {
            return;
        }
        // A flood: what is skipped isn't counted, the end is what shows.
        if len - self.offset > READ {
            self.offset = len - READ;
            self.end.clear();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut added = Vec::new();
        if file
            .take(len - self.offset)
            .read_to_end(&mut added)
            .is_err()
        {
            return;
        }
        self.offset += added.len() as u64;
        self.newlines += added.iter().filter(|&&b| b == b'\n').count();
        self.end.extend_from_slice(&added);
        if self.end.len() > KEPT {
            self.end.drain(..self.end.len() - KEPT);
        }
        self.dirty = !added.is_empty();
    }

    fn lines(&self) -> usize {
        self.newlines + usize::from(!self.end.is_empty() && !self.end.ends_with(b"\n"))
    }
}

/// The last `n` lines as a terminal would show them: a `\r` rewrites its line (progress bars),
/// colour and cursor codes are dropped, blank lines at the end don't count.
pub fn last_lines(bytes: &[u8], n: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = strip_ansi(&text);
    let lines: Vec<&str> = text
        .trim_end_matches(['\n', '\r', ' ', '\t'])
        .split('\n')
        .map(|line| {
            let line = line.trim_end_matches('\r');
            line.rsplit('\r').next().unwrap_or(line)
        })
        .collect();
    // The first line may be cut by what was kept; with more than enough lines it is left out.
    let from = lines.len().saturating_sub(n);
    lines[from..].join("\n")
}

/// `ESC [ … letter`, `ESC ] … BEL/ST` and lone escapes removed.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn last_lines_as_a_terminal_shows_them() {
        assert_eq!(last_lines(b"a\nb\nc\n", 2), "b\nc");
        assert_eq!(last_lines(b"10%\r50%\r100%\ndone\n\n", 6), "100%\ndone");
        assert_eq!(
            last_lines(b"\x1b[32mok\x1b[0m \x1b]0;title\x07x\r\n", 6),
            "ok x"
        );
        assert_eq!(last_lines("ç\u{fffd}".as_bytes(), 6), "ç\u{fffd}");
        assert_eq!(last_lines(b"", 6), "");
    }

    #[test]
    fn grows_with_the_file_and_counts_lines() {
        let path = std::env::temp_dir().join(format!("anysola-tail-{}.out", std::process::id()));
        let mut file = File::create(&path).unwrap();
        let mut tails = Tails::default();
        tails.start("s", "c", path.clone());
        assert!(tails.poll().is_empty());
        file.write_all(b"one\ntwo").unwrap();
        let got = tails.poll();
        assert_eq!(got[0]["lines"], 2);
        assert_eq!(got[0]["tail"], "one\ntwo");
        assert!(tails.poll().is_empty(), "nothing new, nothing sent");
        file.write_all(b"\nthree\n").unwrap();
        let got = tails.poll();
        assert_eq!(got[0]["lines"], 3);
        assert_eq!(got[0]["tail"], "one\ntwo\nthree");
        tails.end_session("s");
        assert!(!tails.active());
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_sends_nothing() {
        let mut tails = Tails::default();
        tails.start("s", "c", PathBuf::from("/nonexistent/anysola/x.out"));
        assert!(tails.poll().is_empty());
    }
}
