//! Declined answers (`stop_reason: "refusal"`): the turn fails and says so, what the model wrote
//! before the refusal goes, and a declined prompt is never sent again, in sessions stored before
//! 1.6.1 too. One-shot calls (compaction, title, `/btw`, subagents) fail plainly as well.

mod common;

use anysola_core::api::{AsideKind, CancellationToken, Prompt, Target};
use anysola_core::events::Event;
use anysola_core::session::gate::{Floor, Reply as Answer};
use anysola_core::session::{Outcome, TurnOptions};
use anysola_core::storage::Db;
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

fn model(request: &Recorded) -> String {
    request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The model declines: after `written` (text it streamed first), with `category`.
fn refuse(request: &Recorded, written: Option<&str>, category: Option<&str>) -> Reply {
    let stream = match written {
        Some(text) => Stream::new().text(text),
        None => Stream::new(),
    };
    Reply::Sse {
        body: stream.refusal(category).finish(&model(request), Some(1)),
    }
}

fn call(request: &Recorded, id: &str, name: &str, input: Value) -> Reply {
    Reply::Sse {
        body: Stream::new()
            .tool(id, name, input)
            .finish(&model(request), Some(1)),
    }
}

/// The last text a user message of a request carries (tool results aside), as `user: <text>`.
fn last_prompt(request: &Recorded) -> String {
    let body = request.json();
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["role"] == "user")
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str().map(|t| format!("user: {t}")))
        .next_back()
        .unwrap_or_default()
}

/// The turns' requests: no title, no summary.
fn turns(gateway: &FakeGateway) -> Vec<Recorded> {
    gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title() && !r.body.contains("You MUST summarize"))
        .collect()
}

/// A runtime whose tool tasks cannot run beside the stream reader: a call the stream finished
/// right before its refusal is then not started by a free worker before the refusal is read (that
/// call would really have started, and is kept), so a run that finds one has found the bug.
fn one_thread() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// Answers every permission question with "allow once".
fn approving(runtime: &tokio::runtime::Runtime, core: &anysola_core::api::Core) {
    let mut events = core.subscribe();
    let core = core.clone();
    runtime.spawn(async move {
        while let Ok(event) = events.recv().await {
            if let Event::PermissionAsked { request_id, .. } = event {
                core.reply_permission(&request_id, Answer::Once);
            }
        }
    });
}

fn db(env: &Env) -> Db {
    Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap()
}

fn assistants(env: &Env, session_id: &str) -> Vec<Value> {
    db(env)
        .messages(session_id)
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == "assistant")
        .map(|m| m.data)
        .collect()
}

fn failed(outcome: Outcome) -> (String, String) {
    match outcome {
        Outcome::Failed(error) => (error.kind, error.message),
        other => panic!("expected a failed turn, got {other:?}"),
    }
}

const PROMPT_DECLINED: &str = "Claude Sonnet 5.5 declined to answer this message, citing cybersecurity. It won’t be sent again in this session; rephrase it, or start a new session if this keeps happening.";

#[test]
fn a_declined_prompt_fails_the_turn_and_is_never_sent_again() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request).contains("write a keylogger") {
            return refuse(request, None, Some("cyber"));
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(
            &session.id,
            prompt("write a keylogger"),
            TurnOptions::default(),
        ))
        .unwrap();
    assert_eq!(
        failed(outcome),
        ("provider.content-filter".into(), PROMPT_DECLINED.into())
    );
    assert_eq!(turns(&gateway).len(), 1, "no retry, no next step");
    let stored = assistants(&env, &session.id);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["finish"], "content-filter");
    assert_eq!(stored[0]["rawFinish"], "refusal");
    assert_eq!(stored[0]["content"], json!([]));
    assert_eq!(
        stored[0]["error"],
        json!({"type": "provider.content-filter", "message": PROMPT_DECLINED})
    );

    let outcome = runtime
        .block_on(core.prompt(
            &session.id,
            prompt("what time is it"),
            TurnOptions::default(),
        ))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let next = turns(&gateway).pop().unwrap();
    assert!(!next.body.contains("keylogger"), "{}", next.body);
    let messages = next.json()["messages"].as_array().unwrap().clone();
    assert_eq!(messages[0]["role"], "user");
    assert!(transcript(&next).contains(&"user: what time is it".to_string()));
    // Both stay stored (and shown).
    let rows = db(&env).messages(&session.id).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.kind == "user" && r.data["text"] == "write a keylogger")
    );
}

