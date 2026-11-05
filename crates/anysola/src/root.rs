//! The bare `anysola [<directory>]` command: the checks that precede the interactive UI, then
//! the UI itself.

use std::io::IsTerminal;

use anysola_core::gateway::Token;
use anysola_core::paths::Paths;
use anysola_core::storage::Db;

use crate::clack;
use crate::cli::RootArgs;

pub async fn root(args: RootArgs) -> i32 {
    if let Some(directory) = &args.directory
        && std::env::set_current_dir(directory).is_err()
    {
        return crate::fail(&format!("No such directory: {directory}"));
    }
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if !terminal {
        return crate::fail(
            "The interactive UI needs a terminal. Use `anysola run` for non-interactive use.",
        );
    }
    if let Some(message) = crate::gate::minimum_version().await {
        eprint!("{message}");
        return 1;
    }
    let db = match Db::open(&Paths::from_env().database()) {
        Ok(db) => db,
        Err(error) => return crate::fail(&error.to_string()),
    };
    match Token::resolve(&db) {
        Ok(Some(_)) => {}
        Ok(None) => {
            clack::intro("Welcome to Anysola Code");
            clack::info("Sign in with your Anysola account to continue.");
            if let Err(error) = crate::account::sign_in(&db).await {
                clack::failed(&error);
                return 1;
            }
            clack::outro("Signed in");
        }
        Err(error) => return crate::fail(&error.to_string()),
    }
    let directory = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => return crate::fail(&error.to_string()),
    };
    let core = match anysola_core::api::Core::start(&directory).await {
        Ok(core) => core,
        Err(error) => return crate::fail(&error.to_string()),
    };
    let mcp = anysola_mcp::Runtime::attach(&core);
    let options = anysola_tui::app::Options {
        session: args.session,
        continue_last: args.continue_last,
        prompt: args.prompt,
        auto: args.auto || args.yolo || args.dangerously_skip_permissions,
    };
    let result = anysola_tui::app::run(core, options).await;
    anysola_core::mcp::Mcp::shutdown(mcp.as_ref()).await;
    match result {
        Ok(()) => 0,
        Err(error) => crate::fail(&error),
    }
}
