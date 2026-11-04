//! Agents: the built-ins (build, plan, general, explore, compaction, title) and those defined in
//! config, with their prompts, modes and permission rules.

use serde_json::Value;

use crate::config::Config;
use crate::paths::Paths;
use crate::permission::{Effect, Rule};

pub const PROMPT_EXPLORE: &str = include_str!("prompt/explore.txt");
pub const PROMPT_TITLE: &str = include_str!("prompt/title.txt");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Primary,
    Subagent,
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub mode: Mode,
    pub hidden: bool,
    /// A custom system prompt replaces the default one (and the appended Anthropic guidance).
    pub system: Option<String>,
    /// `provider/model[#variant]` overriding the session model.
    pub model: Option<String>,
    pub steps: Option<u32>,
    pub permissions: Vec<Rule>,
}

impl Agent {
    fn new(id: &str) -> Agent {
        Agent {
            id: id.into(),
            name: id.into(),
            description: None,
            mode: Mode::Primary,
            hidden: false,
            system: None,
            model: None,
            steps: None,
            permissions: vec![
                Rule::new("*", "*", Effect::Allow),
                Rule::new("read", "*.env", Effect::Ask),
                Rule::new("read", "*.env.*", Effect::Ask),
                Rule::new("read", "*.env.example", Effect::Allow),
            ],
        }
    }

    /// Selectable as a session's primary agent (not a subagent, not hidden).
    pub fn selectable(&self) -> bool {
        self.mode != Mode::Subagent && !self.hidden
    }
}

fn builtins(paths: &Paths) -> Vec<Agent> {
    let rule = Rule::new;
    let mut build = Agent::new("build");
    build.name = "Build".into();
    build.description =
        Some("The default agent. Executes tools based on configured permissions.".into());
    build.permissions.push(rule("question", "*", Effect::Allow));

    let mut plan = Agent::new("plan");
    plan.name = "Plan".into();
    plan.description = Some(
        "Read-only agent for exploring the codebase and planning work before implementation."
            .into(),
    );
    let plan_dir = format!("{}/*", paths.plan_dir().display()).replace('\\', "/");
    plan.permissions.extend([
        rule("question", "*", Effect::Allow),
        rule("edit", "*", Effect::Deny),
        rule("edit", &plan_dir, Effect::Allow),
    ]);

    let mut general = Agent::new("general");
    general.name = "General".into();
    general.description = Some("General-purpose agent for researching complex questions and executing multi-step tasks. Use this agent to execute multiple units of work in parallel.".into());
    general.mode = Mode::Subagent;
    general.permissions.extend([
        rule("question", "*", Effect::Deny),
        rule("subagent", "*", Effect::Deny),
    ]);

    let mut explore = Agent::new("explore");
    explore.name = "Explore".into();
    explore.description = Some("Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. \"src/components/**/*.tsx\"), search code for keywords (eg. \"API endpoints\"), or answer questions about the codebase (eg. \"how do API endpoints work?\"). When calling this agent, specify the desired thoroughness level: \"quick\" for basic searches, \"medium\" for moderate exploration, or \"very thorough\" for comprehensive analysis across multiple locations and naming conventions.".into());
    explore.system = Some(PROMPT_EXPLORE.into());
    explore.mode = Mode::Subagent;
    explore.permissions.extend([
        rule("*", "*", Effect::Deny),
        rule("shell", "*", Effect::Allow),
        rule("grep", "*", Effect::Allow),
        rule("glob", "*", Effect::Allow),
        rule("webfetch", "*", Effect::Allow),
        rule("websearch", "*", Effect::Allow),
        rule("read", "*", Effect::Allow),
        rule("read", "*.env", Effect::Ask),
        rule("read", "*.env.*", Effect::Ask),
        rule("read", "*.env.example", Effect::Allow),
        rule("subagent", "*", Effect::Deny),
        rule("external_directory", "*", Effect::Allow),
    ]);

    let mut compaction = Agent::new("compaction");
    compaction.name = "Compaction".into();
    compaction.hidden = true;

    let mut title = Agent::new("title");
    title.name = "Title".into();
    title.hidden = true;
    title.system = Some(PROMPT_TITLE.into());
    title.permissions.push(rule("*", "*", Effect::Deny));

    vec![build, plan, general, explore, compaction, title]
}

/// `~`, `~/x`, `$HOME`, `$HOME/x` in path-action resources become absolute.
fn expand(rule: Rule, home: &str) -> Rule {
    if !matches!(rule.action.as_str(), "read" | "edit" | "external_directory") {
        return rule;
    }
    let resource = &rule.resource;
    let expanded = if resource == "~" || resource == "$HOME" {
        home.to_string()
    } else if let Some(rest) = resource
        .strip_prefix("~/")
        .or_else(|| resource.strip_prefix("$HOME/"))
        .or_else(|| resource.strip_prefix("$HOME\\"))
    {
        crate::paths::node_join(home, rest)
    } else {
        return rule;
    };
    Rule {
        resource: expanded,
        ..rule
    }
}

fn rules(value: Option<&Value>, home: &str) -> Vec<Rule> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|r| serde_json::from_value::<Rule>(r.clone()).ok())
                .map(|r| expand(r, home))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Agents {
    pub list: Vec<Agent>,
    default_agent: Option<String>,
}

