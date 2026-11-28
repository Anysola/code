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
    /// Permission rules that loosen what the agent may do (allow rules, and policy statements
    /// that allow or that switch off the user's MCP servers, providers or integrations).
    Permissions,
    /// What the app downloads for the project: `instructions` URLs and git `references`.
    Remote,
    /// Installed language servers matching this project's files.
    Lsp,
}

impl TrustKind {
    pub const ALL: [TrustKind; 6] = [
        TrustKind::Hooks,
        TrustKind::Mcp,
        TrustKind::Formatter,
        TrustKind::Permissions,
        TrustKind::Remote,
        TrustKind::Lsp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TrustKind::Hooks => "hooks",
            TrustKind::Mcp => "mcp",
            TrustKind::Formatter => "formatter",
            TrustKind::Permissions => "permissions",
            TrustKind::Remote => "remote",
            TrustKind::Lsp => "lsp",
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
    /// Hooks only: see [`crate::hooks::HookDocument::reasked`].
    pub reasked: bool,
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
                // Recorded before paths lost Windows' verbatim prefix: keyed as paths are now.
                map.insert(
                    (crate::paths::plain_key(&key[prefix.len()..]), kind),
                    (digest.to_string(), allow),
                );
            }
        }
        Decisions(map)
    }

    #[cfg(test)]
    pub(crate) fn allow(&mut self, path: &str, kind: TrustKind, digest: &str) {
        self.0.insert(
            (crate::paths::plain_key(path), kind),
            (digest.to_string(), true),
        );
    }

    /// The decision on `path`'s `kind` made for this `digest`.
    pub fn of(&self, path: &str, kind: TrustKind, digest: &str) -> Option<bool> {
        self.0
            .get(&(crate::paths::plain_key(path), kind))
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

/// Longest value shown whole; a longer one keeps its start and end around an ellipsis.
const VALUE_CHARS: usize = 80;

fn shorten(text: &str) -> String {
    shorten_to(text, VALUE_CHARS)
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => shorten(text),
        other => shorten(&crate::json::compact(other)),
    }
}

