//! The model runs programs through `execute`; the tool results sent back have 0.5.1's format.

mod common;

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply, TestHome};
use common::*;
use serde_json::{Value, json};

const PROGRAMS: [&str; 3] = [
    "console.log('hey'); return await tools.opencode.worktree_list()",
    "return await tools.opencode.nope()",
    "await tools.opencode.session_rename({title: 'Renamed'}); return search({query: 'rename'}).items.map(i => i.path)",
];

fn tool_results(request: &Recorded) -> Vec<Value> {
    request.json()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|p| p["type"] == "tool_result")
        .collect()
}

#[test]
fn programs_run_through_execute() {
    let gateway = FakeGateway::start(|request: &Recorded, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if request.is_title() {
            return Reply::Sse {
                body: Stream::new().text("Title").finish(&model, Some(0)),
            };
        }
        let done = tool_results(request).len();
        let stream = match PROGRAMS.get(done) {
            Some(code) => Stream::new().tool(&format!("t{done}"), "execute", json!({"code": code})),
            None => Stream::new().text("Done."),
        };
        Reply::Sse {
            body: stream.finish(&model, Some(1)),
        }
    });
    let home = TestHome::new(&gateway.url);
    let out = home.run(BIN, &["run", "go"], None, None);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let last = gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title())
        .unwrap();
    let results = tool_results(&last);
    let project = home.project();
    assert_eq!(
        results[0]["content"],
        format!(
            "{{\n  \"worktrees\": [\n    {{\n      \"directory\": {}\n    }}\n  ]\n}}\n\nLogs:\nhey",
            serde_json::Value::from(project.display().to_string())
        )
    );
    assert_eq!(
        results[1]["content"],
        "Unknown tool 'opencode.nope'.\nUse search to find available tools."
    );
    assert_eq!(
        results[2]["content"],
        "[\n  \"tools.opencode.session_rename\"\n]"
    );
    assert!(results.iter().all(|r| r.get("is_error").is_none()));
    let system = last.json()["system"][2]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(system.starts_with("# Code Mode\n\n"));
    assert!(system.contains("- opencode (8 tools) // Tools for managing OpenCode itself"));
}