impl Agents {
    pub fn load(config: &Config, paths: &Paths) -> Agents {
        let home = paths.home.display().to_string();
        let global: Vec<Rule> = config
            .permissions
            .iter()
            .cloned()
            .map(|r| expand(r, &home))
            .collect();
        let mut list = builtins(paths);
        for agent in &mut list {
            agent.permissions.extend(global.iter().cloned());
        }
        let markdown = config
            .directories
            .iter()
            .flat_map(|dir| markdown_agents(dir));
        let configured: Vec<(String, Value)> = markdown
            .chain(
                config
                    .agents
                    .iter()
                    .map(|(id, item)| (id.clone(), item.clone())),
            )
            .collect();
        for (id, item) in &configured {
            if item.get("disabled").and_then(Value::as_bool) == Some(true) {
                list.retain(|a| &a.id != id);
                continue;
            }
            let index = match list.iter().position(|a| &a.id == id) {
                Some(index) => index,
                None => {
                    let mut agent = Agent::new(id);
                    agent.permissions.extend(global.iter().cloned());
                    list.push(agent);
                    list.len() - 1
                }
            };
            apply(&mut list[index], item, &home);
        }
        Agents {
            list,
            default_agent: config.default_agent().map(str::to_string),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Agent> {
        self.list.iter().find(|a| a.id == id)
    }

    /// `default_agent` when selectable, else `build`, else the first selectable agent.
    pub fn default_agent(&self) -> Option<&Agent> {
        self.default_agent
            .as_deref()
            .and_then(|id| self.get(id))
            .filter(|a| a.selectable())
            .or_else(|| self.get("build").filter(|a| a.selectable()))
            .or_else(|| self.list.iter().find(|a| a.selectable()))
    }

    /// Subagents a caller with these rules may spawn, sorted by id.
    pub fn subagents_for(&self, caller: &[Rule]) -> Vec<(String, Option<String>)> {
        let mut out: Vec<_> = self
            .list
            .iter()
            .filter(|a| {
                a.mode != Mode::Primary
                    && !a.hidden
                    && crate::permission::evaluate("subagent", &a.id, caller) != Effect::Deny
            })
            .map(|a| (a.id.clone(), a.description.clone()))
            .collect();
        out.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        out
    }
}

/// `{agent,agents}/**/*.md` (mode from frontmatter) and `{mode,modes}/*.md` (always primary) of a
/// config directory; the id is the path below that folder without `.md`.
fn markdown_agents(dir: &std::path::Path) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for (folder, primary) in [
        ("agent", false),
        ("agents", false),
        ("mode", true),
        ("modes", true),
    ] {
        let root = dir.join(folder);
        if !root.is_dir() {
            continue;
        }
        let depth = if primary { 1 } else { usize::MAX };
        let mut files: Vec<std::path::PathBuf> = ignore::WalkBuilder::new(&root)
            .standard_filters(false)
            .max_depth(Some(depth))
            .build()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
            .collect();
        files.sort();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let (meta, body) = crate::markdown::parse(&text);
            let mut agent = crate::config::markdown_agent(&meta);
            if !body.is_empty() {
                agent["system"] = Value::String(body);
            }
            if primary {
                agent["mode"] = Value::String("primary".into());
            }
            let id = file.strip_prefix(&root).unwrap_or(&file).with_extension("");
            out.push((id.to_string_lossy().replace('\\', "/"), agent));
        }
    }
    out
}

fn apply(agent: &mut Agent, item: &Value, home: &str) {
    let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
    if let Some(model) = text("model") {
        agent.model = Some(model);
    }
    if let Some(system) = text("system") {
        agent.system = Some(system);
    }
    if let Some(description) = text("description") {
        agent.description = Some(description);
    }
    match item.get("mode").and_then(Value::as_str) {
        Some("primary") => agent.mode = Mode::Primary,
        Some("subagent") => agent.mode = Mode::Subagent,
        Some("all") => agent.mode = Mode::All,
        _ => {}
    }
    if let Some(hidden) = item.get("hidden").and_then(Value::as_bool) {
        agent.hidden = hidden;
    }
    if let Some(steps) = item.get("steps").and_then(Value::as_u64).filter(|s| *s > 0) {
        agent.steps = Some(steps as u32);
    }
    agent
        .permissions
        .extend(rules(item.get("permissions"), home));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agents(config: Value) -> Agents {
        let paths = Paths::isolated("/home/u".into());
        let mut cfg = Config::default();
        cfg.agents = config.as_object().unwrap().clone();
        Agents::load(&cfg, &paths)
    }

    #[test]
    fn defaults_and_config() {
        let all = agents(json!({
            "anysola-review": {"mode": "primary", "description": "Review", "permissions": [{"action": "*", "resource": "*", "effect": "deny"}]},
            "general": {"disabled": true},
        }));
        assert_eq!(all.default_agent().unwrap().id, "build");
        assert!(all.get("general").is_none());
        let review = all.get("anysola-review").unwrap();
        assert_eq!(review.permissions.last().unwrap().effect, Effect::Deny);
        assert_eq!(
            all.subagents_for(&all.get("build").unwrap().permissions)
                .iter()
                .map(|s| s.0.as_str())
                .collect::<Vec<_>>(),
            vec!["explore"]
        );
        let plan = all.get("plan").unwrap();
        assert_eq!(
            plan.permissions.last().unwrap().resource,
            "/home/u/.opencode/plan/*"
        );
    }
}
