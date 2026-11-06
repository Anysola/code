//! Step boundaries: what is selected again before each step, and how a step ends.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use anysola_core::api::{Prompt, Target};
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
        skills: Vec::new(),
    }
}

fn sse(request: &anysola_testkit::Recorded, stream: Stream) -> anysola_testkit::Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    anysola_testkit::Reply::Sse {
        body: stream.finish(&model, Some(1)),
    }
}

#[test]
fn config_changes_apply_at_the_next_step() {
    let project: Arc<OnceLock<std::path::PathBuf>> = Arc::default();
    let dir = project.clone();
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
            // Stands in for model time: FSEvents can take a few hundred ms to report the writes.
            std::thread::sleep(std::time::Duration::from_secs(1));
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
    let plan_dir = env.home.home().join(".anysola").join("plan");
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

#[test]
fn session_entries_close_block_two() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    assert!(
        core.set_instruction_entry(&session.id, "Bad/Key", Some(&json!(1)))
            .is_err()
    );
    core.set_instruction_entry(&session.id, "ticket", Some(&json!({"id": 7})))
        .unwrap();
    core.set_instruction_entry(&session.id, "a-note", Some(&json!("Be brief.")))
        .unwrap();
    let turn = |text: &str| {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap()
    };
    turn("one");
    core.set_instruction_entry(&session.id, "ticket", Some(&json!({"id": 8})))
        .unwrap();
    core.set_instruction_entry(&session.id, "a-note", None)
        .unwrap();
    turn("two");
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let block2 = main[0].json()["system"][2]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        block2.ends_with(
            "</env>\n\n<context key=\"a-note\">\nBe brief.\n</context>\n\n<context key=\"ticket\">\n{\n  \"id\": 7\n}\n</context>"
        ),
        "{block2}"
    );
    let second = transcript(&main[1]);
    assert_eq!(
        second[3],
        "system: The context under \"a-note\" no longer applies. Disregard it.\n\nThe context under \"ticket\" changed and supersedes the previous value:\n<context key=\"ticket\">\n{\n  \"id\": 8\n}\n</context>",
        "{second:?}"
    );
}

fn summarize(request: &anysola_testkit::Recorded) -> bool {
    request.body.contains("You MUST summarize")
}

#[test]
fn compaction_uses_the_compaction_agent_model_and_records_usage() {
    let gateway = FakeGateway::start(|request, _| {
        if summarize(request) {
            return text(request, "## Objective\n- x");
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    env.home.write(
        "proj/opencode.json",
        r#"{"agents": {"compaction": {"model": "anysola/claude-haiku-5-5"}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.select_model(
        &session.id,
        &anysola_core::model::ModelRef::parse("anysola/claude-sonnet-5-5").unwrap(),
    )
    .unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    let before = core.usage(&session.id).total;
    assert_eq!(
        runtime.block_on(core.compact(&session.id)).unwrap(),
        Outcome::Succeeded
    );
    let compaction = gateway.messages().into_iter().find(summarize).unwrap();
    assert_eq!(compaction.json()["model"], "claude-haiku-5-5");
    assert_eq!(core.usage(&session.id).total, before + 1.0);
}

#[test]
fn compaction_that_cannot_shrink_fails() {
    let gateway = FakeGateway::start(|request, _| {
        if summarize(request) {
            return anysola_testkit::Reply::Json {
                status: 400,
                body: json!({"type": "error", "error": {"type": "invalid_request_error", "message": "prompt is too long: 1000001 tokens > 1000000 maximum"}}),
                headers: Vec::new(),
            };
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    let Outcome::Failed(error) = runtime.block_on(core.compact(&session.id)).unwrap() else {
        panic!("compaction succeeded")
    };
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        (
            "compaction.failed",
            "The compaction request cannot be reduced further without losing the latest exchange or checkpoint"
        )
    );
}

#[test]
fn warming_pings_after_each_interval_until_the_duration_ends() {
    const PING: &str = "This is a keep-alive request. Do not perform any work or use tools. Reply with exactly: OK";
    let gateway = FakeGateway::start(|request, _| text(request, "OK"));
    let env = env(&gateway.url);
    env.home.write(
        "proj/opencode.json",
        r#"{"warming": {"interval": "300 millis", "duration": "1 seconds"}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
        .unwrap();
    let pings = || {
        gateway
            .messages()
            .into_iter()
            .filter(|r| {
                transcript(r)
                    .last()
                    .is_some_and(|t| t == &format!("user: {PING}"))
            })
            .collect::<Vec<_>>()
    };
    let wait = |ms| {
        runtime.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(ms)).await })
    };
    wait(1600);
    let sent = pings();
    assert!((2..=4).contains(&sent.len()), "{} pings", sent.len());
    // The keep-alive carries the conversation and the tools, and is not recorded.
    let ping = transcript(&sent[0]);
    assert_eq!(
        ping[..2],
        ["user: hello".to_string(), "assistant: OK".to_string()]
    );
    assert!(sent[0].json()["tools"].is_array());
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    assert!(
        db.messages(&session.id)
            .unwrap()
            .iter()
            .all(|m| !m.data.to_string().contains("keep-alive"))
    );
    wait(500);
    assert_eq!(pings().len(), sent.len(), "no pings after the duration");
}

#[test]
fn background_compaction_is_swapped_in_when_due() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if summarize(request) {
            return text(request, "## Objective\n- background");
        }
        let input = if request.body.contains("THIRD") {
            1_000
        } else if request.body.contains("SECOND") {
            900_000
        } else {
            800_000
        };
        sse(
            request,
            Stream::new()
                .usage(json!({"input_tokens": input, "output_tokens": 1}))
                .text("ok"),
        )
    });
    let env = env(&gateway.url);
    env.home.write(
        "proj/opencode.json",
        r#"{"experimental": {"background_compaction": true}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["FIRST", "SECOND", "THIRD"] {
        let outcome = runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
        assert_eq!(outcome, Outcome::Succeeded);
    }
    let summaries: Vec<_> = gateway.messages().into_iter().filter(summarize).collect();
    assert_eq!(summaries.len(), 1, "only the background summary ran");
    // It covered what came before the first response.
    assert_eq!(transcript(&summaries[0])[0], "user: FIRST");
    let last = gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title() && r.body.contains("THIRD"))
        .unwrap();
    let opening = transcript(&last);
    assert!(
        opening[0].contains("<summary>\n## Objective\n- background\n</summary>")
            && opening[0].contains("<recent-context>\n[Assistant]: ok\n\n[User]: SECOND"),
        "{opening:?}"
    );
}
