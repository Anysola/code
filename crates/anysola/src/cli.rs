//! Command line: global flags and the `run` command, with 0.5.1's help text and parse-error format.

use clap::{Args, Parser, Subcommand, ValueEnum};

pub const RUN_HELP: &str = include_str!("help/run.txt");

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
    /// Accepted for compatibility; every command runs in-process.
    #[arg(long, global = true)]
    pub standalone: bool,
    #[arg(long, global = true)]
    pub server: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Run(RunArgs),
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
    pub auto: bool,
    #[arg(long, hide = true)]
    pub yolo: bool,
    #[arg(long, hide = true)]
    pub dangerously_skip_permissions: bool,
}

fn budget(value: &str) -> Result<u64, String> {
    match value.parse::<i64>() {
        Ok(n) if n >= 1 => Ok(n as u64),
        Ok(_) => {
            Err("Schema validation failed: Expected a value greater than or equal to 1".into())
        }
        Err(_) => Err("Schema validation failed: Expected an integer".into()),
    }
}

/// effect/cli's wording for a parse failure.
pub fn error_message(error: &clap::Error, command: &str) -> String {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let context = |kind| match error.get(kind) {
        Some(ContextValue::String(s)) => s.clone(),
        Some(ContextValue::Strings(s)) => s.join(" | "),
        _ => String::new(),
    };
    match error.kind() {
        ErrorKind::UnknownArgument => format!(
            "Unrecognized flag: {} in command {command}",
            context(ContextKind::InvalidArg)
        ),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => {
            let flag = context(ContextKind::InvalidArg);
            let flag = flag
                .split_whitespace()
                .find(|p| p.starts_with("--"))
                .unwrap_or(&flag)
                .to_string();
            let value = context(ContextKind::InvalidValue);
            let expected = match std::error::Error::source(error) {
                Some(source) => source.to_string(),
                None => {
                    let valid = match error.get(ContextKind::ValidValue) {
                        Some(ContextValue::Strings(s)) => {
                            s.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>()
                        }
                        _ => Vec::new(),
                    };
                    valid.join(" | ")
                }
            };
            format!("Invalid value for flag {flag}: \"{value}\". Expected: {expected}")
        }
        ErrorKind::InvalidSubcommand => format!(
            "Unknown command: {}",
            context(ContextKind::InvalidSubcommand)
        ),
        _ => error.kind().to_string(),
    }
}
