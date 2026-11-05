//! The `anysola` binary.

mod cli;
mod gate;
mod run;
mod ui;

use clap::Parser;
use serde_json::Value;

use cli::{Cli, Command};

/// `JSON.stringify(value)`.
pub fn json(value: &Value) -> String {
    anysola_core::json::compact(value)
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
    let flags = args.iter().skip(1).take_while(|a| a.as_str() != "--");
    if flags.into_iter().any(|a| a == "--version" || a == "-v") {
        println!("{}", anysola_core::version::version_line());
        return;
    }
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            let command = if args.get(1).is_some_and(|a| a == "run") {
                "anysola run"
            } else {
                "anysola"
            };
            if command == "anysola run" {
                print!("{}", cli::RUN_HELP);
            }
            eprintln!("\nERROR\n  {}", cli::error_message(&error, command));
            std::process::exit(1);
        }
    };
    if cli.help || cli.command.is_none() {
        print!("{}", cli::RUN_HELP);
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
    let imports = match &cli.command {
        Some(Command::Run(_)) => true,
        None => false,
    };
    if imports {
        import(&paths);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = match cli.command {
        Some(Command::Run(args)) => runtime.block_on(run::run(args)),
        None => 0,
    };
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    std::process::exit(code);
}
