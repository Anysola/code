//! `/goal`: kickoff, the reviewer at each idle, continuations within one execution, stalls,
//! unusable verdicts, a reviewer overflow with its compaction and the reminder, and the controls.

mod common;

use std::sync::Mutex;

use anysola_core::api::Target;
use anysola_core::session::goal::Status;
use anysola_core::session::{Outcome, TurnOptions};
use anysola_core::storage::Db;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::json;

const REVIEWER: &str = "You are now the reviewer for the session's goal.";

fn reviewing(request: &Recorded) -> bool {
    request.body.contains(REVIEWER)
}

fn last_text(request: &Recorded) -> String {
    transcript(request).last().cloned().unwrap_or_default()
}

fn tool_call(request: &Recorded) -> Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Reply::Sse {
        body: Stream::new()
            .usage(json!({"input_tokens": 10, "output_tokens": 1}))
            .tool("toolu_1", "glob", json!({"pattern": "*.md"}))
            .finish(&model, Some(1)),
        before: Some(100),
    }
}

fn goal(
    runtime: &tokio::runtime::Runtime,
    core: &anysola_core::api::Core,
    session: &str,
    arguments: &str,
) -> Outcome {
    runtime
        .block_on(core.command(session, "goal", arguments, TurnOptions::default()))
        .unwrap()
}

