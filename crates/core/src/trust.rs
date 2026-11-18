//! Approval of what a project's config makes Anysola Code run without asking: its hooks, its MCP
//! servers and its formatters. Each kind of each project document is approved for its content
//! (a digest of the section after variable substitution); a section nobody approved is not
//! applied. User and managed documents are never asked about.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::hooks::config::digest;
use crate::storage::Db;

/// What a project document's approval is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrustKind {
    Hooks,
    Mcp,
    Formatter,
}

impl TrustKind {
    pub const ALL: [TrustKind; 3] = [TrustKind::Hooks, TrustKind::Mcp, TrustKind::Formatter];

    pub fn as_str(self) -> &'static str {
        match self {
            TrustKind::Hooks => "hooks",
            TrustKind::Mcp => "mcp",
            TrustKind::Formatter => "formatter",
        }
    }

    pub fn parse(text: &str) -> Option<TrustKind> {
        TrustKind::ALL.into_iter().find(|k| k.as_str() == text)
    }

    /// `hooks.trust:<path>`, `mcp.trust:<path>`, `formatter.trust:<path>`.
    fn key(self, path: &str) -> String {
        format!("{}.trust:{path}", self.as_str())
    }
}

/// One thing a section would run.
#[derive(Debug, Clone, PartialEq)]
pub struct TrustEntry {
    /// `PreToolUse (Bash)`, an MCP server's name, a formatter's name.
    pub label: String,
    /// The command line or URL, as configured (raw: show it through [`crate::hooks::display`]).
    pub command: String,
    /// More that matters for the decision: `disabled`, environment variable names, a timeout.
    pub detail: Option<String>,
}

/// One kind of one project document, with the user's decision on its current content.
#[derive(Debug, Clone, PartialEq)]
pub struct TrustItem {
    pub path: String,
    pub kind: TrustKind,
    pub digest: String,
    /// For this digest: `Some(true)` approved, `Some(false)` refused, `None` undecided.
    pub decision: Option<bool>,
    /// Hooks only: an earlier approved version runs in place of this content in this process.
    pub standing: bool,
    pub entries: Vec<TrustEntry>,
}

/// The stored decisions, read once for a config load. The default holds none: nothing from a
/// project is applied.
#[derive(Debug, Clone, Default)]
pub struct Decisions(HashMap<(String, TrustKind), (String, bool)>);

impl Decisions {
    pub fn read(db: &Db) -> Decisions {
        let mut map = HashMap::new();
        for kind in TrustKind::ALL {
            let prefix = kind.key("");
            for (key, value) in db.kv_prefixed(&prefix).unwrap_or_default() {
                let (Some(digest), Some(allow)) =
                    (value["digest"].as_str(), value["allow"].as_bool())
                else {
                    continue;
                };
                map.insert(
                    (key[prefix.len()..].to_string(), kind),
                    (digest.to_string(), allow),
                );
            }
        }
        Decisions(map)
    }

    #[cfg(test)]
    pub(crate) fn allow(&mut self, path: &str, kind: TrustKind, digest: &str) {
        self.0
            .insert((path.to_string(), kind), (digest.to_string(), true));
    }

    /// The decision on `path`'s `kind` made for this `digest`.
    pub fn of(&self, path: &str, kind: TrustKind, digest: &str) -> Option<bool> {
        self.0
            .get(&(path.to_string(), kind))
            .filter(|(d, _)| d == digest)
            .map(|(_, allow)| *allow)
    }
}

/// Records the user's decision on `path`'s `kind` with this content.
pub fn record(
    db: &Db,
    path: &str,
    kind: TrustKind,
    digest: &str,
    allow: bool,
) -> crate::storage::Result<()> {
    db.kv_set(&kind.key(path), &json!({"digest": digest, "allow": allow}))
}

/// A project document's `mcp` or `formatter` section, as the config load found it.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub path: String,
    pub kind: TrustKind,
    pub digest: String,
    pub decision: Option<bool>,
    pub entries: Vec<TrustEntry>,
}