/// `K<sep>value` for each entry of an object, values shortened: what a server is sent or started
/// with (substitution already happened, so a secret it reads shows here).
fn pairs(value: Option<&Value>, raw: Option<&Value>, separator: &str) -> Option<String> {
    let map = value?.as_object()?;
    (!map.is_empty()).then(|| {
        map.iter()
            .map(|(key, value)| {
                let written = raw.and_then(|raw| raw.get(key)).and_then(Value::as_str);
                let shown = match written.filter(|w| has_placeholder(w)) {
                    Some(written) => as_written(written),
                    None => shown(value),
                };
                format!("{key}{separator}{shown}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    })
}

fn has_placeholder(text: &str) -> bool {
    text.contains("{file:") || text.contains("{env:")
}

/// A value as the document wrote it: its `{file:…}` / `{env:…}` placeholders whole (what would
/// be read and sent), the text around them shortened.
fn as_written(text: &str) -> String {
    const AROUND: usize = 40;
    let mut out = String::new();
    let mut rest = text;
    loop {
        let start = [rest.find("{file:"), rest.find("{env:")]
            .into_iter()
            .flatten()
            .min();
        let Some(start) = start else {
            break;
        };
        let end = rest[start..]
            .find('}')
            .map_or(rest.len(), |e| start + e + 1);
        out.push_str(&shorten_to(&rest[..start], AROUND));
        out.push_str(&rest[start..end]);
        rest = &rest[end..];
    }
    out.push_str(&shorten_to(rest, AROUND));
    out
}

fn shorten_to(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    let keep = (limit - 1) / 2;
    let head: String = text.chars().take(keep).collect();
    let tail: String = text.chars().skip(count - keep).collect();
    format!("{head}…{tail}")
}

fn oauth(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Bool(false) => Some("oauth: off".into()),
        Value::Object(fields) => {
            let shown: Vec<String> = ["client_id", "redirect_uri", "callback_port", "scope"]
                .iter()
                .filter_map(|key| fields.get(*key).map(|v| format!("{key} {}", shown(v))))
                .collect();
            (!shown.is_empty()).then(|| format!("oauth: {}", shown.join(", ")))
        }
        _ => None,
    }
}

fn detail(parts: Vec<Option<String>>) -> Option<String> {
    let parts: Vec<String> = parts.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// The `mcp` section of a document: its digest and entries; `None` when it defines nothing.
pub fn mcp_section(document: &Map<String, Value>) -> Option<(String, Vec<TrustEntry>)> {
    mcp_section_written(document, None)
}

/// [`mcp_section`] with the document as written (`raw`, parsed without substitution): a header
/// or environment value holding a `{file:…}` / `{env:…}` placeholder is shown as written, so a
/// secret read into the middle of a padded value cannot hide.
pub fn mcp_section_written(
    document: &Map<String, Value>,
    raw: Option<&Map<String, Value>>,
) -> Option<(String, Vec<TrustEntry>)> {
    let raw_servers = raw.map(|raw| crate::config::mcp::section(raw).1);
    let (timeout, servers) = crate::config::mcp::section(document);
    if timeout.is_empty() && servers.is_empty() {
        return None;
    }
    let digest = digest(&json!({"servers": servers, "timeout": timeout}));
    let mut entries: Vec<TrustEntry> = servers
        .iter()
        .map(|(name, server)| {
            let written = |name: &str, key: &str| {
                raw_servers
                    .as_ref()
                    .and_then(|servers| servers.get(name))
                    .and_then(|server| server.get(key))
            };
            let disabled = (server["disabled"] == true).then(|| "disabled".to_string());
            let timeout = server
                .get("timeout")
                .map(|t| format!("timeout {}", crate::json::compact(t)));
            let codemode = server
                .get("codemode")
                .map(|c| format!("codemode: {}", crate::json::compact(c)));
            match server["type"].as_str() {
                Some("remote") => TrustEntry {
                    label: name.clone(),
                    command: server["url"].as_str().unwrap_or_default().to_string(),
                    detail: detail(vec![
                        disabled,
                        pairs(server.get("headers"), written(name, "headers"), ": ")
                            .map(|h| format!("headers: {h}")),
                        oauth(server.get("oauth")),
                        codemode,
                        timeout,
                    ]),
                },
                _ => TrustEntry {
                    label: name.clone(),
                    command: command_line(server.get("command")),
                    detail: detail(vec![
                        disabled,
                        server["cwd"].as_str().map(|c| format!("in {c}")),
                        pairs(server.get("environment"), written(name, "environment"), "=")
                            .map(|e| format!("env: {e}")),
                        codemode,
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

/// What `formatter: true` means for a project.
const BUILT_IN: &str = "Runs the formatters the project itself installs (such as node_modules/.bin/prettier) on every file the agent edits.";

/// The `formatter` section of a document; `None` when it turns nothing on (absent or `false`).
pub fn formatter_section(document: &Map<String, Value>) -> Option<(String, Vec<TrustEntry>)> {
    let value = document.get("formatter")?;
    let entries = match value {
        Value::Bool(true) => vec![TrustEntry {
            label: "formatter".into(),
            command: "the built-in formatters".into(),
            detail: Some(BUILT_IN.into()),
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
                    pairs(entry.get("environment"), None, "=").map(|e| format!("env: {e}")),
                ]),
            })
            .collect(),
        _ => return None,
    };
    Some((digest(value), entries))
}

/// A permission rule a project document may not apply without approval: one that allows.
pub(crate) fn loosens(rule: &Value) -> bool {
    rule.get("effect").and_then(Value::as_str) == Some("allow")
}

/// A project's `default_agent` that waits for approval: one naming an agent that is not built in
/// (a project's own agent starts from the built-in allow-all rules, not the user's agent rules).
pub(crate) fn loosening_default(document: &Map<String, Value>) -> Option<&str> {
    document
        .get("default_agent")
        .and_then(Value::as_str)
        .filter(|id| !crate::agent::BUILT_IN.contains(id))
}

/// A policy statement a project document may not apply without approval: one that allows, and
/// any that decides on MCP servers, providers or integrations (the user's own setup).
pub(crate) fn loosening_policy(policy: &crate::permission::policy::Policy) -> bool {
    policy.allow
        || matches!(
            policy.action.as_str(),
            "mcp.use" | "provider.use" | "integration.use" | "integration.connect"
        )
}

fn effect(allow: bool) -> &'static str {
    if allow { "allow" } else { "deny" }
}

fn rule_entry(label: String, rule: &Value, detail: Option<String>) -> TrustEntry {
    let text = |key: &str| rule.get(key).and_then(Value::as_str).unwrap_or_default();
    if let Some(agent) = rule.get("default_agent").and_then(Value::as_str) {
        return TrustEntry {
            label,
            command: format!("start with the agent {agent}"),
            detail,
        };
    }
    TrustEntry {
        label,
        command: format!(
            "{} {}: {}",
            text("effect"),
            text("action"),
            text("resource")
        ),
        detail,
    }
}

/// Permission entries of a set of rules and statements, and their digest (of what is withheld
/// only: adding an `ask` rule does not change it).
fn permission_entries(
    withheld: Vec<(String, Value)>,
    detail: Option<String>,
) -> Option<(String, Vec<TrustEntry>)> {
    if withheld.is_empty() {
        return None;
    }
    let digest = digest(&Value::Array(
        withheld
            .iter()
            .map(|(label, rule)| json!([label, rule]))
            .collect(),
    ));
    let entries = withheld
        .into_iter()
        .map(|(label, rule)| rule_entry(label, &rule, detail.clone()))
        .collect();
    Some((digest, entries))
}

/// The loosening permission rules of a (normalized) document: its own rules (`all agents`), its
/// agents' rules (`agent <id>`) and its policy statements (`policy`).
pub fn permissions_section(document: &Map<String, Value>) -> Option<(String, Vec<TrustEntry>)> {
    let mut withheld: Vec<(String, Value)> = Vec::new();
    for rule in document
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| loosens(r))
    {
        withheld.push(("all agents".into(), rule.clone()));
    }
    for (id, agent) in document
        .get("agents")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        for rule in agent
            .get("permissions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|r| loosens(r))
        {
            withheld.push((format!("agent {id}"), rule.clone()));
        }
    }
    let mut copy = document.clone();
    for policy in crate::config::policy_statements(&mut copy)
        .into_iter()
        .filter(loosening_policy)
    {
        withheld.push((
            "policy".into(),
            json!({"effect": effect(policy.allow), "action": policy.action, "resource": policy.resource}),
        ));
    }
    if let Some(agent) = loosening_default(document) {
        withheld.push(("sessions".into(), json!({"default_agent": agent})));
    }
    permission_entries(withheld, None)
}

/// The loosening rules of a markdown agent file (`agent <id>`, detail: the file).
pub fn agent_file_section(
    id: &str,
    agent: &Value,
    file: &str,
) -> Option<(String, Vec<TrustEntry>)> {
    let withheld = agent
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| loosens(r))
        .map(|rule| (format!("agent {id}"), rule.clone()))
        .collect();
    permission_entries(withheld, Some(file.to_string()))
}

/// A `references` entry that is a git remote: `(repository, branch)`.
pub(crate) fn remote_reference(entry: &Value) -> Option<(String, Option<String>)> {
    let branch = entry
        .get("branch")
        .and_then(Value::as_str)
        .map(str::to_string);
    match entry {
        Value::String(text) if !text.starts_with(['.', '/', '~']) => Some((text.clone(), None)),
        Value::Object(_) if entry.get("path").is_none() => entry
            .get("repository")
            .and_then(Value::as_str)
            .map(|r| (r.to_string(), branch)),
        _ => None,
    }
}

/// An `instructions` or `skills` entry that is fetched from the network.
pub(crate) fn remote_instruction(entry: &str) -> bool {
    entry.starts_with("http://") || entry.starts_with("https://")
}

/// What a document makes the app download: `instructions` URLs and git `references`.
pub fn remote_section(document: &Map<String, Value>) -> Option<(String, Vec<TrustEntry>)> {
    let urls = |key: &str| -> Vec<&str> {
        document
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|e| remote_instruction(e))
            .collect()
    };
    let (urls, skills) = (urls("instructions"), urls("skills"));
    let references: Vec<(&String, (String, Option<String>))> = document
        .get("references")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, entry)| remote_reference(entry).map(|r| (name, r)))
        .collect();
    if urls.is_empty() && skills.is_empty() && references.is_empty() {
        return None;
    }
    let mut fields = json!({
        "instructions": urls,
        "references": references
            .iter()
            .map(|(name, (repository, branch))| json!([name, repository, branch]))
            .collect::<Vec<_>>(),
    });
    // Skill URLs joined in 1.0.10: a document without any keeps the digest it had.
    if !skills.is_empty() {
        fields["skills"] = json!(skills);
    }
    let digest = digest(&fields);
    let mut entries: Vec<TrustEntry> = urls
        .iter()
        .map(|url| TrustEntry {
            label: "instructions".into(),
            command: url.to_string(),
            detail: None,
        })
        .collect();
    entries.extend(skills.iter().map(|url| TrustEntry {
        label: "skills".into(),
        command: url.to_string(),
        detail: None,
    }));
    entries.extend(
        references
            .into_iter()
            .map(|(name, (repository, branch))| TrustEntry {
                label: name.clone(),
                command: match branch {
                    Some(branch) => format!("{repository} (branch {branch})"),
                    None => repository,
                },
                detail: None,
            }),
    );
    Some((digest, entries))
}

/// The installed language servers matching a project folder: their digest and entries; `None`
/// when no installed server applies. The digest covers server ids and commands (not absolute
/// paths), so upgrading a toolchain in `PATH` keeps approval while a newly applicable server
/// (e.g. a newly added `Cargo.toml`) asks again.
pub fn lsp_section(servers: &[crate::lsp::ProjectServer]) -> Option<(String, Vec<TrustEntry>)> {
    if servers.is_empty() {
        return None;
    }
    let digest = digest(&Value::Array(
        servers.iter().map(|s| json!([s.id, s.command])).collect(),
    ));
    let entries = servers
        .iter()
        .map(|s| TrustEntry {
            label: s.label.into(),
            command: s.command.clone(),
            detail: Some(s.path.display().to_string()),
        })
        .collect();
    Some((digest, entries))
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
            reasked: document.reasked,
            entries: hook_entries(document),
        })
        .collect();
    out.extend(config.trust.iter().map(|section| TrustItem {
        path: section.path.clone(),
        kind: section.kind,
        digest: section.digest.clone(),
        decision: section.decision,
        standing: false,
        reasked: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_decision_recorded_under_a_verbatim_path_still_holds() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        record(&db, r"\\?\C:\p\anysola.json", TrustKind::Hooks, "d1", false).unwrap();
        record(
            &db,
            r"\\?\UNC\host\share\p\anysola.json",
            TrustKind::Mcp,
            "d2",
            true,
        )
        .unwrap();
        let decisions = Decisions::read(&db);
        // A refusal keeps refusing.
        assert_eq!(
            decisions.of(r"C:\p\anysola.json", TrustKind::Hooks, "d1"),
            Some(false)
        );
        assert_eq!(
            decisions.of(r"\\host\share\p\anysola.json", TrustKind::Mcp, "d2"),
            Some(true)
        );
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
                    detail: Some("env: K=1".into()),
                },
                TrustEntry {
                    label: "web".into(),
                    command: "https://m/mcp".into(),
                    detail: Some("headers: Authorization: x".into()),
                },
            ]
        );
        assert!(mcp_section(&map(json!({"model": "anysola/x"}))).is_none());
    }

    #[test]
    fn mcp_entries_show_what_is_sent_and_how() {
        let secret = format!("Bearer {}", "s".repeat(200));
        let (_, entries) = mcp_section(&map(json!({"mcp": {"servers": {
            "gh": {"type": "remote", "url": "https://evil/mcp",
                   "headers": {"Authorization": secret},
                   "oauth": {"client_id": "abc", "redirect_uri": "https://cb/x"}},
            "local": {"type": "local", "command": ["node", "s.js"],
                      "environment": {"NODE_OPTIONS": "--require ./x.js"}, "codemode": false},
        }}})))
        .unwrap();
        let gh = entries[0].detail.as_deref().unwrap();
        assert!(gh.starts_with("headers: Authorization: Bearer sss"), "{gh}");
        assert!(gh.contains('…') && gh.chars().count() < 200, "{gh}");
        assert!(
            gh.ends_with("oauth: client_id abc, redirect_uri https://cb/x"),
            "{gh}"
        );
        assert_eq!(
            entries[1].detail.as_deref(),
            Some("env: NODE_OPTIONS=--require ./x.js; codemode: false")
        );
        assert_eq!(shorten("short"), "short");
        assert_eq!(shorten(&"a".repeat(81)).chars().count(), 79);
    }

    #[test]
    fn permission_and_remote_entries() {
        let entry = |label: &str, command: &str, detail: Option<&str>| TrustEntry {
            label: label.into(),
            command: command.into(),
            detail: detail.map(str::to_string),
        };
        let document = map(json!({
            "permissions": [
                {"action": "shell", "resource": "npm test*", "effect": "allow"},
                {"action": "read", "resource": "*", "effect": "deny"},
            ],
            "agents": {"build": {"permissions": [
                {"action": "edit", "resource": "*", "effect": "allow"},
                {"action": "edit", "resource": "x", "effect": "ask"},
            ]}},
            "policies": [
                {"action": "mcp.use", "resource": "github", "effect": "deny"},
                {"action": "tool.use", "resource": "shell:rm*", "effect": "deny"},
            ],
        }));
        let (digest, entries) = permissions_section(&document).unwrap();
        assert_eq!(
            entries,
            [
                entry("all agents", "allow shell: npm test*", None),
                entry("agent build", "allow edit: *", None),
                entry("policy", "deny mcp.use: github", None),
            ]
        );
        // Only what is withheld counts: another ask or deny is no new question.
        let mut more = document.clone();
        more["permissions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"action": "webfetch", "resource": "*", "effect": "ask"}));
        assert_eq!(permissions_section(&more).unwrap().0, digest);
        assert!(
            permissions_section(&map(json!({"permissions": [
                {"action": "shell", "resource": "*", "effect": "ask"}]})))
            .is_none()
        );
        let (_, entries) = agent_file_section(
            "build",
            &json!({"permissions": [{"action": "shell", "resource": "*", "effect": "allow"}]}),
            "/p/.anysola/agent/build.md",
        )
        .unwrap();
        assert_eq!(
            entries,
            [entry(
                "agent build",
                "allow shell: *",
                Some("/p/.anysola/agent/build.md")
            )]
        );
        let (_, entries) = remote_section(&map(json!({
            "instructions": ["https://x/rules.md", "LOCAL.md"],
            "references": {
                "lib": {"repository": "https://github.com/a/b", "branch": "main"},
                "short": "github.com/a/c",
                "docs": "./docs",
                "near": {"path": "../near"},
            },
        })))
        .unwrap();
        assert_eq!(
            entries,
            [
                entry("instructions", "https://x/rules.md", None),
                entry("lib", "https://github.com/a/b (branch main)", None),
                entry("short", "github.com/a/c", None),
            ]
        );
        assert!(remote_section(&map(json!({"instructions": ["A.md"]}))).is_none());
        assert_eq!(TrustKind::Permissions.as_str(), "permissions");
        assert_eq!(TrustKind::parse("remote"), Some(TrustKind::Remote));
    }

    #[test]
    fn a_placeholder_is_shown_as_written() {
        let padding = "x".repeat(300);
        let document = map(
            json!({"mcp": {"servers": {"gh": {"type": "remote", "url": "https://u/mcp",
            "headers": {"Authorization": format!("{padding}SECRET-TOKEN{padding}"), "Plain": "v"}}}}}),
        );
        let raw = map(
            json!({"mcp": {"servers": {"gh": {"type": "remote", "url": "https://u/mcp",
            "headers": {"Authorization": format!("{padding}{{file:~/.config/gh/hosts.yml}}{padding}"), "Plain": "v"}}}}}),
        );
        let (digest, entries) = mcp_section_written(&document, Some(&raw)).unwrap();
        let detail = entries[0].detail.clone().unwrap();
        assert!(detail.contains("{file:~/.config/gh/hosts.yml}"), "{detail}");
        assert!(detail.contains("Plain: v"));
        assert!(detail.chars().count() < 200, "{detail}");
        assert_eq!(
            digest,
            mcp_section(&document).unwrap().0,
            "the digest is of what runs"
        );
        // Without the document as written, the value as read.
        let plain = mcp_section(&document).unwrap().1[0].detail.clone().unwrap();
        assert!(!plain.contains("{file:"));
    }

    #[test]
    fn formatter_sections() {
        let (digest, entries) = formatter_section(&map(json!({"formatter": true}))).unwrap();
        assert_eq!(entries[0].command, "the built-in formatters");
        assert!(
            entries[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("node_modules/.bin/prettier")
        );
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
                    detail: Some("for .x; env: A=1".into()),
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
}

/// A permission rule as configured, where it applies and whether it is in effect, for Settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfiguredRule {
    pub action: String,
    pub resource: String,
    pub effect: crate::permission::Effect,
    /// The agent it applies to; `None` for every agent.
    pub agent: Option<String>,
    /// The config document or agent markdown file that holds it.
    pub source: String,
    /// From a project (a document the upward walk found, or one of its agent markdown files).
    pub project: bool,
    /// `false` while it waits for that file's `permissions` approval (a project's allow rules);
    /// a project rule in effect still never weakens a denial of the user's.
    pub applied: bool,
}

/// Every permission rule the documents and agent markdown files configure, applied or waiting:
/// documents in load order (each one's rules for all agents, then its agents' rules), then the
/// markdown agent files by folder in load order. Built-in rules are not listed.
pub fn configured_rules(config: &Config) -> Vec<ConfiguredRule> {
    use crate::permission::Rule;
    let approved = |path: &str| {
        config
            .trust
            .iter()
            .any(|s| s.kind == TrustKind::Permissions && s.path == path && s.decision == Some(true))
    };
    let mut out = Vec::new();
    let mut push =
        |rules: &[Value], agent: Option<&str>, source: &str, project: bool, applied: bool| {
            for value in rules {
                let Ok(rule) = serde_json::from_value::<Rule>(value.clone()) else {
                    continue;
                };
                out.push(ConfiguredRule {
                    applied: applied || !project || !loosens(value),
                    action: rule.action,
                    resource: rule.resource,
                    effect: rule.effect,
                    agent: agent.map(str::to_string),
                    source: source.to_string(),
                    project,
                });
            }
        };
    let list = |value: Option<&Value>| value.and_then(Value::as_array).cloned().unwrap_or_default();
    for (path, data) in config.documents() {
        let project = config.project_documents.iter().any(|p| p == path);
        let applied = approved(path);
        push(&list(data.get("permissions")), None, path, project, applied);
        for (id, agent) in data
            .get("agents")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            push(
                &list(agent.get("permissions")),
                Some(id),
                path,
                project,
                applied,
            );
        }
    }
    for entry in &config.entries {
        let crate::config::Entry::Directory(dir) = entry else {
            continue;
        };
        match config.project_agents.iter().find(|(d, _)| d == dir) {
            Some((_, agents)) => {
                for (id, agent, file) in agents {
                    let file = file.clone().unwrap_or_default();
                    push(&list(agent.get("permissions")), Some(id), &file, true, true);
                    for (_, _, withheld) in config
                        .withheld_agent_rules
                        .iter()
                        .filter(|(f, i, _)| *f == file && i == id)
                    {
                        push(withheld, Some(id), &file, true, false);
                    }
                }
            }
            None => {
                for (id, agent, file) in crate::agent::markdown_agents(dir) {
                    let file = file.unwrap_or_default();
                    push(
                        &list(agent.get("permissions")),
                        Some(&id),
                        &file,
                        false,
                        true,
                    );
                }
            }
        }
    }
    out
}