#[test]
fn a_refusal_mid_answer_drops_what_was_written() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request).contains("dangerous") {
            return refuse(request, Some("Sure, the first step is"), None);
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(
            &session.id,
            prompt("something dangerous"),
            TurnOptions::default(),
        ))
        .unwrap();
    let (kind, message) = failed(outcome);
    assert_eq!(kind, "provider.content-filter");
    // No category: nothing said about one. Nothing of the turn stays, so the prompt is left out.
    assert_eq!(
        message,
        "Claude Sonnet 5.5 declined to answer this message. It won’t be sent again in this session; rephrase it, or start a new session if this keeps happening."
    );
    let stored = assistants(&env, &session.id);
    assert_eq!(stored[0]["content"], json!([]), "the partial text is gone");
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(!next.body.contains("Sure, the first step"));
    assert!(!next.body.contains("something dangerous"));
}

#[test]
fn a_refusal_after_a_tool_step_keeps_the_earlier_steps() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let results = request.body.matches("\"tool_result\"").count();
        match (last_prompt(request).as_str(), results) {
            ("user: look around", 0) => call(request, "toolu_1", "glob", json!({"pattern": "*"})),
            ("user: look around", _) => refuse(request, None, Some("bio")),
            _ => text(request, "ok"),
        }
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("look around"), TurnOptions::default()))
        .unwrap();
    assert_eq!(
        failed(outcome).1,
        "Claude Sonnet 5.5 declined to continue, citing biology. Something earlier in this turn may have caused it, so later messages in this session may be declined too; if they are, start a new session."
    );
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    let body = next.json();
    let blocks: Vec<Value> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .collect();
    assert!(next.body.contains("look around"), "the prompt stays");
    assert!(
        blocks
            .iter()
            .any(|b| b["type"] == "tool_use" && b["id"] == "toolu_1")
    );
    assert!(
        blocks
            .iter()
            .any(|b| b["type"] == "tool_result" && b["tool_use_id"] == "toolu_1")
    );
    assert_eq!(transcript(&next).last().unwrap(), "user: next");
}

/// A stream that sends everything up to its `message_delta`, waits, then ends. The refusal is in
/// the first part when `refused` is built into `stream` before the split point.
fn held(stream: String, split: &str, hold_ms: u64) -> Reply {
    let at = stream.find(split).expect("split point in the stream");
    Reply::SseHeld {
        head: stream[..at].to_string(),
        hold_ms,
        tail: stream[at..].to_string(),
    }
}

/// A call that was running when the refusal arrived keeps its `tool_use` and its result: it ran.
#[test]
fn a_call_that_started_before_the_refusal_keeps_its_result() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request) == "user: go" {
            // The call is complete and starts; the refusal comes a moment later.
            let stream = Stream::new()
                .text("Let me look.")
                .tool(
                    "toolu_1",
                    "shell",
                    json!({"command": "echo ran > started-marker; sleep 30", "description": "slow"}),
                )
                .refusal(Some("cyber"))
                .finish(&model(request), Some(1));
            return held(stream, "event: message_delta", 1500);
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    // The call asks first and is approved: that is when it begins.
    approving(&runtime, &core);
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.set_permission_floor(&session.id, Floor::Commands)
        .unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(
        failed(outcome)
            .1
            .contains("declined to continue, citing cybersecurity.")
    );
    assert!(
        env.home.project().join("started-marker").exists(),
        "the call really started"
    );
    assert_eq!(turns(&gateway).len(), 1, "no step follows the call");
    let stored = assistants(&env, &session.id);
    let content = stored[0]["content"].as_array().unwrap();
    assert_eq!(
        content.len(),
        1,
        "the text went, the call stayed: {content:?}"
    );
    assert_eq!(content[0]["type"], "tool");
    let status = content[0]["state"]["status"].as_str().unwrap();
    assert_eq!(status, "error", "stopped while it ran");
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(!next.body.contains("Let me look."));
    assert!(next.body.contains("\"tool_use_id\":\"toolu_1\""));
    assert!(
        transcript(&next).contains(&"user: go".to_string()),
        "the prompt stays"
    );
}

