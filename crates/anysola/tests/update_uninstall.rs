//! `anysola update` and `anysola uninstall` against testkit, with stand-in `npm` and `sh` on PATH
//! so nothing is downloaded or installed. Expected texts follow `V/cli/update-uninstall.txt`, with
//! 1.0's cache dir, no service lines and a manual-removal command limited to the bin directory.

// The stand-ins are bash scripts; the Windows install flow is covered by install-test.yml.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anysola_testkit::{FakeGateway, Reply, TestHome};
use common::*;
use serde_json::json;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn gateway() -> FakeGateway {
    FakeGateway::start(|_, _| Reply::Json {
        status: 500,
        body: json!({}),
        headers: Vec::new(),
    })
}

fn executable(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A home whose PATH starts with stand-ins for `npm` and `sh` that log their arguments.
fn home(gateway: &FakeGateway) -> TestHome {
    let mut home = TestHome::new(&gateway.url);
    let fake = home.home().join("fakebin");
    let log = home.home().join("calls.log");
    let script = |name: &str| {
        format!(
            "#!/bin/bash\necho \"{name} $*\" >> '{}'\n[ \"$1 $2\" = \"list -g\" ] && echo '/x/lib\n└── @anysola/code@1.0.0'\nexit ${{FAKE_EXIT:-0}}\n",
            log.display()
        )
    };
    executable(&fake.join("npm"), &script("npm"));
    executable(&fake.join("sh"), &script("sh"));
    let path = format!("{}:{}", fake.display(), std::env::var("PATH").unwrap());
    home.set_env("PATH", &path);
    home.set_env("SHELL", "/bin/bash");
    home
}

fn calls(home: &TestHome) -> String {
    std::fs::read_to_string(home.home().join("calls.log")).unwrap_or_default()
}

/// The binary copied to where the install script puts it.
fn script_install(dir: &Path) -> PathBuf {
    let target = dir.join("anysola");
    place(&target);
    target
}

/// The binary at `target`. A hard link where possible: a fresh copy can be "text file busy" while
/// a parallel test forks.
fn place(target: &Path) {
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    if std::fs::hard_link(BIN, target).is_err() {
        std::fs::copy(BIN, target).unwrap();
    }
}

fn script_dir(home: &TestHome) -> PathBuf {
    home.home().join(".anysola/bin")
}

fn frame(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

const SCRIPT_COMMAND: &str =
    r#"sh -c "curl -fsSL https://www.anysola.com/install | sh -s -- --no-modify-path""#;

#[test]
fn update_print_with_notes_for_npm_and_script_installs() {
    let gateway = gateway();
    gateway.set_version(json!({"latest": "9.0.0", "minimum": "0.2.0", "notes": "Faster startup and a new /review command."}));
    let home = home(&gateway);
    let expected = |command: &str| {
        frame(&[
            "┌  Update Anysola Code",
            "│",
            &format!("●  Installed {VERSION}, latest 9.0.0"),
            "│",
            "│  Faster startup and a new /review command.",
            "│",
            &format!("│  {command}"),
            "│",
            "└  Run the command above to update",
            "",
        ])
    };
    let out = home.run(BIN, &["update", "--print"], None, None);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, expected("npm i -g @anysola/code@latest"));

    let copy = script_install(&script_dir(&home));
    let out = home.run(copy.to_str().unwrap(), &["upgrade", "--print"], None, None);
    assert_eq!(out.stdout, expected(SCRIPT_COMMAND));
    assert_eq!(calls(&home), "", "--print runs nothing");
}

#[test]
fn install_dir_override_selects_the_script() {
    let gateway = gateway();
    gateway.set_version(json!({"latest": "9.0.0"}));
    let mut home = home(&gateway);
    let dir = home.home().join("tools/anysola");
    home.set_env("ANYSOLA_INSTALL_DIR", dir.to_str().unwrap());
    let copy = script_install(&dir);
    let out = home.run(copy.to_str().unwrap(), &["update", "--print"], None, None);
    assert!(out.stdout.contains(SCRIPT_COMMAND), "{}", out.stdout);
    // The default location is not the script install any more.
    let other = script_install(&script_dir(&home));
    let out = home.run(other.to_str().unwrap(), &["update", "--print"], None, None);
    assert!(out.stdout.contains("npm i -g @anysola/code@latest"));
}

#[test]
fn already_up_to_date_unless_forced() {
    let gateway = gateway();
    let home = home(&gateway);
    let out = home.run(BIN, &["update"], None, None);
    assert_eq!(
        out.stdout,
        frame(&[
            "┌  Update Anysola Code",
            "│",
            &format!("●  Installed {VERSION}, latest 0.5.1"),
            "│",
            "└  Already up to date",
            "",
        ])
    );
    assert_eq!(calls(&home), "");

    let out = home.run(BIN, &["update", "--force"], None, None);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        frame(&[
            "┌  Update Anysola Code",
            "│",
            &format!("●  Installed {VERSION}, latest 0.5.1"),
            "│",
            "◇  npm i -g @anysola/code@latest",
            "│",
            "└  Updated. Restart Anysola Code to use the new version.",
            "",
        ])
    );
    assert_eq!(calls(&home), "npm i -g @anysola/code@latest\n");
}

