//! Hooks on every platform, Windows included: whatever shell the `shell` tool picks there runs
//! a hook command, and its exit code decides. No script file, no Unix tools.

mod common;

use anysola_core::api::{Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn last(request: &Recorded) -> String {
    request.json()["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(Value::to_string)
        .unwrap_or_default()
}

/// The tool result the model got for one `glob` call under a PreToolUse hook running `command`.
fn glob_result(command: &str) -> String {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() || last(request).contains("tool_result") {
            return text(request, "done");
        }
        let model = request.json()["model"].as_str().unwrap().to_string();
        Reply::Sse {
            body: Stream::new()
                .tool("toolu_1", "glob", json!({"pattern": "*.md"}))
                .finish(&model, Some(1)),
        }
    });
    let env = env(&gateway.url);
    let config = json!({"hooks": {"PreToolUse": [{"matcher": "Glob",
        "hooks": [{"type": "command", "command": command, "timeout": 60}]}]}});
    env.home.write(".anysola/config.json", &config.to_string());
    env.home.write("proj/notes.md", "x");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let prompt = Prompt {
        text: "go".into(),
        ..Default::default()
    };
    runtime
        .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
        .unwrap();
    let requests: Vec<Recorded> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(requests.len(), 2);
    last(&requests[1])
}

#[test]
fn exit_two_blocks_and_exit_zero_lets_the_call_run() {
    let blocked = glob_result("exit 2");
    assert!(
        blocked.contains("A PreToolUse hook blocked this call"),
        "{blocked}"
    );
    assert!(!blocked.contains("notes.md"), "{blocked}");
    let ran = glob_result("exit 0");
    assert!(!ran.contains("blocked this call"), "{ran}");
    assert!(ran.contains("notes.md"), "{ran}");
}
