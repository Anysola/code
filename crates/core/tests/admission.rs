//! Prompts as the core admits them: one turn at a time per session however quickly they come, and
//! attachments the model can't take refused before anything is stored.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anysola_core::api::{Core, CoreError, Prompt, Target};
use anysola_core::session::{Outcome, TurnOptions};
use anysola_testkit::FakeGateway;
use common::*;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Default::default()
    }
}

#[test]
fn a_second_prompt_sent_at_once_is_queued_for_the_running_turn() {
    let (running, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (handle, queued): (Arc<OnceLock<(Core, String)>>, Arc<AtomicUsize>) = Default::default();
    let gateway = FakeGateway::start({
        let (running, most) = (running.clone(), most.clone());
        let (handle, queued) = (handle.clone(), queued.clone());
        move |request, _| {
            if request.is_title() {
                return text(request, "Title");
            }
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            // The first turn waits until the other prompt is queued behind it.
            let (core, session) = handle.get().unwrap();
            let start = std::time::Instant::now();
            while queued.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(5) {
                if !core.pending(session).unwrap().is_empty() {
                    queued.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            running.fetch_sub(1, Ordering::SeqCst);
            text(request, "ok")
        }
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let id = session.id.clone();
    handle.set((core.clone(), id.clone())).ok();
    let send = |text: &'static str| {
        let (core, id) = (core.clone(), id.clone());
        runtime.spawn(async move { core.prompt(&id, prompt(text), TurnOptions::default()).await })
    };
    let (one, two) = (send("one"), send("two"));
    let (one, two) = runtime.block_on(async { (one.await.unwrap(), two.await.unwrap()) });
    assert_eq!(one.unwrap(), Outcome::Succeeded);
    assert_eq!(two.unwrap(), Outcome::Succeeded);

    assert_eq!(most.load(Ordering::SeqCst), 1, "two turns ran at once");
    assert_eq!(
        queued.load(Ordering::SeqCst),
        1,
        "the second prompt was never queued"
    );
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 2);
    let first = transcript(&main[0]);
    let second = transcript(&main[1]);
    assert_eq!(first.len(), 1);
    let other = if first[0] == "user: one" {
        "user: two"
    } else {
        "user: one"
    };
    assert_eq!(second, vec![first[0].as_str(), "assistant: ok", other]);
    assert!(!core.is_busy(&id));
    assert!(core.pending(&id).unwrap().is_empty());
    // Released, the session starts a turn of its own for the next prompt.
    let third = runtime
        .block_on(core.prompt(&id, prompt("three"), TurnOptions::default()))
        .unwrap();
    assert_eq!(third, Outcome::Succeeded);
    assert_eq!(
        gateway
            .messages()
            .into_iter()
            .filter(|r| !r.is_title())
            .count(),
        3
    );
}

#[test]
fn an_image_the_model_cannot_take_is_refused_before_it_is_stored() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let svg = base64_svg();
    let mut refused = prompt("look");
    refused
        .files
        .push(("image/svg+xml".into(), svg, "a.svg".into()));
    let error = runtime
        .block_on(core.prompt(&session.id, refused, TurnOptions::default()))
        .unwrap_err();
    assert!(
        matches!(&error, CoreError::Image(message) if message == "Image could not be decoded: a.svg"),
        "{error:?}"
    );
    assert!(core.messages(&session.id).unwrap().is_empty());
    assert!(core.pending(&session.id).unwrap().is_empty());
    assert!(gateway.messages().is_empty());
    // The session still takes the next prompt.
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("again"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
}

fn base64_svg() -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1"/></svg>"#,
    )
}
