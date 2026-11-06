//! Core config: discovery of every source, parsing (JSONC, substitution, V1 normalization), the
//! Anysola provider lock, and the merge into one [`Config`].
//!
//! Sources, lowest precedence first: managed policies, the gateway's remote document,
//! `~/.config/opencode` (`ANYSOLA_CONFIG_DIR`), `~/.anysola/config.json`, `ANYSOLA_CONFIG`,
//! project `anysola.json[c]` files farthest first, project `.anysola/` directories farthest
//! first, `ANYSOLA_CONFIG_CONTENT`, then managed settings (the system directory, then macOS MDM
//! preferences). Managed policies outrank all others although their settings load last.
//!
//! The 0.5.x names are still read: `opencode.json[c]` and `.opencode/` load just before their
//! `anysola` counterparts in the same directory, so the new name wins a conflict, and every
//! `ANYSOLA_*` variable falls back to its `OPENCODE_*` name ([`crate::env`]).

pub mod jsonc;
mod mcp;
mod normalize;
mod substitute;

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::paths::Paths;
use crate::permission::Rule;
use crate::permission::policy::{Policies, Policy};
use crate::tool::OutputLimits;

pub use normalize::{markdown_agent, model_ref, normalize_action};

/// Config file names of a directory, in load order (later wins): the 0.5.x names first.
pub const NAMES: [&str; 4] = [
    "opencode.json",
    "opencode.jsonc",
    "anysola.json",
    "anysola.jsonc",
];
/// Project config directories, in load order (later wins): the 0.5.x name first.
pub const DOT_DIRS: [&str; 2] = [".opencode", ".anysola"];
const ENDPOINT_KEYS: [&str; 4] = ["baseURL", "apiKey", "authToken", "accessToken"];

/// One parsed, normalized document.
#[derive(Debug, Clone)]
struct Document {
    data: Map<String, Value>,
    /// From a managed (system) directory: its policies outrank every other document's.
    managed: bool,
    /// The directory of the document's file; relative `references` paths resolve against it.
    dir: Option<PathBuf>,
}

/// A config source as `anysola debug config` lists it.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A directory that exists and was read for files and definitions.
    Directory(PathBuf),
    /// A parsed document (a file path, or `ANYSOLA_CONFIG_CONTENT`), after normalization.
    Document {
        path: String,
        data: Map<String, Value>,
    },
}

/// A source of agent and command definitions, in load order: later entries override earlier ones,
/// so a project's markdown agent overrides a global document's agent of the same id.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// A document's `agents` and `commands`.
    Document {
        agents: Map<String, Value>,
        commands: Map<String, Value>,
    },
    /// A config directory whose `agent/`, `command/`, … folders hold markdown definitions.
    Directory(PathBuf),
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Top-level keys, latest document wins (maps below are merged separately).
    values: Map<String, Value>,
    /// Agent and command sources in load order.
    pub entries: Vec<Entry>,
    /// Top-level `permissions` of every document, in load order.
    pub permissions: Vec<Rule>,
    /// `instructions` of every document, deduplicated.
    pub instructions: Vec<String>,
    /// `skills` sources of every document, deduplicated.
    pub skills: Vec<String>,
    /// `references` of every document by name (a later document's entry replaces an earlier one
    /// in place), with the directory of the document that defined it.
    pub references: Vec<(String, Value, Option<PathBuf>)>,
    /// `providers.anysola` merged.
    pub provider: Map<String, Value>,
    /// Problems worth one warning line each (malformed documents, ignored keys).
    pub warnings: Vec<String>,
    /// Every local source in load order (the gateway document is not one).
    pub sources: Vec<Source>,
    /// `mcp` of every document.
    mcp: mcp::Merged,
    /// Policy statements of ordinary documents, earliest-loaded last.
    policies: Vec<Policy>,
    /// Policy statements of managed documents, earliest-loaded last.
    managed_policies: Vec<Policy>,
}

impl Config {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn model(&self) -> Option<&str> {
        self.get("model").and_then(Value::as_str)
    }

    pub fn default_agent(&self) -> Option<&str> {
        self.get("default_agent").and_then(Value::as_str)
    }