#[test]
fn the_script_install_reruns_the_installer() {
    let gateway = gateway();
    gateway.set_version(json!({"latest": "9.0.0"}));
    let home = home(&gateway);
    let copy = script_install(&script_dir(&home));
    let out = home.run(copy.to_str().unwrap(), &["update"], None, None);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert!(out.stdout.contains(&format!("◇  {SCRIPT_COMMAND}\n")));
    assert_eq!(
        calls(&home),
        "sh -c curl -fsSL https://www.anysola.com/install | sh -s -- --no-modify-path\n"
    );
}

#[test]
fn a_failed_install_and_an_unreachable_gateway() {
    let gateway = gateway();
    let mut home = home(&gateway);
    home.set_env("ANYSOLA_API_URL", "http://127.0.0.1:9");
    home.set_env("FAKE_EXIT", "3");
    let out = home.run(BIN, &["update"], None, None);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stdout,
        frame(&[
            "┌  Update Anysola Code",
            "│",
            "◇  npm i -g @anysola/code@latest",
            "│",
            "■  npm failed with exit code 3",
            "│",
            "●  Run it yourself: npm i -g @anysola/code@latest",
        ])
    );
    assert!(gateway.requests().is_empty());
    assert_eq!(calls(&home), "npm i -g @anysola/code@latest\n");
}

#[test]
fn uninstall_dry_run_unknown_method() {
    let gateway = gateway();
    let home = home(&gateway);
    let cache = home.home().join(".anysola/cache");
    std::fs::create_dir_all(&cache).unwrap();
    let out = home.run(BIN, &["uninstall", "--dry-run"], None, None);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        frame(&[
            "┌  Uninstall Anysola Code",
            "│",
            "●  Installation method: unknown",
            "│",
            "│  The following global files will be removed:",
            "│",
            &format!("●    Cache: {}", cache.display()),
            "│",
            "▲  Could not detect the installation method. Remove the installation manually after cleanup.",
            "│",
            "▲  Dry run - no changes made",
            "│",
            "└  Done",
            "",
        ])
    );
    assert!(cache.exists());
}

fn bashrc(home: &TestHome) -> PathBuf {
    home.write(
        ".bashrc",
        "alias ll='ls -l'\n\n# anysola\nexport PATH=\"$HOME/.anysola/bin:$PATH\"\n",
    )
}

#[test]
fn uninstall_dry_run_script_install() {
    let gateway = gateway();
    let home = home(&gateway);
    let rc = bashrc(&home);
    let copy = script_install(&script_dir(&home));
    let out = home.run(
        copy.to_str().unwrap(),
        &["uninstall", "--dry-run"],
        None,
        None,
    );
    assert_eq!(
        out.stdout,
        frame(&[
            "┌  Uninstall Anysola Code",
            "│",
            "●  Installation method: curl",
            "│",
            "│  The following global files will be removed:",
            "│",
            &format!("●    Shell PATH: {}", rc.display()),
            "│",
            &format!("●    Binary (manual removal): {}", copy.display()),
            "│",
            "▲  Dry run - no changes made",
            "│",
            "└  Done",
            "",
        ])
    );
}

