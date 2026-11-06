//! Command line: the command tree with 0.5.1's flags, help texts and parse-error format.

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "anysola",
    disable_version_flag = true,
    disable_help_flag = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[arg(long, short = 'h', global = true)]
    pub help: bool,
    #[arg(long, global = true)]
    pub print_logs: bool,
    #[arg(long, global = true, value_parser = ["all", "trace", "debug", "info", "warn", "warning", "error", "fatal", "none"])]
    pub log_level: Option<String>,
    #[arg(long, global = true, value_enum)]
    pub completions: Option<Shell>,
    /// Accepted for compatibility; every command runs in-process.
    #[arg(long, global = true)]
    pub standalone: bool,
    #[arg(long, global = true)]
    pub server: Option<String>,
    #[command(flatten)]
    pub root: RootArgs,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The bare `anysola [<directory>]` command (the interactive UI).
#[derive(Debug, Args)]
pub struct RootArgs {
    #[arg(value_name = "directory")]
    pub directory: Option<String>,
    #[arg(long = "continue", short = 'c')]
    pub continue_last: bool,
    #[arg(long, short = 's')]
    pub session: Option<String>,
    #[arg(long)]
    pub prompt: Option<String>,
    #[arg(long)]
    pub auto: bool,
    #[arg(long, hide = true)]
    pub yolo: bool,
    #[arg(long, hide = true)]
    pub dangerously_skip_permissions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Sh,
    Pwsh,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Login(LoginArgs),
    Logout,
    Status,
    Usage(UsageArgs),
    Limits(LimitsArgs),
    Devices(DevicesArgs),
    #[command(alias = "upgrade")]
    Update(UpdateArgs),
    Uninstall(UninstallArgs),
    Acp(AcpArgs),
    Debug(DebugArgs),
    Mcp(McpArgs),
    Plugin(PluginArgs),
    Models,
    Run(RunArgs),
    Session(SessionArgs),
    Reload,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(long, short = 'f')]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum UsageFormat {
    Default,
    Json,
}

#[derive(Debug, Args)]
pub struct UsageArgs {
    #[arg(long, value_parser = ["7", "30"], default_value = "7")]
    pub days: String,
    #[arg(long, value_enum, default_value_t = UsageFormat::Default)]
    pub format: UsageFormat,
}

#[derive(Debug, Args)]
pub struct LimitsArgs {
    #[command(subcommand)]
    pub command: Option<LimitsCommand>,
    #[arg(long, value_enum, default_value_t = UsageFormat::Default)]
    pub format: UsageFormat,
}

#[derive(Debug, Subcommand)]
pub enum LimitsCommand {
    Redeem,
}

#[derive(Debug, Args)]
pub struct DevicesArgs {
    #[command(subcommand)]
    pub command: Option<DevicesCommand>,
}

#[derive(Debug, Subcommand)]
pub enum DevicesCommand {
    Revoke {
        #[arg(value_name = "id")]
        id: String,
    },
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[arg(long)]
    pub print: bool,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, short = 'f')]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct AcpArgs {
    #[arg(long)]
    pub login: bool,
}

#[derive(Debug, Args)]
pub struct DebugArgs {
    #[command(subcommand)]
    pub command: Option<DebugCommand>,
}

#[derive(Debug, Subcommand)]
pub enum DebugCommand {
    Config,
    Paths {
        #[arg(value_name = "name", value_parser = ["db", "home", "data", "config", "cache", "state", "tmp", "bin", "log", "repos"])]
        name: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: Option<McpCommand>,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    List,
    Add(McpAddArgs),
    Auth {
        #[arg(value_name = "name")]
        name: Option<String>,
    },
    Logout {
        #[arg(value_name = "name")]
        name: String,
    },
}

#[derive(Debug, Args)]
pub struct McpAddArgs {
    #[arg(value_name = "name")]
    pub name: String,
    #[arg(value_name = "command", last = true)]
    pub command: Vec<String>,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long, value_parser = key_value)]
    pub header: Vec<(String, String)>,
    #[arg(long, value_parser = key_value)]
    pub env: Vec<(String, String)>,
    #[arg(long)]
    pub global: bool,
}

