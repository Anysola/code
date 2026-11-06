//! The interactive UI under a real pseudo-terminal (`script`), against the fake gateway: home
//! draws, a prompt runs a shell tool behind a permission prompt, the answer streams in,
//! `/usage` works, a full 5-hour window offers a banked reset that sends the prompt again,
//! `/budget 5` and `/undo` work, `exit` leaves with the exit summary.
#![cfg(target_os = "linux")]

mod common;

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Reply};
use common::*;
use serde_json::json;

/// Output with escape sequences removed, so drawn text can be searched.
fn plain(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                    out.push(' ');
                }
                Some(']') => {
                    while let Some(n) = chars.next() {
                        if n == '\x07' || (n == '\x1b' && chars.peek() == Some(&'\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn prompt_permission_answer_exit() {
    if Command::new("script").arg("--version").output().is_err() {
        return;
    }
    let limited = std::sync::atomic::AtomicBool::new(false);
    let gateway = FakeGateway::start(move |request, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        // The first "limit check" prompt finds the 5-hour window full.
        if !request.is_title()
            && request.body.contains("limit check")
            && !limited.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return anysola_testkit::limit_reply(2);
        }
        let body = if request.is_title() {
            Stream::new().text("Smoke title").finish(&model, Some(0))
        } else if request.body.contains("limit check") {
            Stream::new()
                .text("Window cleared.")
                .finish(&model, Some(1))
        } else if request.body.contains("tool_result") {
            Stream::new().text("All done here.").finish(&model, Some(2))
        } else {
            Stream::new()
                .tool(
                    "toolu_1",
                    "bash",
                    json!({"command": "git push", "description": "Push"}),
                )
                .stop("tool_use")
                .finish(&model, Some(3))
        };
        Reply::Sse { body }
    });
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = my_app(&home);
    std::fs::write(
        dir.join("opencode.json"),
        r#"{"permission":{"bash":"ask"}}"#,
    )
    .unwrap();
    let mut child = Command::new("script")
        .args([
            "-qfec",
            &format!("stty cols 120 rows 36; exec {BIN}"),
            "/dev/null",
        ])
        .current_dir(&dir)
        .envs(home.env().iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut seen = Vec::new();
    let mut wait_for = |needle: &str| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if plain(&seen).contains(needle) {
                return true;
            }
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) {
                seen.extend(chunk);
            }
        }
        panic!("never saw {needle:?}:\n{}", plain(&seen));
    };
    wait_for("Ask anything");
    wait_for("5h 42%");
    stdin.write_all(b"run the smoke check").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    stdin.write_all(b"\r").unwrap();
    wait_for("Permission required");
    wait_for("$ git push");
    stdin.write_all(b"\r").unwrap();
    wait_for("All done here.");
    let mut command = |text: &str| {
        std::thread::sleep(Duration::from_millis(300));
        stdin.write_all(text.as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        stdin.write_all(b"\r").unwrap();
    };
    command("/usage");
    wait_for("Usage · last 7 days");
    wait_for("Banked resets  2");
    command("\x1b");
    command("limit check");
    wait_for("Use a banked reset?");
    wait_for("Wait");
    command("");
    wait_for("Window cleared.");
    assert_eq!(gateway.bank(), 1);
    command("/budget 5");
    wait_for("Session budget");
    command("/undo");
    wait_for("limit check");
    std::thread::sleep(Duration::from_millis(300));
    stdin.write_all(b"\x03").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    stdin.write_all(b"exit").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    stdin.write_all(b"\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the UI did not exit");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success());
    wait_for("anysola -s ses_");
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 4);
    assert!(main[1].body.contains("tool_result"));
    // The refused prompt went again as it was, not as a second copy.
    let retried = main[3].json();
    let users = retried["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user" && m.to_string().contains("limit check"))
        .count();
    assert_eq!(users, 1, "{retried}");
}
