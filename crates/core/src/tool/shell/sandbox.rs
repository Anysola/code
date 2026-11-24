//! OS-level filesystem (and optional network) sandboxing for the `shell` tool.
//!
//! On Linux, when `bwrap` (bubblewrap) is installed and unprivileged user namespaces are allowed,
//! commands run with `/` mounted read-only, `/tmp` on a fresh tmpfs, and only the session's
//! working root (`ctx.worktree`, plus an explicitly approved external `cwd`) writable.
//!
//! Defaults: enabled automatically for isolated worktree sessions (`ctx.main != ctx.worktree`)
//! and off in the main folder unless `"sandbox": true` or `{"enabled": true, "network": false}`
//! is set in config. Set `"sandbox": false` to disable everywhere.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub network: bool,
}

impl SandboxConfig {
    pub fn from_value(value: Option<&Value>, isolated_worktree: bool) -> Self {
        match value {
            Some(Value::Bool(enabled)) => Self {
                enabled: *enabled,
                network: true,
            },
            Some(Value::Object(map)) => Self {
                enabled: map.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                network: map.get("network").and_then(Value::as_bool).unwrap_or(true),
            },
            _ => Self {
                enabled: isolated_worktree,
                network: true,
            },
        }
    }
}

/// Returns the `bwrap` binary path if bubblewrap is installed and functional on this machine.
#[cfg(target_os = "linux")]
pub fn bwrap_path() -> Option<&'static Path> {
    static BWRAP: OnceLock<Option<PathBuf>> = OnceLock::new();
    BWRAP
        .get_or_init(|| {
            let bin = crate::which::which("bwrap")?;
            let ok = std::process::Command::new(&bin)
                .args([
                    "--ro-bind",
                    "/",
                    "/",
                    "--dev",
                    "/dev",
                    "--proc",
                    "/proc",
                    "--",
                    "/bin/true",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            ok.then_some(bin)
        })
        .as_deref()
}

#[cfg(not(target_os = "linux"))]
pub fn bwrap_path() -> Option<&'static Path> {
    None
}

/// Wraps `(program, args)` in the OS sandbox when enabled and supported.
/// Returns `(program, args, active)`.
pub fn wrap(
    program: &str,
    args: Vec<String>,
    cwd: &Path,
    writable_root: &Path,
    cfg: SandboxConfig,
) -> (String, Vec<String>, bool) {
    if !cfg.enabled {
        return (program.to_string(), args, false);
    }
    let Some(bwrap) = bwrap_path() else {
        return (program.to_string(), args, false);
    };
    let wrapped = build_bwrap_args(program, &args, cwd, writable_root, cfg.network);
    (bwrap.to_string_lossy().into_owned(), wrapped, true)
}

fn build_bwrap_args(
    program: &str,
    args: &[String],
    cwd: &Path,
    writable_root: &Path,
    network: bool,
) -> Vec<String> {
    let root = std::fs::canonicalize(writable_root).unwrap_or_else(|_| writable_root.to_path_buf());
    let cwd_real = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let root_s = root.to_string_lossy().into_owned();
    let cwd_s = cwd_real.to_string_lossy().into_owned();

    let mut out = vec![
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--bind".into(),
        root_s.clone(),
        root_s,
    ];
    if !cwd_real.starts_with(&root) {
        out.extend(["--bind".into(), cwd_s.clone(), cwd_s.clone()]);
    }
    if !network {
        out.push("--unshare-net".into());
    }
    out.push("--die-with-parent".into());
    out.extend(["--chdir".into(), cwd_s, "--".into(), program.into()]);
    out.extend(args.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn config_defaults_and_overrides() {
        assert_eq!(
            SandboxConfig::from_value(None, false),
            SandboxConfig {
                enabled: false,
                network: true
            }
        );
        assert_eq!(
            SandboxConfig::from_value(None, true),
            SandboxConfig {
                enabled: true,
                network: true
            }
        );
        assert_eq!(
            SandboxConfig::from_value(Some(&json!(false)), true),
            SandboxConfig {
                enabled: false,
                network: true
            }
        );
        assert_eq!(
            SandboxConfig::from_value(Some(&json!({"enabled": true, "network": false})), false),
            SandboxConfig {
                enabled: true,
                network: false
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bwrap_allows_worktree_writes_and_blocks_outside_writes() {
        let Some(_) = bwrap_path() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        let outside = dir.path().join("main");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let cfg = SandboxConfig {
            enabled: true,
            network: true,
        };
        // Writing inside the worktree succeeds.
        let (prog, args, active) = wrap(
            "/bin/sh",
            vec!["-c".into(), "echo ok > inside.txt".into()],
            &worktree,
            &worktree,
            cfg,
        );
        assert!(active);
        let status = std::process::Command::new(&prog)
            .args(&args)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(worktree.join("inside.txt")).unwrap(),
            "ok\n"
        );

        // Writing to the main folder outside the worktree is blocked by the kernel.
        let target = outside.join("escaped.txt");
        let (prog, args, _) = wrap(
            "/bin/sh",
            vec!["-c".into(), format!("echo bad > '{}'", target.display())],
            &worktree,
            &worktree,
            cfg,
        );
        let status = std::process::Command::new(&prog)
            .args(&args)
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success());
        assert!(!target.exists());
    }
}
