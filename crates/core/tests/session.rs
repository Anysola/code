//! Session behaviour driven in-process: drain-start recovery, manual compaction.

mod common;

use anysola_core::api::{Prompt, Target};
use anysola_core::session::{Outcome, TurnOptions};
use anysola_core::storage::Db;
use anysola_testkit::FakeGateway;
use common::*;
use serde_json::json;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        files: vec![],
    }
}

#[test]
fn stale_tool_calls_and_compactions_are_settled() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let time = json!({"created": 1});
    db.append_message(
        &session.id,
        "msg_u",
        "user",
        &json!({"time": time, "text": "read it"}),
    )
    .unwrap();
    db.append_message(
        &session.id,
        "msg_a",
        "assistant",
        &json!({"time": time, "agent": "build", "model": {"id": "claude-sonnet-5-5", "providerID": "anysola"},
            "content": [
                {"type": "tool", "id": "toolu_1", "name": "read", "state": {"status": "running", "input": {"filePath": "a"}, "metadata": {}}, "time": time},
                {"type": "tool", "id": "toolu_2", "name": "subagent", "state": {"status": "running", "input": {}, "metadata": {"sessionID": "ses_child"}}, "time": time},
            ]}),
    )
    .unwrap();
    db.append_message(
        &session.id,
        "msg_c",
        "compaction",
        &json!({"time": time, "status": "running", "reason": "auto", "summary": ""}),
    )
    .unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("next"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let rows = db.messages(&session.id).unwrap();
    let compaction = rows.iter().find(|r| r.id == "msg_c").unwrap();
    assert_eq!(compaction.data["status"], "failed");
    assert_eq!(
        compaction.data["error"],
        json!({"type": "compaction.interrupted", "message": "Compaction was interrupted"})
    );
    let request = gateway
        .messages()
        .into_iter()
        .find(|r| !r.is_title())
        .unwrap();
    let body = request.body;
    assert!(body.contains(r#"{\"error\":{\"type\":\"aborted\",\"message\":\"Tool execution interrupted: read\"},\"content\":[]}"#), "{body}");
    assert!(body.contains("Tool execution interrupted: subagent (sessionID: ses_child)"));
}

#[test]
fn manual_compaction() {
    let gateway = FakeGateway::start(|request, _| {
        if request.body.contains("You MUST summarize") {
            text(request, "## Objective\n- manual")
        } else {
            text(request, "ok")
        }
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    let outcome = runtime.block_on(core.compact(&session.id)).unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    runtime
        .block_on(core.prompt(&session.id, prompt("after"), TurnOptions::default()))
        .unwrap();
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 3);
    let last = transcript(&main[2]);
    assert!(last[0].starts_with("user: <conversation-checkpoint>"));
    assert!(last[0].contains("<summary>\n## Objective\n- manual\n</summary>"));
    assert_eq!(last.last().unwrap(), "user: after");
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let rows = db.messages(&session.id).unwrap();
    let compaction = rows.iter().find(|r| r.kind == "compaction").unwrap();
    assert_eq!(compaction.data["reason"], "manual");
    assert_eq!(compaction.data["status"], "completed");
}

#[test]
fn nothing_to_compact() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime.block_on(core.compact(&session.id)).unwrap();
    let Outcome::Failed(error) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        ("compaction.unavailable", "Nothing to compact yet")
    );
}

fn sse(
    request: &anysola_testkit::Recorded,
    stream: anysola_testkit::sse::Stream,
    cut: bool,
) -> anysola_testkit::Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    anysola_testkit::Reply::Sse {
        body: if cut {
            stream.interrupted(&model)
        } else {
            stream.finish(&model, Some(1))
        },
        before: Some(100),
    }
}

#[test]
fn incomplete_stream_continues_after_output() {
    let gateway = FakeGateway::start(|request, _| {
        let stream = anysola_testkit::sse::Stream::new();
        if request.is_title() {
            return text(request, "Title");
        }
        let continued = request.body.contains("was interrupted");
        sse(
            request,
            stream.text(if continued { "rest" } else { "partial" }),
            !continued,
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
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 2);
    assert_eq!(
        transcript(&main[1]),
        vec![
            "user: go",
            "assistant: partial",
            "user: The previous response was interrupted. Continue from where you left off without repeating completed content.",
        ]
    );
}

#[test]
fn output_limit_continuations() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "Title");
        }
        sse(
            request,
            anysola_testkit::sse::Stream::new()
                .text("long")
                .stop("max_tokens"),
            false,
        )
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let Outcome::Failed(error) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        (
            "output-limit",
            "Response still truncated after two output token limit continuations"
        )
    );
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 3);
    assert_eq!(
        transcript(&main[2]).last().unwrap(),
        "user: Your last response hit the output token limit (stop reason: length). Do not apologize, recap, or repeat yourself. Break the remaining work into smaller pieces."
    );
}

#[test]
fn forks_and_children() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    assert!(session.title.starts_with("New session - "));
    let empty = core
        .open(&Target::Id(session.id.clone()), true)
        .unwrap_err();
    assert_eq!(
        empty.to_string(),
        format!("Cannot fork empty session: {}", session.id)
    );
    runtime
        .block_on(core.prompt(&session.id, prompt("one"), TurnOptions::default()))
        .unwrap();
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let last = db.messages(&session.id).unwrap().pop().unwrap();
    let (fork, _) = core.open(&Target::Id(session.id.clone()), true).unwrap();
    assert_eq!(fork.fork_session_id.as_deref(), Some(session.id.as_str()));
    assert_eq!(
        fork.fork_boundary,
        Some(json!({"type": "through", "messageID": last.id}))
    );
    assert!(fork.title.ends_with(" (fork #1)"));
    assert_eq!(
        db.messages(&fork.id).unwrap().len(),
        db.messages(&session.id).unwrap().len()
    );
    let child = core.create_child(&session.id, None).unwrap();
    assert_eq!(child.parent_id.as_deref(), Some(session.id.as_str()));
    assert!(child.title.starts_with("Child session - "));
}

#[test]
fn prompt_images_are_normalized() {
    use base64::Engine;
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let mut seed: u32 = 1;
    let noise = image::RgbImage::from_fn(3000, 1000, |_, _| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let b = seed.to_le_bytes();
        image::Rgb([b[1], b[2], b[3]])
    });
    let mut png = std::io::Cursor::new(Vec::new());
    noise.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let prompt = Prompt {
        text: "see".into(),
        files: vec![("image/png".into(), data, "noise.png".into())],
    };
    runtime
        .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
        .unwrap();
    let request = gateway
        .messages()
        .into_iter()
        .find(|r| !r.is_title())
        .unwrap();
    let body = request.json();
    let image = &body["messages"][0]["content"][1];
    assert_eq!(image["source"]["media_type"], "image/jpeg");
    assert!(image["source"]["data"].as_str().unwrap().len() <= 5 * 1024 * 1024);
}