/// A call still waiting for permission when the refusal arrives has not begun: it goes with its
/// prompt like any other call that never started.
#[test]
fn a_call_waiting_for_permission_is_dropped_with_its_prompt() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request) == "user: go" {
            // The call starts and asks; the refusal comes while the question is open.
            let stream = Stream::new()
                .tool(
                    "toolu_1",
                    "shell",
                    json!({"command": "echo ran > asked-marker", "description": "asks"}),
                )
                .refusal(Some("cyber"))
                .finish(&model(request), Some(1));
            return held(stream, "event: message_delta", 1000);
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    // Nobody answers the question.
    core.set_permission_floor(&session.id, Floor::Commands)
        .unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(failed(outcome).1, PROMPT_DECLINED);
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!env.home.project().join("asked-marker").exists());
    let stored = assistants(&env, &session.id);
    assert!(stored[0]["content"].as_array().unwrap().is_empty());
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(!next.body.contains("asked-marker"), "{}", next.body);
    assert_eq!(transcript(&next), vec!["user: next".to_string()]);
}

/// A call whose block closed in the same read as the refusal never starts, and nothing of it is
/// kept: the step is a declined prompt (skipped with its prompt).
#[test]
fn a_call_that_never_started_is_dropped_with_its_prompt() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request) == "user: go" {
            return Reply::Sse {
                body: Stream::new()
                    .tool(
                        "toolu_1",
                        "shell",
                        json!({"command": "echo ran > flagged-marker", "description": "flagged"}),
                    )
                    .refusal(Some("cyber"))
                    .finish(&model(request), Some(1)),
            };
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = one_thread();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(failed(outcome).1, PROMPT_DECLINED);
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!env.home.project().join("flagged-marker").exists());
    let stored = assistants(&env, &session.id);
    assert!(stored[0]["content"].as_array().unwrap().is_empty());
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(!next.body.contains("flagged-marker"), "{}", next.body);
    assert!(!next.body.contains("toolu_1"));
    assert_eq!(transcript(&next), vec!["user: next".to_string()]);
}