#[test]
fn uninstall_without_a_terminal_needs_force() {
    let gateway = gateway();
    let home = home(&gateway);
    let out = home.run(BIN, &["uninstall"], None, None);
    assert_eq!(out.code, 1);
    assert!(out.stdout.ends_with(&frame(&[
        "│",
        "■  Use --force to uninstall without an interactive terminal, or --dry-run to preview.",
        "│",
        "└  Failed",
        "",
    ])));
}

#[test]
fn forced_uninstall_of_a_script_install_keeps_data() {
    let gateway = gateway();
    let home = home(&gateway);
    let rc = bashrc(&home);
    let cache = home.home().join(".anysola/cache");
    std::fs::create_dir_all(cache.join("skills")).unwrap();
    let config = home.write(".anysola/config.json", "{}");
    let copy = script_install(&script_dir(&home));
    let out = home.run(
        copy.to_str().unwrap(),
        &["uninstall", "--force"],
        None,
        None,
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    let bin = script_dir(&home);
    assert!(out.stdout.ends_with(&frame(&[
        &format!("●    Binary (manual removal): {}", copy.display()),
        "│",
        "◇  Removed Cache",
        "│",
        "│  To finish removing the binary, run:",
        "│",
        &format!("●    rm -rf '{}'", bin.display()),
        "│",
        "└  Done",
        "",
    ])));
    assert!(!cache.exists());
    assert!(config.exists() && copy.exists());
    assert_eq!(
        std::fs::read_to_string(&rc).unwrap(),
        "alias ll='ls -l'\n\n"
    );
    assert_eq!(calls(&home), "");
}

#[test]
fn npm_installs_are_detected_and_removed() {
    let gateway = gateway();
    let home = home(&gateway);
    let package = home.home().join("npm/lib/node_modules/@anysola/code");
    home.write(
        "npm/lib/node_modules/@anysola/code/package.json",
        r#"{"name": "@anysola/code", "bin": {"anysola": "bin/anysola.exe"}}"#,
    );
    let exe = package.join("bin/anysola.exe");
    place(&exe);
    let out = home.run(exe.to_str().unwrap(), &["uninstall", "-f"], None, None);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(
        out.stdout,
        frame(&[
            "┌  Uninstall Anysola Code",
            "│",
            "●  Installation method: npm",
            "│",
            "│  The following global files will be removed:",
            "│",
            "●    Package: npm uninstall --global @anysola/code",
            "│",
            "◇  Removed Cache",
            "│",
            "◇  Package removed",
            "│",
            "└  Done",
            "",
        ])
    );
    assert!(calls(&home).contains("npm uninstall --global @anysola/code\n"));
}

#[test]
fn a_failed_package_removal() {
    let gateway = gateway();
    let mut home = home(&gateway);
    home.write(
        "npm/lib/node_modules/@anysola/code/package.json",
        r#"{"name": "@anysola/code", "bin": {"anysola": "bin/anysola.exe"}}"#,
    );
    let exe = home
        .home()
        .join("npm/lib/node_modules/@anysola/code/bin/anysola.exe");
    place(&exe);
    home.set_env("FAKE_EXIT", "2");
    let out = home.run(exe.to_str().unwrap(), &["uninstall", "--force"], None, None);
    assert_eq!(out.code, 1);
    assert!(
        out.stdout.ends_with(&frame(&[
            "◇  Removed Cache",
            "│",
            "■  Package manager uninstall failed",
            "│",
            "▲  Run manually: npm uninstall --global @anysola/code",
            "│",
            "■  The command exited with code 2 without any error output.",
            "│",
            "└  Failed",
            "",
        ])),
        "{}",
        out.stdout
    );
}
