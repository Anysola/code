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
        skills: Vec::new(),
        command: None,
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

/// A relay that cuts every stream after output gets three continuations and then a failure,
/// not an endless loop of billed requests.
#[test]
fn incomplete_streams_stop_after_three_continuations() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "Title");
        }
        sse(
            request,
            anysola_testkit::sse::Stream::new().text("partial"),
            true,
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
    assert_eq!(error.kind, "provider.invalid-output");
    assert_eq!(
        error.message,
        "The model's stream was cut off 4 times in a row"
    );
    let main = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .count();
    assert_eq!(main, 4, "the first call and three continuations");
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
    assert_eq!(fork.title, db.session(&session.id).unwrap().unwrap().title);
    assert_eq!(
        db.messages(&fork.id).unwrap().len(),
        db.messages(&session.id).unwrap().len()
    );
    let child = core.create_child(&session.id, None).unwrap();
    assert_eq!(child.parent_id.as_deref(), Some(session.id.as_str()));
    assert!(child.title.starts_with("Child session - "));
}

#[test]
fn a_session_forks_more_than_once() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["one", "two"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let source = db.messages(&session.id).unwrap();

    // The whole session, twice, and once more cut before its second prompt.
    let first = core.fork_session(&session.id, None).unwrap();
    let second = core.fork_session(&session.id, None).unwrap();
    let title = db.session(&session.id).unwrap().unwrap().title;
    assert_eq!(first.title, title);
    assert_eq!(second.title, title);
    for fork in [&first, &second] {
        let rows = db.messages(&fork.id).unwrap();
        assert_eq!(rows.len(), source.len());
        for (copy, row) in rows.iter().zip(&source) {
            assert!(copy.id.starts_with(&format!("{}_", row.id)), "{}", copy.id);
            assert_eq!((&copy.kind, &copy.data), (&row.kind, &row.data));
        }
    }
    let cut = source.iter().filter(|r| r.kind == "user").nth(1).unwrap();
    let before = core.fork_session(&session.id, Some(&cut.id)).unwrap();
    let copied = db.messages(&before.id).unwrap();
    assert_eq!(
        copied.len(),
        source.iter().position(|r| r.id == cut.id).unwrap()
    );
    assert_eq!(before.title, title);

    // A fork forks too, more than once, and its forks answer a prompt.
    let nested = core.fork_session(&first.id, None).unwrap();
    let again = core.fork_session(&first.id, None).unwrap();
    assert_eq!(db.messages(&nested.id).unwrap().len(), source.len());
    assert_eq!(db.messages(&again.id).unwrap().len(), source.len());
    let outcome = runtime
        .block_on(core.prompt(&again.id, prompt("three"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);

    // History copied up to a prompt nobody answered is not a turn the fork left unfinished.
    let answer = source
        .iter()
        .rposition(|r| r.kind == "user")
        .map(|at| &source[at + 1])
        .unwrap();
    let open = core.fork_session(&session.id, Some(&answer.id)).unwrap();
    assert_eq!(db.messages(&open.id).unwrap().last().unwrap().kind, "user");
    assert!(!core.settle_interrupted(&open.id).unwrap());
    let twice = core.fork_session(&open.id, None).unwrap();
    assert!(!core.settle_interrupted(&twice.id).unwrap());

    // Forks are listed like any session (the window's list), unlike a subagent's session; they
    // start with no usage of their own.
    let child = core.create_child(&session.id, None).unwrap();
    let listed: Vec<String> = db
        .list_sessions(None, None, 100)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    for fork in [&first, &second, &before, &nested, &again, &open, &twice] {
        assert!(listed.contains(&fork.id), "{} is not listed", fork.id);
    }
    assert!(!listed.contains(&child.id));
    let usage = core.usage(&first.id);
    assert_eq!((usage.total, usage.turns), (0.0, 0));
}

/// Deleting a session takes its usage with it: the stored rows of the whole subtree, and what the
/// core keeps in memory.
#[test]
fn deleting_a_session_removes_the_usage_of_its_subtree() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (parent, _) = core.open(&Target::New, false).unwrap();
    let (other, _) = core.open(&Target::New, false).unwrap();
    let child = core.create_child(&parent.id, None).unwrap();
    for id in [&parent.id, &other.id] {
        runtime
            .block_on(core.prompt(id, prompt("one"), TurnOptions::default()))
            .unwrap();
    }
    core.set_budget(&child.id, Some(50)).unwrap();
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let key = |id: &str| format!("anysola:usage:{id}");
    for id in [&parent.id, &child.id, &other.id] {
        assert!(db.kv_get(&key(id)).unwrap().is_some(), "{id} has no row");
    }
    assert!(core.usage(&parent.id).total > 0.0);
    assert_eq!(core.usage(&child.id).budget, Some(50));

    assert!(core.delete_session(&parent.id).unwrap());
    for id in [&parent.id, &child.id] {
        assert!(db.kv_get(&key(id)).unwrap().is_none(), "{id} kept its row");
        let usage = core.usage(id);
        assert_eq!((usage.total, usage.turns, usage.budget), (0.0, 0, None));
    }
    assert!(db.kv_get(&key(&other.id)).unwrap().is_some());
    assert!(core.usage(&other.id).total > 0.0);
}

#[test]
fn a_fork_that_fails_leaves_no_session() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("one"), TurnOptions::default()))
        .unwrap();
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let before = db.list_sessions(None, None, 100).unwrap().len();
    // The copy cannot be written while the message table refuses inserts.
    db.immediate(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON message BEGIN SELECT RAISE(ABORT, 'refused'); END",
        )
    })
    .map_err(|e: rusqlite::Error| e)
    .unwrap();
    assert!(core.fork_session(&session.id, None).is_err());
    assert_eq!(db.list_sessions(None, None, 100).unwrap().len(), before);
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
        skills: Vec::new(),
        command: None,
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

