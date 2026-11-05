//! Background shells: a completion wakes the idle session; ctrl+b moves a running foreground
//! command to the background and tells the model.
#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::{Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::json;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        files: vec![],
    }
}

fn reply(request: &Recorded, _: usize) -> Reply {
    if request.is_title() {
        return text(request, "Title");
    }
    let last = request.json()["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .to_string();
    let call = |input| {
        let model = request.json()["model"].as_str().unwrap().to_string();
        Reply::Sse {
            body: Stream::new()
                .tool("toolu_1", "shell", input)
                .finish(&model, Some(1)),
            before: Some(100),
        }
    };
    if last.contains("\"bg\"") {
        return call(json!({"command": "sleep 1; echo bgdone", "background": true}));
    }
    if last.contains("\"fg\"") {
        return call(json!({"command": "sleep 2; echo fgdone"}));
    }
    text(request, "ok")
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn completion_wakes_the_idle_session() {
    let gateway = FakeGateway::start(reply);
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("bg"), TurnOptions::default()))
        .unwrap();
    let main = || -> Vec<Recorded> {
        gateway
            .messages()
            .into_iter()
            .filter(|r| !r.is_title())
            .collect()
    };
    assert_eq!(main().len(), 2);
    wait_for("the wake-up turn", || main().len() == 3);
    let last = transcript(&main()[2]);
    let notice = last.last().unwrap();
    assert!(notice.starts_with("user: <shell id=\"sh_"), "{notice}");
    assert!(notice.ends_with("command=\"sleep 1; echo bgdone\">\nbgdone\n\n</shell>"));
    core.shutdown();
}

#[test]
fn ctrl_b_moves_foreground_commands_to_the_background() {
    let gateway = FakeGateway::start(reply);
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    assert!(!core.background(&session.id));
    let mover = {
        let (core, id) = (core.clone(), session.id.clone());
        std::thread::spawn(move || wait_for("a foreground command", || core.background(&id)))
    };
    runtime
        .block_on(core.prompt(&session.id, prompt("fg"), TurnOptions::default()))
        .unwrap();
    mover.join().unwrap();
    // Stop before the completion would wake the session again.
    core.shutdown();
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let last = transcript(&main[1]);
    assert!(
        main[1]
            .body
            .contains("Command moved to the background (shell ID: sh_")
    );
    assert_eq!(
        last.last().unwrap(),
        "user: User requested that active blocking work be moved to the background.\n\nBackgrounded work:\n- shell: sleep 2; echo fgdone\n\nThe backgrounded work is still unfinished. Move on to other work if you can. If there is nothing else useful to do, finish your response. Do not wait, sleep, poll, or report the backgrounded work as complete until a later completion notification is added to the conversation."
    );
}
