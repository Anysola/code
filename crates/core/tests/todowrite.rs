//! `experimental.todo`: the `todowrite` tool is advertised with its verbatim definition, and the
//! session's list is the input of the latest completed call.

mod common;

use anysola_core::api::{Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_core::tool::todowrite::Todo;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        files: vec![],
        skills: Vec::new(),
    }
}

fn reply(request: &Recorded, _: usize) -> Reply {
    let last = request.json()["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(Value::to_string)
        .unwrap_or_default();
    if request.is_title() || last.contains("tool_result") {
        return text(request, "ok");
    }
    let model = request.json()["model"].as_str().unwrap().to_string();
    let todos = if last.contains("first") {
        json!([{"content": "a", "status": "in_progress"}, {"content": "b", "status": "pending"}])
    } else {
        json!([{"content": "a", "status": "completed"}])
    };
    Reply::Sse {
        body: Stream::new()
            .tool("toolu_1", "todowrite", json!({"todos": todos}))
            .finish(&model, Some(1)),
        before: Some(100),
    }
}

fn tool_names(request: &Recorded) -> Vec<String> {
    request.json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn off_by_default() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hi"), TurnOptions::default()))
        .unwrap();
    let main = gateway
        .messages()
        .into_iter()
        .find(|r| !r.is_title())
        .unwrap();
    assert!(!tool_names(&main).contains(&"todowrite".to_string()));
}

#[test]
fn todo_list_follows_the_latest_call() {
    let gateway = FakeGateway::start(reply);
    let env = env(&gateway.url);
    std::fs::write(
        env.home.project().join("opencode.json"),
        r#"{"experimental":{"todo":true}}"#,
    )
    .unwrap();
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    assert!(core.todos(&session.id).unwrap().is_empty());
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let captured: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spec/verbatim/tools/_captured-request-build-experimental-flags.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let expected = captured["body"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "todowrite")
        .cloned()
        .unwrap();
    let ours = main[0].json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "todowrite")
        .cloned()
        .unwrap();
    assert_eq!(ours.to_string(), expected.to_string());
    assert_eq!(
        tool_names(&main[0])[8..],
        ["todowrite", "webfetch", "websearch", "write", "execute"]
    );
    assert!(
        main[1].body.contains(
            r#"Todos updated: 0/2 completed, 1 in progress, 1 pending. In progress: \"a\""#
        )
    );
    let todo = |content: &str, status: &str| Todo {
        content: content.into(),
        status: status.into(),
    };
    assert_eq!(
        core.todos(&session.id).unwrap(),
        [todo("a", "in_progress"), todo("b", "pending")]
    );
    runtime
        .block_on(core.prompt(&session.id, prompt("second"), TurnOptions::default()))
        .unwrap();
    assert_eq!(core.todos(&session.id).unwrap(), [todo("a", "completed")]);
}
