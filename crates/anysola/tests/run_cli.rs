//! `anysola run` command line: arguments, stdin, sessions, errors, a multi-tool turn, retries,
//! the busy marker and the code-action invocation.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply, TestHome};
use common::*;
use serde_json::{Value, json};

fn answer(text: &str) -> impl Fn(&Recorded, usize) -> Reply + Send + Sync + use<> {
    let text = text.to_string();
    move |request, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Reply::Sse {
            body: Stream::new().text(&text).finish(&model, Some(1)),
            before: Some(100),
        }
    }
}

fn user_text(request: &Recorded) -> String {
    let body = request.json();
    let user = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .unwrap()
        .clone();
    user["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn main_requests(gateway: &FakeGateway) -> Vec<Recorded> {
    gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect()
}

#[test]
fn version_in_any_position() {
    for args in [
        vec!["--version"],
        vec!["-v"],
        vec!["run", "--version"],
        vec!["-v", "run"],
    ] {
        let out = Command::new(BIN).args(&args).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.starts_with("Anysola Code ") && text.ends_with(" (OpenCode 2.2.5)\n"),
            "{args:?}: {text}"
        );
        assert!(out.status.success());
    }
}

#[test]
fn help_and_parse_errors() {
    let out = Command::new(BIN).args(["run", "--help"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        include_str!("../src/help/run.txt")
    );
    let out = Command::new(BIN).args(["run", "--bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "\nERROR\n  Unrecognized flag: --bogus in command anysola run\n"
    );
    let out = Command::new(BIN)
        .args(["run", "--budget", "0", "x"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "\nERROR\n  Invalid value for flag --budget: \"0\". Expected: Schema validation failed: Expected a value greater than or equal to 1\n"
    );
}

#[test]
fn message_arguments_are_joined_unquoted() {
    let gateway = FakeGateway::start(answer("ok"));
    let home = TestHome::new(&gateway.url);
    let out = home.run(
        BIN,
        &["run", "explain this", "x", "--", "b c", "d"],
        None,
        None,
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        user_text(&main_requests(&gateway)[0]),
        "explain this x b c d"
    );
}

#[test]
fn stdin_rules() {
    let gateway = FakeGateway::start(answer("ok"));
    let home = TestHome::new(&gateway.url);
    let out = home.run(BIN, &["run"], None, Some("from stdin"));
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(user_text(&main_requests(&gateway)[0]), "from stdin");
    let out = home.run(BIN, &["run", "message"], None, Some("piped"));
    assert_eq!(out.code, 0);
    assert_eq!(user_text(&main_requests(&gateway)[1]), "message\npiped");
    let out = home.run(BIN, &["run"], None, None);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "\x1b[91m\x1b[1mError: \x1b[0mYou must provide a message\n"
    );
}

#[test]
fn idle_pipe_with_a_message_is_ignored_after_three_seconds() {
    let gateway = FakeGateway::start(answer("ok"));
    let home = TestHome::new(&gateway.url);
    let mut command = Command::new(BIN);
    command
        .args(["run", "hi"])
        .current_dir(home.project())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap());
    command
        .env("HOME", home.home())
        .env("ANYSOLA_API_URL", &gateway.url)
        .env("ANYSOLA_TOKEN", "asc_x");
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pipe = child.stdin.take().unwrap();
    let started = Instant::now();
    let out = child.wait_with_output().unwrap();
    drop(pipe);
    assert!(started.elapsed() >= Duration::from_secs(3));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .starts_with("No input on stdin after 3s; continuing without it.\n")
    );
    assert_eq!(user_text(&main_requests(&gateway)[0]), "hi");
}

#[test]
fn slow_pipe_without_a_message_is_awaited() {
    let gateway = FakeGateway::start(answer("ok"));
    let home = TestHome::new(&gateway.url);
    let mut command = Command::new(BIN);
    command
        .args(["run"])
        .current_dir(home.project())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap());
    command
        .env("HOME", home.home())
        .env("ANYSOLA_API_URL", &gateway.url)
        .env("ANYSOLA_TOKEN", "asc_x");
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    std::thread::sleep(Duration::from_secs(4));
    pipe.write_all(b"late input").unwrap();
    drop(pipe);
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(user_text(&main_requests(&gateway)[0]), "late input");
}