#[test]
fn reviewer_continues_then_completes_in_one_execution() {
    let verdicts = Mutex::new(vec![
        r#"{"verdict":"done","reason":"tests pass"}"#,
        r#"Sure: {"verdict":"continue","reason":"no tests yet","next_step":"run them"}"#,
    ]);
    let gateway = FakeGateway::start(move |request, _| {
        if reviewing(request) {
            let verdict = verdicts.lock().unwrap().pop().unwrap();
            return text(request, verdict);
        }
        let body = request.json();
        let last = &body["messages"].as_array().unwrap().last().unwrap()["content"][0];
        if last["type"] == "tool_result" {
            text(request, "done")
        } else {
            tool_call(request)
        }
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.command(&session.id, "goal", "\"Ship <it>\"", TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);

    let state = core.goal(&session.id).unwrap();
    assert_eq!(state.status, Status::Complete);
    assert_eq!(state.reason.as_deref(), Some("tests pass"));
    assert_eq!(state.rounds, 1);
    assert_eq!(
        core.session(&session.id).unwrap().unwrap().title,
        "Ship <it>"
    );

    let requests: Vec<Recorded> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let kickoff = transcript(&requests[0]).join("\n");
    assert!(kickoff.contains("A goal is now active for this session."));
    assert!(kickoff.contains("<untrusted_objective>\nShip &lt;it&gt;\n</untrusted_objective>"));
    let judge = last_text(&requests[2]);
    assert!(judge.starts_with(&format!("user: Stop acting as the assistant. {REVIEWER}")));
    assert!(judge.contains("No background shells are running."));
    let continued = transcript(&requests[3]).join("\n");
    assert!(continued.contains(
        "</untrusted_objective>\n\nno tests yet\nNext step: run them\n\nTake the next concrete step."
    ));
    assert_eq!(requests.iter().filter(|r| reviewing(r)).count(), 2);

    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let rows = db.messages(&session.id).unwrap();
    assert_eq!(rows.iter().filter(|r| r.kind == "idle").count(), 1);
    let kickoff_row = rows.iter().find(|r| r.kind == "synthetic").unwrap();
    assert_eq!(kickoff_row.data["description"], "Goal: Ship <it>");
    assert_eq!(kickoff_row.data["metadata"]["source"], "goal");
    assert_eq!(kickoff_row.data["metadata"]["goalID"], json!(state.id));

    assert_eq!(
        goal(&runtime, &core, &session.id, "clear"),
        Outcome::Succeeded
    );
    assert!(core.goal(&session.id).is_none());
}

#[test]
fn rounds_without_tools_stall_and_resume_restarts() {
    let gateway = FakeGateway::start(|request, _| {
        if reviewing(request) {
            let verdict = if transcript(request).iter().any(|t| t.contains("resumed")) {
                r#"{"verdict":"blocked","reason":"needs a key"}"#
            } else {
                r#"{"verdict":"continue","reason":"Not yet"}"#
            };
            return text(request, verdict);
        }
        text(request, "thinking")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    assert_eq!(
        goal(&runtime, &core, &session.id, "Do it"),
        Outcome::Succeeded
    );
    let state = core.goal(&session.id).unwrap();
    assert_eq!(
        (state.status, state.reason.as_deref(), state.rounds),
        (Status::Paused, Some("Not making progress: Not yet"), 2)
    );
    let judged = || gateway.messages().into_iter().filter(reviewing).count();
    assert_eq!(judged(), 3);

    assert_eq!(
        goal(&runtime, &core, &session.id, "RESUME"),
        Outcome::Succeeded
    );
    let state = core.goal(&session.id).unwrap();
    assert_eq!(
        (state.status, state.reason.as_deref(), state.rounds),
        (Status::Paused, Some("needs a key"), 1)
    );
    assert_eq!(judged(), 4);
}

#[test]
fn controls_and_usage_errors() {
    let gateway = FakeGateway::start(|request, _| {
        if reviewing(request) {
            return text(request, "not json");
        }
        text(request, "ok")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let failed = |outcome: Outcome| match outcome {
        Outcome::Failed(error) => error.message,
        other => panic!("{other:?}"),
    };
    let usage = "Usage: /goal <objective> | pause | resume | clear";
    assert_eq!(failed(goal(&runtime, &core, &session.id, "  ")), usage);
    assert_eq!(
        failed(goal(&runtime, &core, &session.id, "pause")),
        "No active goal to pause."
    );
    assert_eq!(
        failed(goal(&runtime, &core, &session.id, "resume")),
        "No paused goal to resume."
    );
    assert_eq!(
        failed(goal(&runtime, &core, &session.id, "clear")),
        "No goal is set."
    );
    assert_eq!(
        failed(goal(&runtime, &core, &session.id, &"x".repeat(4001))),
        "A goal objective is limited to 4000 characters (got 4001)."
    );

    // An unusable verdict is asked for once more, then pauses the goal.
    assert_eq!(
        goal(&runtime, &core, &session.id, "Ship"),
        Outcome::Succeeded
    );
    let state = core.goal(&session.id).unwrap();
    assert_eq!(
        (state.status, state.reason.as_deref()),
        (
            Status::Paused,
            Some("The goal reviewer gave no usable verdict")
        )
    );
    assert_eq!(gateway.messages().into_iter().filter(reviewing).count(), 2);
    assert_eq!(
        failed(goal(&runtime, &core, &session.id, "")),
        format!("Goal paused: Ship\nThe goal reviewer gave no usable verdict\n{usage}")
    );
    assert_eq!(
        failed(goal(&runtime, &core, &session.id, "pause")),
        "No active goal to pause."
    );
}

#[test]
fn reviewer_overflow_compacts_once_and_reminds_the_objective() {
    let gateway = FakeGateway::start(|request, _| {
        if request.body.contains("You MUST summarize") {
            return text(request, "## Objective\n- x");
        }
        if reviewing(request) {
            if !request
                .body
                .contains("<system-reminder>\\nA goal is active")
            {
                return Reply::Json {
                    status: 400,
                    body: json!({"type": "error", "error": {"type": "invalid_request_error", "message": "prompt is too long: 1000001 tokens > 1000000 maximum"}}),
                    headers: Vec::new(),
                };
            }
            return text(request, r#"{"verdict":"done","reason":"ok"}"#);
        }
        text(request, "worked")
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    assert_eq!(
        goal(&runtime, &core, &session.id, "Keep it"),
        Outcome::Succeeded
    );
    let state = core.goal(&session.id).unwrap();
    assert_eq!(state.status, Status::Complete);
    let requests = gateway.messages();
    let judge = requests.iter().rfind(|r| reviewing(r)).unwrap();
    let texts = transcript(judge);
    // The compacted history ends in its checkpoint, a user message: the reminder goes before it.
    let reminder = &texts[0];
    assert!(texts[1].starts_with("user: <conversation-checkpoint>"));
    assert!(reminder.starts_with("user: <system-reminder>\nA goal is active for this session."));
    assert!(reminder.contains("<untrusted_objective>\nKeep it\n</untrusted_objective>"));
}
