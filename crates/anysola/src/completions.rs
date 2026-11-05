//! `--completions <shell>`: a completion script generated from the command tree.

use clap::CommandFactory;
use clap_complete::Shell as Target;

use crate::cli::{Cli, Shell};

pub fn print(shell: Shell) {
    let target = match shell {
        Shell::Bash | Shell::Sh => Target::Bash,
        Shell::Zsh => Target::Zsh,
        Shell::Fish => Target::Fish,
        Shell::Powershell | Shell::Pwsh => Target::PowerShell,
    };
    clap_complete::generate(
        target,
        &mut Cli::command(),
        "anysola",
        &mut std::io::stdout(),
    );
}