#[test]
fn errors_before_and_after_the_session() {
    let gateway = FakeGateway::start(answer("ok"));
    let home = TestHome::new(&gateway.url);
    let cases: [(&[&str], &str); 7] = [
        (
            &["run", "--fork", "x"],
            "--fork requires --continue or --session",
        ),
        (&["run", "-m", "nope", "x"], "Invalid model reference: nope"),
        (
            &["run", "-m", "anysola/nope", "x"],
            "Model unavailable: anysola/nope",
        ),
        (
            &["run", "-m", "anysola/claude-sonnet-5-5#bogus", "x"],
            "Variant unavailable for anysola/claude-sonnet-5-5: bogus",
        ),
        (
            &["run", "--agent", "nope", "x"],
            "Agent not found: \"nope\"",
        ),
        (
            &["run", "-s", "ses_unknown", "--fork", "x"],
            "Session not found",
        ),
        (
            &["run", "-f", "missing.txt", "x"],
            "File not found: missing.txt",
        ),
    ];
    for (args, message) in cases {
        let mut json_args = args.to_vec();
        json_args.insert(1, "--format=json");
        let out = home.run(BIN, &json_args, None, None);
        assert_eq!(out.code, 1, "{args:?}");
        let line: Value = serde_json::from_str(out.stdout.trim()).unwrap();
        assert_eq!(
            line["error"],
            json!({"type": "unknown", "message": message}),
            "{args:?}"
        );
        let out = home.run(BIN, args, None, None);
        assert_eq!(
            out.stderr,
            format!("\x1b[91m\x1b[1mError: \x1b[0m{message}\n"),
            "{args:?}"
        );
    }
}

#[test]
fn sessions_continue_resume_and_fork() {
    let gateway = FakeGateway::start(answer("ok"));
    let home = TestHome::new(&gateway.url);
    assert_eq!(
        home.run(BIN, &["run", "--title", "", "first message"], None, None)
            .code,
        0
    );
    assert_eq!(home.run(BIN, &["run", "-c", "second"], None, None).code, 0);
    assert_eq!(
        home.run(BIN, &["run", "-s", "ses_given", "third"], None, None)
            .code,
        0
    );
    let out = home.run(
        BIN,
        &["run", "-s", "ses_given", "--fork", "fourth"],
        None,
        None,
    );
    assert_eq!(out.code, 0);
    let main = main_requests(&gateway);
    let session = |r: &Recorded| r.header("x-opencode-session-id").unwrap().to_string();
    assert_eq!(
        session(&main[0]),
        session(&main[1]),
        "-c continues the latest session"
    );
    assert_eq!(main[1].json()["messages"].as_array().unwrap().len(), 3);
    assert_eq!(session(&main[2]), "ses_given");
    assert_ne!(session(&main[3]), "ses_given");
    assert_eq!(
        main[3].header("x-opencode-session"),
        Some("ses_given"),
        "a fork shares its source's cache affinity"
    );
    assert_eq!(
        main[3].json()["messages"].as_array().unwrap().len(),
        3,
        "the fork carries the history"
    );
    assert_eq!(title(&home, &session(&main[0])), "first message");
    assert_eq!(
        title(&home, &session(&main[3])),
        format!("{} (fork #1)", title(&home, "ses_given"))
    );
}

fn title(home: &TestHome, id: &str) -> String {
    let path = home.home().join(".anysola/anysola.db");
    anysola_core::storage::Db::open(&path)
        .unwrap()
        .session(id)
        .unwrap()
        .unwrap()
        .title
}