/// No tool call of a refused step starts after the refusal: a background subagent's child (its
/// own cancel token; it starts on the tool's first poll) never sends a request.
#[test]
fn a_refused_steps_background_subagent_never_starts_in_40_runs() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.header("x-opencode-parent-session-id").is_some() {
            return text(request, "child ran");
        }
        let last = last_prompt(request);
        if last.starts_with("user: go ") {
            return Reply::Sse {
                body: Stream::new()
                    .tool(
                        "toolu_1",
                        "subagent",
                        json!({"agent": "general", "description": "Do it", "prompt": "child task from a declined step", "background": true}),
                    )
                    .refusal(Some("cyber"))
                    .finish(&model(request), Some(1)),
            };
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = one_thread();
    let core = runtime.block_on(common::core(&env));
    let runs = 40;
    for i in 0..runs {
        let (session, _) = core.open(&Target::New, false).unwrap();
        let outcome = runtime
            .block_on(core.prompt(
                &session.id,
                prompt(&format!("go {i}")),
                TurnOptions::default(),
            ))
            .unwrap();
        assert_eq!(failed(outcome).1, PROMPT_DECLINED, "run {i}");
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let children = gateway
        .messages()
        .into_iter()
        .filter(|r| r.header("x-opencode-parent-session-id").is_some())
        .count();
    assert_eq!(children, 0, "{children} child request(s) in {runs} runs");
}

/// The same for a shell call: it never starts, so no file appears.
#[test]
fn a_refused_steps_shell_call_never_starts_in_40_runs() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let last = last_prompt(request);
        if let Some(name) = last.strip_prefix("user: go ") {
            return Reply::Sse {
                body: Stream::new()
                    .tool(
                        "toolu_1",
                        "shell",
                        json!({"command": format!("echo ran > {name}"), "description": "touch"}),
                    )
                    .refusal(Some("cyber"))
                    .finish(&model(request), Some(1)),
            };
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = one_thread();
    let core = runtime.block_on(common::core(&env));
    let mut ran = Vec::new();
    for i in 0..40 {
        let (session, _) = core.open(&Target::New, false).unwrap();
        let name = format!("marker-{i}");
        let outcome = runtime
            .block_on(core.prompt(
                &session.id,
                prompt(&format!("go {name}")),
                TurnOptions::default(),
            ))
            .unwrap();
        assert_eq!(failed(outcome).1, PROMPT_DECLINED, "run {i}");
        std::thread::sleep(std::time::Duration::from_millis(100));
        if env.home.project().join(&name).exists() {
            ran.push(i);
        }
    }
    assert!(ran.is_empty(), "shell calls ran in runs {ran:?}");
}

/// Stop after the refusal arrived (the stream still open) is a declined answer, not an
/// interruption: nothing of the step is kept.
#[test]
fn a_stop_after_the_refusal_still_drops_the_step() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request) == "user: go" {
            let stream = Stream::new()
                .text("Sure, first")
                .refusal(Some("cyber"))
                .finish(&model(request), Some(1));
            return held(stream, "event: message_stop", 4000);
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let turn = {
        let core = core.clone();
        let id = session.id.clone();
        runtime.spawn(async move { core.prompt(&id, prompt("go"), TurnOptions::default()).await })
    };
    // The request is in; its body (text and the refusal) is read at once, the end is held.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while turns(&gateway).is_empty() {
        assert!(std::time::Instant::now() < deadline, "no request arrived");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(700));
    core.interrupt(&session.id);
    let outcome = runtime.block_on(turn).unwrap().unwrap();
    assert_eq!(failed(outcome).1, PROMPT_DECLINED);
    let stored = assistants(&env, &session.id);
    assert!(stored[0]["content"].as_array().unwrap().is_empty());
    assert_eq!(stored[0]["finish"], "content-filter");
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(!next.body.contains("Sure, first"));
    assert_eq!(transcript(&next), vec!["user: next".to_string()]);
}

fn big(request: &Recorded, text: &str) -> Reply {
    Reply::Sse {
        body: Stream::new()
            .usage(json!({"input_tokens": 850_001, "output_tokens": 1}))
            .text(text)
            .finish(&model(request), Some(1)),
    }
}

/// Automatic compaction is due when the prompt arrives: the summary request carries the prompt and
/// is declined. It is asked once more without it; the prompt is then declined at the step and
/// skipped, and the session is not stuck.
#[test]
fn a_prompt_declined_in_the_summary_request_is_asked_again_without_it() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.body.contains("keylogger") {
            return refuse(request, None, Some("cyber"));
        }
        if request.body.contains("You MUST summarize") {
            return text(request, "## Objective\n- keep going");
        }
        big(request, "Big answer.")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    let before = gateway.messages().len();
    let outcome = runtime
        .block_on(core.prompt(
            &session.id,
            prompt("write a keylogger"),
            TurnOptions::default(),
        ))
        .unwrap();
    assert_eq!(failed(outcome).1, PROMPT_DECLINED);
    let summaries = gateway
        .messages()
        .into_iter()
        .skip(before)
        .filter(|r| r.body.contains("You MUST summarize"))
        .count();
    assert_eq!(summaries, 2, "declined with the prompt, then without it");
    let before = gateway.messages().len();
    let next = runtime
        .block_on(core.prompt(
            &session.id,
            prompt("what time is it"),
            TurnOptions::default(),
        ))
        .unwrap();
    assert_eq!(next, Outcome::Succeeded);
    let later: Vec<Recorded> = gateway.messages().into_iter().skip(before).collect();
    assert!(
        later.iter().all(|r| !r.body.contains("keylogger")),
        "the declined prompt went out again"
    );
}