    pub fn shell(&self) -> Option<&str> {
        self.get("shell")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Off with `websearch: false`, or when a policy denies the Anysola integration that serves it.
    pub fn websearch_enabled(&self) -> bool {
        self.get("websearch") != Some(&Value::Bool(false))
            && self.anysola_allowed("integration.use")
    }

    /// Whether policies let the Anysola provider or integration be used (`provider.use` /
    /// `integration.use` on `anysola`; a device login carries no account to qualify the resource).
    pub fn anysola_allowed(&self, action: &str) -> bool {
        self.policies().allows(action, crate::model::PROVIDER)
    }

    pub fn tool_output(&self) -> OutputLimits {
        let defaults = OutputLimits::default();
        let field = |key: &str| {
            self.get("tool_output")
                .and_then(|t| t.get(key))
                .and_then(Value::as_u64)
        };
        OutputLimits {
            max_lines: field("max_lines").map_or(defaults.max_lines, |v| v as usize),
            max_bytes: field("max_bytes").map_or(defaults.max_bytes, |v| v as usize),
        }
    }

    /// Directories that hold markdown definitions (`agent/`, `command/`, …), lowest precedence first.
    pub fn directories(&self) -> impl Iterator<Item = &PathBuf> {
        self.entries.iter().filter_map(|entry| match entry {
            Entry::Directory(dir) => Some(dir),
            Entry::Document { .. } => None,
        })
    }

    pub fn models(&self) -> Option<&Map<String, Value>> {
        self.provider.get("models").and_then(Value::as_object)
    }

    /// The MCP servers to run, in config order (see [`mcp`]).
    pub fn mcp_servers(&self) -> Vec<(String, Value)> {
        self.mcp.resolve(&self.policies())
    }

    /// Every policy statement; the last one matching decides.
    pub fn policies(&self) -> Policies {
        Policies(
            self.policies
                .iter()
                .chain(&self.managed_policies)
                .cloned()
                .collect(),
        )
    }

    /// A config of plain documents, lowest precedence first.
    #[cfg(test)]
    pub fn from_documents(documents: &[Value]) -> Config {
        let mut config = Config::default();
        for data in documents {
            config.apply(Document {
                data: data.as_object().cloned().unwrap_or_default(),
                managed: false,
                dir: None,
            });
        }
        config
    }

    fn apply(&mut self, document: Document) {
        let mut data = document.data;
        let statements = policy_statements(&mut data);
        let list = if document.managed {
            &mut self.managed_policies
        } else {
            &mut self.policies
        };
        list.splice(0..0, statements);
        if let Some(Value::Array(rules)) = data.remove("permissions") {
            self.permissions.extend(
                rules
                    .into_iter()
                    .filter_map(|r| serde_json::from_value(r).ok()),
            );
        }
        for (key, list) in [
            ("instructions", &mut self.instructions),
            ("skills", &mut self.skills),
        ] {
            if let Some(Value::Array(items)) = data.remove(key) {
                for item in items
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                {
                    if !list.contains(&item) {
                        list.push(item);
                    }
                }
            }
        }
        let map = |value: Option<Value>| match value {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        for (name, value) in map(data.remove("references")) {
            let entry = (name, value, document.dir.clone());
            match self.references.iter_mut().find(|r| r.0 == entry.0) {
                Some(existing) => *existing = entry,
                None => self.references.push(entry),
            }
        }
        let agents = map(data.remove("agents"));
        let commands = map(data.remove("commands"));
        if !agents.is_empty() || !commands.is_empty() {
            self.entries.push(Entry::Document { agents, commands });
        }
        if let Some(Value::Object(mut providers)) = data.remove("providers")
            && let Some(Value::Object(anysola)) = providers.remove("anysola")
        {
            deep_merge(&mut self.provider, anysola);
        }
        if data.contains_key("plugins") && !self.warnings.iter().any(|w| w.contains("\"plugins\""))
        {
            self.warnings.push(
                "Plugins are not supported since Anysola Code 1.0; ignoring \"plugins\".".into(),
            );
        }
        self.mcp.apply(&data);
        data.remove("mcp");
        if let Some(formatter) = data.remove("formatter") {
            merge_formatter(&mut self.values, formatter);
        }
        for (key, value) in data {
            self.values.insert(key, value);
        }
    }
}

/// `formatter`: a boolean replaces everything before it; an object merges each named entry's
/// fields (and its `environment`) over the same name from earlier objects.
fn merge_formatter(values: &mut Map<String, Value>, next: Value) {
    let Value::Object(entries) = next else {
        values.insert("formatter".into(), next);
        return;
    };
    if !matches!(values.get("formatter"), Some(Value::Object(_))) {
        values.insert("formatter".into(), Value::Object(Map::new()));
    }
    let Some(Value::Object(merged)) = values.get_mut("formatter") else {
        return;
    };
    for (name, entry) in entries {
        let Value::Object(entry) = entry else {
            continue;
        };
        let current = merged
            .entry(name)
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(current) = current else {
            continue;
        };
        for (key, value) in entry {
            match (key.as_str(), current.get_mut(&key), value) {
                ("environment", Some(Value::Object(env)), Value::Object(more)) => env.extend(more),
                (_, _, value) => {
                    current.insert(key, value);
                }
            }
        }
    }
}

/// A document's policy statements in its own order: those generated from the legacy
/// `enabled_providers`/`disabled_providers` lists, then `experimental.policies`, then `policies`.
fn policy_statements(data: &mut Map<String, Value>) -> Vec<Policy> {
    let provider = |resource: &str, effect: &str| {
        Policy::parse(
            &serde_json::json!({"action": "provider.use", "resource": resource, "effect": effect}),
        )
    };
    let names = |value: Option<&Value>| -> Option<Vec<String>> {
        value.and_then(Value::as_array).map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
    };
    let mut out: Vec<Policy> = Vec::new();
    if let Some(enabled) = names(data.get("enabled_providers")) {
        out.extend(provider("*", "deny"));
        out.extend(enabled.iter().filter_map(|id| provider(id, "allow")));
    }
    if let Some(disabled) = names(data.get("disabled_providers")) {
        out.extend(disabled.iter().filter_map(|id| provider(id, "deny")));
    }
    let experimental = data
        .get("experimental")
        .and_then(|e| e.get("policies"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let native = match data.remove("policies") {
        Some(Value::Array(list)) => list,
        _ => Vec::new(),
    };
    out.extend(experimental.iter().chain(&native).filter_map(Policy::parse));
    out
}

fn deep_merge(base: &mut Map<String, Value>, next: Map<String, Value>) {
    for (key, value) in next {
        match (base.get_mut(&key), value) {
            (Some(Value::Object(existing)), Value::Object(more)) => deep_merge(existing, more),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn is_anysola_ref(value: &Value) -> bool {
    value.as_str().is_some_and(|m| m.starts_with("anysola/"))
}

/// Drops everything a local document may not set: other providers, models on other providers, and
/// the Anysola provider's endpoint keys (only the remote document sets those).
fn lock_local(data: &mut Map<String, Value>) {
    if data.get("model").is_some_and(|m| !is_anysola_ref(m)) {
        data.remove("model");
    }
    if let Some(Value::Object(providers)) = data.get_mut("providers") {
        providers.retain(|id, _| id == "anysola");
        if let Some(Value::Object(anysola)) = providers.get_mut("anysola") {
            for key in ["package", "canonical", "env"] {
                anysola.remove(key);
            }
            if let Some(Value::Object(settings)) = anysola.get_mut("settings") {
                for key in ENDPOINT_KEYS {
                    settings.remove(key);
                }
            }
        }
    }
    if let Some(Value::Object(agents)) = data.get_mut("agents") {
        for agent in agents.values_mut() {
            if let Value::Object(agent) = agent
                && agent.get("model").is_some_and(|m| !is_anysola_ref(m))
            {
                agent.remove("model");
            }
        }
    }
}

/// The remote document: only `providers.anysola`, an Anysola `model`, `agents`, and the legacy
/// `small_model` as the title agent's model. The provider is pinned to `<api>/v1/code`.
pub fn remote_document(body: &Value, api_url: &str) -> Option<Map<String, Value>> {
    let input = body.as_object()?;
    let mut doc = Map::new();
    if let Some(model) = input.get("model").filter(|m| is_anysola_ref(m)) {
        doc.insert("model".into(), model.clone());
    }
    let mut agents = input
        .get("agents")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(small) = input.get("small_model").filter(|m| is_anysola_ref(m)) {
        let title = agents
            .entry("title")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(title) = title {
            title.entry("model").or_insert(small.clone());
        }
    }
    if !agents.is_empty() {
        doc.insert("agents".into(), Value::Object(agents));
    }
    let mut provider = input
        .get("providers")
        .and_then(|p| p.get("anysola"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    provider.insert("name".into(), Value::String("Anysola".into()));
    let settings = provider
        .entry("settings")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(settings) = settings {
        settings.insert(
            "baseURL".into(),
            Value::String(format!("{api_url}/v1/code")),
        );
    }
    let mut providers = Map::new();
    providers.insert("anysola".into(), Value::Object(provider));
    doc.insert("providers".into(), Value::Object(providers));
    Some(normalize::normalize(doc))
}

/// What to load besides files.
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Body of the gateway's `/config` (live or cached).
    pub remote: Option<Value>,
    pub api_url: String,
}

/// Directories walked upward from `start`: through home (inclusive) when `start` is inside home,
/// else through the project root. Nearest first.
pub fn upward(start: &Path, home: &Path) -> Vec<PathBuf> {
    let stop = if start.starts_with(home) {
        home.to_path_buf()
    } else {
        crate::project::vcs_root(start).unwrap_or_else(|| start.to_path_buf())
    };
    let mut out = Vec::new();
    for dir in start.ancestors() {
        out.push(dir.to_path_buf());
        if dir == stop {
            break;
        }
    }
    out
}

/// The system configuration directory: managed settings that override everything.
fn system_dir() -> PathBuf {
    if cfg!(windows) {
        let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
        PathBuf::from(base).join("opencode")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/opencode")
    } else {
        PathBuf::from("/etc/opencode")
    }
}

/// A path with symlinks resolved as far as it exists, so a symlinked global root is recognized.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(real) = crate::paths::canonicalize(path) {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// macOS MDM managed preferences (`ai.opencode.managed`), converted to JSON by `plutil`.
#[cfg(any(target_os = "macos", test))]
mod preferences {
    use serde_json::Value;

    /// Keys macOS and MDM profiles add that are not config.
    const METADATA: [&str; 6] = [
        "PayloadDisplayName",
        "PayloadIdentifier",
        "PayloadType",
        "PayloadUUID",
        "PayloadVersion",
        "_manualProfile",
    ];

    pub fn strip_metadata(json: &str) -> String {
        match serde_json::from_str::<Value>(json) {
            Ok(Value::Object(mut map)) => {
                map.retain(|key, _| !METADATA.contains(&key.as_str()));
                crate::json::compact(&Value::Object(map))
            }
            _ => json.to_string(),
        }
    }

    /// The most specific plist present, as JSON text.
    #[cfg(target_os = "macos")]
    pub fn load() -> Option<(std::path::PathBuf, String)> {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let base = std::path::Path::new("/Library/Managed Preferences");
        let plist = [
            base.join(&user).join("ai.opencode.managed.plist"),
            base.join("ai.opencode.managed.plist"),
        ]
        .into_iter()
        .find(|p| p.is_file())?;
        let output = std::process::Command::new("plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(&plist)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success());
        let Some(output) = output else {
            crate::log::warn(
                "failed to convert managed preferences",
                &[("source", &plist.display().to_string())],
            );
            return None;
        };
        let text = strip_metadata(&String::from_utf8_lossy(&output.stdout));
        Some((plist, text))
    }
}

struct Loader<'a> {
    paths: &'a Paths,
    config: Config,
    managed: bool,
}

impl Loader<'_> {
    fn text(&mut self, text: &str, dir: &Path, source: &str) {
        let text = substitute::substitute(text, dir, &self.paths.home);
        match jsonc::parse(&text) {
            Ok(Value::Object(map)) => {
                let mut data = normalize::normalize(map);
                lock_local(&mut data);
                self.config.sources.push(Source::Document {
                    path: source.to_string(),
                    data: data.clone(),
                });
                self.config.apply(Document {
                    data,
                    managed: self.managed,
                    dir: Some(dir.to_path_buf()),
                });
            }
            _ => self
                .config
                .warnings
                .push(format!("Ignoring malformed config {source}")),
        }
    }

    fn file(&mut self, path: &Path) {
        if let Ok(text) = std::fs::read_to_string(path) {
            let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
            self.text(&text, &dir, &path.display().to_string());
        }
    }

    fn directory(&mut self, dir: &Path, files: &[&str]) {
        if dir.is_dir() {
            self.config
                .sources
                .push(Source::Directory(dir.to_path_buf()));
        }
        for name in files {
            self.file(&dir.join(name));
        }
        if dir.is_dir() {
            self.config
                .entries
                .push(Entry::Directory(dir.to_path_buf()));
        }
    }
}

/// Loads every source for a session directory.
pub fn load(paths: &Paths, directory: &Path, options: &LoadOptions) -> Config {
    let mut loader = Loader {
        paths,
        config: Config::default(),
        managed: false,
    };
    if let Some(remote) = options
        .remote
        .as_ref()
        .and_then(|body| remote_document(body, &options.api_url))
    {
        loader.config.apply(Document {
            data: remote,
            managed: false,
            dir: None,
        });
    }
    let legacy = paths.legacy_config_dir();
    loader.directory(&legacy, &NAMES);
    loader.directory(&paths.data, &["config.json"]);
    if let Some(file) = crate::env::var_os("CONFIG").filter(|v| !v.is_empty()) {
        loader.file(Path::new(&file));
    }
    let project_disabled =
        crate::env::flag("DISABLE_PROJECT_CONFIG") || crate::env::flag("CONFIG_PROJECT_DISABLE");
    let system = system_dir();
    let config_roots: Vec<PathBuf> = [&system, &legacy, &paths.data]
        .into_iter()
        .map(|dir| resolve(dir))
        .collect();
    let global_roots: Vec<PathBuf> = config_roots
        .iter()
        .cloned()
        .chain([".claude", ".agents"].map(|name| resolve(&paths.home.join(name))))
        .collect();
    let global_files: Vec<PathBuf> = [&system, &legacy]
        .into_iter()
        .flat_map(|dir| NAMES.map(|name| resolve(&dir.join(name))))
        .chain([resolve(&paths.data.join("config.json"))])
        .collect();
    if !project_disabled && !config_roots.contains(&resolve(directory)) {
        let walk: Vec<PathBuf> = upward(directory, &paths.home).into_iter().rev().collect();
        for dir in &walk {
            for name in NAMES {
                let file = dir.join(name);
                if !global_files.contains(&resolve(&file)) {
                    loader.file(&file);
                }
            }
        }
        for dir in &walk {
            for name in DOT_DIRS {
                let dot = dir.join(name);
                if !global_roots.contains(&resolve(&dot)) {
                    loader.directory(&dot, &NAMES);
                }
            }
        }
    }
    if let Some(content) = crate::env::var("CONFIG_CONTENT")
        && !content.trim().is_empty()
    {
        let [new, old] = crate::env::names("CONFIG_CONTENT");
        let name = if std::env::var_os(&new).is_some_and(|v| !v.is_empty()) {
            new
        } else {
            old
        };
        loader.text(&content, directory, &name);
    }
    loader.managed = true;
    loader.directory(&system, &NAMES);
    #[cfg(target_os = "macos")]
    if let Some((plist, text)) = preferences::load() {
        let dir = plist.parent().unwrap_or(Path::new("/")).to_path_buf();
        loader.text(&text, &dir, &plist.display().to_string());
    }
    loader.config
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn earlier_documents_and_managed_documents_win_policies() {
        let mut config = Config::default();
        let doc = |value: Value, managed: bool| Document {
            data: value.as_object().unwrap().clone(),
            managed,
            dir: None,
        };
        let deny =
            |resource: &str| json!({"action": "tool.use", "resource": resource, "effect": "deny"});
        let allow =
            |resource: &str| json!({"action": "tool.use", "resource": resource, "effect": "allow"});
        config.apply(doc(json!({"policies": [deny("shell:*")]}), false));
        config.apply(doc(json!({"policies": [allow("shell:*"), deny("read:*")], "experimental": {"policies": [deny("edit:*")]}}), false));
        let policies = config.policies();
        assert!(policies.tool_denial("shell", &["ls".into()]).is_some());
        assert!(policies.tool_denial("read", &["a".into()]).is_some());
        assert!(policies.tool_denial("edit", &["a".into()]).is_some());
        config.apply(doc(json!({"policies": [allow("read:*")]}), true));
        config.apply(doc(json!({"policies": [deny("read:*")]}), true));
        assert!(
            config
                .policies()
                .tool_denial("read", &["a".into()])
                .is_none()
        );
        assert!(config.get("policies").is_none());
    }

    #[test]
    fn remote_document_keeps_only_anysola() {
        let body = json!({
            "model": "anysola/claude-sonnet-5-5",
            "small_model": "anysola/claude-haiku-5-5",
            "providers": {"anysola": {"settings": {"baseURL": "https://elsewhere"}, "models": {"m": {}}}, "other": {}},
        });
        let doc = remote_document(&body, "http://x").unwrap();
        assert_eq!(doc["agents"]["title"]["model"], "anysola/claude-haiku-5-5");
        assert_eq!(
            doc["providers"]["anysola"]["settings"]["baseURL"],
            "http://x/v1/code"
        );
        assert!(doc["providers"].get("other").is_none());
    }

    #[test]
    fn local_documents_are_locked() {
        let mut data = normalize::normalize(
            json!({"model": "openai/gpt", "providers": {"openai": {}, "anysola": {"settings": {"apiKey": "k"}}}})
                .as_object()
                .unwrap()
                .clone(),
        );
        lock_local(&mut data);
        assert!(!data.contains_key("model"));
        assert_eq!(
            Value::Object(data)["providers"],
            json!({"anysola": {"settings": {}}})
        );
    }

    #[test]
    fn precedence_and_merge() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        std::fs::create_dir_all(project.join(".opencode")).unwrap();
        std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
        std::fs::write(
            home.join(".config/opencode/opencode.json"),
            r#"{"shell":"zsh","permissions":[{"action":"a","resource":"*","effect":"deny"}]}"#,
        )
        .unwrap();
        std::fs::write(
            project.join("opencode.jsonc"),
            "{\"shell\": \"bash\", // c\n \"instructions\": [\"x.md\",],}",
        )
        .unwrap();
        std::fs::write(
            project.join(".opencode/opencode.json"),
            r#"{"agents":{"build":{"steps":3}}}"#,
        )
        .unwrap();
        std::fs::write(
            root.path().join("opencode.json"),
            r#"{"shell":"never-read-above-home"}"#,
        )
        .unwrap();
        let paths = Paths::isolated(home.clone());
        let config = load(&paths, &project, &LoadOptions::default());
        assert_eq!(config.shell(), Some("bash"));
        assert_eq!(config.instructions, vec!["x.md"]);
        assert_eq!(config.permissions.len(), 1);
        assert_eq!(
            config.entries.iter().rev().nth(1),
            Some(&Entry::Document {
                agents: json!({"build": {"steps": 3}}).as_object().unwrap().clone(),
                commands: Map::new(),
            })
        );
    }

    #[test]
    fn new_names_win_over_the_old_ones_still_read() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        let write = |path: PathBuf, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            project.join("opencode.json"),
            r#"{"shell":"zsh","instructions":["old.md"]}"#,
        );
        write(project.join("anysola.jsonc"), r#"{"shell":"bash"}"#);
        let paths = Paths::isolated(home.clone());
        let config = load(&paths, &project, &LoadOptions::default());
        assert_eq!(config.shell(), Some("bash"));
        assert_eq!(config.instructions, vec!["old.md"]);
        write(
            project.join(".opencode/opencode.json"),
            r#"{"agents":{"build":{"steps":3}}}"#,
        );
        write(
            project.join(".anysola/anysola.json"),
            r#"{"agents":{"build":{"steps":5}}}"#,
        );
        let config = load(&paths, &project, &LoadOptions::default());
        let dirs: Vec<&Entry> = config
            .entries
            .iter()
            .filter(|e| matches!(e, Entry::Directory(_)))
            .collect();
        assert_eq!(
            dirs,
            [
                &Entry::Directory(project.join(".opencode")),
                &Entry::Directory(project.join(".anysola")),
            ]
        );
        assert_eq!(
            config.entries.iter().rev().nth(1),
            Some(&Entry::Document {
                agents: json!({"build": {"steps": 5}}).as_object().unwrap().clone(),
                commands: Map::new(),
            })
        );
    }

    #[test]
    fn preference_metadata_is_stripped() {
        assert_eq!(
            preferences::strip_metadata(
                r#"{"PayloadUUID":"x","_manualProfile":true,"model":"anysola/m","PayloadType":"t"}"#
            ),
            r#"{"model":"anysola/m"}"#
        );
        assert_eq!(preferences::strip_metadata("[1]"), "[1]");
    }

    #[cfg(unix)]
    #[test]
    fn walk_skipped_in_a_symlinked_global_root() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let real = root.path().join("dotfiles/opencode");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::os::unix::fs::symlink(&real, home.join(".config/opencode")).unwrap();
        std::fs::write(real.join("opencode.json"), r#"{"shell":"zsh"}"#).unwrap();
        std::fs::create_dir_all(real.join(".opencode")).unwrap();
        std::fs::write(real.join(".opencode/opencode.json"), r#"{"shell":"fish"}"#).unwrap();
        let paths = Paths::isolated(home.clone());
        let config = load(&paths, &real, &LoadOptions::default());
        assert_eq!(config.shell(), Some("zsh"));
        let project = home.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("opencode.json"), r#"{"shell":"bash"}"#).unwrap();
        assert_eq!(
            load(&paths, &project, &LoadOptions::default()).shell(),
            Some("bash")
        );
    }
}
