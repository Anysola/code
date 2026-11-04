//! Step boundaries: what is selected again before each step, and how a step ends.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::events::{Event, InterruptReason};
use anysola_core::session::gate::Reply;
use anysola_core::session::{Outcome, TurnOptions};
use anysola_core::storage::Db;
use anysola_testkit::FakeGateway;
use anysola_testkit::sse::Stream;
use common::*;
use serde_json::{Value, json};

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        files: vec![],
    }
}

fn sse(request: &anysola_testkit::Recorded, stream: Stream) -> anysola_testkit::Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    anysola_testkit::Reply::Sse {
        body: stream.finish(&model, Some(1)),
        before: Some(100),
    }
}

#[test]
fn config_changes_apply_at_the_next_step() {
    let core: Arc<OnceLock<Core>> = Arc::default();
    let project: Arc<OnceLock<std::path::PathBuf>> = Arc::default();
    let (handle, dir) = (core.clone(), project.clone());
    let steps = AtomicUsize::new(0);
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if steps.fetch_add(1, Ordering::SeqCst) == 0 {
            let dir = dir.get().unwrap();
            std::fs::write(
                dir.join("opencode.json"),
                r#"{"agents": {"build": {"system": "Reloaded prompt."}}}"#,
            )
            .unwrap();
            std::fs::write(dir.join("AGENTS.md"), "Reloaded rules.\n").unwrap();
            handle.get().unwrap().reload();
            return sse(
                request,
                Stream::new().tool("toolu_1", "glob", json!({"pattern": "*.md"})),
            );
        }
        text(request, "done")
    });
    let env = env(&gateway.url);
    project.set(env.home.project()).unwrap();
    let runtime = runtime();
    let started = runtime.block_on(common::core(&env));
    let _ = core.set(started.clone());
    let (session, _) = started.open(&Target::New, false).unwrap();
    runtime
        .block_on(started.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let requests: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(requests.len(), 2);
    let first = requests[0].json();
    let second = requests[1].json();
    assert_ne!(first["system"][0]["text"], "Reloaded prompt.");
    assert_eq!(second["system"][0]["text"], "Reloaded prompt.");
    let last = transcript(&requests[1]);
    assert!(
        last.iter()
            .any(|t| t.starts_with("system: Instructions from: ")
                && t.ends_with("Reloaded rules.\n")),
        "{last:?}"
    );
}

/// Two parallel glob calls that both ask; the first ask is answered with `reply`.
fn rejected(reply: Reply) -> (Vec<Value>, Outcome, usize) {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.body.contains("toolu_a") {
            return text(request, "after");
        }
        sse(
            request,
            Stream::new()
                .tool("toolu_a", "glob", json!({"pattern": "*.md"}))
                .tool("toolu_b", "glob", json!({"pattern": "*.rs"})),
        )
    });
    let env = env(&gateway.url);
    env.home.write(
        "proj/opencode.json",
        r#"{"permissions": [{"action": "glob", "resource": "*", "effect": "ask"}]}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let mut events = core.subscribe();
    let answering = core.clone();
    let answer = runtime.spawn(async move {
        let mut asked = Vec::new();
        while asked.len() < 2 {
            if let Ok(Event::PermissionAsked { request_id, .. }) = events.recv().await {
                asked.push(request_id);
            }
        }
        answering.reply_permission(&asked[0], reply);
    });
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    runtime.block_on(answer).unwrap();
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let assistants = db
        .messages(&session.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == "assistant")
        .map(|m| m.data)
        .collect();
    let requests = gateway.messages().iter().filter(|r| !r.is_title()).count();
    (assistants, outcome, requests)
}

#[test]
fn decline_without_message_ends_the_step() {
    let (assistants, outcome, requests) = rejected(Reply::Reject(None));
    assert_eq!(outcome, Outcome::Interrupted(InterruptReason::Declined));
    assert_eq!(requests, 1);
    let first = &assistants[0];
    assert_eq!(
        first["error"],
        json!({"type": "aborted", "message": "Step interrupted"})
    );
    for part in first["content"].as_array().unwrap() {
        assert_eq!(
            part["state"]["error"],
            json!({"type": "aborted", "message": "The user declined this tool call"})
        );
    }
}

#[test]
fn reject_with_message_continues() {
    let (assistants, outcome, requests) = rejected(Reply::Reject(Some("Use grep.".into())));
    assert_eq!(outcome, Outcome::Succeeded);
    assert_eq!(requests, 2);
    assert!(assistants[0].get("error").is_none());
    for part in assistants[0]["content"].as_array().unwrap() {
        assert_eq!(
            part["state"]["error"],
            json!({"type": "permission.rejected", "message": "Use grep."})
        );
    }
}

