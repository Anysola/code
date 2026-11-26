//! Experimental OS-level filesystem, PID/session, and optional network sandboxing for shell
//! commands (`shell` tool, scheduled `{run}` checks, and template ``!`cmd` ``).
//!
//! On Linux, with `bwrap` (bubblewrap) installed and unprivileged user namespaces allowed,
//! commands run in a private PID namespace (`--unshare-pid`) and session (`--new-session`), with
//! `/` mounted read-only, `/tmp` on a private tmpfs, and only the session's working root
//! (`ctx.worktree`) plus an optional per-session cache directory writable. If `<root>/.git`
//! exists (a directory in main-folder repos or a gitfile in worktrees), `<root>/.git` itself is
//! re-mounted read-only so sandboxed commands cannot modify, remove, or rename `.git` to poison
//! git hooks or repository config. Nothing else is ever bound writable: not an external `cwd` (it
//! is entered read-only), not a worktree's shared `.git` admin directory. It only limits writes
//! and process visibility: reads and the network stay open (unless `"network": false`), and
//! hooks, LSP servers, formatters, MCP servers, and the embedded terminal run outside it.
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

/// Appended to a failed sandboxed command's result when its output reports a read-only filesystem.
pub const READ_ONLY_NOTE: &str = "[sandbox: writes are limited to the session's working folder]";

/// Whether a command's output mentions a read-only filesystem error (covers GNU/BSD
/// `Read-only file system`, Node/libuv/Go `read-only file system`, and `EROFS`).
pub fn mentions_read_only(output: &str) -> bool {
    output
        .to_ascii_lowercase()
        .contains("read-only file system")
        || output.contains("EROFS")
}

/// Subdirectories prepared inside a session's writable cache folder (`Paths::sandbox_cache_dir`).
#[cfg(target_os = "linux")]
const CACHE_SUBDIRS: [(&str, &str); 7] = [
    ("TMPDIR", "tmp"),
    ("XDG_CACHE_HOME", "xdg"),
    ("npm_config_cache", "npm"),
    ("PIP_CACHE_DIR", "pip"),
    ("GOCACHE", "go-build"),
    ("GOMODCACHE", "go-mod"),
    ("UV_CACHE_DIR", "uv"),
];

/// Fallback cache environment paths under the private `/tmp` tmpfs when no session cache dir is
/// supplied.
#[cfg(target_os = "linux")]
const TMP_CACHES: [(&str, &str); 7] = [
    ("TMPDIR", "/tmp"),
    ("XDG_CACHE_HOME", "/tmp/.cache"),
    ("npm_config_cache", "/tmp/.npm"),
    ("PIP_CACHE_DIR", "/tmp/.pip"),
    ("GOCACHE", "/tmp/.go-build"),
    ("GOMODCACHE", "/tmp/.go-mod"),
    ("UV_CACHE_DIR", "/tmp/.uv"),
];

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
                    "--tmpfs",
                    "/tmp",
                    "--unshare-pid",
                    "--new-session",
                    "--die-with-parent",
                    "--",
                    "/bin/sh",
                    "-c",
                    ":",
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

