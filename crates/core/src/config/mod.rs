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
pub(crate) mod mcp;

pub use mcp::{local_server, remote_server};
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
    /// The file (or variable) it came from; `None` for the gateway's document.
    source: Option<String>,
    /// Found by the upward walk from the session directory (its hooks need approval).
    project: bool,
    /// Sections of a project document nobody approved: not applied.
    withheld: Withheld,
}

#[derive(Debug, Clone, Copy, Default)]
struct Withheld {
    mcp: bool,
    formatter: bool,
    /// Loosening permission rules and policy statements ([`crate::trust::loosens`]).
    permissions: bool,
    /// `instructions` URLs and git `references`.
    remote: bool,
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
        /// The document's file (or variable); `None` for the gateway's document.
        source: Option<String>,
        /// Found by the upward project walk: its agents' rules are a project's
        /// ([`crate::permission::Rule::project`]).
        project: bool,
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
    /// `hooks` of every local document, each with its source and scope (never in `values`).
    pub hooks: crate::hooks::Configured,
    /// Files of user and managed documents the loader read, and the `ANYSOLA_CONFIG` file even
    /// when it does not exist yet: writing them changes what runs without asking.
    pub own_documents: Vec<PathBuf>,
    /// The `mcp` and `formatter` sections of project documents, with the decision on each (see
    /// [`crate::trust`]); those not approved were not applied.
    pub trust: Vec<crate::trust::Section>,
    /// Project documents whose `mcp` section was not applied.
    withheld_mcp: Vec<String>,
    /// `instructions` entries only project documents name: what they find must lie inside the
    /// project ([`crate::instructions::discover_scoped`]).
    pub project_instructions: Vec<String>,
    /// The markdown agents of each project directory as read once at load, `(id, agent, file)`,
    /// with the loosening rules of unapproved files left out: the rules applied are the rules
    /// judged.
    pub(crate) project_agents: Vec<(PathBuf, Vec<crate::agent::Defined>)>,
    /// What [`Config::project_agents`] left out: `(file, agent id, rules)` of unapproved files.
    pub(crate) withheld_agent_rules: Vec<(String, String, Vec<Value>)>,
    /// The documents the upward project walk found, by path.
    pub project_documents: Vec<String>,
    /// The last project document's `default_agent` (see [`Config::default_agent`]).
    project_default_agent: Option<Value>,
}

impl Config {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn model(&self) -> Option<&str> {
        self.get("model").and_then(Value::as_str)
    }