fn assistant_parts(env: &Env, session_id: &str) -> Vec<Vec<Value>> {
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    db.messages(session_id)
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == "assistant")
        .map(|m| m.data["content"].as_array().cloned().unwrap_or_default())
        .collect()
}

#[test]
fn provider_tool_calls_without_a_result() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        sse(
            request,
            Stream::new()
                .block(
                    json!({"type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {}}),
                    Some(r#"{"query":"a"}"#),
                )
                .block(
                    json!({"type": "server_tool_use", "id": "srvtoolu_2", "name": "web_search", "input": {"query": "b"}}),
                    None,
                )
                .block(
                    json!({"type": "web_search_tool_result", "tool_use_id": "srvtoolu_2", "content": [{"type": "web_search_result", "url": "https://x"}]}),
                    None,
                )
                .text("answer"),
        )
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let parts = &assistant_parts(&env, &session.id)[0];
    assert_eq!(parts[0]["executed"], true);
    assert_eq!(parts[0]["state"]["status"], "error");
    assert_eq!(
        parts[0]["state"]["error"],
        json!({"type": "tool.result-missing", "message": "Provider did not return a tool result"})
    );
    assert_eq!(parts[1]["state"]["status"], "completed");
    assert_eq!(parts[1]["state"]["input"], json!({"query": "b"}));
    assert_eq!(
        parts[1]["state"]["content"],
        json!([{"type": "text", "text": r#"[{"type":"web_search_result","url":"https://x"}]"#}])
    );
    assert_eq!(parts[1]["native"]["blockType"], "web_search_tool_result");
    let requests = gateway.messages().iter().filter(|r| !r.is_title()).count();
    assert_eq!(requests, 1, "provider-executed calls need no further step");
}

#[test]
fn malformed_and_cut_off_tool_input_is_not_executed() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.body.contains("toolu_bad") {
            return text(request, "fixed");
        }
        sse(
            request,
            Stream::new()
                .block(
                    json!({"type": "tool_use", "id": "toolu_bad", "name": "glob", "input": {}}),
                    Some(r#"{"pattern": "*.md""#),
                )
                .stop("tool_use"),
        )
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let parts = &assistant_parts(&env, &session.id)[0];
    assert_eq!(
        parts[0]["state"]["error"],
        json!({"type": "tool.input-json", "message": "The tool input could not be parsed as JSON, so the call was not executed. Reissue the call with valid JSON arguments.\n\nInput excerpt (first 18 of 18 characters):\n{\"pattern\": \"*.md\""})
    );
    let requests = gateway.messages().iter().filter(|r| !r.is_title()).count();
    assert_eq!(requests, 2, "the model sees the failed call and continues");
}

#[test]
fn plan_mode_reminder_survives_compaction_and_guards_edits() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.body.contains("You MUST summarize") {
            return text(request, "## Objective\n- plan");
        }
        if request.body.contains("PLANSECOND") && !request.body.contains("toolu_w") {
            return sse(
                request,
                Stream::new().tool(
                    "toolu_w",
                    "write",
                    json!({"path": "notes.md", "content": "x"}),
                ),
            );
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.select_agent(&session.id, "plan").unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    runtime.block_on(core.compact(&session.id)).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("PLANSECOND"), TurnOptions::default()))
        .unwrap();
    let plan_dir = env.home.home().join(".opencode/plan");
    let enter = format!(
        "user: <system-reminder>\nYou are in Plan mode. Discuss the plan with the user directly in the conversation. Do not create or update plan files unless the user explicitly asks you to; when they do, write them only in:\n{}\n",
        plan_dir.display()
    );
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title() && !r.body.contains("You MUST summarize"))
        .collect();
    let after = transcript(&main[1]);
    assert!(
        after[0].starts_with("user: <conversation-checkpoint>"),
        "{after:?}"
    );
    assert!(after[1].starts_with(&enter), "{after:?}");
    assert_eq!(after[2], "user: PLANSECOND");
    // Recorded after the prompt, so the next request finds it in the history and adds no other.
    let next = transcript(&main[2]);
    assert!(
        next[1] == "user: PLANSECOND" && next[2].starts_with(&enter),
        "{next:?}"
    );
    assert_eq!(next.len(), 5, "{next:?}");
    let parts = assistant_parts(&env, &session.id);
    let write = parts
        .iter()
        .flatten()
        .find(|p| p["name"] == "write")
        .unwrap();
    assert_eq!(
        write["state"]["error"],
        json!({"type": "tool.execution", "message": format!("Cannot use write to modify files outside the Plan directory: {}", plan_dir.display())})
    );
}
