//! Which shell runs commands (0.5.1's `ShellSelect` with the "compat" priority): the configured
//! shell, else `$SHELL`, as long as it is not fish or nu and resolves to an executable; else the
//! OS fallback. A configured shell that is unusable goes straight to the fallback.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::tool::access::windows_path;

/// Shells whose syntax the tool cannot drive.
const DENIED: [&str; 2] = ["fish", "nu"];

/// The shell's display name: lower-cased file name, without extension on Windows.
pub fn name(file: &str) -> String {
    let file = windows_path(file);
    let path = Path::new(&file);
    let part = if cfg!(windows) {
        path.file_stem()
    } else {
        path.file_name()
    };
    part.map_or_else(|| file.clone(), |p| p.to_string_lossy().into_owned())
        .to_lowercase()
}

/// The shell for a session with this `shell` config value. The `$SHELL`-based choice is made once
/// per process.
pub fn resolve(configured: Option<&str>) -> String {
    if let Some(configured) = configured.filter(|c| !c.is_empty()) {
        return select(Some(configured));
    }
    if git_bash_option().is_some() {
        return select(env::var("SHELL").ok().as_deref());
    }
    static DEFAULT: OnceLock<String> = OnceLock::new();
    DEFAULT
        .get_or_init(|| select(env::var("SHELL").ok().as_deref()))
        .clone()
}

/// Arguments that make `shell` run `command` (not a login shell).
pub fn args(shell: &str, command: &str) -> Vec<String> {
    match name(shell).as_str() {
        "cmd" => vec!["/c".into(), command.into()],
        "pwsh" | "powershell" => [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            command,
        ]
        .map(String::from)
        .to_vec(),
        _ => vec!["-c".into(), command.into()],
    }
}

fn select(file: Option<&str>) -> String {
    if let Some(file) = file.filter(|f| !f.is_empty() && !DENIED.contains(&name(f).as_str()))
        && let Some(shell) = executable(file)
    {
        return shell;
    }
    if cfg!(windows) {
        windows_fallback()
    } else {
        unix_fallback()
    }
}

fn executable(file: &str) -> Option<String> {
    let shell = full(file);
    if Path::new(&shell).is_absolute() {
        return Path::new(&shell).is_file().then_some(shell);
    }
    crate::which::which(&shell).map(display)
}

/// On Windows: drive-path rewriting, Git Bash for POSIX-style bash paths, PATH lookup of bare
/// names. Elsewhere the path as given.
fn full(file: &str) -> String {
    if !cfg!(windows) {
        return file.to_string();
    }
    let shell = windows_path(file);
    let bare = Path::new(&shell)
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty());
    let is_bash = name(&shell) == "bash";
    if !bare {
        if shell.starts_with('/') && is_bash {
            return git_bash().unwrap_or(shell);
        }
        return shell;
    }
    let found = || crate::which::which(&shell).map(display);
    if is_bash {
        return git_bash().or_else(found).unwrap_or(shell);
    }
    found().unwrap_or(shell)
}

fn unix_fallback() -> String {
    if cfg!(target_os = "macos") {
        return "/bin/zsh".into();
    }
    crate::which::which("bash").map_or_else(|| "/bin/sh".into(), display)
}

fn windows_fallback() -> String {
    let comspec = env::var("COMSPEC")
        .ok()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "cmd.exe".into());
    crate::which::which("pwsh")
        .or_else(|| crate::which::which("powershell"))
        .map(display)
        .or_else(git_bash)
        .map_or_else(|| full(&comspec), |shell| full(&shell))
}

/// `OPENCODE_GIT_BASH_PATH`, 0.5.1's `gitbash` server option (Windows only).
fn git_bash_option() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    env::var("OPENCODE_GIT_BASH_PATH")
        .ok()
        .filter(|p| !p.is_empty())
}

/// Git Bash: the option, else `bash.exe` next to Git's `bin` if it is non-empty.
fn git_bash() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    git_bash_option().or_else(|| {
        let git = crate::which::which("git")?;
        let bash = git.parent()?.parent()?.join("bin").join("bash.exe");
        let size = std::fs::metadata(&bash).ok()?.len();
        (size > 0).then(|| display(bash))
    })
}

fn display(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(name("/bin/bash"), "bash");
        assert_eq!(name("/usr/bin/ZSH"), "zsh");
        assert_eq!(name("sh"), "sh");
    }

    #[test]
    fn args_per_shell_kind() {
        assert_eq!(args("/bin/bash", "ls"), ["-c", "ls"]);
        assert_eq!(args("cmd", "dir"), ["/c", "dir"]);
        assert_eq!(
            args("pwsh", "ls"),
            ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "ls"]
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn configured_shell_wins_unless_unusable() {
        assert_eq!(resolve(Some("/bin/sh")), "/bin/sh");
        assert_eq!(name(&resolve(Some("sh"))), "sh");
        let fallback = unix_fallback();
        assert_eq!(resolve(Some("fish")), fallback);
        assert_eq!(resolve(Some("/usr/bin/nu")), fallback);
        assert_eq!(resolve(Some("/no/such/shell")), fallback);
        assert_eq!(name(&fallback), "bash");
    }
}
