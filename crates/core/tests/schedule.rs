//! `experimental.schedules`: a schedule created from Code Mode fires its prompt into the session
//! while the process runs, as `msg_<schedule>_<run>`.

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::{Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

const CODE: &str = r#"return await tools.opencode.schedule_create({ name: "nudge", when: { in: "1 seconds" }, prompt: "wake up now" })"#;

fn last_message(request: &Recorded) -> String {
    request.json()["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(Value::to_string)
        .unwrap_or_default()
}

fn reply(request: &Recorded, _: usize) -> Reply {
    let last = last_message(request);
    if request.is_title() || last.contains("tool_result") {
        return text(request, "ok");
    }
    if last.contains("wake up now") {
        return text(request, "woken");
    }
    let model = request.json()["model"].as_str().unwrap().to_string();
    Reply::Sse {
        body: Stream::new()
            .tool("toolu_1", "execute", json!({"code": CODE}))
            .finish(&model, Some(1)),
    }
}

#[test]
fn a_schedule_fires_into_its_session() {
    let gateway = FakeGateway::start(reply);
    let env = env(&gateway.url);
    std::fs::write(
        env.home.project().join("opencode.json"),
        r#"{"experimental":{"schedules":true}}"#,
    )
    .unwrap();
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let prompt = Prompt {
        text: "remind me".into(),
        ..Default::default()
    };
    runtime
        .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
        .unwrap();
    let schedules = core.db().schedules(Some(&session.id)).unwrap();
    assert_eq!(
        schedules.len(),
        1,
        "{:?}",
        gateway.messages().last().map(|r| r.body.clone())
    );
    let id = schedules[0].id.clone();
    assert_eq!(schedules[0].name.as_deref(), Some("nudge"));

    let deadline = Instant::now() + Duration::from_secs(20);
    let woken = loop {
        let woken = gateway
            .messages()
            .into_iter()
            .find(|r| !r.is_title() && last_message(r).contains("wake up now"));
        if let Some(woken) = woken {
            break woken;
        }
        assert!(Instant::now() < deadline, "the schedule never fired");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(last_message(&woken).contains("wake up now"));
    let fired = loop {
        let row = core.db().schedule(&id).unwrap().unwrap();
        if row.status == "done" {
            break row;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(fired.run_count, 1);
    assert_eq!(fired.last_outcome.as_deref(), Some("fired"));
    assert_eq!(fired.last_message_id, Some(format!("msg_{id}_1")));
    assert_eq!(fired.next_due_at, None);
    core.shutdown();
}

/// Review 4 (M3): a folder's schedules fire only from that folder's core, under its config. With
/// folder A's core running, B's scheduled turn never runs from A (not while B's delete hold
/// stands either), and when it runs it runs under B's config.
#[test]
fn a_schedule_fires_only_from_its_own_folders_core() {
    let gateway = FakeGateway::start(reply);
    let env = env(&gateway.url);
    let a = env.home.project();
    let b = env.home.home().join("other");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(
        a.join("opencode.json"),
        r#"{"experimental":{"schedules":true}}"#,
    )
    .unwrap();
    std::fs::write(
        b.join("opencode.json"),
        r#"{"experimental":{"schedules":true},"agent":{"build":{"prompt":"B-CONFIG agent prompt"}}}"#,
    )
    .unwrap();
    let runtime = runtime();
    let core_a = runtime
        .block_on(anysola_core::api::Core::start(&a))
        .unwrap();
    let core_b = runtime
        .block_on(anysola_core::api::Core::start(&b))
        .unwrap();
    let remind = || Prompt {
        text: "remind me".into(),
        ..Default::default()
    };
    let woken = |session: &str| {
        gateway.messages().into_iter().find(|r| {
            let last = last_message(r);
            // The schedule's own prompt, not the tool result that echoes it.
            !r.is_title()
                && last.contains("wake up now")
                && !last.contains("tool_result")
                && r.header("x-opencode-session-id") == Some(session)
        })
    };

    // Held for delete in B: the schedule's time comes, A's core is running, no turn starts.
    let (held, _) = core_b.open(&Target::New, false).unwrap();
    runtime
        .block_on(core_b.prompt(&held.id, remind(), TurnOptions::default()))
        .unwrap();
    assert_eq!(core_b.db().schedules(Some(&held.id)).unwrap().len(), 1);
    let hold = core_b.hold_for_delete(&held.id).unwrap();
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        woken(&held.id).is_none(),
        "no core ran the held session's scheduled turn"
    );
    drop(hold);

    // Unheld: it fires, from B, under B's config.
    let (session, _) = core_b.open(&Target::New, false).unwrap();
    runtime
        .block_on(core_b.prompt(&session.id, remind(), TurnOptions::default()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let request = loop {
        if let Some(request) = woken(&session.id) {
            break request;
        }
        assert!(Instant::now() < deadline, "B's schedule never fired");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        request.body.contains("B-CONFIG agent prompt"),
        "the scheduled turn ran under B's config"
    );
    core_a.shutdown();
    core_b.shutdown();
}
