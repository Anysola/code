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

/// The shell for hooks: an explicit `shell` config wins; otherwise, on Windows, Git Bash is
/// preferred over PowerShell when installed so Claude Code's POSIX hook commands
/// (`"$CLAUDE_PROJECT_DIR"/.claude/hooks/x.sh`) work out of the box.
pub fn resolve_for_hooks(configured: Option<&str>) -> String {
    if let Some(configured) = configured.filter(|c| !c.is_empty()) {
        return select(Some(configured));
    }
    if !cfg!(windows) {
        return resolve(None);
    }
    if git_bash_option().is_some() {
        return select_hooks_windows(env::var("SHELL").ok().as_deref());
    }
    static DEFAULT_HOOKS: OnceLock<String> = OnceLock::new();
    DEFAULT_HOOKS
        .get_or_init(|| select_hooks_windows(env::var("SHELL").ok().as_deref()))
        .clone()
}

fn select_hooks_windows(file: Option<&str>) -> String {
    // Prefer Git Bash on Windows (even if SHELL=powershell.exe is set) so Claude Code-style
    // POSIX hooks work out of the box when Git for Windows is installed.
    if let Some(bash) = git_bash()
        && let Some(shell) = executable(&bash)
    {
        return shell;
    }
    if let Some(file) = file.filter(|f| !f.is_empty() && !DENIED.contains(&name(f).as_str()))
        && let Some(shell) = executable(file)
    {
        return shell;
    }
    windows_fallback()
}

/// Arguments that make `shell` run `command` (not a login shell).
pub fn args(shell: &str, command: &str) -> Vec<String> {
    args_for(shell, command, cfg!(windows))
}

/// Redirected PowerShell output is in the console's OEM code page (cp857 on Turkish Windows),
/// which reads back as U+FFFD; this switches it to UTF-8 without BOM and ignores a failure (no
/// console). It runs before the command, so `$?` and the exit code stay the command's.
const PWSH_UTF8: &str = "try{[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$OutputEncoding=[Console]::OutputEncoding}catch{};";
/// The same for cmd; `&` runs the command whatever chcp returns.
const CMD_UTF8: &str = "chcp 65001>nul 2>&1 & ";

fn args_for(shell: &str, command: &str, windows: bool) -> Vec<String> {
    match name(shell).as_str() {
        "cmd" => {
            let command = if windows {
                format!("{CMD_UTF8}{command}")
            } else {
                command.into()
            };
            vec!["/c".into(), command]
        }
        "pwsh" | "powershell" => {
            // `using` and `param` must open the script, so those commands go unprefixed.
            let first = command.trim_start().to_ascii_lowercase();
            let leads = ["using ", "param(", "param ("]
                .iter()
                .any(|word| first.starts_with(word));
            let command = if windows && !leads {
                format!("{PWSH_UTF8}{command}")
            } else {
                command.into()
            };
            ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]
                .map(String::from)
                .into_iter()
                .chain([command])
                .collect()
        }
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
    which_shell(&shell).map(display)
}

/// A PATH lookup for a shell, never Windows' WSL launcher: a bare `bash` would find
/// `System32\bash.exe` (or the Store's `WindowsApps\bash.exe`) when Git Bash isn't installed, and
/// run commands in Linux with Windows paths. An absolute path the user configured is theirs.
fn which_shell(name: &str) -> Option<PathBuf> {
    let found = crate::which::which(name)?;
    let root = env::var("SystemRoot")
        .or_else(|_| env::var("windir"))
        .unwrap_or_default();
    (!(cfg!(windows) && wsl_launcher(&display(found.clone()), &root))).then_some(found)
}

/// Whether `path` is the WSL `bash.exe` launcher: in the system folder under `system_root`
/// (`C:\Windows` when unknown) or an app execution alias under `WindowsApps`.
fn wsl_launcher(path: &str, system_root: &str) -> bool {
    let path = path.replace('/', "\\").to_lowercase();
    let Some(folder) = path.strip_suffix("\\bash.exe") else {
        return false;
    };
    let root = match system_root.trim_end_matches(['\\', '/']) {
        "" => r"c:\windows".to_string(),
        root => root.replace('/', "\\").to_lowercase(),
    };
    ["system32", "sysnative"]
        .iter()
        .any(|name| folder == format!("{root}\\{name}"))
        || folder.split('\\').any(|part| part == "windowsapps")
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
    let found = || which_shell(&shell).map(display);
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

/// `ANYSOLA_GIT_BASH_PATH` (or `OPENCODE_…`), 0.5.1's `gitbash` server option (Windows only).
fn git_bash_option() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    crate::env::var("GIT_BASH_PATH").filter(|p| !p.is_empty())
}

/// Git Bash: the option, else `bash.exe` beside Git's install (from `git` on PATH, `cmd\git.exe`
/// or `mingw64\bin\git.exe`), else the standard machine-wide and per-user install folders.
fn git_bash() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    if let Some(opt) = git_bash_option()
        && Path::new(&opt).is_file()
    {
        return Some(opt);
    }
    let git = crate::which::which("git");
    let local = env::var("LOCALAPPDATA").ok();
    git_bash_candidates(git.as_deref(), local.as_deref())
        .into_iter()
        .find(|candidate| {
            std::fs::metadata(candidate)
                .ok()
                .is_some_and(|m| m.is_file() && m.len() > 0)
        })
}

