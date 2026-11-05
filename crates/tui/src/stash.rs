//! Stashed prompts, kept in `~/.anysola/state/prompt-stash.jsonl` as `{prompt, timestamp}`
//! lines (oldest first, at most 50) like 0.5.1's `<state>/prompt-stash.jsonl`.

use std::path::{Path, PathBuf};

use serde_json::json;

const LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub prompt: String,
    pub timestamp: u64,
}

#[derive(Debug, Default)]
pub struct Stash {
    path: Option<PathBuf>,
    entries: Vec<Entry>,
}

impl Stash {
    pub fn load(path: &Path) -> Stash {
        let entries: Vec<Entry> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|v| {
                Some(Entry {
                    prompt: v.get("prompt")?.as_str()?.to_string(),
                    timestamp: v.get("timestamp").and_then(|t| t.as_u64()).unwrap_or(0),
                })
            })
            .collect();
        let skip = entries.len().saturating_sub(LIMIT);
        Stash {
            path: Some(path.to_path_buf()),
            entries: entries.into_iter().skip(skip).collect(),
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn push(&mut self, prompt: String, timestamp: u64) {
        self.entries.push(Entry { prompt, timestamp });
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
        self.save();
    }

    pub fn pop(&mut self) -> Option<String> {
        let entry = self.entries.pop()?;
        self.save();
        Some(entry.prompt)
    }

    pub fn remove(&mut self, index: usize) -> Option<String> {
        if index >= self.entries.len() {
            return None;
        }
        let entry = self.entries.remove(index);
        self.save();
        Some(entry.prompt)
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let text: String = self
            .entries
            .iter()
            .map(|e| {
                format!(
                    "{}\n",
                    json!({"prompt": e.prompt, "timestamp": e.timestamp})
                )
            })
            .collect();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, text);
    }
}

/// The dialog's description: `just now`, `Nm ago`, `Nh ago`, `Nd ago`, then the date.
pub fn age(timestamp: u64, now: u64) -> String {
    match now.saturating_sub(timestamp) / 60_000 {
        0 => "just now".into(),
        m if m >= 10_080 => crate::text::datetime(timestamp),
        _ => crate::text::ago(timestamp, now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_keeps_the_newest_fifty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("prompt-stash.jsonl");
        let mut stash = Stash::load(&path);
        for i in 0..52 {
            stash.push(format!("prompt {i}"), i);
        }
        assert_eq!(stash.pop().as_deref(), Some("prompt 51"));
        assert_eq!(stash.remove(0).as_deref(), Some("prompt 2"));
        let again = Stash::load(&path);
        assert_eq!(again.entries().len(), 48);
        assert_eq!(age(0, 30_000), "just now");
        assert_eq!(age(0, 3 * 3_600_000), "3h ago");
        assert_eq!(
            again.entries()[0],
            Entry {
                prompt: "prompt 3".into(),
                timestamp: 3
            }
        );
    }
}
