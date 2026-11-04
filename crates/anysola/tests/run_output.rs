//! `anysola run` output against the scenarios captured in `spec/verbatim/cli/run/` (the fake2
//! gateway). Text mode is compared byte for byte with the 1.0 deltas applied: one blank line around
//! each block (§B14), the lowest balance of the turn (§B19), no billing URL twice (§B5), no stack
//! dump (§B15). JSON lines are compared with ids and times normalized.

mod common;

use std::collections::HashMap;

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply, TestHome};
use common::*;
use regex::Regex;
use serde_json::{Value, json};

/// fake2.mjs: a shell call for "run ls", thinking for "think", 402 for "nocredits", a web search
/// for "latest"; 2 credits per request, titles free.
fn fake2(request: &Recorded, _: usize) -> Reply {
    let body = request.json();
    let last = body["messages"]
        .as_array()
        .and_then(|m| m.last())
        .cloned()
        .unwrap_or(Value::Null);
    let after_tool = last["content"]
        .as_array()
        .is_some_and(|c| c.iter().any(|p| p["type"] == "tool_result"));
    let text = last.to_string().to_lowercase();
    let title = request.is_title();
    if !title && text.contains("nocredits") {
        return Reply::Json {
            status: 402,
            body: json!({"type": "error", "error": {"type": "insufficient_credits", "message": "You've used this cycle's Anysola credits. They renew on 23 Nov. account.anysola.com/billing"}}),
            headers: Vec::new(),
        };
    }
    let stream = if !title && !after_tool && text.contains("run ls") {
        Stream::new().tool(
            "toolu_sh1",
            "shell",
            json!({"command": "ls", "description": "List files"}),
        )
    } else if !title && !after_tool && text.contains("latest") {
        Stream::new().tool("toolu_fake1", "websearch", json!({"query": "latest bun"}))
    } else if !title && text.contains("think") {
        Stream::new()
            .thinking("The user wants a short answer.", "sig")
            .text("Plain answer.")
    } else {
        Stream::new().text(if title {
            "Fake title"
        } else if after_tool {
            "Done after the tool."
        } else {
            "Plain answer."
        })
    };
    let model = body["model"].as_str().unwrap_or_default().to_string();
    Reply::Sse {
        body: stream.finish(&model, Some(if title { 0 } else { 2 })),
        before: Some(731),
    }
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn setup() -> (FakeGateway, TestHome) {
    let gateway = FakeGateway::start(fake2);
    let home = TestHome::new(&gateway.url);
    std::fs::write(home.project().join("a.txt"), "a\n").unwrap();
    // The captures ran in a git project, so their steps carry snapshot trees.
    git(&home.project(), &["init", "-q"]);
    (gateway, home)
}

fn run_in(home: &TestHome, args: &[&str]) -> Run {
    let out = home.run(BIN, args, None, None);
    Run {
        stdout: out.stdout,
        stderr: out.stderr,
        code: out.code,
    }
}

const HEADER: &str = "\x1b[0m\n> build · claude-sonnet-5-5\n\x1b[0m\n";

#[test]
fn text_plain() {
    let (_g, home) = setup();
    let out = run_in(&home, &["run", "--standalone", "hello"]);
    assert_eq!(out.stdout, "Plain answer.\n");
    assert_eq!(
        out.stderr,
        format!("{HEADER}\x1b[90mAnysola · 2 credits · 729 left\x1b[0m\n")
    );
    assert_eq!(out.code, 0);
}

#[test]
fn text_thinking() {
    let (_g, home) = setup();
    let out = run_in(
        &home,
        &["run", "--standalone", "--thinking", "please", "think"],
    );
    assert_eq!(
        out.stdout,
        "Thinking: The user wants a short answer.\nPlain answer.\n"
    );
    assert_eq!(
        out.stderr,
        format!("{HEADER}\x1b[90mAnysola · 2 credits · 729 left\x1b[0m\n")
    );
}

#[test]
fn text_shell_and_auto() {
    for auto in [false, true] {
        let (_g, mut home) = setup();
        if !auto {
            home.set_env("OPENCODE_CONFIG_CONTENT", "{}");
        }
        let mut args = vec!["run", "--standalone", "run", "ls", "please"];
        if auto {
            args.insert(2, "--auto");
        }
        let out = run_in(&home, &args);
        assert_eq!(out.stdout, "Done after the tool.\n");
        assert_eq!(
            out.stderr,
            format!(
                "{HEADER}\x1b[0m$ \x1b[0mls\na.txt\n\x1b[0m\n> build · claude-sonnet-5-5\n\x1b[0m\n\x1b[90mAnysola · 4 credits · 729 left\x1b[0m\n"
            )
        );
        assert_eq!(out.code, 0);
    }
}

#[test]
fn text_permission_ask() {
    let (_g, mut home) = setup();
    home.set_env(
        "OPENCODE_CONFIG_CONTENT",
        r#"{"permission":{"bash":"ask"}}"#,
    );
    let out = run_in(&home, &["run", "--standalone", "run", "ls", "please"]);
    assert_eq!(out.stdout, "Done after the tool.\n");
    assert_eq!(
        out.stderr,
        format!(
            "{HEADER}\x1b[93m\x1b[1m! \x1b[0mpermission requested: shell (ls); auto-rejecting\n\x1b[0m✗ \x1b[0mls failed\n\x1b[91m\x1b[1mError: \x1b[0mThis non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action.\n\x1b[0m\n> build · claude-sonnet-5-5\n\x1b[0m\n\x1b[90mAnysola · 4 credits · 729 left\x1b[0m\n"
        )
    );
    assert_eq!(out.code, 0);
}

#[test]
fn text_agent_plan() {
    let (_g, home) = setup();
    let out = run_in(&home, &["run", "--standalone", "--agent", "plan", "hello"]);
    assert_eq!(out.stdout, "Plain answer.\n");
    assert_eq!(
        out.stderr,
        "\x1b[0m\n> plan · claude-sonnet-5-5\n\x1b[0m\n\x1b[90mAnysola · 2 credits · 729 left\x1b[0m\n"
    );
}

#[test]
fn text_402() {
    let (_g, home) = setup();
    let out = run_in(&home, &["run", "--standalone", "nocredits", "now"]);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!(
            "{HEADER}\x1b[91m\x1b[1mError: \x1b[0mYou've used this cycle's Anysola credits. They renew on 23 Nov. account.anysola.com/billing\n"
        )
    );
    assert_eq!(out.code, 1);
}

