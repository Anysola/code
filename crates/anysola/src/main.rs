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
