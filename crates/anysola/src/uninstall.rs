//! `anysola uninstall`: removes the cache, the install script's PATH lines and the npm package.
//! Sessions, credentials, config and state stay; the binary's own directory is left for the user to
//! delete because a running binary cannot always remove itself.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anysola_core::paths::Paths;
use serde_json::Value;

use crate::clack::{self, Spinner};
use crate::cli::UninstallArgs;
use crate::install;

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const REMOVAL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Curl,
    Npm,
    Pnpm,
    Bun,
    Yarn,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::Curl => "curl",
            Method::Npm => "npm",
            Method::Pnpm => "pnpm",
            Method::Bun => "bun",
            Method::Yarn => "yarn",
        }
    }
}

/// The npm-style package whose `bin` is this executable (`<package>/bin/anysola.exe`), or that
/// installed the platform package (`@anysola/code-<platform>`) the npm launcher runs it from.
fn installed_package(exe: &Path) -> Option<String> {
    let directory = exe.parent()?.parent()?;
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join("package.json")).ok()?)
            .ok()?;
    let name = manifest.get("name")?.as_str()?;
    if name.starts_with("@anysola/code-") && manifest.get("bin").is_none() {
        return Some("@anysola/code".to_string());
    }
    let owns = manifest
        .get("bin")?
        .as_object()?
        .values()
        .filter_map(Value::as_str)
        .any(|bin| anysola_core::paths::normalize(&directory.join(bin)) == exe);
    owns.then(|| name.to_string())
}

async fn output(command: &[&str], timeout: Duration) -> Option<std::process::Output> {
    let child = tokio::process::Command::new(anysola_core::which::program(command[0]))
        .args(&command[1..])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(timeout, child).await.ok()?.ok()
}

/// Asks every package manager whether it installed `package`; a manager named in the
/// executable's path is preferred.
async fn package_manager(exe: &Path, package: &str) -> Option<Method> {
    let lower = exe.to_string_lossy().to_lowercase();
    let parts: Vec<&str> = lower
        .split(std::path::MAIN_SEPARATOR)
        .map(|p| p.trim_start_matches('.'))
        .collect();
    let mut checks: Vec<(Method, Vec<&str>)> = vec![
        (Method::Npm, vec!["npm", "list", "-g", "--depth=0", package]),
        (
            Method::Pnpm,
            vec!["pnpm", "list", "-g", "--depth=0", package],
        ),
        (Method::Bun, vec!["bun", "pm", "ls", "-g"]),
        (Method::Yarn, vec!["yarn", "global", "list"]),
    ];
    checks.sort_by_key(|(method, _)| !parts.contains(&method.name()));
    let results =
        futures_util::future::join_all(checks.iter().map(|(_, c)| output(c, PROBE_TIMEOUT))).await;
    checks
        .iter()
        .zip(results)
        .find(|(_, out)| {
            out.as_ref()
                .is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains(package))
        })
        .map(|((method, _), _)| *method)
}

fn removal_command(method: Method, package: &str) -> Option<Vec<String>> {
    let parts: &[&str] = match method {
        Method::Curl => return None,
        Method::Npm => &["npm", "uninstall", "--global"],
        Method::Pnpm => &["pnpm", "remove", "--global"],
        Method::Bun => &["bun", "remove", "--global"],
        Method::Yarn => &["yarn", "global", "remove"],
    };
    Some(
        parts
            .iter()
            .chain([&package])
            .map(|p| p.to_string())
            .collect(),
    )
}

fn path_entry(line: &str) -> bool {
    let line = line.trim();
    let command = line.starts_with("export PATH=")
        || line
            .strip_prefix("fish_add_path")
            .is_some_and(|rest| rest.starts_with(char::is_whitespace));
    command && line.contains(".anysola/bin")
}