#[test]
fn text_budget_websearch_and_already_reached() {
    let (gateway, home) = setup();
    let out = run_in(
        &home,
        &[
            "run",
            "--standalone",
            "--budget",
            "1",
            "What",
            "is",
            "the",
            "latest",
            "Bun?",
        ],
    );
    let session = session_of(&gateway);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!(
            "{HEADER}\x1b[0m◈ \x1b[0mWeb Search via Anysola \"latest bun\" \n\x1b[91m\x1b[1mError: \x1b[0mBudget reached: 4/1 credits. Continue with `anysola run -s {session} --budget <credits>`.\n\x1b[90mAnysola · 4 credits · 729 left\x1b[0m\n"
        )
    );
    assert_eq!(out.code, 1);
    let again = run_in(
        &home,
        &["run", "--standalone", "-s", &session, "--budget", "1", "hi"],
    );
    assert_eq!(
        again.stderr,
        format!(
            "\x1b[91m\x1b[1mError: \x1b[0mBudget reached: 4/1 credits. Continue with `anysola run -s {session} --budget <credits>`.\n"
        )
    );
    assert_eq!(again.code, 1);
}

fn session_of(gateway: &FakeGateway) -> String {
    gateway.messages()[0]
        .header("x-opencode-session")
        .unwrap()
        .to_string()
}

/// Ids become `<n>` by first appearance (a message id and the part ids derived from it share the
/// number), times become 0, and snapshot tree ids become `<tree>`.
fn normalize_json(text: &str) -> Vec<Value> {
    let ids = Regex::new(r"(ses|msg|prt)_([0-9a-f]{12}[0-9A-Za-z]{14})").unwrap();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let text = ids.replace_all(text, |c: &regex::Captures| {
        let next = seen.len();
        let n = *seen.entry(c[2].to_string()).or_insert(next);
        format!("{}_<{n}>", &c[1])
    });
    text.lines()
        .map(|line| {
            let mut value: Value = serde_json::from_str(line).expect("one JSON object per line");
            strip(&mut value);
            value
        })
        .collect()
}