#[test]
fn multi_tool_turn() {
    let gateway = FakeGateway::start(|request: &Recorded, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if request.is_title() {
            return Reply::Sse {
                body: Stream::new().text("Title").finish(&model, Some(0)),
                before: None,
            };
        }
        let stream = match request.json()["messages"].as_array().unwrap().len() {
            1 => Stream::new().text("Working.").tool(
                "t1",
                "shell",
                json!({"command": "printf 'one\\ntwo\\n' > notes.txt"}),
            ),
            3 => Stream::new().tool(
                "t2",
                "edit",
                json!({"path": "notes.txt", "oldString": "two", "newString": "three"}),
            ),
            5 => Stream::new().tool("t3", "read", json!({"path": "notes.txt"})),
            _ => Stream::new().text("All done."),
        };
        Reply::Sse {
            body: stream.finish(&model, Some(1)),
            before: Some(50),
        }
    });
    let home = TestHome::new(&gateway.url);
    let out = home.run(
        BIN,
        &["run", "--format", "json", "edit", "notes"],
        None,
        None,
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(home.project().join("notes.txt")).unwrap(),
        "one\nthree\n"
    );
    let lines: Vec<Value> = out
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let tools: Vec<(&str, &str)> = lines
        .iter()
        .filter(|l| l["type"] == "tool_use")
        .map(|l| {
            (
                l["part"]["tool"].as_str().unwrap(),
                l["part"]["state"]["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(tools.len(), 3);
    assert!(
        tools.iter().all(|(_, status)| *status == "completed"),
        "{tools:?}"
    );
    let main = main_requests(&gateway);
    assert_eq!(
        main[2].json()["messages"][4]["content"][0]["content"],
        "Edited notes.txt (1 replacement)"
    );
    assert_eq!(
        main[3].json()["messages"][6]["content"][0]["content"],
        "Read file notes.txt, lines 1-2\n1: one\n2: three"
    );
    let answer: Vec<&str> = lines
        .iter()
        .filter(|l| l["type"] == "text")
        .map(|l| l["part"]["text"].as_str().unwrap())
        .collect();
    assert_eq!(answer, ["Working.", "All done."]);
}

#[test]
fn retries_honour_retry_after() {
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let gateway = FakeGateway::start(move |request: &Recorded, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if !request.is_title() && calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Reply::Json {
                status: 429,
                body: json!({"type": "error", "error": {"type": "rate_limit_error", "message": "slow down"}}),
                headers: vec![("retry-after".into(), "3".into())],
            };
        }
        Reply::Sse {
            body: Stream::new().text("ok").finish(&model, Some(1)),
            before: None,
        }
    });
    let home = TestHome::new(&gateway.url);
    let started = Instant::now();
    let out = home.run(BIN, &["run", "--standalone", "hi"], None, None);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(started.elapsed() >= Duration::from_secs(3));
    assert_eq!(
        out.stderr.matches("> build · claude-sonnet-5-5").count(),
        2,
        "the header repeats per attempt"
    );
}

#[test]
fn final_errors_are_not_retried() {
    let gateway = FakeGateway::start(|request: &Recorded, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if request.is_title() {
            return Reply::Sse {
                body: Stream::new().text("t").finish(&model, None),
                before: None,
            };
        }
        Reply::Json {
            status: 401,
            body: json!({"type": "error", "error": {"type": "authentication_error", "message": "Your Anysola token has expired or was revoked."}}),
            headers: vec![("x-should-retry".into(), "false".into())],
        }
    });
    let home = TestHome::new(&gateway.url);
    let out = home.run(BIN, &["run", "--format", "json", "hi"], None, None);
    assert_eq!(out.code, 1);
    assert_eq!(main_requests(&gateway).len(), 1);
    let last: Value = serde_json::from_str(out.stdout.lines().last().unwrap()).unwrap();
    assert_eq!(last["error"]["type"], "provider.auth");
    assert_eq!(
        last["error"]["message"],
        "Your Anysola token has expired or was revoked."
    );
}

#[test]
fn busy_session_in_another_process() {
    let gateway = FakeGateway::start(|request: &Recorded, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if request.is_title() {
            return Reply::Sse {
                body: Stream::new().text("t").finish(&model, None),
                before: None,
            };
        }
        Reply::Sse {
            body: Stream::new()
                .tool("s1", "shell", json!({"command": "sleep 3"}))
                .finish(&model, None),
            before: None,
        }
    });
    let home = TestHome::new(&gateway.url);
    let mut first = Command::new(BIN);
    first
        .args(["run", "-s", "ses_shared", "--auto", "go"])
        .current_dir(home.project());
    first
        .env("HOME", home.home())
        .env("ANYSOLA_API_URL", &gateway.url)
        .env("ANYSOLA_TOKEN", "asc_x");
    let mut first = first
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // The session is marked busy before its first model request.
    let started = Instant::now();
    while !gateway.messages().iter().any(|r| !r.is_title()) {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "no model request"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = home.run(BIN, &["run", "-s", "ses_shared", "again"], None, None);
    let _ = first.kill();
    let _ = first.wait();
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "\x1b[91m\x1b[1mError: \x1b[0mSession ses_shared is busy in another Anysola Code process.\n"
    );
}

