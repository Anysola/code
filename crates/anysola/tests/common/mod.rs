//! Shared helpers for the integration tests: the scripted gateway behaviour of 0.5.1's capture
//! logger, the project fixtures of the captures, and the normalization applied before comparing
//! with a capture.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Output, Recorded, Reply, TestHome};
use regex::Regex;
use serde_json::{Map, Value, json};

pub const BIN: &str = env!("CARGO_BIN_EXE_anysola");

pub fn spec(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/verbatim")
        .join(path)
}

pub fn read_json(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(spec(path)).expect("capture"))
        .expect("capture json")
}

fn text(message: &Value) -> String {
    match &message["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// The capture logger's answers, keyed by markers in the conversation.
pub fn logger(request: &Recorded, _index: usize, read_file: &str) -> Reply {
    let body = request.json();
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let last = messages.last().cloned().unwrap_or(Value::Null);
    let after_tool = last["content"]
        .as_array()
        .is_some_and(|c| c.iter().any(|p| p["type"] == "tool_result"));
    let all = Value::Array(messages.clone()).to_string();
    let marker = ["THINKTEST", "TOOLTEST", "TWOTOOLS", "LONGSTEPS"]
        .into_iter()
        .find(|m| all.contains(m));
    let results = messages
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|p| p["type"] == "tool_result")
        .count();
    let model = body["model"].as_str().unwrap_or_default().to_string();
    let usage = json!({"input_tokens": 10, "output_tokens": 1, "cache_read_input_tokens": 5, "cache_creation_input_tokens": 3});
    let stream = Stream::new().usage(usage);
    let stream = if body["tool_choice"]["type"] == "none" {
        stream.text("Max steps reached summary.")
    } else if request.is_title() {
        stream.text("Fake title")
    } else if marker == Some("THINKTEST") {
        stream
            .thinking("Let me think about it.", "SIG_1")
            .text("Thought answer.")
    } else if marker == Some("TOOLTEST") && !after_tool {
        stream
            .thinking("I should read the file.", "SIG_1")
            .text("Reading.")
            .tool("toolu_r1", "read", json!({"filePath": read_file}))
    } else if marker == Some("TWOTOOLS") && !after_tool {
        stream
            .tool("toolu_a", "glob", json!({"pattern": "*"}))
            .tool(
                "toolu_b",
                "bash",
                json!({"command": "echo hi", "description": "Echo hi"}),
            )
    } else if marker == Some("LONGSTEPS") && results < 50 {
        stream.tool(
            &format!("toolu_l{results}"),
            "glob",
            json!({"pattern": "*.md"}),
        )
    } else {
        stream.text(if after_tool {
            "Done after tool."
        } else {
            "Hello answer."
        })
    };
    let charged = if request.is_title() { 0 } else { 1 };
    let _ = text(&last);
    Reply::Sse {
        body: stream.finish(&model, Some(charged)),
        before: Some(731),
    }
}

pub fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git");
    assert!(status.status.success(), "git {args:?}");
}

/// The `my-app` capture project: inside home, a git repository without commits.
pub fn my_app(home: &TestHome) -> PathBuf {
    let dir = home.home().join("my-app");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    dir
}

