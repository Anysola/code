//! Slash commands: the built-ins (`init`, `review`), markdown commands from every config directory
//! (`{command,commands}/**/*.md`) and the config's `commands`. Running one turns its template and
//! the user's arguments into the prompt text.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use futures_util::StreamExt;
use regex::Regex;
use serde_json::Value;

use crate::config::{Config, Entry};

const INITIALIZE: &str = include_str!("prompt/initialize.txt");
const REVIEW: &str = include_str!("prompt/review.txt");
const SCHEDULE: &str = include_str!("prompt/schedule.txt");

#[derive(Debug, Clone, PartialEq)]
pub enum Template {
    /// `init` and `review`: `${path}` is the project directory; `$ARGUMENTS` or appended input.
    Builtin(&'static str),
    /// A user template: `$ARGUMENTS`, `$1..$N`, ``!`cmd` ``.
    User(String),
    /// `/goal`: the arguments are the objective or a control word, handled by the goal loop.
    Goal,
    /// An MCP prompt: the arguments fill its arguments positionally.
    Mcp(crate::mcp::PromptCommand),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub name: String,
    pub description: Option<String>,
    pub agent: Option<String>,
    /// `provider/model[#variant]`.
    pub model: Option<String>,
    /// Runs in a child session.
    pub subagent: bool,
    pub template: Template,
}

#[derive(Debug, Clone, Default)]
pub struct Commands {
    list: Vec<Command>,
}

fn from_value(name: &str, value: &Value) -> Option<Command> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let flag = |key: &str| value.get(key).and_then(Value::as_bool);
    Some(Command {
        name: name.to_string(),
        description: text("description"),
        agent: text("agent"),
        model: value
            .get("model")
            .and_then(|m| crate::config::model_ref(m, value.get("variant"))),
        subagent: flag("subagent")
            .or_else(|| flag("subtask"))
            .unwrap_or(false),
        template: Template::User(text("template")?),
    })
}

