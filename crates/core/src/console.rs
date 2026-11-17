//! Child processes of a front end without a console. On Windows the desktop app is a GUI process,
//! so each console program it starts (git, the shell tool, formatters, MCP servers) would open a
//! console window of its own unless created with `CREATE_NO_WINDOW`. Processes with a console
//! (the CLI and TUI) start their children as before, sharing the console they run in.

use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};

static NO_CONSOLE: AtomicBool = AtomicBool::new(false);

/// Declares that this process has no console: console children then start without a window.
pub fn set_no_console() {
    NO_CONSOLE.store(true, Ordering::Relaxed);
}

/// Whether this process declared it has no console.
pub fn no_console() -> bool {
    NO_CONSOLE.load(Ordering::Relaxed)
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `std::process::Command::new` that opens no console window when this process has none.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    if no_console() {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// `tokio::process::Command::new` that opens no console window when this process has none.
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = tokio::process::Command::new(program);
    #[cfg(windows)]
    if no_console() {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