    /// The user's `default_agent`; a project's only when no other document sets one.
    pub fn default_agent(&self) -> Option<&str> {
        self.get("default_agent")
            .or(self.project_default_agent.as_ref())
            .and_then(Value::as_str)
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

    /// A config directory the upward project walk found (its definitions are a project's).
    pub fn is_project_directory(&self, dir: &Path) -> bool {
        self.project_agents.iter().any(|(d, _)| d == dir)
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

    /// The last local document that sets the top-level `key` (a later document replaces the
    /// whole value), as its path; the gateway's document is not one.
    pub fn source_of(&self, key: &str) -> Option<&str> {
        self.documents()
            .filter(|(_, data)| data.contains_key(key))
            .last()
            .map(|(path, _)| path)
    }

    /// The last local document that defines MCP server `name`.
    pub fn mcp_source(&self, name: &str) -> Option<&str> {
        self.documents()
            .filter(|(path, _)| !self.withheld_mcp.iter().any(|w| w == path))
            .filter(|(_, data)| mcp::defines(data, name))
            .last()
            .map(|(path, _)| path)
    }

    /// The local documents in load order: `(path, data)`.
    pub fn documents(&self) -> impl Iterator<Item = (&str, &Map<String, Value>)> {
        self.sources.iter().filter_map(|source| match source {
            Source::Document { path, data } => Some((path.as_str(), data)),
            Source::Directory(_) => None,
        })
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
                source: None,
                project: false,
                withheld: Withheld::default(),
            });
        }
        config
    }

    fn apply(&mut self, document: Document) {
        let mut data = document.data;
        let hooks = data.remove("hooks");
        let disable = data.remove("disable_hooks");
        // Where worktrees are checked out: a repository could aim it at the user's own agent
        // folders.
        if document.project
            && let Some(Value::Object(worktree)) = data.get_mut("worktree")
            && worktree.remove("directory").is_some()
        {
            self.warnings.push(format!(
                "Ignoring \"worktree.directory\" in {}: it only works in your own config (~/.anysola/config.json).",
                document.source.as_deref().unwrap_or("a project document")
            ));
        }
        // The shell runs every command and hook: a repository must not choose it.
        if document.project && data.remove("shell").is_some() {
            self.warnings.push(format!(
                "Ignoring \"shell\" in {}: it only works in your own config (~/.anysola/config.json).",
                document.source.as_deref().unwrap_or("a project document")
            ));
        }
        // The gateway's document has no source and runs no hooks.
        if let Some(source) = &document.source {
            let scope = if document.managed {
                crate::hooks::HookScope::Managed
            } else if document.project {
                crate::hooks::HookScope::Project
            } else {
                crate::hooks::HookScope::User
            };
            crate::hooks::apply(
                &mut self.hooks,
                &data,
                hooks,
                disable,
                source,
                scope,
                &mut self.warnings,
            );
        }
        let mut statements = policy_statements(&mut data);
        if document.withheld.permissions {
            if crate::trust::loosening_default(&data).is_some() {
                data.remove("default_agent");
            }
            statements.retain(|p| !crate::trust::loosening_policy(p));
            if let Some(Value::Array(rules)) = data.get_mut("permissions") {
                rules.retain(|r| !crate::trust::loosens(r));
            }
            if let Some(Value::Object(agents)) = data.get_mut("agents") {
                for agent in agents.values_mut() {
                    if let Some(Value::Array(rules)) = agent.get_mut("permissions") {
                        rules.retain(|r| !crate::trust::loosens(r));
                    }
                }
            }
        }
        if document.project
            && let Some(agent) = data.remove("default_agent")
        {
            self.project_default_agent = Some(agent);
        }
        if document.withheld.remote {
            for key in ["instructions", "skills"] {
                if let Some(Value::Array(items)) = data.get_mut(key) {
                    items.retain(|i| !i.as_str().is_some_and(crate::trust::remote_instruction));
                }
            }
            if let Some(Value::Object(references)) = data.get_mut("references") {
                references.retain(|_, r| crate::trust::remote_reference(r).is_none());
            }
        }
        if document.project {
            for statement in &mut statements {
                statement.project = true;
            }
        }
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
                    .filter_map(|r| serde_json::from_value::<Rule>(r).ok())
                    .map(|r| {
                        if document.project {
                            r.from_project()
                        } else {
                            r
                        }
                    }),
            );
        }
        if let Some(Value::Array(items)) = data.get_mut("instructions") {
            let source = document.source.as_deref().unwrap_or_default();
            if document.project {
                // A project names files inside the project only: `~`, absolute paths and `..`
                // would read the user's private files into the prompt with no ask.
                items.retain(|item| {
                    let Some(entry) = item.as_str() else {
                        return false;
                    };
                    let outside = !crate::trust::remote_instruction(entry)
                        && (entry.starts_with('~')
                            || Path::new(entry).is_absolute()
                            || entry.starts_with('/')
                            || entry.starts_with('\\')
                            || Path::new(entry)
                                .components()
                                .any(|c| c == std::path::Component::ParentDir));
                    if outside {
                        self.warnings.push(format!(
                            "Ignoring the instruction \"{entry}\" in {source}: a project's instructions can only name files inside the project; put it in your own config."
                        ));
                    }
                    !outside
                });
                for entry in items.iter().filter_map(Value::as_str) {
                    if !crate::trust::remote_instruction(entry)
                        && !self.instructions.iter().any(|i| i == entry)
                        && !self.project_instructions.iter().any(|i| i == entry)
                    {
                        self.project_instructions.push(entry.to_string());
                    }
                }
            } else {
                // The user's own config names it too: any file it names is the user's choice.
                for entry in items.iter().filter_map(Value::as_str) {
                    self.project_instructions.retain(|i| i != entry);
                }
            }
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
            self.entries.push(Entry::Document {
                agents,
                commands,
                source: document.source.clone(),
                project: document.project,
            });
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
        if document.withheld.mcp {
            // Not approved: as if the document had no MCP section (its legacy timeout included).
            if let Some(Value::Object(experimental)) = data.get_mut("experimental") {
                experimental.remove("mcp_timeout");
            }
            if let Some(source) = &document.source {
                self.withheld_mcp.push(source.clone());
            }
        } else {
            self.mcp.apply(&data);
        }
        data.remove("mcp");
        if let Some(formatter) = data.remove("formatter")
            && !document.withheld.formatter
        {
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
pub(crate) fn policy_statements(data: &mut Map<String, Value>) -> Vec<Policy> {
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
    /// The decisions on project documents' sections ([`crate::trust::Decisions::read`]). The
    /// default holds none: no project MCP server or formatter is applied.
    pub trust: crate::trust::Decisions,
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
pub(crate) fn system_dir() -> PathBuf {
    if cfg!(windows) {
        let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
        PathBuf::from(base).join("opencode")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/opencode")
    } else {
        PathBuf::from("/etc/opencode")
    }
}

/// Whether `path` is a file that changes what runs without asking: a config document the loader
/// reads from a global root (`~/.anysola/config.json`, the legacy config dir's files, anything
/// under the system dir), the database (with its `-wal`, `-shm` and `-journal` files), or the
/// desktop app's preferences (the mode new sessions start in). The rest of the data dir (plans,
/// tool output, state, …) is not.
pub fn protected(paths: &Paths, path: &Path) -> bool {
    let target = resolve(path);
    if target.starts_with(resolve(&system_dir())) {
        return true;
    }
    // Definitions in the global roots that change what the agent may do or runs.
    const DEFINITIONS: [&str; 8] = [
        "agent", "agents", "mode", "modes", "command", "commands", "skill", "skills",
    ];
    let roots = [paths.data.clone(), paths.legacy_config_dir()];
    if roots.iter().any(|root| {
        DEFINITIONS
            .iter()
            .any(|folder| target.starts_with(resolve(&root.join(folder))))
    }) {
        return true;
    }
    let database = paths.database().display().to_string();
    // The global instructions join every project's system prompt.
    let mut files = vec![
        paths.config_file(),
        paths.desktop_prefs(),
        paths.data.join("AGENTS.md"),
        paths.legacy_config_dir().join("AGENTS.md"),
    ];
    files.extend(["", "-wal", "-shm", "-journal"].map(|s| PathBuf::from(format!("{database}{s}"))));
    files.extend(NAMES.map(|name| paths.legacy_config_dir().join(name)));
    files.iter().any(|file| resolve(file) == target)
}

impl Config {
    /// [`protected`], plus every user or managed document this config was read from.
    pub fn protects(&self, paths: &Paths, path: &Path) -> bool {
        let target = resolve(path);
        protected(paths, path)
            || self
                .own_documents
                .iter()
                .any(|file| resolve(file) == target)
    }
}

/// `path` with symlinks resolved as far as it exists: the file a write through it changes.
pub fn real_path(path: &Path) -> PathBuf {
    resolve(path)
}

/// Symlinks followed before giving up on a chain (a loop, or a very long one).
const MAX_LINKS: usize = 40;

/// A path with symlinks resolved as far as it exists, so a symlinked global root is recognized:
/// the deepest existing ancestor is resolved and the rest joined on, and a dangling symlink is
/// followed to where it points (writing through it would land there).
fn resolve(path: &Path) -> PathBuf {
    resolve_within(path, MAX_LINKS)
}

fn resolve_within(path: &Path, links: usize) -> PathBuf {
    if let Ok(real) = crate::paths::canonicalize(path) {
        return crate::paths::plain(&real);
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return path.to_path_buf();
    };
    let parent = resolve_within(parent, links);
    let joined = parent.join(name);
    if links > 0
        && let Ok(target) = std::fs::read_link(&joined)
    {
        return resolve_within(&crate::paths::normalize(&parent.join(target)), links - 1);
    }
    joined
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
    /// Loading the documents of the upward project walk.
    project: bool,
    decisions: &'a crate::trust::Decisions,
}

/// One local document as the loader takes it: variables substituted, the 0.5.x keys moved to
/// their current names, and the provider lock applied. `None` when it isn't a JSON object.
pub fn document(text: &str, dir: &Path, paths: &Paths) -> Option<Map<String, Value>> {
    checked_document(text, dir, paths, false, &mut Vec::new())
}

/// [`document`] with the `{file:…}` limits of a project or user document; what substitution left
/// as written goes to `problems`.
fn checked_document(
    text: &str,
    dir: &Path,
    paths: &Paths,
    project: bool,
    problems: &mut Vec<String>,
) -> Option<Map<String, Value>> {
    let text = substitute::checked(text, dir, &paths.home, project, problems);
    match jsonc::parse(&text) {
        Ok(Value::Object(map)) => {
            let mut data = normalize::normalize(map);
            lock_local(&mut data);
            Some(data)
        }
        _ => None,
    }
}

impl Loader<'_> {
    fn text(&mut self, text: &str, dir: &Path, source: &str) {
        let mut problems = Vec::new();
        let parsed = checked_document(text, dir, self.paths, self.project, &mut problems);
        for problem in problems {
            self.config
                .warnings
                .push(format!("In {source}, {problem}."));
        }
        match parsed {
            Some(data) => {
                let withheld = if self.project {
                    // The document as written, for showing placeholders in the approval text.
                    let raw = match jsonc::parse(text) {
                        Ok(Value::Object(map)) => Some(normalize::normalize(map)),
                        _ => None,
                    };
                    self.withhold(&data, raw.as_ref(), source)
                } else {
                    Withheld::default()
                };
                if self.project {
                    self.config.project_documents.push(source.to_string());
                }
                self.config.sources.push(Source::Document {
                    path: source.to_string(),
                    data: data.clone(),
                });
                self.config.apply(Document {
                    data,
                    managed: self.managed,
                    dir: Some(dir.to_path_buf()),
                    source: Some(source.to_string()),
                    project: self.project,
                    withheld,
                });
            }
            _ => self
                .config
                .warnings
                .push(format!("Ignoring malformed config {source}")),
        }
    }

    /// Records the project document's MCP and formatter sections with the decisions on them;
    /// what is not approved is withheld.
    fn withhold(
        &mut self,
        data: &Map<String, Value>,
        raw: Option<&Map<String, Value>>,
        source: &str,
    ) -> Withheld {
        use crate::trust::{Section, TrustKind};
        let mut withheld = Withheld::default();
        for (kind, section) in [
            (TrustKind::Mcp, crate::trust::mcp_section_written(data, raw)),
            (TrustKind::Formatter, crate::trust::formatter_section(data)),
            (
                TrustKind::Permissions,
                crate::trust::permissions_section(data),
            ),
            (TrustKind::Remote, crate::trust::remote_section(data)),
        ] {
            let Some((digest, entries)) = section else {
                continue;
            };
            let decision = self.decisions.of(source, kind, &digest);
            if decision != Some(true) {
                match kind {
                    TrustKind::Mcp => withheld.mcp = true,
                    TrustKind::Formatter => withheld.formatter = true,
                    TrustKind::Permissions => withheld.permissions = true,
                    TrustKind::Remote => withheld.remote = true,
                    TrustKind::Hooks => {}
                }
            }
            self.config.trust.push(Section {
                path: source.to_string(),
                kind,
                digest,
                decision,
                entries,
            });
        }
        withheld
    }

    fn file(&mut self, path: &Path) {
        // A project's documents must be regular files of a sane size (a symlink to a device must
        // not exhaust memory); the user's own are read as they always were.
        let read = if self.project {
            match crate::paths::read_text(path, crate::paths::DOCUMENT_CAP) {
                Ok(text) => Ok(text),
                Err(crate::paths::Unread::Io(error)) => Err(error),
                Err(reason) => {
                    let why = match reason {
                        crate::paths::Unread::TooLarge => "it is larger than 1 MiB",
                        _ => "it is not a regular file",
                    };
                    self.config
                        .warnings
                        .push(format!("Ignoring {}: {why}.", path.display()));
                    return;
                }
            }
        } else {
            std::fs::read_to_string(path)
        };
        if let Ok(text) = read {
            if !self.project {
                self.config.own_documents.push(path.to_path_buf());
            }
            let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
            self.text(&text, &dir, &path.display().to_string());
        }
    }

    /// A project directory's markdown agents: each file is its own document for approval of the
    /// rules that loosen what the agent may do; unapproved, those rules are withheld.
    fn agent_files(&mut self, dir: &Path) {
        use crate::trust::{Section, TrustKind};
        let mut agents = crate::agent::markdown_agents(dir);
        for (id, agent, file) in &mut agents {
            let Some(file) = file else {
                continue;
            };
            let Some((digest, entries)) = crate::trust::agent_file_section(id, agent, file) else {
                continue;
            };
            let decision = self.decisions.of(file, TrustKind::Permissions, &digest);
            if decision != Some(true)
                && let Some(Value::Array(rules)) = agent.get_mut("permissions")
            {
                let (withheld, kept) = rules.drain(..).partition(crate::trust::loosens);
                *rules = kept;
                self.config
                    .withheld_agent_rules
                    .push((file.clone(), id.clone(), withheld));
            }
            self.config.trust.push(Section {
                path: file.clone(),
                kind: TrustKind::Permissions,
                digest,
                decision,
                entries,
            });
        }
        self.config.project_agents.push((dir.to_path_buf(), agents));
    }

    fn directory(&mut self, dir: &Path, files: &[&str]) {
        if dir.is_dir() {
            self.config
                .sources
                .push(Source::Directory(dir.to_path_buf()));
            if self.project {
                self.agent_files(dir);
            }
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
        project: false,
        decisions: &options.trust,
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
            source: None,
            project: false,
            withheld: Withheld::default(),
        });
    }
    let legacy = paths.legacy_config_dir();
    loader.directory(&legacy, &NAMES);
    loader.directory(&paths.data, &["config.json"]);
    if let Some(file) = crate::env::var_os("CONFIG").filter(|v| !v.is_empty()) {
        loader.file(Path::new(&file));
        let file = PathBuf::from(file);
        if !loader.config.own_documents.contains(&file) {
            loader.config.own_documents.push(file);
        }
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
        loader.project = true;
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
        loader.project = false;
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
    let config = &mut loader.config;
    if let (Some(own), Some(project)) = (config.get("default_agent"), &config.project_default_agent)
        && own != project
    {
        config.warnings.push(format!(
            "Ignoring the project's \"default_agent\" ({}): your own config chooses the agent.",
            crate::json::compact(project)
        ));
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
            source: None,
            project: false,
            withheld: Withheld::default(),
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
    fn a_project_policy_never_lifts_the_users_denial_and_keeps_its_rank() {
        let doc = |value: Value, project: bool| Document {
            data: value.as_object().unwrap().clone(),
            managed: false,
            dir: None,
            source: None,
            project,
            withheld: Withheld::default(),
        };
        let statement = |resource: &str, effect: &str| json!({"action": "tool.use", "resource": resource, "effect": effect});
        // The user's deny, then an approved project allow: still denied.
        let mut config = Config::default();
        config.apply(doc(
            json!({"policies": [statement("shell:*", "deny")]}),
            false,
        ));
        config.apply(doc(
            json!({"policies": [statement("shell:*", "allow")]}),
            true,
        ));
        assert!(
            config
                .policies()
                .tool_denial("shell", &["ls".into()])
                .is_some()
        );
        // An approved project allow, then `ANYSOLA_CONFIG_CONTENT`'s deny: denied.
        let mut config = Config::default();
        config.apply(doc(
            json!({"policies": [statement("shell:*", "allow")]}),
            true,
        ));
        config.apply(doc(
            json!({"policies": [statement("shell:*", "deny")]}),
            false,
        ));
        assert!(
            config
                .policies()
                .tool_denial("shell", &["ls".into()])
                .is_some()
        );
        // As in 1.1.0, a project's deny outranks a later user allow (it loaded earlier).
        let mut config = Config::default();
        config.apply(doc(
            json!({"policies": [statement("read:*", "deny")]}),
            true,
        ));
        config.apply(doc(
            json!({"policies": [statement("read:*", "allow")]}),
            false,
        ));
        assert!(
            config
                .policies()
                .tool_denial("read", &["a".into()])
                .is_some()
        );
        // A project allow lifts nothing of a user's, even an earlier-ranked one.
        let mut config = Config::default();
        config.apply(doc(
            json!({"policies": [statement("mcp_x:*", "allow")]}),
            false,
        ));
        config.apply(doc(
            json!({"policies": [statement("edit:*", "deny")]}),
            false,
        ));
        config.apply(doc(
            json!({"policies": [statement("edit:*", "allow")]}),
            true,
        ));
        assert!(
            config
                .policies()
                .tool_denial("edit", &["a".into()])
                .is_some()
        );
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
            r#"{"theme":"zsh","permissions":[{"action":"a","resource":"*","effect":"deny"}]}"#,
        )
        .unwrap();
        std::fs::write(
            project.join("opencode.jsonc"),
            "{\"theme\": \"bash\", // c\n \"instructions\": [\"x.md\",],}",
        )
        .unwrap();
        std::fs::write(
            project.join(".opencode").join("opencode.json"),
            r#"{"agents":{"build":{"steps":3}}}"#,
        )
        .unwrap();
        std::fs::write(
            root.path().join("opencode.json"),
            r#"{"theme":"never-read-above-home"}"#,
        )
        .unwrap();
        let paths = Paths::isolated(home.clone());
        let config = load(&paths, &project, &LoadOptions::default());
        assert_eq!(config.get("theme").and_then(Value::as_str), Some("bash"));
        assert_eq!(config.instructions, vec!["x.md"]);
        assert_eq!(config.permissions.len(), 1);
        assert_eq!(
            config.entries.iter().rev().nth(1),
            Some(&Entry::Document {
                agents: json!({"build": {"steps": 3}}).as_object().unwrap().clone(),
                commands: Map::new(),
                source: Some(
                    project
                        .join(".opencode")
                        .join("opencode.json")
                        .display()
                        .to_string()
                ),
                project: true,
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
            r#"{"theme":"zsh","instructions":["old.md"]}"#,
        );
        write(project.join("anysola.jsonc"), r#"{"theme":"bash"}"#);
        let paths = Paths::isolated(home.clone());
        let config = load(&paths, &project, &LoadOptions::default());
        assert_eq!(config.get("theme").and_then(Value::as_str), Some("bash"));
        assert_eq!(config.instructions, vec!["old.md"]);
        write(
            project.join(".opencode").join("opencode.json"),
            r#"{"agents":{"build":{"steps":3}}}"#,
        );
        write(
            project.join(".anysola").join("anysola.json"),
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
                source: Some(
                    project
                        .join(".anysola")
                        .join("anysola.json")
                        .display()
                        .to_string()
                ),
                project: true,
            })
        );
    }

    #[test]
    fn a_hand_written_trailing_comma_before_a_comment_loads() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join(".anysola")).unwrap();
        let paths = Paths::isolated(home.clone());
        std::fs::write(paths.config_file(), "{\n  \"shell\": \"zsh\", // mine\n}\n").unwrap();
        let config = load(&paths, &home, &LoadOptions::default());
        assert_eq!(config.shell(), Some("zsh"));
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    }

    #[test]
    fn which_document_defines_a_key_or_server() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        std::fs::create_dir_all(home.join(".anysola")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let paths = Paths::isolated(home.clone());
        std::fs::write(
            paths.config_file(),
            r#"{"websearch": false, "experimental": {"todo": true},
                "mcp": {"servers": {"mine": {"type": "remote", "url": "https://m/mcp"},
                                    "shared": {"type": "local", "command": ["a"]}},
                        "old": {"type": "local", "command": ["b"]}}}"#,
        )
        .unwrap();
        std::fs::write(
            project.join("anysola.json"),
            r#"{"experimental": {"subagent_depth": 2},
                "mcp": {"servers": {"shared": {"type": "local", "command": ["c"]}}}}"#,
        )
        .unwrap();
        let config = load(&paths, &project, &LoadOptions::default());
        let global = paths.config_file().display().to_string();
        let local = project.join("anysola.json").display().to_string();
        assert_eq!(config.source_of("websearch"), Some(global.as_str()));
        assert_eq!(config.source_of("experimental"), Some(local.as_str()));
        assert_eq!(config.source_of("shell"), None);
        assert_eq!(config.mcp_source("mine"), Some(global.as_str()));
        assert_eq!(
            config.mcp_source("old"),
            Some(global.as_str()),
            "a 0.5.x entry"
        );
        assert_eq!(config.mcp_source("none"), None);
        // The project's redefinition of `shared` waits for approval: the user's stays as it was.
        assert_eq!(config.mcp_source("shared"), Some(global.as_str()));
        let shared = |config: &Config| {
            config
                .mcp_servers()
                .into_iter()
                .find(|(n, _)| n == "shared")
                .map(|(_, s)| s["command"].clone())
        };
        assert_eq!(shared(&config), Some(json!(["a"])));
        let section = config.trust[0].clone();
        assert_eq!(
            (section.path.as_str(), section.kind, section.decision),
            (local.as_str(), crate::trust::TrustKind::Mcp, None)
        );
        let mut trust = crate::trust::Decisions::default();
        trust.allow(&section.path, section.kind, &section.digest);
        let config = load(
            &paths,
            &project,
            &LoadOptions {
                trust,
                ..LoadOptions::default()
            },
        );
        assert_eq!(config.mcp_source("shared"), Some(local.as_str()));
        assert_eq!(shared(&config), Some(json!(["c"])));
        assert_eq!(config.trust[0].decision, Some(true));
    }

    #[test]
    fn project_agent_files_apply_the_rules_that_were_judged() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        std::fs::create_dir_all(project.join(".anysola/agent")).unwrap();
        let file = project.join(".anysola/agent/mine.md");
        std::fs::write(
            &file,
            "---\npermission:\n  bash: ask\n  edit: allow\n---\nMine.\n",
        )
        .unwrap();
        let paths = Paths::isolated(home.clone());
        let config = load(&paths, &project, &LoadOptions::default());
        // Rewritten after the loader judged it: what applies is what was judged.
        std::fs::write(&file, "---\nmode: primary\n---\nMine.\n").unwrap();
        let agents = crate::agent::Agents::load(&config, &paths);
        let mine = agents.get("mine").unwrap();
        let effect = |action: &str| crate::permission::evaluate(action, "x", &mine.permissions);
        assert_eq!(effect("shell"), crate::permission::Effect::Ask);
        assert_eq!(config.trust.len(), 1);
        assert_eq!(config.trust[0].entries[0].command, "allow edit: *");
    }

    #[test]
    fn unapproved_project_sections_are_not_applied() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        std::fs::create_dir_all(home.join(".anysola")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        let paths = Paths::isolated(home.clone());
        std::fs::write(
            paths.config_file(),
            r#"{"formatter": {"mine": {"command": ["user-fmt"]}},
                "mcp": {"servers": {"kept": {"type": "local", "command": ["k"]}}}}"#,
        )
        .unwrap();
        std::fs::write(
            project.join("anysola.json"),
            r#"{"formatter": true, "experimental": {"mcp_timeout": 5},
                "mcp": {"servers": {"kept": {"type": "local", "command": ["k"], "disabled": true},
                                    "new": {"type": "remote", "url": "https://r/mcp"}}}}"#,
        )
        .unwrap();
        let config = load(&paths, &project, &LoadOptions::default());
        let servers = config.mcp_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].1,
            json!({"type": "local", "command": ["k"], "timeout": {}})
        );
        assert_eq!(
            config.get("formatter"),
            Some(&json!({"mine": {"command": ["user-fmt"]}})),
            "the user's formatter only"
        );
        let kinds: Vec<_> = config.trust.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            [
                crate::trust::TrustKind::Mcp,
                crate::trust::TrustKind::Formatter
            ]
        );
    }

    #[test]
    fn hooks_accumulate_with_their_scope_and_stay_out_of_values() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        std::fs::create_dir_all(project.join(".anysola")).unwrap();
        std::fs::create_dir_all(home.join(".anysola")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let hooks = |command: &str| {
            format!(
                r#"{{"hooks": {{"Stop": [{{"hooks": [{{"type": "command", "command": "{command}"}}]}}]}}}}"#
            )
        };
        let paths = Paths::isolated(home.clone());
        std::fs::write(paths.config_file(), hooks("user")).unwrap();
        std::fs::write(
            project.join("anysola.json"),
            hooks("project").replacen('{', r#"{"shell": "./evil.sh", "#, 1),
        )
        .unwrap();
        std::fs::write(
            project.join(".anysola").join("anysola.json"),
            r#"{"disable_hooks": true}"#,
        )
        .unwrap();
        std::fs::write(home.join(".claude").join("settings.json"), hooks("claude")).unwrap();
        let options = LoadOptions {
            remote: Some(serde_json::from_str(&hooks("remote")).unwrap()),
            api_url: "http://x".into(),
            ..LoadOptions::default()
        };
        let config = load(&paths, &project, &options);
        let summary: Vec<(String, crate::hooks::HookScope)> = config
            .hooks
            .documents
            .iter()
            .map(|d| (d.handlers[0].command.clone(), d.scope))
            .collect();
        assert_eq!(
            summary,
            [
                ("user".to_string(), crate::hooks::HookScope::User),
                ("project".to_string(), crate::hooks::HookScope::Project),
            ]
        );
        assert!(!config.hooks.disabled_by_project && !config.hooks.disabled_by_user);
        assert!(
            config
                .warnings
                .iter()
                .any(|w| w.contains("\"disable_hooks\"") && w.contains("your own config")),
            "{:?}",
            config.warnings
        );
        assert_eq!(
            config.hooks.shell, None,
            "a project's shell does not run hooks"
        );
        assert!(config.get("hooks").is_none() && config.get("disable_hooks").is_none());
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
        std::fs::write(real.join("opencode.json"), r#"{"theme":"zsh"}"#).unwrap();
        std::fs::create_dir_all(real.join(".opencode")).unwrap();
        std::fs::write(
            real.join(".opencode").join("opencode.json"),
            r#"{"theme":"fish"}"#,
        )
        .unwrap();
        let paths = Paths::isolated(home.clone());
        let config = load(&paths, &real, &LoadOptions::default());
        assert_eq!(config.get("theme").and_then(Value::as_str), Some("zsh"));
        let project = home.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("opencode.json"), r#"{"theme":"bash"}"#).unwrap();
        assert_eq!(
            load(&paths, &project, &LoadOptions::default())
                .get("theme")
                .and_then(Value::as_str),
            Some("bash")
        );
    }
}

#[cfg(test)]
mod protected_tests {
    use super::*;

    #[test]
    fn only_files_that_change_what_runs_are_protected() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::isolated(root.path().join("home"));
        for path in [
            paths.config_file(),
            paths.database(),
            PathBuf::from(format!("{}-wal", paths.database().display())),
            paths.desktop_prefs(),
            paths.legacy_config_dir().join("opencode.jsonc"),
            system_dir().join("opencode.json"),
            paths.data.join("agent/x.md"),
            paths.data.join("AGENTS.md"),
            paths.legacy_config_dir().join("AGENTS.md"),
            paths.data.join("skills/s/SKILL.md"),
            paths.legacy_config_dir().join("command/c.md"),
        ] {
            assert!(protected(&paths, &path), "{}", path.display());
        }
        for path in [
            paths.plan_dir().join("plan.md"),
            paths.tool_output_dir().join("x"),
            paths.state_dir().join("x"),
            paths.data.join("other.json"),
            paths.data.join("agentx/a.md"),
        ] {
            assert!(!protected(&paths, &path), "{}", path.display());
        }
    }
}