const REVIEW_CONFIG: &str = r#"{"agents":{"anysola-review":{"mode":"primary","description":"Reviews pull requests","permissions":[{"action":"*","resource":"*","effect":"deny"},{"action":"read","resource":"*","effect":"allow"},{"action":"grep","resource":"*","effect":"allow"},{"action":"glob","resource":"*","effect":"allow"},{"action":"read","resource":"*.env","effect":"deny"},{"action":"read","resource":"*.env.*","effect":"deny"},{"action":"read","resource":"*.env.example","effect":"allow"}]}}}"#;

#[test]
fn code_action_review_and_issue_modes() {
    let gateway = FakeGateway::start(|request: &Recorded, _| {
        let body = request.json();
        let model = body["model"].as_str().unwrap_or_default().to_string();
        let tools: Vec<String> = body["tools"]
            .as_array()
            .map(|t| {
                t.iter()
                    .map(|t| t["name"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let after = body["messages"].as_array().unwrap().last().unwrap()["content"][0]["type"]
            == "tool_result";
        let stream = if request.is_title() || after {
            Stream::new().text("Review done.")
        } else if tools.iter().any(|t| t == "edit") {
            Stream::new().tool(
                "e1",
                "write",
                json!({"path": "fix.txt", "content": "fixed\n"}),
            )
        } else {
            Stream::new().tool(
                "e1",
                "write",
                json!({"path": "fix.txt", "content": "nope\n"}),
            )
        };
        Reply::Sse {
            body: stream.finish(&model, Some(1)),
            before: Some(10),
        }
    });
    let mut home = TestHome::new(&gateway.url);
    home.set_env("OPENCODE_DISABLE_AUTOUPDATE", "1");
    home.set_env("OPENCODE_DISABLE_PROJECT_CONFIG", "1");
    home.set_env("OPENCODE_DISABLE_CLAUDE_CODE", "1");
    home.set_env("OPENCODE_CONFIG_CONTENT", REVIEW_CONFIG);
    let review = home.run(
        BIN,
        &[
            "run",
            "--standalone",
            "--format",
            "json",
            "-m",
            "anysola/claude-sonnet-5-5",
            "--budget",
            "50",
            "--agent",
            "anysola-review",
        ],
        None,
        Some("Review this pull request."),
    );
    assert_eq!(review.code, 0, "{}", review.stderr);
    assert!(
        !home.project().join("fix.txt").exists(),
        "review mode cannot edit"
    );
    let tools = &main_requests(&gateway)[0].json()["tools"];
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["glob", "grep", "read", "skill"]);
    let events: Vec<Value> = review
        .stdout
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let last_text = events.iter().rev().find(|e| e["type"] == "text").unwrap();
    assert_eq!(last_text["part"]["text"], "Review done.");

    home.remove_env("OPENCODE_CONFIG_CONTENT");
    let issue = home.run(
        BIN,
        &[
            "run",
            "--standalone",
            "--format",
            "json",
            "-m",
            "anysola/claude-sonnet-5-5",
            "--budget",
            "50",
            "--auto",
        ],
        None,
        Some("Fix the issue."),
    );
    assert_eq!(issue.code, 0, "{}", issue.stderr);
    assert_eq!(
        std::fs::read_to_string(home.project().join("fix.txt")).unwrap(),
        "fixed\n"
    );
}

/// Starts `run --auto --format json go` whose turn runs `sleep 20` in the shell tool.
#[cfg(unix)]
fn sleeping_run() -> (FakeGateway, TestHome, std::process::Child) {
    let gateway = FakeGateway::start(|request: &Recorded, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if request.is_title() {
            return Reply::Sse {
                body: Stream::new().text("t").finish(&model, None),
                before: None,
            };
        }
        Reply::Sse {
            body: Stream::new()
                .tool("s1", "shell", json!({"command": "sleep 20"}))
                .finish(&model, None),
            before: None,
        }
    });
    let home = TestHome::new(&gateway.url);
    let mut command = Command::new(BIN);
    command
        .args(["run", "--auto", "--format", "json", "go"])
        .current_dir(home.project());
    command
        .env("HOME", home.home())
        .env("ANYSOLA_API_URL", &gateway.url)
        .env("ANYSOLA_TOKEN", "asc_x");
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The signal handlers are installed once the main request is out; the shell starts after it.
    let started = Instant::now();
    while !gateway.messages().iter().any(|r| !r.is_title()) {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "no main request"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    (gateway, home, child)
}

#[cfg(unix)]
#[test]
fn sigint_interrupts_with_exit_130() {
    let (_gateway, _home, child) = sleeping_run();
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let started = Instant::now();
    let out = child.wait_with_output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(out.status.code(), Some(130));
    let lines: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let tool = lines.iter().find(|l| l["type"] == "tool_use").unwrap();
    assert_eq!(tool["part"]["state"]["error"], "Tool execution interrupted");
    assert!(
        lines.iter().all(|l| l["type"] != "error"),
        "a user interrupt reports no error"
    );
}

/// SIGTERM tears the run down like an interrupt, exits 1, and a teardown that finishes in time
/// reports nothing pending. SIGHUP without a terminal keeps its default action.
#[cfg(unix)]
#[test]
fn sigterm_tears_down_with_exit_1() {
    let (_gateway, _home, child) = sleeping_run();
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let started = Instant::now();
    let out = child.wait_with_output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(out.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("Still pending"));

    let (_gateway, _home, child) = sleeping_run();
    unsafe { libc::kill(child.id() as i32, libc::SIGHUP) };
    let out = child.wait_with_output().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(out.status.signal(), Some(libc::SIGHUP));
}

#[test]
fn shell_permissions_policies_and_scanner() {
    let gateway = FakeGateway::start(|request: &Recorded, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let stream = if request.is_title() {
            Stream::new().text("Title")
        } else if request.json()["messages"].as_array().unwrap().len() == 1 {
            Stream::new()
                .tool("t1", "shell", json!({"command": "git push"}))
                .tool("t2", "shell", json!({"command": "touch made; echo 'open"}))
                .tool("t3", "shell", json!({"command": "cd /tmp && ls"}))
        } else {
            Stream::new().text("Done.")
        };
        Reply::Sse {
            body: stream.finish(&model, Some(1)),
            before: None,
        }
    });
    let home = TestHome::new(&gateway.url);
    std::fs::write(
        home.project().join("opencode.json"),
        r#"{"policies":[{"action":"tool.use","resource":"shell:git *","effect":"deny"}],"permission":{"external_directory":"ask"}}"#,
    )
    .unwrap();
    let out = home.run(BIN, &["run", "go"], None, None);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stderr
            .contains("permission requested: external_directory (/tmp/*); auto-rejecting"),
        "{}",
        out.stderr
    );
    assert!(!home.project().join("made").exists());
    let main = main_requests(&gateway);
    let results = &main[1].json()["messages"][2]["content"];
    let text = |i: usize| {
        results[i]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(
        text(0),
        r#"{"error":{"type":"permission.rejected","message":"Blocked by configuration policy"},"content":[]}"#
    );
    assert_eq!(
        text(1),
        r#"{"error":{"type":"unknown","message":"Portable shell scanner cannot analyze command: unterminated-quote"},"content":[]}"#
    );
}

#[test]
fn log_file_levels_and_printing() {
    let gateway = FakeGateway::start(answer("ok"));
    let mut home = TestHome::new(&gateway.url);
    let log = home.home().join(".anysola/log/anysola.log");
    let output = home.run(BIN, &["run", "hi"], None, None);
    assert_eq!(output.code, 0);
    let text = std::fs::read_to_string(&log).unwrap();
    let line = text
        .lines()
        .find(|l| l.contains("message=\"session idle\""))
        .expect("an idle line");
    let run = regex::Regex::new(
        r"^timestamp=\d{4}-\d\d-\d\dT[\d:.]+Z level=INFO run=[0-9a-f]{8} message=",
    )
    .unwrap();
    assert!(run.is_match(line), "{line}");
    assert!(line.ends_with(" outcome=succeeded role=cli"), "{line}");
    assert!(!output.stderr.contains("level=INFO"));

    let printed = home.run(BIN, &["run", "--print-logs", "hi"], None, None);
    assert!(printed.stderr.contains("level=INFO"), "{}", printed.stderr);

    let before = std::fs::read_to_string(&log).unwrap().lines().count();
    home.set_env("OPENCODE_LOG_LEVEL", "ERROR");
    home.run(BIN, &["run", "hi"], None, None);
    let quiet = home.run(BIN, &["run", "--log-level", "warn", "hi"], None, None);
    assert_eq!(quiet.code, 0);
    let after = std::fs::read_to_string(&log).unwrap();
    assert!(
        after
            .lines()
            .skip(before)
            .all(|l| !l.contains("level=INFO")),
        "{after}"
    );
}

#[test]
fn config_sources_in_precedence_order() {
    let gateway = FakeGateway::start(answer("ok"));
    let mut home = TestHome::new(&gateway.url);
    let agents: String = (1..=7)
        .map(|n| format!("\"a{n}\": {{\"mode\": \"primary\", \"description\": \"layer {n}\"}}"))
        .collect::<Vec<_>>()
        .join(", ");
    let layer = |n: u32| format!("{{\"default_agent\": \"a{n}\"}}");
    home.write(
        ".config/opencode/opencode.json",
        &format!("{{\"default_agent\": \"a1\", \"agents\": {{{agents}}}}}"),
    );
    home.write(".anysola/config.json", &layer(2));
    let explicit = home.write("explicit.json", &layer(3));
    home.write("opencode.json", &layer(4));
    home.write("proj/opencode.jsonc", &layer(5));
    home.write("proj/.opencode/opencode.json", &layer(6));
    home.set_env("OPENCODE_CONFIG", &explicit.display().to_string());
    home.set_env("OPENCODE_CONFIG_CONTENT", &layer(7));
    let agent = |home: &TestHome| {
        let output = home.run(BIN, &["run", "hi"], None, None);
        assert_eq!(output.code, 0, "{}", output.stderr);
        output
            .stderr
            .lines()
            .find_map(|l| l.strip_prefix("> "))
            .and_then(|l| l.split(" · ").next())
            .unwrap()
            .to_string()
    };
    assert_eq!(agent(&home), "a7");
    home.remove_env("OPENCODE_CONFIG_CONTENT");
    assert_eq!(agent(&home), "a6");
    std::fs::remove_file(home.home().join("proj/.opencode/opencode.json")).unwrap();
    assert_eq!(agent(&home), "a5");
    std::fs::remove_file(home.home().join("proj/opencode.jsonc")).unwrap();
    assert_eq!(agent(&home), "a4");
    home.set_env("OPENCODE_DISABLE_PROJECT_CONFIG", "1");
    assert_eq!(agent(&home), "a3");
    home.remove_env("OPENCODE_CONFIG");
    assert_eq!(agent(&home), "a2");
    std::fs::remove_file(home.home().join(".anysola/config.json")).unwrap();
    assert_eq!(agent(&home), "a1");
}

#[test]
fn ignored_keys_and_the_plugins_warning() {
    let gateway = FakeGateway::start(answer("ok"));
    let mut home = TestHome::new(&gateway.url);
    home.set_env(
        "OPENCODE_CONFIG_CONTENT",
        r#"{"lsp": {"x": {}}, "share": "manual", "enterprise": {"url": "u"}, "update": false, "plugin": ["a"], "plugins": ["b"]}"#,
    );
    home.write(".anysola/config.json", r#"{"plugins": ["c"]}"#);
    let output = home.run(BIN, &["run", "--print-logs", "hi"], None, None);
    assert_eq!(output.code, 0, "{}", output.stderr);
    let warnings = output
        .stderr
        .matches("Plugins are not supported since Anysola Code 1.0; ignoring \\\"plugins\\\".")
        .count();
    assert_eq!(warnings, 1, "{}", output.stderr);
}

#[test]
fn instruction_urls_are_fetched() {
    let gateway = FakeGateway::start(answer("ok"));
    gateway.serve("/rules.md", "Remote rule.\n");
    let mut home = TestHome::new(&gateway.url);
    let content = format!(
        r#"{{"instructions": ["{0}/rules.md", "{0}/missing.md"]}}"#,
        gateway.url
    );
    home.set_env("OPENCODE_CONFIG_CONTENT", &content);
    assert_eq!(home.run(BIN, &["run", "hi"], None, None).code, 0);
    let request = main_requests(&gateway).remove(0);
    let block = request.json()["system"][2]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(block.contains(&format!(
        "Instructions from: {}/rules.md\nRemote rule.\n",
        gateway.url
    )));
    assert!(!block.contains("missing.md"));
    let fetch = gateway
        .requests()
        .into_iter()
        .find(|r| r.path == "/rules.md")
        .unwrap();
    assert_eq!(
        fetch.header("user-agent"),
        Some("opencode/latest/2.2.5/cli")
    );
}
