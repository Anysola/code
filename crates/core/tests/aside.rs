//! `/btw`: a side question answered from the session's context, also while its turn runs; it
//! writes nothing to the session, can be cancelled, and says why it has no answer.

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::{AsideKind, CancellationToken, Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Prompt::default()
    }
}

fn is_aside(request: &Recorded) -> bool {
    request.body.contains("quick side question")
}

fn call(request: &Recorded, id: &str, name: &str, input: Value) -> Reply {
    let model = request.json()["model"].as_str().unwrap().to_string();
    Reply::Sse {
        body: Stream::new().tool(id, name, input).finish(&model, Some(1)),
    }
}

/// Every `tool_use` of a request is answered by a `tool_result` in the next message, ids are
/// unique: what the provider insists on.
fn assert_paired(request: &Recorded) {
    let messages = request.json()["messages"].as_array().unwrap().clone();
    let mut seen = std::collections::HashSet::new();
    for (index, message) in messages.iter().enumerate() {
        for block in message["content"].as_array().into_iter().flatten() {
            if block["type"] != "tool_use" {
                continue;
            }
            let id = block["id"].as_str().unwrap();
            assert!(seen.insert(id.to_string()), "duplicate tool id {id}");
            assert!(block["input"].is_object());
            let answered = messages.get(index + 1).is_some_and(|next| {
                next["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|b| b["type"] == "tool_result" && b["tool_use_id"] == id)
            });
            assert!(answered, "tool_use {id} has no result");
        }
    }
}

#[test]
fn a_side_question_while_a_turn_runs_sends_only_settled_steps_and_writes_nothing() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if is_aside(request) {
            return text(request, "  It is sleeping.  ");
        }
        let body = &request.body;
        if body.contains("toolu_slow") {
            return text(request, "done");
        }
        if body.contains("toolu_quick") {
            return call(
                request,
                "toolu_slow",
                "shell",
                json!({"command": "sleep 20"}),
            );
        }
        call(
            request,
            "toolu_quick",
            "shell",
            json!({"command": "echo hi"}),
        )
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let turn = {
        let (core, id) = (core.clone(), session.id.clone());
        runtime.spawn(async move {
            core.prompt(&id, prompt("work"), TurnOptions::default())
                .await
        })
    };
    // Wait for the slow call to be running: its step is stored, not completed.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !core
        .db()
        .messages(&session.id)
        .unwrap()
        .iter()
        .any(|r| r.kind == "assistant" && r.data.to_string().contains("sleep 20"))
    {
        assert!(Instant::now() < deadline, "the slow call never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    tokio_sleep(&runtime, 300);
    // The running step may update its own row; the rows themselves must not change.
    let rows = || -> Vec<(String, String)> {
        core.db()
            .messages(&session.id)
            .unwrap()
            .into_iter()
            .map(|r| (r.id, r.kind))
            .collect()
    };
    let rows_before = rows();
    let inbox_before = core.pending(&session.id).unwrap().len();
    let answer = runtime
        .block_on(core.aside(&session.id, "what is it doing?", &CancellationToken::new()))
        .unwrap();
    assert_eq!(answer, "It is sleeping.");
    assert_eq!(rows(), rows_before, "nothing written");
    assert_eq!(core.pending(&session.id).unwrap().len(), inbox_before);
    let aside = gateway.messages().into_iter().find(is_aside).unwrap();
    assert_paired(&aside);
    assert!(aside.body.contains("toolu_quick"), "the settled step goes");
    assert!(
        !aside.body.contains("toolu_slow"),
        "the running step does not"
    );
    assert!(
        aside.json()["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty())
    );
    let last = aside.json()["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert!(last.to_string().contains("what is it doing?"));
    core.interrupt(&session.id);
    let _ = runtime.block_on(turn).unwrap();
}

fn tokio_sleep(runtime: &tokio::runtime::Runtime, ms: u64) {
    runtime.block_on(async { tokio::time::sleep(Duration::from_millis(ms)).await });
}

#[test]
fn a_side_question_says_why_it_has_no_answer() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if !is_aside(request) {
            return text(request, "hi");
        }
        if request.body.contains("tool please") {
            return call(request, "toolu_x", "shell", json!({"command": "ls"}));
        }
        if request.body.contains("busy please") {
            return Reply::Json {
                status: 429,
                body: json!({"type": "error", "error": {"type": "rate_limit_error", "message": "Too many requests"}}),
                headers: vec![("x-should-retry".into(), "false".into())],
            };
        }
        if request.body.contains("slow please") {
            std::thread::sleep(Duration::from_secs(5));
        }
        text(request, "fine")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
        .unwrap();
    let ask = |question: &str| {
        runtime.block_on(core.aside(&session.id, question, &CancellationToken::new()))
    };
    assert_eq!(ask("fine?").unwrap(), "fine");
    let empty = ask("tool please").unwrap_err();
    assert_eq!(empty.kind, AsideKind::Empty);
    let busy = ask("busy please").unwrap_err();
    assert_eq!(busy.kind, AsideKind::RateLimit);
    assert!(!busy.message.is_empty());
    let missing = runtime
        .block_on(core.aside("ses_missing", "x", &CancellationToken::new()))
        .unwrap_err();
    assert_eq!(missing.kind, AsideKind::Other);
    // Cancelling ends it at once.
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    runtime.spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop.cancel();
    });
    let at = Instant::now();
    let cancelled = runtime
        .block_on(core.aside(&session.id, "slow please", &cancel))
        .unwrap_err();
    assert_eq!(cancelled.kind, AsideKind::Cancelled);
    assert!(at.elapsed() < Duration::from_secs(3), "{:?}", at.elapsed());
    assert_eq!(
        serde_json::to_value(&cancelled).unwrap()["kind"],
        "cancelled"
    );
    assert_eq!(
        serde_json::to_value(AsideKind::RateLimit).unwrap(),
        "rate_limit"
    );
}

/// Review 2 (R9): a question cancelled before it starts (the card closed while the folder's
/// core was starting) sends nothing and records nothing.
#[test]
fn a_side_question_cancelled_before_it_starts_makes_no_call() {
    let gateway = FakeGateway::start(|request, _| text(request, "hi"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
        .unwrap();
    let calls = gateway.messages().len();
    let spent = core.usage(&session.id).total;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = runtime
        .block_on(core.aside(&session.id, "what now?", &cancel))
        .unwrap_err();
    assert_eq!(error.kind, AsideKind::Cancelled);
    assert_eq!(gateway.messages().len(), calls, "no request");
    assert_eq!(core.usage(&session.id).total, spent, "no usage");
}