/// A word of a command line, quoted when a shell would split or expand it.
fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

fn command_line(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .map(|words| {
            words
                .iter()
                .filter_map(Value::as_str)
                .map(quote)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn names(value: Option<&Value>) -> Option<String> {
    let map = value?.as_object()?;
    (!map.is_empty()).then(|| map.keys().cloned().collect::<Vec<_>>().join(", "))
}

fn detail(parts: Vec<Option<String>>) -> Option<String> {
    let parts: Vec<String> = parts.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// The `mcp` section of a document: its digest and entries; `None` when it defines nothing.
pub fn mcp_section(document: &Map<String, Value>) -> Option<(String, Vec<TrustEntry>)> {
    let (timeout, servers) = crate::config::mcp::section(document);
    if timeout.is_empty() && servers.is_empty() {
        return None;
    }
    let digest = digest(&json!({"servers": servers, "timeout": timeout}));
    let mut entries: Vec<TrustEntry> = servers
        .iter()
        .map(|(name, server)| {
            let disabled = (server["disabled"] == true).then(|| "disabled".to_string());
            let timeout = server
                .get("timeout")
                .map(|t| format!("timeout {}", crate::json::compact(t)));
            match server["type"].as_str() {
                Some("remote") => TrustEntry {
                    label: name.clone(),
                    command: server["url"].as_str().unwrap_or_default().to_string(),
                    detail: detail(vec![
                        disabled,
                        names(server.get("headers")).map(|h| format!("headers: {h}")),
                        timeout,
                    ]),
                },
                _ => TrustEntry {
                    label: name.clone(),
                    command: command_line(server.get("command")),
                    detail: detail(vec![
                        disabled,
                        server["cwd"].as_str().map(|c| format!("in {c}")),
                        names(server.get("environment")).map(|e| format!("env: {e}")),
                        timeout,
                    ]),
                },
            }
        })
        .collect();
    if !timeout.is_empty() {
        entries.push(TrustEntry {
            label: "MCP timeouts".into(),
            command: crate::json::compact(&Value::Object(timeout)),
            detail: None,
        });
    }
    Some((digest, entries))
}

/// The `formatter` section of a document; `None` when it turns nothing on (absent or `false`).
pub fn formatter_section(document: &Map<String, Value>) -> Option<(String, Vec<TrustEntry>)> {
    let value = document.get("formatter")?;
    let entries = match value {
        Value::Bool(true) => vec![TrustEntry {
            label: "formatter".into(),
            command: "the built-in formatters".into(),
            detail: None,
        }],
        Value::Object(map) => map
            .iter()
            .map(|(name, entry)| TrustEntry {
                label: name.clone(),
                command: match entry.get("command") {
                    Some(command) => command_line(Some(command)),
                    None => format!("the built-in {name} formatter"),
                },
                detail: detail(vec![
                    (entry["disabled"] == true).then(|| "disabled".to_string()),
                    entry["extensions"].as_array().map(|list| {
                        let list: Vec<&str> = list.iter().filter_map(Value::as_str).collect();
                        format!("for {}", list.join(", "))
                    }),
                    names(entry.get("environment")).map(|e| format!("env: {e}")),
                ]),
            })
            .collect(),
        _ => return None,
    };
    Some((digest(value), entries))
}

fn hook_entries(document: &crate::hooks::HookDocument) -> Vec<TrustEntry> {
    document
        .hooks
        .iter()
        .map(|hook| TrustEntry {
            label: match hook
                .matcher
                .as_deref()
                .filter(|m| !m.is_empty() && *m != "*")
            {
                Some(matcher) => format!("{} ({matcher})", hook.event),
                None => hook.event.clone(),
            },
            command: hook.command.clone(),
            detail: (hook.timeout != crate::hooks::default_timeout(&hook.event))
                .then(|| format!("timeout {} s", hook.timeout)),
        })
        .collect()
}

/// Every project document × kind that defines something, documents in load order.
fn assemble(config: &Config, hooks: Vec<crate::hooks::HookDocument>) -> Vec<TrustItem> {
    let mut out: Vec<TrustItem> = hooks
        .iter()
        .map(|document| TrustItem {
            path: document.path.clone(),
            kind: TrustKind::Hooks,
            digest: document.digest.clone(),
            decision: document.decision,
            standing: document.standing,
            entries: hook_entries(document),
        })
        .collect();
    out.extend(config.trust.iter().map(|section| TrustItem {
        path: section.path.clone(),
        kind: section.kind,
        digest: section.digest.clone(),
        decision: section.decision,
        standing: false,
        entries: section.entries.clone(),
    }));
    let order: Vec<&str> = config.documents().map(|(path, _)| path).collect();
    let rank = |item: &TrustItem| {
        let at = order
            .iter()
            .position(|p| *p == item.path)
            .unwrap_or(usize::MAX);
        let kind = TrustKind::ALL.iter().position(|k| *k == item.kind);
        (at, kind)
    };
    out.sort_by_key(rank);
    out
}

/// [`TrustItem`]s of a config without a running core (the CLI): hooks without standing versions.
pub fn items(config: &Config, db: &Db) -> Vec<TrustItem> {
    assemble(config, crate::hooks::documents(&config.hooks, db))
}

pub(crate) fn items_with(
    config: &Config,
    db: &Db,
    approved: &crate::hooks::Approved,
) -> Vec<TrustItem> {
    assemble(
        config,
        crate::hooks::documents_with(&config.hooks, db, approved),
    )
}

/// What `anysola run` skips: per kind, how many things wait for a first decision (refused ones
/// are not counted: the user answered).
pub fn waiting(config: &Config, db: &Db) -> Vec<(TrustKind, usize)> {
    let mut counts: Vec<(TrustKind, usize)> = Vec::new();
    let hooks = crate::hooks::unapproved(&config.hooks, db);
    if hooks > 0 {
        counts.push((TrustKind::Hooks, hooks));
    }
    for kind in [TrustKind::Mcp, TrustKind::Formatter] {
        let count: usize = config
            .trust
            .iter()
            .filter(|s| s.kind == kind && s.decision.is_none())
            .map(|s| s.entries.len())
            .sum();
        if count > 0 {
            counts.push((kind, count));
        }
    }
    counts
}

/// The one line `anysola run` prints for [`waiting`]; `None` when nothing waits.
pub fn skipped_line(waiting: &[(TrustKind, usize)]) -> Option<String> {
    let total: usize = waiting.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return None;
    }
    let kinds: Vec<&str> = waiting
        .iter()
        .map(|(kind, _)| match kind {
            TrustKind::Hooks => "hooks",
            TrustKind::Mcp => "MCP servers",
            TrustKind::Formatter => "formatters",
        })
        .collect();
    let (things, are) = if total == 1 {
        ("thing", "is")
    } else {
        ("things", "are")
    };
    Some(format!(
        "Skipped {total} {things} this project's config wants to run ({}) that {are} not approved. Run `anysola trust` to review them.",
        kinds.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn mcp_digests_ignore_layout_and_follow_what_runs() {
        let base = json!({"mcp": {"servers": {
            "fs": {"type": "local", "command": ["npx", "-y", "server fs"], "environment": {"K": "1"}},
            "web": {"type": "remote", "url": "https://m/mcp", "headers": {"Authorization": "x"}},
        }}});
        let (digest, entries) = mcp_section(&map(base.clone())).unwrap();
        let reordered: Value = serde_json::from_str(
            r#"{"mcp":{"servers":{"web":{"headers":{"Authorization":"x"},"url":"https://m/mcp","type":"remote"},
                "fs":{"environment":{"K":"1"},"command":["npx","-y","server fs"],"type":"local"}}}}"#,
        )
        .unwrap();
        assert_eq!(mcp_section(&map(reordered)).unwrap().0, digest);
        let changed = |path: &[&str], value: Value| {
            let mut doc = base.clone();
            let mut at = &mut doc;
            for key in path {
                at = &mut at[*key];
            }
            *at = value;
            mcp_section(&map(doc)).unwrap().0
        };
        let fs = ["mcp", "servers", "fs"];
        assert_ne!(
            changed(&[&fs[..], &["command"]].concat(), json!(["evil"])),
            digest
        );
        assert_ne!(
            changed(&[&fs[..], &["environment"]].concat(), json!({"K": "2"})),
            digest
        );
        assert_ne!(changed(&[&fs[..], &["timeout"]].concat(), json!(5)), digest);
        assert_ne!(
            changed(&["mcp", "servers", "web", "url"], json!("https://evil/mcp")),
            digest
        );
        assert_ne!(changed(&["mcp", "timeout"], json!({"startup": 1})), digest);
        assert_eq!(
            entries,
            [
                TrustEntry {
                    label: "fs".into(),
                    command: "npx -y 'server fs'".into(),
                    detail: Some("env: K".into()),
                },
                TrustEntry {
                    label: "web".into(),
                    command: "https://m/mcp".into(),
                    detail: Some("headers: Authorization".into()),
                },
            ]
        );
        assert!(mcp_section(&map(json!({"model": "anysola/x"}))).is_none());
    }

    #[test]
    fn formatter_sections() {
        let (digest, entries) = formatter_section(&map(json!({"formatter": true}))).unwrap();
        assert_eq!(entries[0].command, "the built-in formatters");
        assert_eq!(digest, crate::hooks::config::digest(&json!(true)));
        let (_, entries) = formatter_section(&map(json!({"formatter": {
            "mine": {"command": ["fmt", "$FILE"], "extensions": [".x"], "environment": {"A": "1"}},
            "prettier": {"disabled": true},
        }})))
        .unwrap();
        assert_eq!(
            entries,
            [
                TrustEntry {
                    label: "mine".into(),
                    command: "fmt '$FILE'".into(),
                    detail: Some("for .x; env: A".into()),
                },
                TrustEntry {
                    label: "prettier".into(),
                    command: "the built-in prettier formatter".into(),
                    detail: Some("disabled".into()),
                },
            ]
        );
        assert!(formatter_section(&map(json!({"formatter": false}))).is_none());
        let a = formatter_section(&map(json!({"formatter": {"m": {"command": ["a"]}}}))).unwrap();
        let b = formatter_section(&map(json!({"formatter": {"m": {"command": ["b"]}}}))).unwrap();
        assert_ne!(a.0, b.0);
    }

    #[test]
    fn decisions_are_per_path_kind_and_digest() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        record(&db, "/p/anysola.json", TrustKind::Mcp, "d1", true).unwrap();
        record(&db, "/p/anysola.json", TrustKind::Formatter, "d2", false).unwrap();
        crate::hooks::trust(&db, "/p/anysola.json", "d3", true).unwrap();
        let decisions = Decisions::read(&db);
        assert_eq!(
            decisions.of("/p/anysola.json", TrustKind::Mcp, "d1"),
            Some(true)
        );
        assert_eq!(
            decisions.of("/p/anysola.json", TrustKind::Mcp, "other"),
            None
        );
        assert_eq!(
            decisions.of("/p/anysola.json", TrustKind::Formatter, "d2"),
            Some(false)
        );
        assert_eq!(
            decisions.of("/p/anysola.json", TrustKind::Hooks, "d3"),
            Some(true),
            "1.0.9 records"
        );
        assert_eq!(decisions.of("/q/anysola.json", TrustKind::Mcp, "d1"), None);
    }

    #[test]
    fn the_skipped_line_names_what_waits() {
        assert_eq!(skipped_line(&[]), None);
        assert_eq!(
            skipped_line(&[(TrustKind::Mcp, 1)]).unwrap(),
            "Skipped 1 thing this project's config wants to run (MCP servers) that is not approved. Run `anysola trust` to review them."
        );
        assert_eq!(
            skipped_line(&[(TrustKind::Hooks, 2), (TrustKind::Formatter, 1)]).unwrap(),
            "Skipped 3 things this project's config wants to run (hooks, formatters) that are not approved. Run `anysola trust` to review them."
        );
    }
}