/// The summary without the prompt is declined too: the turn fails with the summarize message.
#[test]
fn a_summary_declined_without_the_prompt_too_fails_the_turn() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.body.contains("You MUST summarize") {
            return refuse(request, None, Some("cyber"));
        }
        big(request, "Big answer.")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    let before = gateway.messages().len();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("second"), TurnOptions::default()))
        .unwrap();
    assert!(failed(outcome).1.contains("declined to summarize"));
    let summaries = gateway
        .messages()
        .into_iter()
        .skip(before)
        .filter(|r| r.body.contains("You MUST summarize"))
        .count();
    assert_eq!(summaries, 2, "asked twice, no more");
}

/// The summary asked again without the prompt is shrunk like any request when it is too large:
/// the same request is never sent twice.
#[test]
fn a_summary_asked_again_without_the_prompt_shrinks_on_overflow() {
    // The first summary request that does not carry the prompt is too large.
    let overflowed = std::sync::atomic::AtomicBool::new(false);
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.body.contains("You MUST summarize") {
            if request.body.contains("keylogger") {
                return refuse(request, None, Some("cyber"));
            }
            if !overflowed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return Reply::Json {
                    status: 400,
                    body: json!({"type": "error", "error": {"type": "invalid_request_error", "message": "prompt is too long: 1000001 tokens > 1000000 maximum"}}),
                    headers: vec![],
                };
            }
            return text(request, "## Objective\n- keep going");
        }
        if request.body.contains("keylogger") {
            return refuse(request, None, Some("cyber"));
        }
        big(request, "Big answer.")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("alpha"), TurnOptions::default()))
        .unwrap();
    let before = gateway.messages().len();
    runtime
        .block_on(core.prompt(
            &session.id,
            prompt("write a keylogger"),
            TurnOptions::default(),
        ))
        .unwrap();
    let summaries: Vec<String> = gateway
        .messages()
        .into_iter()
        .skip(before)
        .filter(|r| r.body.contains("You MUST summarize"))
        .map(|r| r.body)
        .collect();
    assert!(summaries.len() >= 3, "{} summary requests", summaries.len());
    let mut distinct = summaries.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        summaries.len(),
        "the same summary request was sent twice"
    );
}

/// A summary written in the declined turn saw the prompt: it goes with the prompt, and the next
/// step summarizes again without it.
#[test]
fn a_compaction_written_in_the_declined_turn_is_left_out_with_the_prompt() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.body.contains("You MUST summarize") {
            // A summary names the request the conversation ends on.
            let latest = request.json()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "user")
                .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
                .filter_map(|b| b["text"].as_str().map(str::to_string))
                .rfind(|t| !t.starts_with("You MUST summarize"))
                .unwrap_or_default();
            return text(
                request,
                &format!("## Objective\n- The user's latest request: {latest}"),
            );
        }
        if last_prompt(request).contains("keylogger") {
            return refuse(request, None, Some("cyber"));
        }
        big(request, "Big answer.")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    let outcome = runtime
        .block_on(core.prompt(
            &session.id,
            prompt("write a keylogger"),
            TurnOptions::default(),
        ))
        .unwrap();
    assert_eq!(failed(outcome).1, PROMPT_DECLINED);
    let before = gateway.messages().len();
    let next = runtime
        .block_on(core.prompt(
            &session.id,
            prompt("what time is it"),
            TurnOptions::default(),
        ))
        .unwrap();
    assert_eq!(next, Outcome::Succeeded);
    let later: Vec<Recorded> = gateway.messages().into_iter().skip(before).collect();
    assert!(
        later.iter().all(|r| !r.body.contains("keylogger")),
        "the prompt went out again, in a summary or the kept text"
    );
    // What came before the prompt stays: history does not start at the left-out summary.
    let summary = later
        .iter()
        .find(|r| r.body.contains("You MUST summarize"))
        .expect("the next turn summarizes again");
    assert!(summary.body.contains("first"));
    assert!(summary.body.contains("Big answer."));
}

