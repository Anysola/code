//! Pending input: steered prompts join the running turn at its next step boundary, queued ones
//! start their own turn once it ends.

mod common;

use std::sync::{Arc, OnceLock};

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_core::session::inbox::Delivery;
use anysola_testkit::FakeGateway;
use anysola_testkit::sse::Stream;
use common::*;
use serde_json::json;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        files: vec![],
    }
}

#[test]
fn steer_and_queue() {
    let core: Arc<OnceLock<(Core, String)>> = Arc::default();
    let handle = core.clone();
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "Title");
        }
        let body = request.body.clone();
        if body.contains("\"go\"") && !body.contains("tool_result") {
            let (core, session) = handle.get().unwrap();
            core.enqueue(session, prompt("steered"), Delivery::Steer)
                .unwrap();
            core.enqueue(session, prompt("queued"), Delivery::Queue)
                .unwrap();
            let model = request.json()["model"].as_str().unwrap().to_string();
            return anysola_testkit::Reply::Sse {
                body: Stream::new()
                    .tool("toolu_1", "glob", json!({"pattern": "*.none"}))
                    .finish(&model, Some(1)),
                before: Some(100),
            };
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let started = runtime.block_on(common::core(&env));
    let (session, _) = started.open(&Target::New, false).unwrap();
    core.set((started.clone(), session.id.clone())).ok();
    let outcome = runtime
        .block_on(started.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, anysola_core::session::Outcome::Succeeded);
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 3);
    assert_eq!(
        transcript(&main[1]),
        vec![
            "user: go",
            "assistant: glob",
            "user: No files found",
            "user: steered"
        ]
    );
    let last = transcript(&main[2]);
    assert_eq!(last.last().unwrap(), "user: queued");
    assert!(started.pending(&session.id).unwrap().is_empty());
}

#[test]
fn pending_items_can_switch_or_cancel() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let a = core
        .enqueue(&session.id, prompt("a"), Delivery::Queue)
        .unwrap();
    let b = core
        .enqueue(&session.id, prompt("b"), Delivery::Queue)
        .unwrap();
    assert!(core.cancel_pending(&a).unwrap());
    assert!(core.set_delivery(&b, Delivery::Steer).unwrap());
    let pending = core.pending(&session.id).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].delivery, "steer");
    runtime
        .block_on(core.prompt(&session.id, prompt("c"), TurnOptions::default()))
        .unwrap();
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 1, "both steers start one turn");
    assert_eq!(transcript(&main[0]), vec!["user: b", "user: c"]);
}