#[test]
fn titles_fall_back_to_the_session_model_and_regenerate() {
    let gateway = FakeGateway::start(|request, _| {
        let body = request.json();
        if request.is_title() && body["model"] == "claude-haiku-5-5" {
            return anysola_testkit::Reply::Json {
                status: 400,
                body: json!({"type": "error", "error": {"type": "invalid_request_error", "message": "no"}}),
                headers: vec![],
            };
        }
        if request.body.contains("Here is the conversation so far") {
            return text(request, "Renamed title");
        }
        text(
            request,
            if request.is_title() {
                "First title"
            } else {
                "ok"
            },
        )
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
        .unwrap();
    let titled = core.session(&session.id).unwrap().unwrap();
    assert_eq!(titled.title, "First title");
    let titles: Vec<String> = gateway
        .messages()
        .into_iter()
        .filter(|r| r.is_title())
        .map(|r| r.json()["model"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, vec!["claude-haiku-5-5", "claude-sonnet-5-5"]);
    runtime
        .block_on(core.regenerate_title(&session.id))
        .unwrap();
    assert_eq!(
        core.session(&session.id).unwrap().unwrap().title,
        "Renamed title"
    );
}

#[test]
fn compaction_nudges_once_then_fails() {
    let gateway = FakeGateway::start(|request, _| {
        let body = &request.body;
        if body.contains("did not fill in the required summary template") {
            return text(request, "still prose");
        }
        if body.contains("You MUST summarize") {
            return text(request, "prose only");
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
    let Outcome::Failed(error) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        (
            "compaction.failed",
            "Compaction summary did not match the required template"
        )
    );
    let summaries = gateway
        .messages()
        .into_iter()
        .filter(|r| r.body.contains("You MUST summarize"))
        .count();
    assert_eq!(summaries, 2, "the prompt, then one nudge");
}

/// A core told to stop at budgets (the desktop app) makes no model call for a session whose
/// budget is spent: no turn step, no title, no `/compact`; the turn ends with reason `budget`.
/// A session without one never stops, a raised budget lets the next prompt run, and a
/// subagent's calls count toward its root's budget and stop with it. Each fake model call
/// (title calls included) takes 1% of the 5-hour window.
#[test]
fn a_spent_budget_stops_every_model_call_until_it_is_raised() {
    use anysola_core::events::InterruptReason;
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    core.stop_at_budget();
    let run = |id: &str, text: &str| {
        runtime
            .block_on(core.prompt(id, prompt(text), TurnOptions::default()))
            .unwrap()
    };
    let calls = || gateway.messages().len();
    let budget = Outcome::Interrupted(InterruptReason::Budget);

    let (free, _) = core.open(&Target::New, false).unwrap();
    for n in 0..3 {
        assert_eq!(run(&free.id, &format!("free {n}")), Outcome::Succeeded);
    }
    assert!(core.usage(&free.id).total >= 3.0);
    assert_eq!(core.usage(&free.id).budget, None);

    let (capped, _) = core.open(&Target::New, false).unwrap();
    // 2: the first turn's title call (1%) may land before its first step is checked.
    core.set_budget(&capped.id, Some(2)).unwrap();
    assert_eq!(run(&capped.id, "one"), Outcome::Succeeded);
    assert!(core.usage(&capped.id).total >= 2.0);
    let before = calls();
    assert_eq!(run(&capped.id, "two"), budget);
    assert_eq!(
        runtime.block_on(core.compact(&capped.id)).unwrap(),
        budget,
        "/compact waits for the budget too"
    );
    let aside = runtime.block_on(core.aside(
        &capped.id,
        "what now?",
        &anysola_core::api::CancellationToken::new(),
    ));
    assert_eq!(
        aside.map_err(|e| e.kind),
        Err(anysola_core::api::AsideKind::Budget),
        "/btw waits for the budget too"
    );
    assert_eq!(calls(), before, "no model call once the budget is spent");
    // Each stop the user asked for says why (no `repeat` mark), the second prompt's too.
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let idles: Vec<_> = db
        .messages(&capped.id)
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == "idle" && m.data["reason"] == "budget")
        .collect();
    assert_eq!(idles.len(), 2);
    assert!(idles.iter().all(|m| m.data.get("repeat").is_none()));

    let total = core.usage(&capped.id).total;
    core.set_budget(&capped.id, Some(total.floor() as u64 + 2))
        .unwrap();
    assert_eq!(run(&capped.id, "three"), Outcome::Succeeded);
    assert!(calls() > before);

    // A subagent spends its root's budget, and stops once it is spent.
    let (root, _) = core.open(&Target::New, false).unwrap();
    core.set_budget(&root.id, Some(1)).unwrap();
    let child = core.create_child(&root.id, None).unwrap();
    assert_eq!(run(&child.id, "child one"), Outcome::Succeeded);
    assert!(
        core.usage(&root.id).total >= 1.0,
        "the child's call counts for the root"
    );
    let before = calls();
    assert_eq!(run(&child.id, "child two"), budget);
    assert_eq!(run(&root.id, "root"), budget);
    assert_eq!(calls(), before);
}

/// A session being deleted starts no turn (review 3, N4): `hold_for_delete` closes it from the
/// delete's first check until its row is gone; a session whose turn runs can't be held.
#[test]
fn a_session_held_for_delete_starts_no_turn() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let hold = core.hold_for_delete(&session.id).unwrap();
    let calls = gateway.messages().len();
    let refused =
        runtime.block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()));
    assert!(
        matches!(refused, Err(anysola_core::api::CoreError::Deleting)),
        "{refused:?}"
    );
    assert_eq!(gateway.messages().len(), calls, "no model call");
    assert!(!core.is_busy(&session.id));
    drop(hold);
    assert_eq!(
        runtime
            .block_on(core.prompt(&session.id, prompt("hello"), TurnOptions::default()))
            .unwrap(),
        Outcome::Succeeded,
        "a refused delete opens the session again"
    );
}