fn strip(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(tree) = map.get_mut("snapshot") {
                *tree = json!("<tree>");
            }
            for (key, child) in map.iter_mut() {
                if matches!(key.as_str(), "timestamp" | "start" | "end") && child.is_number() {
                    *child = json!(0);
                } else {
                    strip(child);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip),
        _ => {}
    }
}

fn json_capture(name: &str) -> Vec<Value> {
    normalize_json(&std::fs::read_to_string(spec(&format!("cli/run/{name}.stdout"))).unwrap())
}

fn json_run(home: &TestHome, args: &[&str]) -> (Vec<Value>, Run) {
    let out = run_in(home, args);
    (normalize_json(&out.stdout), out)
}

#[test]
fn json_matches_captures() {
    for (name, args, config) in [
        ("json-plain", vec!["hello"], None),
        ("json-thinking", vec!["--thinking", "please", "think"], None),
        ("json-shell", vec!["run", "ls", "please"], None),
        (
            "json-permission-ask",
            vec!["run", "ls", "please"],
            Some(r#"{"permission":{"bash":"ask"}}"#),
        ),
        (
            "json-permission-deny",
            vec!["--auto", "run", "ls", "please"],
            Some(r#"{"permission":{"bash":"deny"}}"#),
        ),
    ] {
        let (_g, mut home) = setup();
        if let Some(config) = config {
            home.set_env("OPENCODE_CONFIG_CONTENT", config);
        }
        let mut full = vec!["run", "--standalone", "--format", "json"];
        full.extend(args);
        let (lines, out) = json_run(&home, &full);
        assert_eq!(lines, json_capture(name), "{name}\n{}", out.stdout);
        assert_eq!(out.code, 0, "{name}");
        let permission = if name == "json-permission-ask" {
            "\x1b[93m\x1b[1m! \x1b[0mpermission requested: shell (ls); auto-rejecting\n"
        } else {
            ""
        };
        assert_eq!(out.stderr, permission, "{name}");
    }
}

#[test]
fn json_402_without_doubled_url() {
    let (_g, home) = setup();
    let (lines, out) = json_run(
        &home,
        &[
            "run",
            "--standalone",
            "--format",
            "json",
            "nocredits",
            "now",
        ],
    );
    let mut expected = json_capture("json-402");
    let message = "You've used this cycle's Anysola credits. They renew on 23 Nov. account.anysola.com/billing";
    expected[1]["error"]["message"] = json!(message);
    expected[1]["error"]["response"]["body"] = json!(
        json!({"type": "error", "error": {"type": "billing_error", "message": message}})
            .to_string()
    );
    assert_eq!(lines, expected);
    assert_eq!(out.code, 1);
}

#[test]
fn json_budget() {
    let (gateway, home) = setup();
    let (lines, out) = json_run(
        &home,
        &[
            "run",
            "--standalone",
            "--format",
            "json",
            "--budget",
            "1",
            "What",
            "is",
            "the",
            "latest",
            "Bun?",
        ],
    );
    assert_eq!(out.code, 1);
    let kinds: Vec<&str> = lines.iter().map(|l| l["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["step_start", "tool_use", "step_finish", "error"]);
    assert_eq!(
        lines[1]["part"]["state"]["output"],
        "## [Bun](https://bun.com)\n\nBun 1.4.4"
    );
    let session = session_of(&gateway);
    assert_eq!(
        lines[3]["error"],
        json!({"type": "unknown", "message": "Budget reached: 4/1 credits. Continue with `anysola run -s ses_<0> --budget <credits>`."})
    );
    let (again, out) = json_run(
        &home,
        &[
            "run", "--format", "json", "-s", &session, "--budget", "1", "hi",
        ],
    );
    let mut expected = json_capture("json-budget-already-reached");
    expected[0]["error"]["message"] = json!(
        "Budget reached: 4/1 credits. Continue with `anysola run -s ses_<0> --budget <credits>`."
    );
    assert_eq!(again, expected);
    assert_eq!(out.code, 1);
}