#[derive(Debug, Args)]
pub struct PluginArgs {
    /// Every plugin subcommand answers with the same error.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: Option<SessionCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListFormat {
    Table,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    List {
        #[arg(long, short = 'n', value_parser = max_count)]
        max_count: Option<u64>,
        #[arg(long, value_enum, default_value_t = ListFormat::Table)]
        format: ListFormat,
    },
    Delete {
        #[arg(value_name = "sessionID")]
        session_id: String,
    },
    Export {
        #[arg(value_name = "session")]
        session: Option<String>,
        #[arg(long)]
        sanitize: bool,
    },
    Import {
        #[arg(value_name = "file")]
        file: String,
        #[arg(long)]
        directory: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Default,
    Json,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    pub message: Vec<String>,
    #[arg(long = "continue", short = 'c')]
    pub continue_last: bool,
    #[arg(long, short = 's')]
    pub session: Option<String>,
    #[arg(long)]
    pub fork: bool,
    #[arg(long, short = 'm')]
    pub model: Option<String>,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long, value_enum, default_value_t = Format::Default)]
    pub format: Format,
    #[arg(long, short = 'f')]
    pub file: Vec<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub thinking: bool,
    #[arg(long, value_parser = budget)]
    pub budget: Option<u64>,
    #[arg(long)]
    pub use_banked_reset: bool,
    #[arg(long)]
    pub auto: bool,
    #[arg(long, hide = true)]
    pub yolo: bool,
    #[arg(long, hide = true)]
    pub dangerously_skip_permissions: bool,
}

/// Percent of the 5-hour window, 1 to 100.
fn budget(value: &str) -> Result<u64, String> {
    match value.trim_end_matches('%').parse::<i64>() {
        Ok(n) if (1..=100).contains(&n) => Ok(n as u64),
        Ok(n) if n < 1 => {
            Err("Schema validation failed: Expected a value greater than or equal to 1".into())
        }
        Ok(_) => Err("Schema validation failed: Expected a value less than or equal to 100".into()),
        Err(_) => Err("Schema validation failed: Expected an integer".into()),
    }
}

fn key_value(value: &str) -> Result<(String, String), String> {
    value
        .split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| {
            format!("Invalid key=value format. Expected format: key=value, got: {value}")
        })
}

fn max_count(value: &str) -> Result<u64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| n.max(0.0) as u64)
        .ok_or_else(|| "Expected a string representing a finite number".into())
}

/// The help text of each command, by its path (`session_list`, `devices_revoke`, …).
pub fn help(path: &[String]) -> &'static str {
    match path.join("_").as_str() {
        "login" => include_str!("help/login.txt"),
        "logout" => include_str!("help/logout.txt"),
        "status" => include_str!("help/status.txt"),
        "usage" => include_str!("help/usage.txt"),
        "limits" => include_str!("help/limits.txt"),
        "limits_redeem" => include_str!("help/limits_redeem.txt"),
        "devices" => include_str!("help/devices.txt"),
        "devices_revoke" => include_str!("help/devices_revoke.txt"),
        "update" => include_str!("help/update.txt"),
        "uninstall" => include_str!("help/uninstall.txt"),
        "acp" => include_str!("help/acp.txt"),
        "debug" => include_str!("help/debug.txt"),
        "debug_config" => include_str!("help/debug_config.txt"),
        "debug_paths" => include_str!("help/debug_paths.txt"),
        "mcp" => include_str!("help/mcp.txt"),
        "mcp_list" => include_str!("help/mcp_list.txt"),
        "mcp_add" => include_str!("help/mcp_add.txt"),
        "mcp_auth" => include_str!("help/mcp_auth.txt"),
        "mcp_logout" => include_str!("help/mcp_logout.txt"),
        "plugin" => include_str!("help/plugin.txt"),
        "models" => include_str!("help/models.txt"),
        "run" => include_str!("help/run.txt"),
        "session" => include_str!("help/session.txt"),
        "session_list" => include_str!("help/session_list.txt"),
        "session_delete" => include_str!("help/session_delete.txt"),
        "session_export" => include_str!("help/session_export.txt"),
        "session_import" => include_str!("help/session_import.txt"),
        "reload" => include_str!("help/reload.txt"),
        _ => include_str!("help/root.txt"),
    }
}