/// Drops the install script's PATH lines and the `# anysola` comment above each.
fn clean_shell_config(content: &str) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    lines
        .iter()
        .enumerate()
        .filter(|(index, line)| {
            !path_entry(line)
                && !(line.trim() == "# anysola"
                    && lines.get(index + 1).is_some_and(|next| path_entry(next)))
        })
        .map(|(_, line)| *line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The shell rc files that carry the install script's PATH line, unless another binary shares
/// the directory.
fn shell_configs(home: &Path, exe: &Path) -> Vec<PathBuf> {
    let (Some(bin), Some(name)) = (exe.parent(), exe.file_name()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(bin) else {
        return Vec::new();
    };
    if entries.flatten().any(|entry| entry.file_name() != name) {
        return Vec::new();
    }
    let env_dir = |name: &str, default: PathBuf| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or(default)
    };
    let xdg = env_dir("XDG_CONFIG_HOME", home.join(".config"));
    let zsh = env_dir("ZDOTDIR", home.to_path_buf());
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty());
    let shell = shell
        .as_deref()
        .and_then(|s| Path::new(s).file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("bash");
    let candidates = match shell {
        "fish" => vec![
            home.join(".config/fish/config.fish"),
            xdg.join("fish/config.fish"),
        ],
        "zsh" => vec![
            zsh.join(".zshrc"),
            zsh.join(".zshenv"),
            xdg.join("zsh/.zshrc"),
            xdg.join("zsh/.zshenv"),
        ],
        "ash" | "sh" => vec![home.join(".ashrc"), home.join(".profile")],
        _ => vec![
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
            xdg.join("bash/.bashrc"),
            xdg.join("bash/.bash_profile"),
        ],
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for file in candidates {
        if !files.contains(&file)
            && std::fs::read_to_string(&file).is_ok_and(|c| clean_shell_config(&c) != c)
        {
            files.push(file);
        }
    }
    files
}

/// The tail of a failed command's output, as 0.5.1 reported it.
fn concise(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .trim()
        .split(['\r', '\n'])
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(12)..].join("\n");
    let chars: Vec<char> = tail.chars().collect();
    let clipped = chars.len() > 2000;
    let detail = if clipped {
        format!(
            "…{}",
            chars[chars.len() - 1999..].iter().collect::<String>()
        )
    } else {
        tail
    };
    if detail.is_empty() {
        return None;
    }
    if lines.len() <= 12 && !clipped {
        return Some(detail);
    }
    Some(format!(
        "{detail}\n\nOutput shortened to the last 12 lines."
    ))
}

async fn remove_package(command: &[String]) -> Result<(), String> {
    let args: Vec<&str> = command.iter().map(String::as_str).collect();
    let Some(out) = output(&args, REMOVAL_TIMEOUT).await else {
        return Err(format!("{} did not finish", command.join(" ")));
    };
    if out.status.success() {
        return Ok(());
    }
    Err(concise(&String::from_utf8_lossy(&out.stderr))
        .or_else(|| concise(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_else(|| {
            format!(
                "The command exited with code {} without any error output.",
                out.status.code().unwrap_or(-1)
            )
        }))
}

/// Drops the install script's directory from the user PATH; an error message on failure.
fn remove_windows_path(directory: &Path) -> Option<String> {
    let dir = directory.display().to_string();
    let script = [
        format!("$dir = '{}'", dir.replace('\'', "''")),
        "$path = [Environment]::GetEnvironmentVariable('Path', 'User')".to_string(),
        "$kept = @($path -split ';' | Where-Object { $_ -and $_ -ne $dir })".to_string(),
        "[Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')".to_string(),
    ]
    .join("; ");
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    (!status.is_ok_and(|s| s.success())).then(|| format!("User PATH: remove {dir} yourself"))
}

pub async fn uninstall(args: UninstallArgs) -> i32 {
    clack::intro("Uninstall Anysola Code");
    let paths = Paths::from_env();
    let exe = install::exe();
    let real = install::real_exe();
    let package = installed_package(&real);
    let method = if install::by_script(&paths.home) {
        Some(Method::Curl)
    } else {
        match &package {
            Some(package) => package_manager(&real, package).await,
            None => None,
        }
    };
    let removal = method
        .zip(package.as_deref())
        .and_then(|(method, package)| removal_command(method, package));
    let windows = cfg!(windows);
    let cache = paths.data.join("cache");
    let shell = if method == Some(Method::Curl) {
        shell_configs(&paths.home, &exe)
    } else {
        Vec::new()
    };
    let bin_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();

    clack::info(&format!(
        "Installation method: {}",
        method.map_or("unknown", Method::name)
    ));
    clack::message("The following global files will be removed:");
    if cache.exists() {
        clack::info(&format!("  Cache: {}", cache.display()));
    }
    for file in &shell {
        clack::info(&format!("  Shell PATH: {}", file.display()));
    }
    if let Some(command) = &removal {
        clack::info(&format!("  Package: {}", command.join(" ")));
    }
    if method == Some(Method::Curl) && windows {
        clack::info(&format!("  User PATH entry: {}", bin_dir.display()));
    }
    if method == Some(Method::Curl) {
        clack::info(&format!("  Binary (manual removal): {}", exe.display()));
    }
    if method.is_none() {
        clack::warn(
            "Could not detect the installation method. Remove the installation manually after cleanup.",
        );
    }

    if args.dry_run {
        clack::warn("Dry run - no changes made");
        clack::outro("Done");
        return 0;
    }
    if !args.force {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            clack::failed(
                "Use --force to uninstall without an interactive terminal, or --dry-run to preview.",
            );
            return 1;
        }
        match crate::prompt::confirm("Are you sure you want to uninstall?", false) {
            None => {
                clack::cancel("Cancelled");
                return 130;
            }
            Some(false) => {
                clack::outro("Cancelled");
                return 0;
            }
            Some(true) => {}
        }
    }

    let mut errors = Vec::new();
    let progress = Spinner::start("Removing Cache...");
    match std::fs::remove_dir_all(&cache) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            progress.stop("Failed to remove Cache", false);
            errors.push(format!("Cache: {error}"));
        }
        _ => progress.stop("Removed Cache", true),
    }
    for file in &shell {
        let cleaned = std::fs::read_to_string(file)
            .and_then(|content| std::fs::write(file, clean_shell_config(&content)));
        if let Err(error) = cleaned {
            errors.push(format!("Shell config {}: {error}", file.display()));
        }
    }
    if let Some(command) = &removal {
        let shown = command.join(" ");
        let progress = Spinner::start(&format!("Running {shown}..."));
        match remove_package(command).await {
            Ok(()) => progress.stop("Package removed", true),
            Err(error) => {
                progress.stop("Package manager uninstall failed", false);
                errors.push(error);
                clack::warn(&format!("Run manually: {shown}"));
            }
        }
    }
    if method == Some(Method::Curl) {
        if windows && let Some(error) = remove_windows_path(&bin_dir) {
            errors.push(error);
        }
        // Only the bin directory: its parent `~/.anysola` holds the sessions and config uninstall keeps.
        let dir = bin_dir.display().to_string();
        clack::message("To finish removing the binary, run:");
        if windows {
            clack::info(&format!(
                "  Remove-Item -Recurse -Force '{}'",
                dir.replace('\'', "''")
            ));
        } else {
            clack::info(&format!("  rm -rf '{}'", dir.replace('\'', "'\\''")));
        }
    }
    if !errors.is_empty() {
        clack::failed(&errors.join("\n"));
        return 1;
    }
    clack::outro("Done");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_config_cleanup() {
        let content = "alias x=y\n\n# anysola\nexport PATH=/home/u/.anysola/bin:$PATH\nexport PATH=/usr/bin:$PATH\n# anysola\nfish_add_path /home/u/.anysola/bin\n# anysola\n";
        assert_eq!(
            clean_shell_config(content),
            "alias x=y\n\nexport PATH=/usr/bin:$PATH\n# anysola\n"
        );
    }

    #[test]
    fn concise_output() {
        assert_eq!(concise("\n\x20 \n"), None);
        assert_eq!(concise("a\r\nb\n").as_deref(), Some("a\nb"));
        let long: String = (0..20).map(|n| format!("{n}\n")).collect();
        assert_eq!(
            concise(&long).unwrap(),
            "8\n9\n10\n11\n12\n13\n14\n15\n16\n17\n18\n19\n\nOutput shortened to the last 12 lines."
        );
    }
}
