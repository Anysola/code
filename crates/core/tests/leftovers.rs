//! What a turn that does not succeed leaves behind in the desktop app: prompts sent while it ran
//! stay queued for the user, and a prompt that could not start is dropped and said to be unsent.
//! (The terminal keeps its own behaviour: `leftovers_terminal.rs`.)

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anysola_core::api::{Core, Prompt, Steered, Target};
use anysola_core::events::Event;
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
fn a_failed_turn_leaves_what_was_sent_meanwhile_queued_until_sent() {
    anysola_core::version::set_desktop_client();
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
    // The steer did not join anything; both wait, in the order they were sent.
    assert_eq!(
        pending(&started, &session.id),
        [
            ("steered".to_string(), "queue".to_string()),
            ("queued".to_string(), "queue".to_string())
        ]
    );
    assert!(!started.is_busy(&session.id));
    let outcome = runtime
        .block_on(started.send_queued(&session.id, TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    assert!(pending(&started, &session.id).is_empty());
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 3);
    assert_eq!(transcript(&main[1]).last().unwrap(), "user: steered");
    assert_eq!(transcript(&main[2]).last().unwrap(), "user: queued");
}

#[test]
fn a_prompt_that_cannot_start_is_dropped_as_unsent_and_the_queue_stays() {
    anysola_core::version::set_desktop_client();
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.enqueue(&session.id, prompt("left over"), Delivery::Queue)
        .unwrap();
    // An agent the config no longer has: the turn fails before it starts.
    core.db().set_agent(&session.id, "gone").unwrap();
    let mut events = core.subscribe();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("new"), TurnOptions::default()))
        .unwrap();
    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    let unsent = std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event {
        Event::Failed { unsent, .. } => Some(unsent),
        _ => None,
    });
    assert_eq!(unsent, Some(true));
    assert_eq!(
        pending(&core, &session.id),
        [("left over".to_string(), "queue".to_string())]
    );
    let stored = core
        .messages(&session.id)
        .unwrap()
        .into_iter()
        .filter(|(_, m)| matches!(m, anysola_core::session::message::Message::User(_)))
        .count();
    assert_eq!(stored, 0, "the new prompt was not kept");
}

#[test]
fn sending_the_queue_keeps_it_when_the_turn_cannot_start() {
    anysola_core::version::set_desktop_client();
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let mut events = core.subscribe();
    // Nothing waits: no turn starts (none would end).
    let outcome = runtime
        .block_on(core.send_queued(&session.id, TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    assert!(
        std::iter::from_fn(|| events.try_recv().ok())
            .next()
            .is_none()
    );
    core.enqueue(&session.id, prompt("waiting"), Delivery::Queue)
        .unwrap();
    core.db().set_agent(&session.id, "gone").unwrap();
    let outcome = runtime
        .block_on(core.send_queued(&session.id, TurnOptions::default()))
        .unwrap();
    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    let unsent = std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event {
        Event::Failed { unsent, .. } => Some(unsent),
        _ => None,
    });
    assert_eq!(unsent, Some(false), "the composer never had its text");
    assert_eq!(
        pending(&core, &session.id),
        [("waiting".to_string(), "queue".to_string())]
    );
}

#[test]
fn a_turn_the_process_died_under_ends_as_interrupted_when_settled() {
    anysola_core::version::set_desktop_client();
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("done"), TurnOptions::default()))
        .unwrap();
    let _guard = runtime.enter();
    assert!(
        !core.settle_interrupted(&session.id).unwrap(),
        "a finished turn"
    );
    // A prompt stored with no turn after it: what a process killed mid-turn leaves.
    core.db()
        .append_message(
            &session.id,
            &anysola_core::ids::message(),
            "user",
            &json!({"time": {"created": 1}, "text": "cut off"}),
        )
        .unwrap();
    assert!(core.settle_interrupted(&session.id).unwrap());
    let last = core.db().messages(&session.id).unwrap().pop().unwrap();
    assert_eq!(last.kind, "idle");
    assert_eq!(
        (&last.data["outcome"], &last.data["reason"]),
        (&json!("interrupted"), &json!("closed"))
    );
    assert!(!core.settle_interrupted(&session.id).unwrap(), "once");
}

