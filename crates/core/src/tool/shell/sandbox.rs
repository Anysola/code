//! Experimental OS-level filesystem (and optional network) sandboxing for the `shell` tool.
//!
//! On Linux, with `bwrap` (bubblewrap) installed and unprivileged user namespaces allowed,
//! commands run with `/` mounted read-only, `/tmp` on a private tmpfs, and only the session's
//! working root (`ctx.worktree`) writable. Nothing else is ever bound writable: not an external
//! `cwd` (it is entered read-only), not the repository's `.git` data (so `git add`/`commit` don't
//! work inside it). It only limits the shell tool's writes: reads and the network stay open
//! (unless `"network": false`), and hooks, LSP servers, scheduled checks and the embedded
//! terminal run outside it.
//!
//! Off by default, worktree sessions included. Only user or managed config turns it on:
//! `"sandbox": true` or an object such as `{"network": false}`; any other value is off. Project
//! config files cannot set it. When it is on and `bwrap` can't start (not installed, user
//! namespaces restricted, not Linux), commands are refused rather than run unconfined.

use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub network: bool,
}

impl SandboxConfig {
    /// The `"sandbox"` config value: `true`, or an object (on unless it says `"enabled": false`:
    /// `{"network": false}` alone asks for the sandbox too, and must never mean "nothing").
    pub fn from_value(value: Option<&Value>) -> Self {
        match value {
            Some(Value::Bool(enabled)) => Self {
                enabled: *enabled,
                network: true,
            },
            Some(Value::Object(map)) => Self {
                enabled: map.get("enabled") != Some(&Value::Bool(false)),
                // Open only when left out or `true`: a misspelt `"false"` never opens it.
                network: map.get("network").is_none_or(|v| v == &Value::Bool(true)),
            },
            _ => Self {
                enabled: false,
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

/// Why a command was refused when the sandbox is on but can't start here.
fn unavailable() -> String {
    if cfg!(target_os = "linux") {
        "The shell sandbox is on (\"sandbox\" in your config) but bubblewrap (bwrap) can't start \
         on this machine, so the command was not run. Install bubblewrap and allow unprivileged \
         user namespaces, or remove \"sandbox\" from ~/.anysola/config.json."
            .into()
    } else {
        "The shell sandbox is on (\"sandbox\" in your config) but it only works on Linux, so the \
         command was not run. Remove \"sandbox\" from ~/.anysola/config.json to run commands."
            .into()
    }
}

/// Wraps `(program, args)` in the OS sandbox when enabled. Returns `(program, args, active)`;
/// `Err` (the command must not run) when it is enabled but can't start here.
pub fn wrap(
    program: &str,
    args: Vec<String>,
    cwd: &Path,
    writable_root: &Path,
    cfg: SandboxConfig,
) -> Result<(String, Vec<String>, bool), String> {
    if !cfg.enabled {
        return Ok((program.to_string(), args, false));
    }
    #[cfg(target_os = "linux")]
    if let Some(bwrap) = bwrap_path() {
        let wrapped = build_bwrap_args(program, &args, cwd, writable_root, cfg.network);
        return Ok((bwrap.to_string_lossy().into_owned(), wrapped, true));
    }
    let _ = (cwd, writable_root);
    Err(unavailable())
}

/// The `bwrap` arguments: everything read-only but the root; the command starts in `cwd` (which
/// may be outside the root, read-only).
#[cfg(target_os = "linux")]
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
    if !network {
        out.push("--unshare-net".into());
    }
    // Redirect temp and XDG cache dirs into the private /tmp tmpfs so build tools don't fail
    // trying to write caches under a read-only $HOME.
    out.extend([
        "--setenv".into(),
        "TMPDIR".into(),
        "/tmp".into(),
        "--setenv".into(),
        "XDG_CACHE_HOME".into(),
        "/tmp/.cache".into(),
        "--die-with-parent".into(),
        "--chdir".into(),
        cwd_s,
        "--".into(),
        program.into(),
    ]);
    out.extend(args.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ON: SandboxConfig = SandboxConfig {
        enabled: true,
        network: true,
    };

    #[test]
    fn only_the_users_own_request_turns_it_on() {
        let off = SandboxConfig {
            enabled: false,
            network: true,
        };
        // Off by default, worktree sessions included (there is no other input).
        assert_eq!(SandboxConfig::from_value(None), off);
        assert_eq!(SandboxConfig::from_value(Some(&json!(false))), off);
        assert_eq!(SandboxConfig::from_value(Some(&json!("on"))), off);
        assert_eq!(SandboxConfig::from_value(Some(&json!(1))), off);
        assert_eq!(
            SandboxConfig::from_value(Some(&json!({"enabled": false, "network": false}))),
            SandboxConfig {
                enabled: false,
                network: false
            }
        );
        assert_eq!(SandboxConfig::from_value(Some(&json!(true))), ON);
        assert_eq!(
            SandboxConfig::from_value(Some(&json!({"network": "false"}))),
            SandboxConfig {
                enabled: true,
                network: false
            }
        );
        // An object asks for it unless it says otherwise: cutting the network is never a no-op.
        assert_eq!(
            SandboxConfig::from_value(Some(&json!({"network": false}))),
            SandboxConfig {
                enabled: true,
                network: false
            }
        );
        assert_eq!(
            SandboxConfig::from_value(Some(&json!({"enabled": true, "network": false}))),
            SandboxConfig {
                enabled: true,
                network: false
            }
        );
    }

    fn available() -> bool {
        #[cfg(target_os = "linux")]
        {
            bwrap_path().is_some()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    /// Asked for where it can't start: the command is refused, never run unconfined.
    #[test]
    fn an_unavailable_sandbox_refuses_instead_of_running_unconfined() {
        if available() {
            return;
        }
        let here = std::env::current_dir().unwrap();
        let err = wrap(
            "/bin/sh",
            vec!["-c".into(), "true".into()],
            &here,
            &here,
            ON,
        )
        .unwrap_err();
        assert!(err.contains("was not run"), "{err}");
        assert!(err.contains("\"sandbox\""), "{err}");
        // Off: the command goes as it is.
        let (program, args, active) = wrap(
            "/bin/sh",
            vec!["-c".into()],
            &here,
            &here,
            SandboxConfig {
                enabled: false,
                network: true,
            },
        )
        .unwrap();
        assert_eq!(
            (program.as_str(), args.len(), active),
            ("/bin/sh", 1, false)
        );
    }

    /// The targets of every writable bind.
    #[cfg(target_os = "linux")]
    fn writable(args: &[String]) -> Vec<String> {
        args.windows(3)
            .filter(|w| w[0] == "--bind")
            .map(|w| w[2].clone())
            .collect()
    }

    #[cfg(target_os = "linux")]
    fn real(path: &Path) -> String {
        std::fs::canonicalize(path)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_root_is_the_only_writable_bind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("wt");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // An approved external working directory is entered, not bound writable.
        let args = build_bwrap_args("/bin/sh", &[], &outside, &root, true);
        assert_eq!(writable(&args), vec![real(&root)]);
        let chdir = args.iter().position(|a| a == "--chdir").unwrap();
        assert_eq!(args[chdir + 1], real(&outside));
        // Nor is `/` itself.
        let args = build_bwrap_args("/bin/sh", &[], Path::new("/"), &root, true);
        assert_eq!(writable(&args), vec![real(&root)]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_gitfile_never_adds_a_writable_bind() {
        let dir = tempfile::tempdir().unwrap();
        let admin = dir.path().join("main/.git/worktrees/wt");
        let other = dir.path().join("data/worktrees/proj-other");
        let root = dir.path().join("wt");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        for target in [&admin, &other] {
            std::fs::write(root.join(".git"), format!("gitdir: {}\n", target.display())).unwrap();
            let args = build_bwrap_args("/bin/sh", &[], &root, &root, true);
            assert_eq!(writable(&args), vec![real(&root)], "gitdir {target:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_network_unshares_it() {
        let dir = tempfile::tempdir().unwrap();
        let off = build_bwrap_args("/bin/sh", &[], dir.path(), dir.path(), false);
        assert!(off.iter().any(|a| a == "--unshare-net"));
        let on = build_bwrap_args("/bin/sh", &[], dir.path(), dir.path(), true);
        assert!(!on.iter().any(|a| a == "--unshare-net"));
    }

    /// Runs under bwrap for real. Its folders sit outside `/tmp`: the sandbox's private `/tmp`
    /// would hide them and a refused write would pass for the wrong reason. Skipped where bwrap
    /// can't start, unless `ANYSOLA_REQUIRE_BWRAP=1` (CI) makes that a failure.
    #[cfg(target_os = "linux")]
    #[test]
    fn bwrap_allows_root_writes_and_blocks_outside_writes() {
        if bwrap_path().is_none() {
            assert!(
                std::env::var("ANYSOLA_REQUIRE_BWRAP").as_deref() != Ok("1"),
                "ANYSOLA_REQUIRE_BWRAP=1 but bwrap can't start here"
            );
            return;
        }
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sandbox-tests");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::tempdir_in(&base).unwrap();
        assert!(!dir.path().starts_with("/tmp"));
        let worktree = dir.path().join("wt");
        let outside = dir.path().join("main");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let run = |cwd: &Path, script: String| {
            let (prog, args, active) =
                wrap("/bin/sh", vec!["-c".into(), script], cwd, &worktree, ON).unwrap();
            assert!(active);
            std::process::Command::new(&prog)
                .args(&args)
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .code()
        };

        // Writing inside the root succeeds.
        assert_eq!(run(&worktree, "echo ok > inside.txt".into()), Some(0));
        assert_eq!(
            std::fs::read_to_string(worktree.join("inside.txt")).unwrap(),
            "ok\n"
        );
        // The folder outside is there (seen) but read-only: exit 3 would mean it was hidden.
        let target = outside.join("escaped.txt");
        let code = run(
            &worktree,
            format!(
                "test -d '{}' || exit 3; echo bad > '{}'",
                outside.display(),
                target.display()
            ),
        );
        assert!(!matches!(code, Some(0) | Some(3)), "{code:?}");
        assert!(!target.exists());
        // Starting in that folder doesn't make it writable either.
        let code = run(
            &outside,
            "pwd >/dev/null || exit 3; echo bad > here.txt".into(),
        );
        assert!(!matches!(code, Some(0) | Some(3)), "{code:?}");
        assert!(!outside.join("here.txt").exists());
    }
}
