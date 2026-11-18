//! `anysola hooks` lists hooks and approves a project's; `anysola run` skips project hooks nobody
//! approved, with one warning line.
#![cfg(unix)]

mod common;

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Reply, TestHome};
use common::*;
use serde_json::json;

fn shell_once() -> FakeGateway {
    FakeGateway::start(|request, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let answered = request.body.contains("tool_result");
        let body = if request.is_title() || answered {
            Stream::new().text("done")
        } else {
            Stream::new().tool("toolu_1", "shell", json!({"command": "true"}))
        };
        Reply::Sse {
            body: body.finish(&model, Some(1)),
        }
    })
}

#[test]
fn project_hooks_wait_for_approval() {
    let gateway = shell_once();
    let home = TestHome::new(&gateway.url);
    let marker = home.home().join("project-hook-ran");
    let hooks = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [
        {"type": "command", "command": format!("touch '{}'", marker.display())}
    ]}]}});
    let file = home.write("proj/anysola.json", &hooks.to_string());
    home.write(
        ".anysola/config.json",
        &json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "true"}]}]}})
            .to_string(),
    );

    let listed = home.run(BIN, &["hooks"], None, None);
    assert_eq!(listed.code, 0, "{}", listed.stderr);
    let user = home.home().join(".anysola/config.json");
    assert_eq!(
        listed.stdout,
        format!(
            "{} (user) · runs\n  Stop: true\n\n{} (project) · not approved\n  PreToolUse (Bash): touch '{}'\n\nThis project's hooks run only once approved. Run `anysola hooks trust` to review them.\n",
            user.display(),
            file.display(),
            marker.display()
        )
    );

    let out = home.run(BIN, &["run", "go"], None, None);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stderr
            .lines()
            .filter(|l| l.starts_with("Skipped "))
            .collect::<Vec<_>>(),
        ["Skipped 1 project hook that is not approved. Run `anysola hooks trust` to review them."]
    );
    assert!(!marker.exists(), "the unapproved hook did not run");

    let refused = home.run(BIN, &["hooks", "trust"], None, None);
    assert_eq!(refused.code, 1, "no terminal, no --yes");
    assert!(!marker.exists());

    let trusted = home.run(BIN, &["hooks", "trust", "--yes"], None, None);
    assert_eq!(trusted.code, 0, "{}", trusted.stderr);
    assert!(
        trusted
            .stdout
            .contains(&format!("touch '{}'", marker.display()))
    );
    assert!(
        trusted
            .stdout
            .ends_with(&format!("Approved the hooks of {}\n", file.display()))
    );

    let out = home.run(BIN, &["run", "again"], None, None);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(!out.stderr.contains("Skipped "), "{}", out.stderr);
    assert!(marker.exists(), "the approved hook ran");

    let denied = home.run(BIN, &["hooks", "deny"], None, None);
    assert_eq!(
        denied.stdout,
        format!("Refused the hooks of {}\n", file.display())
    );
    let listed = home.run(BIN, &["hooks"], None, None);
    assert!(
        listed.stdout.contains("(project) · refused"),
        "{}",
        listed.stdout
    );
}

#[test]
fn no_hooks() {
    let home = TestHome::new("http://127.0.0.1:9");
    let out = home.run(BIN, &["hooks"], None, None);
    assert_eq!(
        (out.stdout.as_str(), out.code),
        ("No hooks configured.\n", 0)
    );
    let out = home.run(BIN, &["hooks", "trust", "--yes"], None, None);
    assert_eq!(out.stdout, "No project hooks wait for approval.\n");
    let help = home.run(BIN, &["hooks", "--help"], None, None);
    assert_eq!(help.stdout, include_str!("../src/help/hooks.txt"));
}
