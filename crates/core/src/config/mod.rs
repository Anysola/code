//! Core config: discovery of every source, parsing (JSONC, substitution, V1 normalization), the
//! Anysola provider lock, and the merge into one [`Config`].
//!
//! Sources, lowest precedence first: the gateway's remote document, `~/.config/opencode`
//! (`OPENCODE_CONFIG_DIR`), `~/.anysola/config.json`, `OPENCODE_CONFIG`, project `opencode.json[c]`
//! files farthest first, project `.opencode/` directories farthest first, `OPENCODE_CONFIG_CONTENT`,
//! then managed settings.

mod jsonc;
mod normalize;
mod substitute;

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::paths::Paths;
use crate::permission::Rule;
use crate::tool::OutputLimits;

pub use normalize::{markdown_agent, model_ref, normalize_action};

const NAMES: [&str; 2] = ["opencode.json", "opencode.jsonc"];
const ENDPOINT_KEYS: [&str; 4] = ["baseURL", "apiKey", "authToken", "accessToken"];

/// One parsed, normalized document.
#[derive(Debug, Clone)]
struct Document {
    data: Map<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Top-level keys, latest document wins (maps below are merged separately).
    values: Map<String, Value>,
    /// `agents.<id>` merged field by field, `permissions` concatenated.
    pub agents: Map<String, Value>,
    /// Top-level `permissions` of every document, in load order.
    pub permissions: Vec<Rule>,
    /// `instructions` of every document, deduplicated.
    pub instructions: Vec<String>,
    /// `providers.anysola` merged.
    pub provider: Map<String, Value>,
    /// Directories that hold markdown definitions (`agent/`, `command/`, …), lowest precedence first.
    pub directories: Vec<PathBuf>,
    /// Problems worth one warning line each (malformed documents, ignored keys).
    pub warnings: Vec<String>,
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

    pub fn websearch_enabled(&self) -> bool {
        self.get("websearch") != Some(&Value::Bool(false))
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

    pub fn models(&self) -> Option<&Map<String, Value>> {
        self.provider.get("models").and_then(Value::as_object)
    }

    fn apply(&mut self, document: Document) {
        let mut data = document.data;
        if let Some(Value::Array(rules)) = data.remove("permissions") {
            self.permissions.extend(
                rules
                    .into_iter()
                    .filter_map(|r| serde_json::from_value(r).ok()),
            );
        }
        if let Some(Value::Array(items)) = data.remove("instructions") {
            for item in items
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
            {
                if !self.instructions.contains(&item) {
                    self.instructions.push(item);
                }
            }
        }
        if let Some(Value::Object(agents)) = data.remove("agents") {
            for (id, agent) in agents {
                merge_agent(
                    self.agents
                        .entry(id)
                        .or_insert_with(|| Value::Object(Map::new())),
                    agent,
                );
            }
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
        for (key, value) in data {
            self.values.insert(key, value);
        }
    }
}

fn merge_agent(base: &mut Value, next: Value) {
    let (Value::Object(base), Value::Object(next)) = (base, next) else {
        return;
    };
    for (key, value) in next {
        match (key.as_str(), base.get_mut(&key), value) {
            ("permissions", Some(Value::Array(existing)), Value::Array(more)) => {
                existing.extend(more)
            }
            (_, _, value) => {
                base.insert(key, value);
            }
        }
    }
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
pub fn remote_document(body: &str, api_url: &str) -> Option<Map<String, Value>> {
    let Value::Object(input) = serde_json::from_str::<Value>(body).ok()? else {
        return None;
    };
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
    /// Raw body of the gateway's `/config` (live or cached).
    pub remote: Option<String>,
    pub api_url: String,
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0" && v != "false")
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

fn managed_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("ProgramData")
            .map(|d| PathBuf::from(d).join("opencode"))
            .into_iter()
            .collect()
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Library/Application Support/opencode")]
    } else {
        vec![PathBuf::from("/etc/opencode")]
    }
}

struct Loader<'a> {
    paths: &'a Paths,
    config: Config,
}

impl Loader<'_> {
    fn text(&mut self, text: &str, dir: &Path, source: &str) {
        let text = substitute::substitute(text, dir, &self.paths.home);
        match jsonc::parse(&text) {
            Ok(Value::Object(map)) => {
                let mut data = normalize::normalize(map);
                lock_local(&mut data);
                self.config.apply(Document { data });
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
        for name in files {
            self.file(&dir.join(name));
        }
        if dir.is_dir() {
            self.config.directories.push(dir.to_path_buf());
        }
    }
}

/// Loads every source for a session directory.
pub fn load(paths: &Paths, directory: &Path, options: &LoadOptions) -> Config {
    let mut loader = Loader {
        paths,
        config: Config::default(),
    };
    if let Some(remote) = options
        .remote
        .as_deref()
        .and_then(|body| remote_document(body, &options.api_url))
    {
        loader.config.apply(Document { data: remote });
    }
    let legacy = paths.legacy_config_dir();
    loader.directory(&legacy, &NAMES);
    loader.directory(&paths.data, &["config.json"]);
    if let Some(file) = std::env::var_os("OPENCODE_CONFIG").filter(|v| !v.is_empty()) {
        loader.file(Path::new(&file));
    }
    let project_disabled =
        env_flag("OPENCODE_DISABLE_PROJECT_CONFIG") || env_flag("OPENCODE_CONFIG_PROJECT_DISABLE");
    let global_roots = [legacy.clone(), paths.data.clone()];
    if !project_disabled && !global_roots.iter().any(|root| root == directory) {
        let walk: Vec<PathBuf> = upward(directory, &paths.home).into_iter().rev().collect();
        for dir in &walk {
            for name in NAMES {
                let file = dir.join(name);
                if !global_roots.iter().any(|root| file.starts_with(root)) {
                    loader.file(&file);
                }
            }
        }
        for dir in &walk {
            let dot = dir.join(".opencode");
            if !global_roots.contains(&dot) {
                loader.directory(&dot, &NAMES);
            }
        }
    }
    if let Ok(content) = std::env::var("OPENCODE_CONFIG_CONTENT")
        && !content.trim().is_empty()
    {
        loader.text(&content, directory, "OPENCODE_CONFIG_CONTENT");
    }
    for dir in managed_dirs() {
        loader.directory(&dir, &NAMES);
    }
    loader.config
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remote_document_keeps_only_anysola() {
        let body = json!({
            "model": "anysola/claude-sonnet-5-5",
            "small_model": "anysola/claude-haiku-5-5",
            "providers": {"anysola": {"settings": {"baseURL": "https://elsewhere"}, "models": {"m": {}}}, "other": {}},
        });
        let doc = remote_document(&body.to_string(), "http://x").unwrap();
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
        assert_eq!(config.agents["build"]["steps"], 3);
    }
}