/// The subcommand path `args` invoke, as far as it can be read without a full parse.
pub fn path(args: &[String]) -> Vec<String> {
    let mut command = Cli::command();
    command.build();
    let mut path = Vec::new();
    let mut tokens = args.iter().skip(1).take_while(|a| a.as_str() != "--");
    while let Some(token) = tokens.next() {
        if let Some(flag) = token.strip_prefix('-') {
            let flag = flag.trim_start_matches('-');
            if token.contains('=') {
                continue;
            }
            let takes_value = command.get_arguments().any(|arg| {
                let named = if token.starts_with("--") {
                    arg.get_long() == Some(flag)
                } else {
                    flag.len() == 1 && arg.get_short() == flag.chars().next()
                };
                named && arg.get_action().takes_values()
            });
            if takes_value {
                tokens.next();
            }
            continue;
        }
        let Some(sub) = command.find_subcommand(token).cloned() else {
            break;
        };
        path.push(sub.get_name().to_string());
        if sub.get_name() == "plugin" {
            break;
        }
        command = sub;
    }
    path
}

/// `anysola session list` style name of a command path.
pub fn command_name(path: &[String]) -> String {
    std::iter::once("anysola")
        .chain(path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// effect/cli's wording for a parse failure.
pub fn error_message(error: &clap::Error, command: &str) -> String {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let context = |kind| match error.get(kind) {
        Some(ContextValue::String(s)) => s.clone(),
        Some(ContextValue::Strings(s)) => s.join(" | "),
        _ => String::new(),
    };
    let valid = |quote: bool| match error.get(ContextKind::ValidValue) {
        Some(ContextValue::Strings(s)) => s
            .iter()
            .map(|v| if quote { format!("\"{v}\"") } else { v.clone() })
            .collect::<Vec<_>>()
            .join(" | "),
        _ => String::new(),
    };
    // clap names a flag `--days <DAYS>` and a positional `<name>` (or `[name]`).
    let argument = context(ContextKind::InvalidArg);
    let flag = argument
        .split_whitespace()
        .find(|p| p.starts_with('-'))
        .map(|f| match f.split_once(',') {
            Some((_, long)) => long.trim().to_string(),
            None => f.to_string(),
        });
    let positional = argument
        .trim_matches(|c| matches!(c, '<' | '>' | '[' | ']' | '.'))
        .to_string();
    match error.kind() {
        ErrorKind::UnknownArgument if argument.starts_with('-') => {
            format!("Unrecognized flag: {argument} in command {command}")
        }
        ErrorKind::UnknownArgument => format!("Unexpected positional argument: \"{argument}\""),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => {
            let value = context(ContextKind::InvalidValue);
            if value.is_empty() && error.kind() == ErrorKind::InvalidValue {
                let flag = flag.unwrap_or(argument);
                return match valid(false) {
                    choices if choices.is_empty() => format!("Missing value for flag {flag}"),
                    choices => format!("Missing value for flag {flag}. Expected: {choices}"),
                };
            }
            let expected = match std::error::Error::source(error) {
                Some(source) => source.to_string(),
                None => valid(true),
            };
            // A number decoder states its own expectation.
            let expected = match expected.strip_prefix("Expected ") {
                Some(rest) => format!("Expected {rest}"),
                None => format!("Expected: {expected}"),
            };
            match flag {
                Some(flag) => {
                    format!("Invalid value for flag {flag}: \"{value}\". {expected}")
                }
                None => {
                    format!("Invalid value for argument <{positional}>: \"{value}\". {expected}")
                }
            }
        }
        ErrorKind::MissingRequiredArgument => format!("Missing required argument: {positional}"),
        ErrorKind::InvalidSubcommand => format!(
            "Unknown subcommand \"{}\" for \"{command}\"",
            context(ContextKind::InvalidSubcommand)
        ),
        _ => error.kind().to_string(),
    }
}