/// `{command,commands}/**/*.md` of a config directory, sorted by path; the name is the path below
/// that folder without `.md`.
fn markdown_commands(dir: &Path) -> Vec<Command> {
    let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
    for folder in ["command", "commands"] {
        let root = dir.join(folder);
        if !root.is_dir() {
            continue;
        }
        files.extend(
            ignore::WalkBuilder::new(&root)
                .standard_filters(false)
                .follow_links(true)
                .build()
                .filter_map(Result::ok)
                .map(|e| e.into_path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md"))
                .map(|p| (p, root.clone())),
        );
    }
    files.sort();
    files
        .into_iter()
        .filter_map(|(file, root)| {
            let text = std::fs::read_to_string(&file).ok()?;
            let (mut meta, body) = crate::markdown::parse(&text)?;
            meta.insert("template".into(), Value::String(body.trim().to_string()));
            let name = file.strip_prefix(&root).ok()?.with_extension("");
            from_value(
                &name.to_string_lossy().replace('\\', "/"),
                &Value::Object(meta),
            )
        })
        .collect()
}

impl Commands {
    pub fn load(config: &Config) -> Commands {
        let mut list = vec![
            Command {
                name: "init".into(),
                description: Some("guided AGENTS.md setup".into()),
                agent: None,
                model: None,
                subagent: false,
                template: Template::Builtin(INITIALIZE),
            },
            Command {
                name: "review".into(),
                description: Some(
                    "review changes [commit|branch|pr], defaults to uncommitted".into(),
                ),
                agent: None,
                model: None,
                subagent: false,
                template: Template::Builtin(REVIEW),
            },
            Command {
                name: "goal".into(),
                description: Some(
                    "keep working until an objective is met [pause|resume|clear]".into(),
                ),
                agent: None,
                model: None,
                subagent: false,
                template: Template::Goal,
            },
        ];
        if crate::schedule::enabled(config) {
            list.push(Command {
                name: "schedule".into(),
                description: Some(
                    "schedule a prompt for later, or list, pause and delete this session's schedules"
                        .into(),
                ),
                agent: None,
                model: None,
                subagent: false,
                template: Template::Builtin(SCHEDULE),
            });
        }
        let defined = config.entries.iter().flat_map(|entry| match entry {
            Entry::Directory(dir) => markdown_commands(dir),
            Entry::Document { commands, .. } => commands
                .iter()
                .filter_map(|(name, value)| from_value(name, value))
                .collect(),
        });
        for command in defined {
            list.retain(|c| c.name != command.name);
            list.push(command);
        }
        Commands { list }
    }

    /// The same commands plus MCP prompts `<server>:<prompt>` (names already taken stay).
    pub fn with_prompts(mut self, prompts: Vec<crate::mcp::PromptCommand>) -> Commands {
        for prompt in prompts {
            if self.get(&prompt.name).is_none() {
                self.list.push(Command {
                    name: prompt.name.clone(),
                    description: prompt.description.clone(),
                    agent: None,
                    model: None,
                    subagent: false,
                    template: Template::Mcp(prompt),
                });
            }
        }
        self
    }

    pub fn list(&self) -> &[Command] {
        &self.list
    }

    pub fn get(&self, name: &str) -> Option<&Command> {
        self.list.iter().find(|c| c.name == name)
    }
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid pattern"))
}

/// Arguments split on whitespace; quotes and `[Image N]` stay together, outer quotes trimmed.
pub fn parse_arguments(input: &str) -> Vec<String> {
    static ARGS: OnceLock<Regex> = OnceLock::new();
    static QUOTES: OnceLock<Regex> = OnceLock::new();
    let args = regex(&ARGS, r#"(?i)\[Image\s+\d+\]|"[^"]*"|'[^']*'|[^\s"']+"#);
    let quotes = regex(&QUOTES, r#"^["']|["']$"#);
    args.find_iter(input)
        .map(|m| quotes.replace_all(m.as_str(), "").into_owned())
        .collect()
}

fn append(template: &str, input: &str) -> String {
    let value = input.trim();
    if template.contains("$ARGUMENTS") {
        return template.replace("$ARGUMENTS", value);
    }
    [template, value]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `$ARGUMENTS` and `$1..$N` substituted (the highest placeholder takes the rest); with neither,
/// the input is appended after a blank line.
pub fn substitute(template: &str, input: &str) -> String {
    static PLACEHOLDER: OnceLock<Regex> = OnceLock::new();
    static ARGUMENT: OnceLock<Regex> = OnceLock::new();
    let placeholder = regex(&PLACEHOLDER, r"\$([1-9]\d*)");
    let argument = regex(&ARGUMENT, r"\$(?:ARGUMENTS|([1-9]\d*))");
    let args = parse_arguments(input);
    let positions: Vec<usize> = placeholder
        .captures_iter(template)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    let last = positions.iter().copied().max().unwrap_or(0);
    let replaced = argument.replace_all(template, |c: &regex::Captures| {
        let Some(index) = c.get(1) else {
            return input.to_string();
        };
        let position: usize = index.as_str().parse().unwrap_or(usize::MAX);
        let at = position - 1;
        match args.get(at) {
            None => String::new(),
            Some(_) if position == last => args[at..].join(" "),
            Some(arg) => arg.clone(),
        }
    });
    if positions.is_empty() && !template.contains("$ARGUMENTS") && !input.trim().is_empty() {
        format!("{replaced}\n\n{input}").trim().to_string()
    } else {
        replaced.trim().to_string()
    }
}

/// Replaces every ``!`cmd` `` with the command's combined output (two at a time, the configured
/// shell, run in `cwd`).
pub async fn interpolate(
    text: &str,
    shell: Option<&str>,
    cwd: &Path,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<String, String> {
    static SHELL: OnceLock<Regex> = OnceLock::new();
    let pattern = regex(&SHELL, r"!`([^`]+)`");
    let sources: Vec<String> = pattern
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect();
    if sources.is_empty() {
        return Ok(text.to_string());
    }
    let outputs: Vec<Result<String, String>> = futures_util::stream::iter(sources)
        .map(|source| async move {
            let (program, args) = crate::tool::shell::invocation(shell, &source);
            let output = tokio::process::Command::new(&program)
                .args(&args)
                .current_dir(cwd)
                .env_clear()
                .envs(env.iter().cloned())
                .stdin(std::process::Stdio::null())
                .output()
                .await
                .map_err(|e| {
                    format!(
                        "Shell interpolation failed for {}: {e}",
                        Value::String(source.clone())
                    )
                })?;
            let mut combined = output.stdout;
            combined.extend(output.stderr);
            Ok(String::from_utf8_lossy(&combined).into_owned())
        })
        .buffered(2)
        .collect()
        .await;
    let mut outputs = outputs
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter();
    Ok(pattern
        .replace_all(text, |_: &regex::Captures| {
            outputs.next().unwrap_or_default()
        })
        .into_owned())
}

/// Where a command renders: the project, the shell for ``!`cmd` ``, and the MCP runtime.
pub struct RenderEnv<'a> {
    pub project: &'a Path,
    pub shell: Option<&'a str>,
    pub cwd: &'a Path,
    pub env: &'a [(std::ffi::OsString, std::ffi::OsString)],
    pub mcp: Option<&'a dyn crate::mcp::Mcp>,
}

/// The prompt text a command produces from the user's arguments.
pub async fn render(command: &Command, input: &str, at: &RenderEnv<'_>) -> Result<String, String> {
    let RenderEnv {
        project,
        shell,
        cwd,
        env,
        mcp,
    } = *at;
    match &command.template {
        Template::Builtin(template) => Ok(append(
            &template.replacen("${path}", &project.display().to_string(), 1),
            input,
        )),
        Template::User(template) => {
            interpolate(&substitute(template, input), shell, cwd, env).await
        }
        Template::Goal => Ok(input.to_string()),
        Template::Mcp(prompt) => match mcp {
            Some(mcp) => mcp.expand_prompt(prompt, parse_arguments(input)).await,
            None => Err(format!(
                "MCP prompt not found: {}:{}",
                prompt.server, prompt.prompt
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        assert_eq!(
            parse_arguments(r#"a "b c" 'd' [Image 1] e"#),
            vec!["a", "b c", "d", "[Image 1]", "e"]
        );
        assert_eq!(
            substitute("x $1 y $2", "one two three"),
            "x one y two three"
        );
        assert_eq!(substitute("x $2", "one"), "x");
        assert_eq!(substitute("all: $ARGUMENTS", " a b "), "all:  a b");
        assert_eq!(substitute("plain", "extra"), "plain\n\nextra");
        assert_eq!(substitute("plain", "  "), "plain");
        assert_eq!(substitute("$1 and $1", "a b"), "a b and a b");
    }

    #[test]
    fn builtins_append_or_replace() {
        assert_eq!(append("T", " focus "), "T\n\nfocus");
        assert_eq!(append("T", ""), "T");
        assert_eq!(append("x $ARGUMENTS y", " a "), "x a y");
    }

    // `pwd` through a POSIX shell; Windows runs commands in PowerShell, which prints a table.
    #[cfg(unix)]
    #[tokio::test]
    async fn shell_interpolation() {
        let dir = tempfile::tempdir().unwrap();
        let env: Vec<_> = std::env::vars_os().collect();
        let text = interpolate("a !`echo hi` b !`pwd`", Some("/bin/sh"), dir.path(), &env)
            .await
            .unwrap();
        let cwd = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(text, format!("a hi\n b {}\n", cwd.display()));
    }

    #[test]
    fn markdown_and_config_commands() {
        let dir = tempfile::tempdir().unwrap();
        let command = dir.path().join(".opencode/command/git/commit.md");
        std::fs::create_dir_all(command.parent().unwrap()).unwrap();
        std::fs::write(
            &command,
            "---\ndescription: Commit\nagent: build\nmodel: anysola/claude-haiku-5-5\nvariant: high\nsubtask: true\n---\nCommit $ARGUMENTS\n",
        )
        .unwrap();
        let mut config = Config::default();
        config.entries = vec![
            Entry::Directory(dir.path().join(".opencode")),
            Entry::Document {
                agents: Default::default(),
                commands: serde_json::json!({"init": {"template": "custom init"}})
                    .as_object()
                    .unwrap()
                    .clone(),
            },
        ];
        let commands = Commands::load(&config);
        let commit = commands.get("git/commit").unwrap();
        assert_eq!(commit.description.as_deref(), Some("Commit"));
        assert_eq!(
            commit.model.as_deref(),
            Some("anysola/claude-haiku-5-5#high")
        );
        assert!(commit.subagent);
        assert_eq!(commit.template, Template::User("Commit $ARGUMENTS".into()));
        assert_eq!(
            commands.get("init").unwrap().template,
            Template::User("custom init".into())
        );
        assert!(matches!(
            commands.get("review").unwrap().template,
            Template::Builtin(_)
        ));
    }
}
