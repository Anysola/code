//! How this copy of Anysola Code was installed, and the commands that update it.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Unix,
    Windows,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Unix
        }
    }
}

/// The running executable as the OS reports it (`process.execPath`).
pub fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("anysola"))
}

fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| std::path::absolute(path).unwrap_or_default())
}

/// The executable with links resolved.
pub fn real_exe() -> PathBuf {
    real(&exe())
}

/// Where the install scripts put the binary: `${ANYSOLA_INSTALL_DIR:-~/.anysola/bin}/anysola[.exe]`.
pub fn script_path(home: &Path) -> PathBuf {
    let dir = std::env::var_os("ANYSOLA_INSTALL_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".anysola").join("bin"));
    dir.join(if cfg!(windows) {
        "anysola.exe"
    } else {
        "anysola"
    })
}

/// True when this binary is the one the install script manages.
pub fn by_script(home: &Path) -> bool {
    let lower = |p: PathBuf| p.to_string_lossy().to_lowercase();
    lower(real_exe()) == lower(real(&script_path(home)))
}

/// The command that installs the latest release the way this copy was installed.
pub fn update_command(platform: Platform, script: bool) -> Vec<String> {
    let parts: &[&str] = match (script, platform) {
        (false, _) => &["npm", "i", "-g", "@anysola/code@latest"],
        (true, Platform::Windows) => &[
            "powershell",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "irm https://www.anysola.com/install.ps1 | iex",
        ],
        (true, Platform::Unix) => &[
            "sh",
            "-c",
            "curl -fsSL https://www.anysola.com/install | sh -s -- --no-modify-path",
        ],
    };
    parts.iter().map(|p| p.to_string()).collect()
}

/// A command as shown to the user: arguments with whitespace in JSON quotes.
pub fn display(command: &[String]) -> String {
    command
        .iter()
        .map(|arg| {
            if arg.chars().any(char::is_whitespace) {
                serde_json::to_string(arg).expect("string serializes")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_per_method_and_platform() {
        let show = |platform, script| display(&update_command(platform, script));
        assert_eq!(show(Platform::Unix, false), "npm i -g @anysola/code@latest");
        assert_eq!(
            show(Platform::Windows, false),
            "npm i -g @anysola/code@latest"
        );
        assert_eq!(
            show(Platform::Unix, true),
            r#"sh -c "curl -fsSL https://www.anysola.com/install | sh -s -- --no-modify-path""#
        );
        assert_eq!(
            show(Platform::Windows, true),
            r#"powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://www.anysola.com/install.ps1 | iex""#
        );
    }
}
