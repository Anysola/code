//! The terminal after a turn that does not succeed, as before the desktop app held leftovers:
//! prompts steered into it stay steers (the next prompt takes them along), and a prompt that
//! cannot start takes every pending prompt with it.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::session::inbox::Delivery;
use anysola_core::session::{Outcome, TurnOptions};
use anysola_testkit::{FakeGateway, Reply};
use common::*;
use serde_json::json;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Prompt::default()
    }
}

fn pending(core: &Core, session: &str) -> Vec<(String, String)> {
    core.pending(session)
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.payload["text"].as_str().unwrap().to_string(),
                row.delivery,
            )
        })
        .collect()
}

#[test]
fn a_failed_turn_leaves_its_steers_as_they_were() {
    assert_eq!(anysola_core::version::client(), "cli");
    let core: Arc<OnceLock<(Core, String)>> = Arc::default();
    let failing = Arc::new(AtomicBool::new(true));
    let gateway = FakeGateway::start({
        let (core, failing) = (core.clone(), failing.clone());
        move |request, _| {
            if request.is_title() {
                return text(request, "Title");
            }
            if failing.swap(false, Ordering::SeqCst) {
                let (core, session) = core.get().unwrap();
                core.enqueue(session, prompt("steered"), Delivery::Steer)
                    .unwrap();
                core.enqueue(session, prompt("queued"), Delivery::Queue)
                    .unwrap();
                return Reply::Json {
                    status: 400,
                    body: json!({"error": {"type": "invalid_request_error", "message": "bad"}}),
                    headers: Vec::new(),
                };
            }
            text(request, "ok")
        }
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let started = runtime.block_on(common::core(&env));
    let (session, _) = started.open(&Target::New, false).unwrap();
    core.set((started.clone(), session.id.clone())).ok();
    let outcome = runtime
        .block_on(started.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    assert_eq!(
        pending(&started, &session.id),
        [
            ("steered".to_string(), "steer".to_string()),
            ("queued".to_string(), "queue".to_string())
        ]
    );
}

#[test]
fn a_prompt_that_cannot_start_drops_everything_pending() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.enqueue(&session.id, prompt("left over"), Delivery::Queue)
        .unwrap();
    core.db().set_agent(&session.id, "gone").unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("new"), TurnOptions::default()))
        .unwrap();
    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    assert!(pending(&core, &session.id).is_empty());
}
