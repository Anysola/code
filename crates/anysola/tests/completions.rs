//! `--completions <shell>` scripts load in their shell; shells that are not installed are skipped.

mod common;

use std::process::Command;

use common::*;

fn script(shell: &str) -> String {
    let out = Command::new(BIN)
        .args(["--completions", shell])
        .output()
        .unwrap();
    assert!(out.status.success(), "{shell}");
    String::from_utf8(out.stdout).unwrap()
}

fn installed(shell: &str) -> bool {
    Command::new(shell)
        .arg("-c")
        .arg("exit 0")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Loads `script` into `shell` with `check` run afterwards; the check's output must be `expected`.
fn loads(shell: &str, flags: &[&str], script: &str, check: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("completion");
    std::fs::write(&file, script).unwrap();
    let out = Command::new(shell)
        .args(flags)
        .arg(format!(". '{}'; {check}", file.display()))
        .output()
        .unwrap();
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "{shell}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn bash_and_sh() {
    assert_eq!(script("sh"), script("bash"));
    if installed("bash") {
        let out = loads("bash", &["-c"], &script("bash"), "complete -p anysola");
        assert!(out.contains("anysola"), "{out}");
    }
}

#[test]
fn zsh() {
    if installed("zsh") {
        loads(
            "zsh",
            &["-c"],
            &script("zsh"),
            "autoload -U compinit; compinit -u; compdef _anysola anysola",
        );
    }
}

#[test]
fn fish() {
    if installed("fish") {
        let out = loads(
            "fish",
            &["-c"],
            &script("fish"),
            "complete -C 'anysola ses'",
        );
        assert!(out.contains("session"), "{out}");
    }
}

#[test]
fn powershell() {
    assert_eq!(script("pwsh"), script("powershell"));
    if installed("pwsh") {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("completion.ps1");
        std::fs::write(&file, script("pwsh")).unwrap();
        let out = Command::new("pwsh")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(". '{}'", file.display()))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn every_subcommand_is_offered() {
    let bash = script("bash");
    for command in [
        "login",
        "logout",
        "status",
        "usage",
        "devices",
        "update",
        "uninstall",
        "models",
        "run",
        "session",
        "debug",
        "mcp",
        "reload",
    ] {
        assert!(bash.contains(command), "{command}");
    }
}