#[test]
fn a_prompt_that_cannot_start_drops_only_itself() {
    anysola_core::version::set_desktop_client();
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    // A steer no turn took (what a process that died under its turn leaves).
    core.enqueue(&session.id, prompt("older"), Delivery::Steer)
        .unwrap();
    core.db().set_agent(&session.id, "gone").unwrap();
    let mut events = core.subscribe();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("new"), TurnOptions::default()))
        .unwrap();
    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    let unsent = std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event {
        Event::Failed { unsent, .. } => Some(unsent),
        _ => None,
    });
    assert_eq!(unsent, Some(true));
    // The older message is kept where the user sees it; only the new prompt went back.
    assert_eq!(
        pending(&core, &session.id),
        [("older".to_string(), "queue".to_string())]
    );
}

#[test]
fn a_queued_message_steers_only_a_running_turn() {
    anysola_core::version::set_desktop_client();
    let core: Arc<OnceLock<(Core, String, String)>> = Arc::default();
    let steered: Arc<OnceLock<Steered>> = Arc::default();
    let gateway = FakeGateway::start({
        let (core, steered) = (core.clone(), steered.clone());
        move |request, _| {
            if request.is_title() {
                return text(request, "Title");
            }
            if let Some((core, session, waiting)) = core.get()
                && steered.get().is_none()
            {
                let _ = steered.set(core.steer_pending(session, waiting).unwrap());
            }
            text(request, "ok")
        }
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let started = runtime.block_on(common::core(&env));
    let (session, _) = started.open(&Target::New, false).unwrap();
    let waiting = started
        .enqueue(&session.id, prompt("waiting"), Delivery::Queue)
        .unwrap();
    // No turn: nothing to join, and the message stays queued.
    assert_eq!(
        started.steer_pending(&session.id, &waiting).unwrap(),
        Steered::NoTurn
    );
    assert_eq!(
        pending(&started, &session.id),
        [("waiting".to_string(), "queue".to_string())]
    );
    core.set((started.clone(), session.id.clone(), waiting.clone()))
        .ok();
    let outcome = runtime
        .block_on(started.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    assert_eq!(steered.get(), Some(&Steered::Taken));
    assert!(pending(&started, &session.id).is_empty());
    assert_eq!(
        started.steer_pending(&session.id, &waiting).unwrap(),
        Steered::NoTurn
    );
}

fn append(core: &Core, session: &str, kind: &str, data: serde_json::Value) {
    core.db()
        .append_message(session, &anysola_core::ids::message(), kind, &data)
        .unwrap();
}

fn last_kind(core: &Core, session: &str) -> String {
    core.db().messages(session).unwrap().pop().unwrap().kind
}

#[test]
fn forks_are_not_marked_for_where_their_source_was_cut() {
    anysola_core::version::set_desktop_client();
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let _guard = runtime.enter();
    let (source, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&source.id, prompt("done"), TurnOptions::default()))
        .unwrap();
    // A step that called tools, cut there: a fork taken through it is not a turn of its own.
    let mut step = core
        .db()
        .messages(&source.id)
        .unwrap()
        .into_iter()
        .rev()
        .find(|r| r.kind == "assistant")
        .unwrap()
        .data;
    step["finish"] = json!("tool-calls");
    append(&core, &source.id, "assistant", step);
    let at_step = core.fork_session(&source.id, None).unwrap();
    assert!(!core.settle_interrupted(&at_step.id).unwrap());
    assert_eq!(last_kind(&core, &at_step.id), "assistant");
    // Cut on a prompt, and a fork of that fork. (Another source: forking one session twice
    // copies rows under the same ids.)
    let (source, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&source.id, prompt("done"), TurnOptions::default()))
        .unwrap();
    append(
        &core,
        &source.id,
        "user",
        json!({"time": {"created": 1}, "text": "cut off"}),
    );
    let at_prompt = core.fork_session(&source.id, None).unwrap();
    assert!(!core.settle_interrupted(&at_prompt.id).unwrap());
    assert_eq!(last_kind(&core, &at_prompt.id), "user");
    let again = core.fork_session(&at_prompt.id, None).unwrap();
    assert!(!core.settle_interrupted(&again.id).unwrap());
    assert_eq!(last_kind(&core, &again.id), "user");
    // A turn of the fork's own that was cut off is.
    append(
        &core,
        &again.id,
        "user",
        json!({"time": {"created": 2}, "text": "mine"}),
    );
    assert!(core.settle_interrupted(&again.id).unwrap());
    assert_eq!(last_kind(&core, &again.id), "idle");
    // The source itself was cut too.
    assert!(core.settle_interrupted(&source.id).unwrap());
}