#[test]
fn a_declined_summary_leaves_the_history_as_it_was() {
    let gateway = FakeGateway::start(|request, _| {
        if request.body.contains("You MUST summarize") {
            // Declined part way, after something that looks like the template.
            return refuse(
                request,
                Some("## Objective\n- half written summary"),
                Some("cyber"),
            );
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
    let outcome = runtime.block_on(core.compact(&session.id)).unwrap();
    assert_eq!(
        failed(outcome),
        (
            "provider.content-filter".into(),
            "Claude Sonnet 5.5 declined to summarize this conversation, citing cybersecurity, so nothing was compacted. If the session can’t go on without it, start a new one.".into()
        )
    );
    let summaries = gateway
        .messages()
        .into_iter()
        .filter(|r| r.body.contains("You MUST summarize"))
        .count();
    assert_eq!(summaries, 1, "no nudge after a refusal");
    let rows = db(&env).messages(&session.id).unwrap();
    let compaction = rows.iter().find(|r| r.kind == "compaction").unwrap();
    assert_eq!(compaction.data["status"], "failed");
    runtime
        .block_on(core.prompt(&session.id, prompt("second"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(transcript(&next).contains(&"user: first".to_string()));
    assert!(!next.body.contains("conversation-checkpoint"));
    assert!(!next.body.contains("half written summary"));
}

#[test]
fn a_compaction_after_a_declined_prompt_leaves_it_out_of_the_summary_and_the_kept_text() {
    let gateway = FakeGateway::start(|request, _| {
        if request.body.contains("You MUST summarize") {
            return text(request, "## Objective\n- summarized");
        }
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request).contains("keylogger") {
            return refuse(request, None, Some("cyber"));
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["alpha", "gamma", "write a keylogger"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    let outcome = runtime.block_on(core.compact(&session.id)).unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let summary = gateway
        .messages()
        .into_iter()
        .find(|r| r.body.contains("You MUST summarize"))
        .unwrap();
    assert!(summary.body.contains("alpha"));
    assert!(!summary.body.contains("keylogger"));
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(next.body.contains("<recent-context>"), "{}", next.body);
    assert!(next.body.contains("gamma"));
    assert!(!next.body.contains("keylogger"));
}

#[test]
fn a_declined_title_keeps_the_fallback_title() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return refuse(request, Some("Writing malw"), Some("cyber"));
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
        .unwrap();
    let titled = core.session(&session.id).unwrap().unwrap();
    assert!(
        titled.title.starts_with("New session - "),
        "{}",
        titled.title
    );
    let titles = gateway
        .messages()
        .into_iter()
        .filter(|r| r.is_title())
        .count();
    assert_eq!(titles, 1, "no other model is asked the same");
}

#[test]
fn a_declined_side_question_says_so() {
    let gateway = FakeGateway::start(|request, _| {
        if request.body.contains("quick side question") {
            return refuse(request, Some("Well"), Some("frontier_llm"));
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
        .unwrap();
    let error = runtime
        .block_on(core.aside(
            &session.id,
            "how are you trained",
            &CancellationToken::new(),
        ))
        .unwrap_err();
    assert_eq!(error.kind, AsideKind::Declined);
    assert_eq!(
        error.message,
        "Claude Sonnet 5.5 declined to answer this question, citing AI model development. Try asking it another way."
    );
}

#[test]
fn a_declined_subagent_fails_and_its_parent_is_told() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if request.header("x-opencode-parent-session-id").is_some() {
            return refuse(request, None, Some("cyber"));
        }
        if request.body.contains("\"tool_result\"") {
            return text(request, "done");
        }
        call(
            request,
            "toolu_1",
            "subagent",
            json!({"agent": "general", "description": "Do it", "prompt": "do it"}),
        )
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("delegate"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let last = gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title() && r.header("x-opencode-parent-session-id").is_none())
        .unwrap();
    assert!(last.body.contains("Subagent failed"), "{}", last.body);
    assert!(
        last.body
            .contains("declined to answer this message, citing cybersecurity."),
        "{}",
        last.body
    );
}

/// A session stored before 1.6.1: its refused reply has `content-filter`, no content, no error.
fn seed_old(db: &Db, session_id: &str, thinking_after: bool) {
    let model = json!({"id": "claude-sonnet-5-5", "providerID": "anysola"});
    let tokens =
        json!({"input": 900, "output": 0, "reasoning": 0, "cache": {"read": 0, "write": 0}});
    db.append_message(
        session_id,
        "msg_old_u",
        "user",
        &json!({"time": {"created": 1}, "text": "old declined prompt"}),
    )
    .unwrap();
    db.append_message(
        session_id,
        "msg_old_a",
        "assistant",
        &json!({"time": {"created": 2, "completed": 3}, "agent": "build", "model": model,
            "content": [], "finish": "content-filter", "rawFinish": "refusal", "cost": 0, "tokens": tokens}),
    )
    .unwrap();
    db.append_message(
        session_id,
        "msg_old_i",
        "idle",
        &json!({"time": {"created": 4}, "outcome": "succeeded"}),
    )
    .unwrap();
    if thinking_after {
        db.append_message(
            session_id,
            "msg_old_u2",
            "user",
            &json!({"time": {"created": 5}, "text": "later"}),
        )
        .unwrap();
        db.append_message(
            session_id,
            "msg_old_a2",
            "assistant",
            &json!({"time": {"created": 6, "completed": 7}, "agent": "build", "model": model,
                "content": [
                    {"type": "reasoning", "text": "thought", "native": {"signature": "sig_old"}, "time": {"created": 6, "completed": 6}},
                    {"type": "text", "text": "later answer"}],
                "finish": "stop", "rawFinish": "end_turn", "cost": 0, "tokens": tokens}),
        )
        .unwrap();
        db.append_message(
            session_id,
            "msg_old_i2",
            "idle",
            &json!({"time": {"created": 8}, "outcome": "succeeded"}),
        )
        .unwrap();
    }
}

#[test]
fn a_declined_prompt_stored_before_1_6_1_is_skipped_and_stays_skipped() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    seed_old(&db(&env), &session.id, false);
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(!next.body.contains("old declined prompt"), "{}", next.body);
    // Marked once left out: the steps written without it may carry thinking from now on.
    let rows = db(&env).messages(&session.id).unwrap();
    let old = rows.iter().find(|r| r.id == "msg_old_a").unwrap();
    assert_eq!(old.data["declined"], true);
    assert!(
        old.data.get("error").is_none(),
        "nothing new to show for it"
    );
}

#[test]
fn a_declined_prompt_stored_before_1_6_1_stays_when_signed_thinking_followed_it() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    seed_old(&db(&env), &session.id, true);
    runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    // The later answer's thinking was written with it in the request: the history before that
    // block stays as it was sent.
    assert!(next.body.contains("old declined prompt"), "{}", next.body);
    assert!(next.body.contains("sig_old"));
    let rows = db(&env).messages(&session.id).unwrap();
    let old = rows.iter().find(|r| r.id == "msg_old_a").unwrap();
    assert!(old.data.get("declined").is_none());
}

#[test]
fn an_exported_session_keeps_its_declined_prompt_out_once_imported() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request).contains("keylogger") {
            return refuse(request, None, Some("cyber"));
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["hello", "write a keylogger"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    let file = core.export(&session.id).unwrap().unwrap();
    let row = runtime
        .block_on(core.import_session(file.to_string().into_bytes(), &CancellationToken::new()))
        .unwrap();
    let imported = assistants(&env, &row.id);
    assert!(
        imported
            .iter()
            .any(|a| a["finish"] == "content-filter" && a["content"] == json!([])),
        "{imported:?}"
    );
    runtime
        .block_on(core.prompt(&row.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    let next = turns(&gateway).pop().unwrap();
    assert!(next.body.contains("hello"));
    assert!(!next.body.contains("keylogger"), "{}", next.body);
}
