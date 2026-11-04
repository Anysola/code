//! What a finished command shows the model: the tail of its output file, the notice, the metadata,
//! and the `<shell …>` notification of a background command.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Map, Value};

use super::exec::{Ended, Outcome};
use crate::tool::OutputLimits;

const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);

/// A completed command as 0.5.1's `ShellResult.output` described it.
#[derive(Debug, PartialEq)]
pub struct Finished {
    pub output: String,
    pub truncated: bool,
    pub exit: Option<i32>,
    pub signal: Option<String>,
    pub timeout: bool,
}

impl Finished {
    /// Reads the output file of an ended command; `timeout_ms` is the limit it ran with.
    pub fn read(
        file: &Path,
        ended: Ended,
        limits: OutputLimits,
        timeout_ms: u64,
    ) -> io::Result<Finished> {
        let (output, truncated) = tail(file, ended.size, ended.newlines, limits)?;
        Ok(match ended.outcome {
            Outcome::Exited { code, signal } => Finished {
                output,
                truncated,
                exit: code,
                signal,
                timeout: false,
            },
            // The timeout won the race to record the outcome, so the kill's signal is not reported.
            Outcome::TimedOut => Finished {
                output: format!(
                    "{output}\n\nCommand exceeded timeout of {timeout_ms} ms. Retry with a larger timeout if the command is expected to take longer."
                ),
                truncated,
                exit: None,
                signal: None,
                timeout: true,
            },
        })
    }

    fn notice(&self) -> Option<String> {
        if self.timeout {
            return Some("Timed out before completion".into());
        }
        if let Some(signal) = &self.signal {
            return Some(format!("Killed by {signal}"));
        }
        self.exit
            .filter(|&exit| exit != 0)
            .map(|exit| format!("Exited with code {exit}"))
    }

    /// The model-visible text blocks: the output, then the notice if any.
    pub fn content(&self) -> Vec<String> {
        std::iter::once(self.output.clone())
            .chain(self.notice())
            .collect()
    }

    pub fn metadata(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("status".into(), "completed".into());
        map.insert("truncated".into(), self.truncated.into());
        if let Some(exit) = self.exit {
            map.insert("exit".into(), exit.into());
        }
        if let Some(signal) = &self.signal {
            map.insert("signal".into(), signal.clone().into());
        }
        if self.timeout {
            map.insert("timeout".into(), true.into());
        }
        map
    }
}

/// The last `max_bytes` bytes of the output (possibly starting mid-line), then at most its last
/// `max_lines` lines, plus the `[showing lines …]` notice when anything was cut. The line numbers
/// assume the byte cut fell on a line boundary, as 0.5.1's did.
fn tail(file: &Path, size: u64, newlines: u64, limits: OutputLimits) -> io::Result<(String, bool)> {
    let start = size.saturating_sub(limits.max_bytes as u64);
    let mut page = Vec::new();
    if size > 0 {
        let mut reader = File::open(file)?;
        reader.seek(SeekFrom::Start(start))?;
        reader.take(size - start).read_to_end(&mut page)?;
    }
    let page = String::from_utf8_lossy(&page);
    let mut lines: Vec<&str> = page.split('\n').collect();
    let ends_with_newline = page.ends_with('\n');
    if ends_with_newline {
        lines.pop();
    }
    let truncated = size > limits.max_bytes as u64 || lines.len() > limits.max_lines;
    let text = if lines.len() > limits.max_lines {
        lines[lines.len() - limits.max_lines..].join("\n")
    } else {
        page.to_string()
    };
    let text = if text.is_empty() {
        "(no output)".to_string()
    } else {
        text
    };
    if !truncated {
        return Ok((text, false));
    }
    let total = newlines + u64::from(!ends_with_newline);
    let shown = lines.len().min(limits.max_lines) as u64;
    let first = total as i64 - shown as i64 + 1;
    Ok((
        format!(
            "{text}\n\n[showing lines {first}-{total} of {total}; full output saved to {}]",
            file.display()
        ),
        true,
    ))
}

/// The synthetic message posted when a background command ends. The command is not escaped.
pub fn notification(id: &str, state: &str, command: &str, text: &str) -> String {
    format!("<shell id=\"{id}\" state=\"{state}\" command=\"{command}\">\n{text}\n</shell>")
}

/// Deletes output files older than 7 days in every project under `root` (`<data>/shell`), at most
/// once an hour per process.
pub fn sweep(root: &Path) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| at.elapsed() < SWEEP_EVERY) {
            return;
        }
        *last = Some(Instant::now());
    }
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || remove_expired(&root, SystemTime::now()));
}

