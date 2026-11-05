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
        before: Some(100),
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