/// The `scratchpad/proj` capture project: outside home, committed, with instruction files and the
/// markdown agents of the captures.
pub fn proj(home: &TestHome) -> PathBuf {
    let dir = home.home().parent().unwrap().join("proj");
    let write = |rel: &str, content: &str| {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    };
    write("AGENTS.md", "Project rule: always answer in English.\n");
    write("README.md", "# Proj\nHello from proj README.\n");
    write("docs/rules.md", "Docs rule from instructions glob.\n");
    write("sub/AGENTS.md", "Sub rule: sub dir.\n");
    write(
        "opencode.json",
        "{\n  \"instructions\": [\"docs/*.md\"]\n}\n",
    );
    write(
        ".opencode/agent/reviewer.md",
        "---\ndescription: Reviews code for bugs\nmode: subagent\nmodel: anysola/claude-haiku-5-5\nsteps: 3\n---\nYou are a code reviewer. Find bugs.\n",
    );
    home.write(
        ".config/opencode/AGENTS.md",
        "Global rule from ~/.config/opencode/AGENTS.md\n",
    );
    git(&dir, &["init", "-q"]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

pub fn run(home: &TestHome, cwd: &Path, args: &[&str]) -> Output {
    home.run(BIN, args, Some(cwd), None)
}

/// Text replacements that turn absolute locations of one environment into placeholders.
pub struct Places(pub Vec<(String, &'static str)>);

impl Places {
    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (from, to) in &self.0 {
            out = out.replace(from.as_str(), to);
        }
        out
    }
}

/// The capture environments.
pub fn capture_places() -> Places {
    Places(vec![
        (
            "/home/ubuntu/anysola-code/rust/testing/runs/session/home/my-app".into(),
            "<PROJECT>",
        ),
        (
            "/tmp/claude-1001/-home-ubuntu/8ca5656e-d1fe-58c1-b843-42c4ccb2771b/scratchpad/proj"
                .into(),
            "<PROJECT>",
        ),
        (
            "/home/ubuntu/anysola-code/rust/testing/runs/session/home".into(),
            "<HOME>",
        ),
        (
            "/home/ubuntu/anysola-code/rust/testing/runs/tools/proj".into(),
            "<PROJECT>",
        ),
    ])
}

pub fn our_places(home: &TestHome, project: &Path) -> Places {
    Places(vec![
        (project.display().to_string(), "<PROJECT>"),
        (home.home().display().to_string(), "<HOME>"),
    ])
}

fn map_strings(value: &Value, f: &dyn Fn(&str) -> String) -> Value {
    match value {
        Value::String(s) => Value::String(f(s)),
        Value::Array(items) => Value::Array(items.iter().map(|v| map_strings(v, f)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), map_strings(v, f)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Normalizes a request body for comparison. `capture` applies the fixed-bug deltas to 0.5.1's
/// side: argv quoting (§B1) and the skills read from outside the discovery bound (decision 3).
pub fn normalize_body(body: &Value, places: &Places, capture: bool) -> Value {
    let date = Regex::new(r"Today's date: \w{3} \w{3} \d{2} \d{4}").unwrap();
    let signature = Regex::new(r"SIG_\d+").unwrap();
    let skills =
        Regex::new(r"(?s)\n\nSkills provide specialized instructions.*?</available_skills>")
            .unwrap();
    let quoted = Regex::new(r#"^"([^"]*)"(\n|$)"#).unwrap();
    // The same quoting inside a compaction transcript line.
    let quoted_transcript = Regex::new(r#"(?m)^\[User\]: "([^"]*)"$"#).unwrap();
    let paths = Regex::new(r"^(<PROJECT>/[^\n]*\n?)+$").unwrap();
    let f = |s: &str| {
        let mut s = places.apply(s);
        s = date.replace_all(&s, "Today's date: <DATE>").into_owned();
        s = signature.replace_all(&s, "SIG_N").into_owned();
        if capture {
            s = skills.replace_all(&s, "").into_owned();
            s = quoted.replace(&s, "$1$2").into_owned();
            s = quoted_transcript.replace_all(&s, "[User]: $1").into_owned();
        }
        // Search results are sorted by path in 1.0; 0.5.1 kept ripgrep's walk order.
        if paths.is_match(&s) {
            let mut lines: Vec<&str> = s.lines().collect();
            lines.sort();
            s = lines.join("\n");
        }
        s
    };
    map_strings(body, &f)
}

const COMPARED_HEADERS: &[&str] = &[
    "content-type",
    "user-agent",
    "anthropic-beta",
    "anthropic-version",
    "x-anysola-client",
    "x-anysola-credits-event",
    "x-anysola-version",
    "x-anysola-variant",
    "x-api-key",
    "authorization",
    "x-opencode-client",
    "x-opencode-project",
    "x-opencode-session",
    "x-opencode-session-id",
    "x-session-affinity",
    "x-session-id",
    "x-opencode-parent-session-id",
    "x-parent-session-id",
    "accept",
];

/// Header values with versions, tokens, session and project ids replaced.
pub fn normalize_headers(headers: &[(String, String)]) -> Map<String, Value> {
    let session = Regex::new(r"ses_[0-9A-Za-z]+").unwrap();
    let version = Regex::new(r"\+anysola\.[^/]+/").unwrap();
    let mut out = Map::new();
    for (name, value) in headers {
        if !COMPARED_HEADERS.contains(&name.as_str()) {
            continue;
        }
        let value = match name.as_str() {
            "x-anysola-version" => "<ver>".to_string(),
            "x-api-key" => "<token>".to_string(),
            "user-agent" => version.replace(value, "+anysola.<ver>/").into_owned(),
            "x-opencode-project" if value != "global" => "<project>".to_string(),
            _ => session.replace_all(value, "ses_<id>").into_owned(),
        };
        out.insert(name.clone(), Value::String(value));
    }
    out
}

pub fn capture_headers(capture: &Value) -> Vec<(String, String)> {
    capture["headers"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
        .collect()
}

/// Asserts body and headers equal after normalization, with a readable diff on failure.
pub fn assert_request(name: &str, ours: &Recorded, capture: &Value, our_places: &Places) {
    let expected = normalize_body(&capture["body"], &capture_places(), true);
    let actual = normalize_body(&ours.json(), our_places, false);
    if expected != actual {
        let e = serde_json::to_string_pretty(&expected).unwrap();
        let a = serde_json::to_string_pretty(&actual).unwrap();
        let diff = similar_lines(&e, &a);
        panic!("{name}: request body differs from the capture\n{diff}");
    }
    let expected = normalize_headers(&capture_headers(capture));
    let actual = normalize_headers(&ours.headers);
    assert_eq!(
        Value::Object(expected),
        Value::Object(actual),
        "{name}: headers differ"
    );
}

fn similar_lines(expected: &str, actual: &str) -> String {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let first = e
        .iter()
        .zip(&a)
        .position(|(x, y)| x != y)
        .unwrap_or(e.len().min(a.len()));
    let from = first.saturating_sub(3);
    format!(
        "first difference at line {first}\n--- capture\n{}\n+++ ours\n{}",
        e[from..(first + 5).min(e.len())].join("\n"),
        a[from..(first + 5).min(a.len())].join("\n")
    )
}

pub fn gateway_logger(read_file: String) -> FakeGateway {
    FakeGateway::start(move |request, index| logger(request, index, &read_file))
}