fn remove_expired(root: &Path, now: SystemTime) {
    let Some(cutoff) = now.checked_sub(RETENTION) else {
        return;
    };
    let projects = std::fs::read_dir(root).into_iter().flatten().flatten();
    for project in projects.filter(|p| p.file_type().is_ok_and(|t| t.is_dir())) {
        let files = std::fs::read_dir(project.path())
            .into_iter()
            .flatten()
            .flatten();
        for file in files.filter(|f| is_output_file(&f.file_name().to_string_lossy())) {
            let expired = file
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|modified| modified < cutoff);
            if expired && file.file_type().is_ok_and(|t| t.is_file()) {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }
}

/// `sh_[0-9a-f]{12}.*\.out`
fn is_output_file(name: &str) -> bool {
    name.strip_prefix("sh_").is_some_and(|rest| {
        rest.len() >= 16
            && rest.as_bytes()[..12]
                .iter()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            && rest.ends_with(".out")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_lines: usize, max_bytes: usize) -> OutputLimits {
        OutputLimits {
            max_lines,
            max_bytes,
        }
    }

    fn tail_of(content: &str, limits: OutputLimits) -> (String, bool, String) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sh_x.out");
        std::fs::write(&file, content).unwrap();
        let newlines = content.matches('\n').count() as u64;
        let (text, truncated) = tail(&file, content.len() as u64, newlines, limits).unwrap();
        (text, truncated, file.display().to_string())
    }

    #[test]
    fn small_output_is_kept() {
        assert_eq!(tail_of("hi\nerr\n", OutputLimits::default()).0, "hi\nerr\n");
        assert_eq!(tail_of("", OutputLimits::default()).0, "(no output)");
    }

    #[test]
    fn line_limit_keeps_the_tail() {
        let content: String = (1..=10).map(|i| format!("{i}\n")).collect();
        let (text, truncated, file) = tail_of(&content, limits(3, 51200));
        assert!(truncated);
        assert_eq!(
            text,
            format!("8\n9\n10\n\n[showing lines 8-10 of 10; full output saved to {file}]")
        );
    }

    #[test]
    fn byte_limit_cut_mid_line() {
        // 0.5.1's captured case scaled down: the byte cut lands inside the first kept line.
        let content = format!("{}\ntail\n", "a".repeat(100));
        let (text, truncated, file) = tail_of(&content, limits(2000, 20));
        assert!(truncated);
        assert_eq!(
            text,
            format!(
                "{}\ntail\n\n\n[showing lines 1-2 of 2; full output saved to {file}]",
                "a".repeat(14)
            )
        );
    }

    #[test]
    fn notices_and_metadata() {
        let finished = |exit, signal: Option<&str>, timeout| Finished {
            output: "x".into(),
            truncated: false,
            exit,
            signal: signal.map(String::from),
            timeout,
        };
        let ok = finished(Some(0), None, false);
        assert_eq!(ok.content(), ["x"]);
        assert_eq!(
            serde_json::to_string(&ok.metadata()).unwrap(),
            r#"{"status":"completed","truncated":false,"exit":0}"#
        );
        assert_eq!(
            finished(Some(3), None, false).content(),
            ["x", "Exited with code 3"]
        );
        let killed = finished(None, Some("SIGTERM"), false);
        assert_eq!(killed.content(), ["x", "Killed by SIGTERM"]);
        assert_eq!(
            serde_json::to_string(&killed.metadata()).unwrap(),
            r#"{"status":"completed","truncated":false,"signal":"SIGTERM"}"#
        );
        let timed_out = finished(None, None, true);
        assert_eq!(timed_out.content(), ["x", "Timed out before completion"]);
        assert_eq!(
            serde_json::to_string(&timed_out.metadata()).unwrap(),
            r#"{"status":"completed","truncated":false,"timeout":true}"#
        );
    }

    #[test]
    fn notification_text() {
        assert_eq!(
            notification(
                "sh_1",
                "completed",
                "echo started; sleep 1; echo finished; exit 4",
                "started\nfinished\n\n\nExited with code 4"
            ),
            "<shell id=\"sh_1\" state=\"completed\" command=\"echo started; sleep 1; echo finished; exit 4\">\nstarted\nfinished\n\n\nExited with code 4\n</shell>"
        );
    }

    #[test]
    fn retention() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("global");
        std::fs::create_dir_all(&project).unwrap();
        let old = project.join("sh_0123456789abXYZ.out");
        let fresh = project.join("sh_0123456789acXYZ.out");
        let other = project.join("notes.out");
        for file in [&old, &fresh, &other] {
            std::fs::write(file, "x").unwrap();
        }
        let eight_days_ago = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
        for file in [&old, &other] {
            File::options()
                .write(true)
                .open(file)
                .unwrap()
                .set_modified(eight_days_ago)
                .unwrap();
        }
        remove_expired(dir.path(), SystemTime::now());
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists());
        assert!(!is_output_file("sh_0123456789AB.out"));
    }
}
