//! The command tree as users meet it: every `--help` text against the 0.5.1 capture, `--version` in
//! any position, and effect/cli's parse-error format.

mod common;

use std::process::Command;

use common::*;

/// The 0.5.1 help capture minus what 1.0 removed: the `$ …` line, the exit marker, `--wizard`
/// and the `service`/`serve` commands.
fn expected_help(file: &str) -> String {
    let text = std::fs::read_to_string(spec(&format!("cli/help/{file}"))).unwrap();
    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        let removed = (index == 0 && line.starts_with("$ "))
            || line == "[exit=0]"
            || line.starts_with("  --wizard ")
            || line.starts_with("  service  ")
            || line.starts_with("  serve  ");
        if !removed {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn run(args: &[&str]) -> (String, String, i32) {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(BIN)
        .args(args)
        .current_dir(home.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn every_kept_command_prints_its_captured_help() {
    let cases: &[(&[&str], &str)] = &[
        (&["--help"], "root_.txt"),
        (&["-h"], "root_.txt"),
        (&["login", "--help"], "root_login.txt"),
        (&["logout", "--help"], "root_logout.txt"),
        (&["status", "--help"], "root_status.txt"),
        (&["usage", "--help"], "root_usage.txt"),
        (&["limits", "--help"], "root_limits.txt"),
        (&["limits", "redeem", "--help"], "root_limits_redeem.txt"),
        (&["devices", "--help"], "root_devices.txt"),
        (&["devices", "revoke", "--help"], "root_devices_revoke.txt"),
        (&["update", "--help"], "root_update.txt"),
        (&["upgrade", "--help"], "root_upgrade.txt"),
        (&["uninstall", "--help"], "root_uninstall.txt"),
        (&["acp", "--help"], "root_acp.txt"),
        (&["debug", "--help"], "root_debug.txt"),
        (&["debug", "config", "--help"], "root_debug_config.txt"),
        (&["debug", "paths", "-h"], "root_debug_paths.txt"),
        (&["mcp", "--help"], "root_mcp.txt"),
        (&["mcp", "list", "--help"], "root_mcp_list.txt"),
        (&["mcp", "add", "--help"], "root_mcp_add.txt"),
        (&["mcp", "auth", "--help"], "root_mcp_auth.txt"),
        (&["mcp", "logout", "--help"], "root_mcp_logout.txt"),
        (&["plugin", "--help"], "root_plugin.txt"),
        (&["models", "--help"], "root_models.txt"),
        (&["run", "--help"], "root_run.txt"),
        (&["session", "--help"], "root_session.txt"),
        (&["session", "list", "--help"], "root_session_list.txt"),
        (&["session", "delete", "--help"], "root_session_delete.txt"),
        (&["session", "export", "--help"], "root_session_export.txt"),
        (&["session", "import", "--help"], "root_session_import.txt"),
        (&["reload", "--help"], "root_reload.txt"),
        (
            &["--log-level", "debug", "status", "--help"],
            "root_status.txt",
        ),
    ];
    for (args, file) in cases {
        let (stdout, stderr, code) = run(args);
        assert_eq!(code, 0, "{args:?}: {stderr}");
        assert_eq!(stdout, expected_help(file), "{args:?}");
    }
}

#[test]
fn bare_groups_print_their_help() {
    for group in ["session", "debug", "mcp", "plugin"] {
        let (stdout, _, code) = run(&[group]);
        assert_eq!(code, 0);
        assert_eq!(stdout, expected_help(&format!("root_{group}.txt")));
    }
}

#[test]
fn version_in_every_position_without_network() {
    for args in [
        &["--version"][..],
        &["-v"],
        &["run", "--version"],
        &["-v", "run"],
        &["--version", "extra"],
        &["status", "-v"],
        &["session", "list", "--version"],
        &["update", "--help", "-v"],
    ] {
        let out = Command::new(BIN)
            .args(args)
            .env("ANYSOLA_API_URL", "http://127.0.0.1:9")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!(
                "Anysola Code {} (OpenCode 2.2.5)\n",
                env!("CARGO_PKG_VERSION")
            ),
            "{args:?}"
        );
        assert!(out.stderr.is_empty() && out.status.success(), "{args:?}");
    }
}

#[test]
fn parse_errors_print_help_then_the_message() {
    let cases: &[(&[&str], &str, &str)] = &[
        (
            &["--bogus"],
            "root_.txt",
            "Unrecognized flag: --bogus in command anysola",
        ),
        (
            &["session", "list", "--bogus"],
            "root_session_list.txt",
            "Unrecognized flag: --bogus in command anysola session list",
        ),
        (
            &["debug", "paths", "nope"],
            "root_debug_paths.txt",
            "Invalid value for argument <name>: \"nope\". Expected: \"db\" | \"home\" | \"data\" | \"config\" | \"cache\" | \"state\" | \"tmp\" | \"bin\" | \"log\" | \"repos\"",
        ),
        (
            &["session", "delete"],
            "root_session_delete.txt",
            "Missing required argument: sessionID",
        ),
        (
            &["devices", "revoke"],
            "root_devices_revoke.txt",
            "Missing required argument: id",
        ),
        (
            &["usage", "--days", "9"],
            "root_usage.txt",
            "Invalid value for flag --days: \"9\". Expected: \"7\" | \"30\"",
        ),
        (
            &["usage", "--days"],
            "root_usage.txt",
            "Missing value for flag --days. Expected: 7 | 30",
        ),
        (
            &["--completions"],
            "root_.txt",
            "Missing value for flag --completions. Expected: bash | zsh | fish | powershell | sh | pwsh",
        ),
        (
            &["status", "extra"],
            "root_status.txt",
            "Unexpected positional argument: \"extra\"",
        ),
        (
            &["login", "-f", "extra"],
            "root_login.txt",
            "Unexpected positional argument: \"extra\"",
        ),
        (
            &["session", "bogus"],
            "root_session.txt",
            "Unknown subcommand \"bogus\" for \"anysola session\"",
        ),
        (
            &["session", "list", "-n", "x"],
            "root_session_list.txt",
            "Invalid value for flag --max-count: \"x\". Expected a string representing a finite number",
        ),
        (
            &["mcp", "add"],
            "root_mcp_add.txt",
            "Missing required argument: name",
        ),
    ];
    for (args, help, message) in cases {
        let (stdout, stderr, code) = run(args);
        assert_eq!(code, 1, "{args:?}");
        assert_eq!(stdout, expected_help(help), "{args:?}");
        assert_eq!(stderr, format!("\nERROR\n  {message}\n"), "{args:?}");
    }
}

#[test]
fn reload_and_plugin() {
    let (stdout, _, code) = run(&["reload"]);
    assert_eq!((stdout.as_str(), code), ("Configuration reloaded\n", 0));
    for args in [&["plugin", "list"][..], &["plugin", "add", "x", "--global"]] {
        let (stdout, stderr, code) = run(args);
        assert_eq!(code, 1);
        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            "Error: Plugins are not supported since Anysola Code 1.0; built-in features are native.\n"
        );
    }
}
