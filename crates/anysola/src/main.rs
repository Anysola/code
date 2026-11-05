//! The `anysola` binary.

mod account;
mod clack;
mod cli;
mod completions;
mod debug;
mod gate;
mod install;
mod mcp;
mod models;
mod prompt;
mod root;
mod run;
mod sessions;
mod ui;
mod uninstall;
mod update;

use clap::Parser;
use serde_json::Value;

use cli::{Cli, Command, DebugArgs, McpArgs, SessionArgs};

/// `JSON.stringify(value)`.
pub fn json(value: &Value) -> String {
    anysola_core::json::compact(value)
}

/// The single error line every plain command handler prints on failure.
pub fn fail(message: &str) -> i32 {
    eprintln!("Error: {message}");
    1
}

/// The first start after 0.5.x imports its data. A failure is reported and retried next start;
/// stdout stays untouched.
fn import(paths: &anysola_core::paths::Paths) {
    match anysola_migrate::run(paths) {
        Ok(Some(counts)) if counts.sessions > 0 => ui::println(&format!(
            "Imported {} sessions from Anysola Code 0.5.x.",
            counts.sessions
        )),
        Ok(_) => {}
        Err(error @ anysola_migrate::Error::Outdated) => ui::println(&error.to_string()),
        Err(error) => {
            anysola_core::log::warn("import from 0.5.x failed", &[("error", &error.to_string())]);
            ui::println(&format!(
                "Could not import data from Anysola Code 0.5.x ({error}); it will be tried again next start."
            ));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flags: Vec<&str> = args
        .iter()
        .skip(1)
        .map(String::as_str)
        .take_while(|a| *a != "--")
        .collect();
    if flags.iter().any(|a| *a == "--version" || *a == "-v") {
        println!("{}", anysola_core::version::version_line());
        return;
    }
    let path = cli::path(&args);
    if flags.iter().any(|a| *a == "--help" || *a == "-h") {
        print!("{}", cli::help(&path));
        return;
    }
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            print!("{}", cli::help(&path));
            let message = cli::error_message(&error, &cli::command_name(&path));
            eprintln!("\nERROR\n  {message}");
            std::process::exit(1);
        }
    };
    if let Some(shell) = cli.completions {
        completions::print(shell);
        return;
    }
    if cli.server.is_some()
        || std::env::var_os("OPENCODE_PASSWORD").is_some()
        || std::env::var_os("OPENCODE_SERVER_PASSWORD").is_some()
    {
        ui::println("--server is not supported since Anysola Code 1.0; running locally.");
    }
    let paths = anysola_core::paths::Paths::from_env();
    anysola_core::log::init(&paths, cli.log_level.as_deref(), cli.print_logs);
    let version = anysola_core::version::VERSION;
    anysola_core::log::info(
        "cli starting",
        &[
            ("version", version),
            ("args", &crate::json(&serde_json::json!(args[1..]))),
        ],
    );
    // Commands that read sessions, the token or config import 0.5.x data first.
    let imports = match &cli.command {
        None
        | Some(
            Command::Run(_)
            | Command::Login(_)
            | Command::Logout
            | Command::Status
            | Command::Usage(_)
            | Command::Devices(_)
            | Command::Acp(_)
            | Command::Mcp(_)
            | Command::Models
            | Command::Session(_),
        ) => true,
        Some(
            Command::Update(_)
            | Command::Uninstall(_)
            | Command::Debug(_)
            | Command::Plugin(_)
            | Command::Reload,
        ) => false,
    };
    if imports {
        import(&paths);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let group_help = |name: &str| {
        print!("{}", cli::help(&[name.to_string()]));
        0
    };
    let code = runtime.block_on(async {
        match cli.command {
            None => root::root(cli.root).await,
            Some(Command::Login(args)) => account::login(args.force).await,
            Some(Command::Logout) => account::logout().await,
            Some(Command::Status) => account::status().await,
            Some(Command::Usage(args)) => account::usage(args).await,
            Some(Command::Devices(args)) => account::devices(args.command).await,
            Some(Command::Update(args)) => update::update(args).await,
            Some(Command::Uninstall(args)) => uninstall::uninstall(args).await,
            Some(Command::Acp(args)) if args.login => account::login(false).await,
            // The ACP server arrives with milestone M7.
            Some(Command::Acp(_)) => {
                eprintln!("anysola acp: the ACP server is not available in this build yet");
                1
            }
            Some(Command::Debug(DebugArgs { command: None })) => group_help("debug"),
            Some(Command::Debug(DebugArgs {
                command: Some(command),
            })) => debug::debug(command).await,
            Some(Command::Mcp(McpArgs { command: None })) => group_help("mcp"),
            Some(Command::Mcp(McpArgs {
                command: Some(command),
            })) => mcp::mcp(command).await,
            Some(Command::Plugin(args)) if args.args.is_empty() => group_help("plugin"),
            Some(Command::Plugin(_)) => fail(
                "Plugins are not supported since Anysola Code 1.0; built-in features are native.",
            ),
            Some(Command::Models) => models::models().await,
            Some(Command::Run(args)) => run::run(args).await,
            Some(Command::Session(SessionArgs { command: None })) => group_help("session"),
            Some(Command::Session(SessionArgs {
                command: Some(command),
            })) => sessions::session(command).await,
            Some(Command::Reload) => {
                println!("Configuration reloaded");
                0
            }
        }
    });
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    std::process::exit(code);
}
