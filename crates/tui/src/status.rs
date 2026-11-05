//! Program status records (OSC 7501): a root summary plus one record per open tab or viewed
//! session that is working or waiting for input, sent when they change and cleared on exit
//! (`context/program-status.ts`).

use std::collections::HashMap;

use base64::Engine;

const APP: &str = "opencode";
/// The root record's key; it has no `id`.
const ROOT: &str = "";

#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    Working,
    /// `permission` or `question`, and the line saying so.
    Blocked(&'static str, &'static str),
}

pub struct Report {
    pub id: String,
    pub title: String,
    pub state: Pending,
}

#[derive(Default)]
pub struct Records {
    sent: HashMap<String, String>,
}

/// One line within `bytes`, with `…` when cut.
fn clip(text: &str, bytes: usize) -> Option<String> {
    let line = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if line.is_empty() {
        return None;
    }
    if line.len() <= bytes {
        return Some(line);
    }
    let mut end = bytes - 3;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!("{}…", &line[..end]))
}

/// Record IDs are 1–32 bytes of `[A-Za-z0-9_.+-]`; other session IDs are hashed.
fn record_id(id: &str) -> String {
    let valid = (1..=32).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.+-".contains(c));
    if valid {
        return id.to_string();
    }
    use sha2::Digest;
    let digest = sha2::Sha256::digest(id.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("ses_{}", &hex[..16])
}

fn encode(
    state: &str,
    id: Option<&str>,
    kind: Option<&str>,
    title: Option<&str>,
    message: Option<&str>,
) -> String {
    let b64 = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = format!("\x1b]7501;state={state}");
    if let Some(id) = id {
        out.push_str(&format!(":id={id}"));
    }
    if let Some(kind) = kind {
        out.push_str(&format!(":kind={kind}"));
    }
    out.push_str(&format!(":app={APP}"));
    if let Some(title) = title {
        out.push_str(&format!(":title={}", b64(title)));
    }
    if let Some(message) = message {
        out.push_str(&format!(":msg={}", b64(message)));
    }
    out.push_str("\x1b\\");
    out
}

fn record(report: &Report, id: Option<&str>) -> String {
    let title = clip(&report.title, 192);
    match &report.state {
        Pending::Working => encode("working", id, None, title.as_deref(), None),
        Pending::Blocked(kind, message) => encode(
            "blocked",
            id,
            Some(kind),
            title.as_deref(),
            clip(message, 2048).as_deref(),
        ),
    }
}

fn root(reports: &[Report]) -> String {
    let blocked: Vec<&Report> = reports
        .iter()
        .filter(|r| matches!(r.state, Pending::Blocked(..)))
        .collect();
    let working: Vec<&Report> = reports
        .iter()
        .filter(|r| r.state == Pending::Working)
        .collect();
    match (blocked.as_slice(), working.as_slice()) {
        ([one], _) => record(one, None),
        ([first, ..], _) => {
            let same = blocked.iter().all(|r| r.state == first.state);
            let kind = match (&first.state, same) {
                (Pending::Blocked(kind, _), true) => Some(*kind),
                _ => None,
            };
            let message = format!("{} sessions need input", blocked.len());
            encode("blocked", None, kind, None, Some(&message))
        }
        ([], [one]) => record(one, None),
        ([], [_, ..]) => {
            let message = format!("{} sessions working", working.len());
            encode("working", None, None, None, Some(&message))
        }
        ([], []) => encode("idle", None, None, None, None),
    }
}

impl Records {
    /// The escape sequences that bring the terminal's records to `reports`.
    pub fn update(&mut self, reports: &[Report]) -> String {
        let mut wanted: Vec<(String, String)> = vec![(ROOT.to_string(), root(reports))];
        for report in reports {
            let id = record_id(&report.id);
            wanted.push((id.clone(), record(report, Some(&id))));
        }
        let mut out = String::new();
        let stale: Vec<String> = self
            .sent
            .keys()
            .filter(|k| k.as_str() != ROOT && wanted.iter().all(|(w, _)| w != *k))
            .cloned()
            .collect();
        for key in stale {
            out.push_str(&format!("\x1b]7501;state=clear:id={key}\x1b\\"));
            self.sent.remove(&key);
        }
        for (key, sequence) in wanted {
            if self.sent.get(&key) != Some(&sequence) {
                out.push_str(&sequence);
                self.sent.insert(key, sequence);
            }
        }
        out
    }

    /// Clears the root record and every record beneath it.
    pub fn clear(&mut self) -> String {
        self.sent.clear();
        "\x1b]7501;state=clear\x1b\\".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: &str, state: Pending) -> Report {
        Report {
            id: id.into(),
            title: "Fix\ttypo".into(),
            state,
        }
    }

    #[test]
    fn root_summary_records_and_clears() {
        let mut records = Records::default();
        assert_eq!(
            records.update(&[]),
            "\x1b]7501;state=idle:app=opencode\x1b\\"
        );
        assert_eq!(records.update(&[]), "");
        let out = records.update(&[report("ses_a", Pending::Working)]);
        // "Fix typo" in base64; the root repeats the only record without an id.
        assert_eq!(
            out,
            "\x1b]7501;state=working:app=opencode:title=Rml4IHR5cG8=\x1b\\\x1b]7501;state=working:id=ses_a:app=opencode:title=Rml4IHR5cG8=\x1b\\"
        );
        let blocked = Pending::Blocked("permission", "Permission needs input");
        let out = records.update(&[
            report("ses_a", blocked.clone()),
            report("ses_b", blocked.clone()),
        ]);
        assert!(out.starts_with(
            "\x1b]7501;state=blocked:kind=permission:app=opencode:msg=MiBzZXNzaW9ucyBuZWVkIGlucHV0\x1b\\"
        ));
        let out = records.update(&[report("ses_b", blocked)]);
        assert!(out.starts_with("\x1b]7501;state=clear:id=ses_a\x1b\\"));
        assert_eq!(records.clear(), "\x1b]7501;state=clear\x1b\\");
        assert_eq!(record_id("x".repeat(40).as_str()).len(), 20);
    }
}