/// Prepares a session's writable cache directory and its subdirectories if neither `dir` nor its
/// parent is a symlink, returning its canonical path verified to sit inside its canonical parent.
#[cfg(target_os = "linux")]
fn prepare_cache_dir(dir: &Path) -> Option<PathBuf> {
    let parent = dir.parent()?;
    if let Ok(meta) = std::fs::symlink_metadata(parent)
        && (!meta.is_dir() || meta.file_type().is_symlink())
    {
        return None;
    }
    if let Ok(meta) = std::fs::symlink_metadata(dir)
        && (!meta.is_dir() || meta.file_type().is_symlink())
    {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    let real_parent = std::fs::canonicalize(parent).ok()?;
    let real = std::fs::canonicalize(dir).ok()?;
    if real == Path::new("/") || real.parent() != Some(real_parent.as_path()) {
        return None;
    }
    for (_, sub) in CACHE_SUBDIRS {
        std::fs::create_dir_all(real.join(sub)).ok()?;
    }
    Some(real)
}

/// Wraps `(program, args)` in the OS sandbox when enabled. Returns `(program, args, active)`;
/// `Err` (the command must not run) when it is enabled but can't start here or `writable_root`
/// resolves to `/`.
pub fn wrap(
    program: &str,
    args: Vec<String>,
    cwd: &Path,
    writable_root: &Path,
    cache_dir: Option<&Path>,
    cfg: SandboxConfig,
) -> Result<(String, Vec<String>, bool), String> {
    if !cfg.enabled {
        return Ok((program.to_string(), args, false));
    }
    #[cfg(target_os = "linux")]
    if let Some(bwrap) = bwrap_path() {
        let root =
            std::fs::canonicalize(writable_root).unwrap_or_else(|_| writable_root.to_path_buf());
        if root == Path::new("/") {
            return Err(
                "The shell sandbox refused to run with `/` as the writable working folder, so the command was not run."
                    .into(),
            );
        }
        let prepared_cache = cache_dir.and_then(prepare_cache_dir);
        let wrapped = build_bwrap_args(
            program,
            &args,
            cwd,
            &root,
            prepared_cache.as_deref(),
            cfg.network,
        );
        return Ok((bwrap.to_string_lossy().into_owned(), wrapped, true));
    }
    let _ = (cwd, writable_root, cache_dir);
    Err(unavailable())
}

/// The `bwrap` arguments: everything read-only except `writable_root` (with `<root>/.git` itself
/// re-mounted read-only so it cannot be edited, removed, or renamed) and optional `cache_dir`;
/// private PID namespace and session; the command starts in `cwd` (which may be outside the root,
/// read-only).
#[cfg(target_os = "linux")]
fn build_bwrap_args(
    program: &str,
    args: &[String],
    cwd: &Path,
    writable_root: &Path,
    cache_dir: Option<&Path>,
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
        "--unshare-pid".into(),
        "--new-session".into(),
        "--bind".into(),
        root_s.clone(),
        root_s,
    ];
    // Protect `<root>/.git` itself as a read-only mountpoint (both main-folder `.git/` dirs and
    // worktree `.git` gitfiles): blocks writes, missing-subdir creation, `rm .git`, and
    // `mv .git .git.old` (`EBUSY` on a mountpoint).
    let git_path = root.join(".git");
    if std::fs::symlink_metadata(&git_path)
        .is_ok_and(|m| (m.is_dir() || m.is_file()) && !m.file_type().is_symlink())
    {
        let text = git_path.to_string_lossy().into_owned();
        out.extend(["--ro-bind".into(), text.clone(), text]);
    }
    if !network {
        out.push("--unshare-net".into());
    }
    // Clear uppercase `NPM_CONFIG_CACHE` so host overrides never shadow `npm_config_cache`.
    out.extend(["--unsetenv".into(), "NPM_CONFIG_CACHE".into()]);
    // Redirect temp and toolchain caches into either the persistent per-session cache dir or the
    // private /tmp tmpfs so build tools don't fail trying to write under a read-only $HOME.
    let cache_real = cache_dir
        .map(|c| std::fs::canonicalize(c).unwrap_or_else(|_| c.to_path_buf()))
        .filter(|c| c != Path::new("/"));
    if let Some(cache_real) = cache_real {
        let cache_s = cache_real.to_string_lossy().into_owned();
        out.extend(["--bind".into(), cache_s.clone(), cache_s]);
        for (var, sub) in CACHE_SUBDIRS {
            out.extend([
                "--setenv".into(),
                var.into(),
                cache_real.join(sub).to_string_lossy().into_owned(),
            ]);
        }
    } else {
        for (var, value) in TMP_CACHES {
            out.extend(["--setenv".into(), var.into(), value.into()]);
        }
    }
    out.extend([
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
            None,
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
            None,
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

    /// The targets of every read-only bind.
    #[cfg(target_os = "linux")]
    fn readonly(args: &[String]) -> Vec<String> {
        args.windows(3)
            .filter(|w| w[0] == "--ro-bind")
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
        let args = build_bwrap_args("/bin/sh", &[], &outside, &root, None, true);
        assert_eq!(writable(&args), vec![real(&root)]);
        assert!(args.iter().any(|a| a == "--unshare-pid"));
        assert!(args.iter().any(|a| a == "--new-session"));
        let chdir = args.iter().position(|a| a == "--chdir").unwrap();
        assert_eq!(args[chdir + 1], real(&outside));
        // Nor is `/` itself.
        let args = build_bwrap_args("/bin/sh", &[], Path::new("/"), &root, None, true);
        assert_eq!(writable(&args), vec![real(&root)]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dot_git_is_mounted_read_only_for_repos_and_worktrees() {
        let dir = tempfile::tempdir().unwrap();
        // Main-folder repo (.git directory)
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let args = build_bwrap_args("/bin/sh", &[], &repo, &repo, None, true);
        let ro = readonly(&args);
        let git_real = real(&repo.join(".git"));
        assert!(
            ro.contains(&git_real),
            "missing ro-bind for .git dir: {ro:?}"
        );
        let bind_pos = args.iter().position(|a| a == "--bind").unwrap();
        let git_pos = args.iter().rposition(|a| a == &git_real).unwrap();
        assert!(git_pos > bind_pos);

        // Worktree (.git regular file)
        let wt = dir.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: /tmp/admin\n").unwrap();
        let args = build_bwrap_args("/bin/sh", &[], &wt, &wt, None, true);
        assert!(readonly(&args).contains(&real(&wt.join(".git"))));
        assert_eq!(writable(&args), vec![real(&wt)]);
    }

    #[test]
    fn mentions_read_only_matches_posix_node_and_go() {
        assert!(mentions_read_only(
            "cannot create /x: Read-only file system"
        ));
        assert!(mentions_read_only(
            "Error: EROFS: read-only file system, open '/x'"
        ));
        assert!(mentions_read_only("open /x: read-only file system"));
        assert!(mentions_read_only("failed with EROFS"));
        assert!(!mentions_read_only("Permission denied"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn session_cache_dir_is_bound_writable_and_sets_toolchain_envs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("wt");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&root).unwrap();
        let prepared = prepare_cache_dir(&cache).unwrap();
        let args = build_bwrap_args("/bin/sh", &[], &root, &root, Some(&prepared), true);
        assert_eq!(writable(&args), vec![real(&root), real(&cache)]);
        for (var, sub) in CACHE_SUBDIRS {
            let pos = args
                .windows(3)
                .position(|w| w[0] == "--setenv" && w[1] == var)
                .unwrap_or_else(|| panic!("missing --setenv {var}: {args:?}"));
            assert_eq!(args[pos + 2], prepared.join(sub).to_string_lossy());
        }
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
            let args = build_bwrap_args("/bin/sh", &[], &root, &root, None, true);
            assert_eq!(writable(&args), vec![real(&root)], "gitdir {target:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_network_unshares_it() {
        let dir = tempfile::tempdir().unwrap();
        let off = build_bwrap_args("/bin/sh", &[], dir.path(), dir.path(), None, false);
        assert!(off.iter().any(|a| a == "--unshare-net"));
        let on = build_bwrap_args("/bin/sh", &[], dir.path(), dir.path(), None, true);
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
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(worktree.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(worktree.join(".git/info")).unwrap();
        std::fs::write(worktree.join(".git/config"), "[core]\n").unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let run = |cwd: &Path, cache_opt: Option<&Path>, script: String| {
            let (prog, args, active) = wrap(
                "/bin/sh",
                vec!["-c".into(), script],
                cwd,
                &worktree,
                cache_opt,
                ON,
            )
            .unwrap();
            assert!(active);
            std::process::Command::new(&prog)
                .args(&args)
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .code()
        };

        // Writing inside the root succeeds.
        assert_eq!(
            run(&worktree, Some(&cache), "echo ok > inside.txt".into()),
            Some(0)
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("inside.txt")).unwrap(),
            "ok\n"
        );
        // Per-session cache persists across commands.
        assert_eq!(
            run(
                &worktree,
                Some(&cache),
                "echo cached > \"$npm_config_cache/item\"".into()
            ),
            Some(0)
        );
        assert_eq!(
            run(
                &worktree,
                Some(&cache),
                "test \"$(cat \"$npm_config_cache/item\")\" = cached".into()
            ),
            Some(0)
        );
        // PID namespace isolation: host process (our own PID) is not visible in /proc.
        let host_pid = std::process::id();
        assert_ne!(
            run(&worktree, None, format!("test -d /proc/{host_pid}")),
            Some(0),
            "host PID {host_pid} should not be visible inside --unshare-pid sandbox"
        );
        // Main-folder .git cannot be written, replaced, or renamed (`mv .git .git.old` -> EBUSY).
        assert_ne!(
            run(&worktree, None, "echo bad >> .git/config".into()),
            Some(0)
        );
        assert_ne!(run(&worktree, None, "mv .git .git.old".into()), Some(0));
        assert_ne!(
            run(
                &worktree,
                None,
                "echo '#!/bin/sh' > .git/hooks/pre-commit".into()
            ),
            Some(0)
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join(".git/config")).unwrap(),
            "[core]\n"
        );
        assert!(!worktree.join(".git/hooks/pre-commit").exists());
        assert!(!worktree.join(".git.old").exists());
        // Worktree `.git` gitfile cannot be overwritten or removed inside the sandbox.
        let wt_file = dir.path().join("wt-gitfile");
        std::fs::create_dir_all(&wt_file).unwrap();
        std::fs::write(wt_file.join(".git"), "gitdir: /nonexistent\n").unwrap();
        let (prog, args, active) = wrap(
            "/bin/sh",
            vec!["-c".into(), "rm .git || echo bad > .git".into()],
            &wt_file,
            &wt_file,
            None,
            ON,
        )
        .unwrap();
        assert!(active);
        let st = std::process::Command::new(&prog)
            .args(&args)
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!st.success());
        assert_eq!(
            std::fs::read_to_string(wt_file.join(".git")).unwrap(),
            "gitdir: /nonexistent\n"
        );
        // Writable root `/` is always refused.
        assert!(
            wrap(
                "/bin/sh",
                vec!["-c".into(), "true".into()],
                Path::new("/"),
                Path::new("/"),
                None,
                ON,
            )
            .is_err()
        );

        // The folder outside is there (seen) but read-only: exit 3 would mean it was hidden.
        let target = outside.join("escaped.txt");
        let code = run(
            &worktree,
            None,
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
            None,
            "pwd >/dev/null || exit 3; echo bad > here.txt".into(),
        );
        assert!(!matches!(code, Some(0) | Some(3)), "{code:?}");
        assert!(!outside.join("here.txt").exists());
    }
}
