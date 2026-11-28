//! Declined answers (`stop_reason: "refusal"`): the turn fails and says so, what the model wrote
//! before the refusal goes, and a declined prompt is never sent again, in sessions stored before
//! 1.6.1 too. One-shot calls (compaction, title, `/btw`, subagents) fail plainly as well.

mod common;

use anysola_core::api::{AsideKind, CancellationToken, Prompt, Target};
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

#[test]
fn a_call_that_started_before_the_refusal_keeps_its_result() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if last_prompt(request) == "user: go" {
            return Reply::Sse {
                body: Stream::new()
                    .text("Let me look.")
                    .tool("toolu_1", "glob", json!({"pattern": "*"}))
                    .refusal(Some("cyber"))
                    .finish(&model(request), Some(1)),
            };
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(
        failed(outcome)
            .1
            .contains("declined to continue, citing cybersecurity.")
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
    assert!(status == "completed" || status == "error", "{status}");
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