/// Where Git Bash may be, in order, for `git` found on PATH and `%LOCALAPPDATA%`.
fn git_bash_candidates(git: Option<&Path>, local_app_data: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    // `<git>\cmd\git.exe` and `<git>\bin\git.exe`: two levels up; `<git>\mingw64\bin\git.exe`:
    // three.
    let mut dir = git.and_then(Path::parent);
    for _ in 0..2 {
        dir = dir.and_then(Path::parent);
        if let Some(dir) = dir {
            out.push(display(dir.join("bin").join("bash.exe")));
        }
    }
    out.push(r"C:\Program Files\Git\bin\bash.exe".to_string());
    out.push(r"C:\Program Files (x86)\Git\bin\bash.exe".to_string());
    if let Some(local) = local_app_data.filter(|l| !l.is_empty()) {
        // Git for Windows installed for one user ("Only for me").
        let local = local.trim_end_matches(['\\', '/']);
        out.push(format!(r"{local}\Programs\Git\bin\bash.exe"));
    }
    out
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
        assert_eq!(args_for("cmd", "dir", false), ["/c", "dir"]);
        assert_eq!(
            args_for("pwsh", "ls", false),
            ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "ls"]
        );
    }

    #[test]
    fn windows_output_is_utf8() {
        assert_eq!(
            args_for("cmd", "dir", true),
            ["/c", "chcp 65001>nul 2>&1 & dir"]
        );
        assert_eq!(
            args_for("powershell", "ls", true),
            [
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("{PWSH_UTF8}ls")
            ]
        );
        assert_eq!(
            args_for("pwsh", "using namespace System.Text; ls", true)[4],
            "using namespace System.Text; ls"
        );
        assert_eq!(args_for("/usr/bin/bash", "ls", true), ["-c", "ls"]);
    }

    /// The prefixed command keeps non-ASCII output and the command's exit code.
    #[test]
    #[cfg(windows)]
    fn windows_shells_print_turkish_letters() {
        let text = "şğıİçöü";
        for shell in ["powershell", "pwsh", "cmd"] {
            let Some(path) = crate::which::which(shell) else {
                continue;
            };
            let command = if shell == "cmd" {
                format!("echo {text}& exit /b 3")
            } else {
                format!("Write-Output '{text}'; exit 3")
            };
            let out = std::process::Command::new(path)
                .args(args(shell, &command))
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert_eq!(stdout.trim(), text, "{shell}");
            assert_eq!(out.status.code(), Some(3), "{shell}");
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn configured_shell_wins_unless_unusable() {
        assert_eq!(resolve(Some("/bin/sh")), "/bin/sh");
        assert_eq!(resolve_for_hooks(Some("/bin/sh")), "/bin/sh");
        assert_eq!(name(&resolve(Some("sh"))), "sh");
        let fallback = unix_fallback();
        assert_eq!(resolve(Some("fish")), fallback);
        assert_eq!(resolve(Some("/usr/bin/nu")), fallback);
        assert_eq!(resolve(Some("/no/such/shell")), fallback);
        assert_eq!(name(&fallback), "bash");
    }

    /// A bare `bash` on PATH may be WSL's launcher: never taken for a shell (Windows only).
    #[test]
    fn the_wsl_launcher_is_told_apart() {
        let root = r"C:\Windows";
        assert!(wsl_launcher(r"C:\Windows\System32\bash.exe", root));
        assert!(wsl_launcher(r"c:\windows\system32\BASH.EXE", root));
        assert!(wsl_launcher(r"C:\Windows\Sysnative\bash.exe", root));
        assert!(wsl_launcher(r"D:\Win\System32\bash.exe", r"D:\Win\"));
        assert!(wsl_launcher(r"C:\Windows\System32\bash.exe", ""));
        assert!(wsl_launcher(
            r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\bash.exe",
            root
        ));
        assert!(!wsl_launcher(r"C:\Program Files\Git\bin\bash.exe", root));
        assert!(!wsl_launcher(r"C:\Windows\System32\cmd.exe", root));
        assert!(!wsl_launcher(r"C:\tools\System32\bash.exe", root));
        assert!(!wsl_launcher("/usr/bin/bash", root));
    }

    /// Git Bash beside `git` on PATH (its `cmd`, `bin` or `mingw64\bin`), then the machine-wide
    /// folders, then a per-user install under `%LOCALAPPDATA%`.
    #[test]
    fn git_bash_is_looked_for_where_git_installs_it() {
        let candidates = git_bash_candidates(None, Some(r"C:\Users\me\AppData\Local\"));
        assert_eq!(
            candidates.last().unwrap(),
            r"C:\Users\me\AppData\Local\Programs\Git\bin\bash.exe"
        );
        assert!(candidates.contains(&r"C:\Program Files\Git\bin\bash.exe".to_string()));
        assert_eq!(git_bash_candidates(None, None).len(), 2);
        let git = Path::new("/opt/Git/mingw64/bin/git.exe");
        let found = git_bash_candidates(Some(git), None);
        assert_eq!(
            found[..2],
            [
                display(Path::new("/opt/Git/mingw64").join("bin").join("bash.exe")),
                display(Path::new("/opt/Git").join("bin").join("bash.exe")),
            ]
        );
    }

    #[test]
    #[cfg(windows)]
    fn hooks_prefer_git_bash_on_windows_when_installed() {
        if let Some(bash) = git_bash() {
            assert_eq!(name(&resolve_for_hooks(None)), "bash");
            assert_eq!(name(&bash), "bash");
        }
        assert_eq!(name(&resolve_for_hooks(Some("cmd"))), "cmd");
    }
}
