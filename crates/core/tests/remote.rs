//! What the app downloads for a folder: when instruction URLs, skill indexes and git references
//! are fetched, how a failure is kept for the window to show, and what taking back a project's
//! approval does in the open process.

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::{Core, Prompt, Target, TrustKind};
use anysola_core::events::Event;
use anysola_core::permission::{Effect, evaluate};
use anysola_core::session::TurnOptions;
use anysola_testkit::FakeGateway;
use common::*;
use serde_json::json;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Default::default()
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn status(core: &Core, url: &str) -> Option<Option<String>> {
    core.loaded()
        .remote_instruction_status
        .iter()
        .find(|s| s.url == url)
        .map(|s| s.error.clone())
}

#[test]
fn a_failing_instruction_url_says_why_and_is_tried_again_at_the_next_load() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    gateway.serve("/ok.md", "FROM THE NETWORK");
    gateway.serve("/big.md", &"x".repeat((1 << 20) + 1));
    let env = env(&gateway.url);
    let url = |path: &str| format!("{}{path}", gateway.url);
    env.home.write(
        ".anysola/config.json",
        &json!({"instructions": [url("/ok.md"), url("/missing.md"), url("/big.md")]}).to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert_eq!(status(&core, &url("/ok.md")), Some(None));
    assert_eq!(
        status(&core, &url("/missing.md")),
        Some(Some("404 Not Found".into()))
    );
    assert_eq!(
        status(&core, &url("/big.md")),
        Some(Some("larger than 1 MiB".into()))
    );
    let fetched = |path: &str| gateway.requests_to(path).len();
    assert_eq!((fetched("/ok.md"), fetched("/missing.md")), (1, 1));
    // Turns do not fetch.
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["one", "two"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    let body = gateway.messages().last().unwrap().body.clone();
    assert!(body.contains("FROM THE NETWORK"));
    assert_eq!(fetched("/ok.md"), 1, "not per turn");
    // A config load tries the failed ones again, only those.
    runtime.block_on(core.reload());
    assert_eq!((fetched("/ok.md"), fetched("/missing.md")), (1, 2));
    assert_eq!(status(&core, &url("/ok.md")), Some(None));
    // Once every URL works, a load with the same list fetches nothing.
    gateway.serve("/missing.md", "now here");
    env.home.write(
        ".anysola/config.json",
        &json!({"instructions": [url("/ok.md"), url("/missing.md")]}).to_string(),
    );
    runtime.block_on(core.reload());
    assert_eq!(status(&core, &url("/missing.md")), Some(None));
    let before = fetched("/ok.md");
    runtime.block_on(core.reload());
    assert_eq!(fetched("/ok.md"), before, "kept in memory");
    // Another core (a folder opened, the app started) fetches for itself.
    drop(core);
    let _core = runtime.block_on(common::core(&env));
    assert_eq!(fetched("/ok.md"), before + 1);
}

#[test]
fn a_skill_index_is_fetched_at_every_load_and_its_failure_kept() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    gateway.serve("/good/index.json", r#"{"skills": []}"#);
    let env = env(&gateway.url);
    let good = format!("{}/good/", gateway.url);
    let bad = format!("{}/bad/", gateway.url);
    env.home.write(
        ".anysola/config.json",
        &json!({"skills": [good, bad]}).to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let skill_status = core.loaded().skill_status.clone();
    assert_eq!(skill_status.len(), 2);
    assert_eq!(skill_status[0].error, None);
    assert_eq!(
        skill_status[1].error.as_deref(),
        Some("index.json: 404 Not Found")
    );
    let fetched = || gateway.requests_to("/good/index.json").len();
    assert_eq!(fetched(), 1);
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("one"), TurnOptions::default()))
        .unwrap();
    assert_eq!(fetched(), 1, "not per turn");
    runtime.block_on(core.reload());
    assert_eq!(fetched(), 2, "at every config load");
}

#[test]
fn a_git_reference_that_fails_to_clone_says_why() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        &json!({"references": {"lib": {"repository": format!("{}/lib.git", gateway.url)}, "docs": "./docs"}})
            .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    // Cloned in the background at start.
    wait_for("the clone attempt", || {
        core.reference_status()
            .first()
            .is_some_and(|s| s.error.is_some())
    });
    let status = core.reference_status();
    assert_eq!(status.len(), 1, "local references have none");
    assert_eq!(status[0].name, "lib");
    assert!(!status[0].cloned);
    assert!(status[0].attempted_at.is_some());
    let error = status[0].error.clone().unwrap();
    assert!(!error.starts_with("fatal:"), "{error}");
    assert!(!error.contains('\n'), "{error}");
    // When: at start (above); not at a reload that leaves the list alone; again (failed, so not
    // throttled) when the list changes.
    let asked = || {
        gateway
            .requests()
            .iter()
            .filter(|r| r.path.starts_with("/lib.git"))
            .count()
    };
    let at_start = asked();
    assert!(at_start > 0);
    runtime.block_on(core.reload());
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(asked(), at_start, "an unchanged reload clones nothing");
    env.home.write(
        ".anysola/config.json",
        &json!({"references": {"lib": {"repository": format!("{}/lib.git", gateway.url)}, "docs": "./other"}})
            .to_string(),
    );
    runtime.block_on(core.reload());
    wait_for("another clone attempt", || asked() > at_start);
}

/// Approved, then taken back: the allow rule stops allowing, the instruction URL leaves the
/// instructions (and the model is told), the reference and the skill URL are no longer used, in
/// the open process and for a session already open.
#[test]
fn taking_an_approval_back_works_at_once() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    gateway.serve("/proj-rules.md", "PROJECT RULES FROM THE NETWORK");
    gateway.serve("/skills/index.json", r#"{"skills": []}"#);
    let env = env(&gateway.url);
    let url = format!("{}/proj-rules.md", gateway.url);
    let skills = format!("{}/skills/", gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "shell", "resource": "*", "effect": "ask"}]}"#,
    );
    env.home.write(
        "proj/anysola.json",
        &json!({
            "permissions": [{"action": "shell", "resource": "*", "effect": "allow"}],
            "instructions": [url],
            "skills": [skills],
            "references": {"lib": {"repository": format!("{}/lib.git", gateway.url)}},
        })
        .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let shell = |core: &Core| {
        evaluate(
            "shell",
            "ls",
            &core.loaded().agents.get("build").unwrap().permissions,
        )
    };
    let decide = |allow: bool| {
        for item in core.trust_items() {
            if matches!(item.kind, TrustKind::Permissions | TrustKind::Remote) {
                core.trust(&item.path, item.kind, &item.digest, allow)
                    .unwrap();
            }
        }
        runtime.block_on(core.reload());
    };
    decide(true);
    // What a "Stop allowing" button lists: the approved items.
    let approved: Vec<TrustKind> = core
        .trust_items()
        .into_iter()
        .filter(|i| i.decision == Some(true))
        .map(|i| i.kind)
        .collect();
    assert_eq!(approved, [TrustKind::Permissions, TrustKind::Remote]);
    assert_eq!(shell(&core), Effect::Allow);
    assert!(core.loaded().config.instructions.contains(&url));
    assert_eq!(core.references().len(), 1);
    let index = || gateway.requests_to("/skills/index.json").len();
    assert!(index() > 0);
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("one"), TurnOptions::default()))
        .unwrap();
    assert!(
        gateway
            .messages()
            .last()
            .unwrap()
            .body
            .contains("PROJECT RULES FROM THE NETWORK")
    );
    let mut events = core.subscribe();
    decide(false);
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert!(seen.contains(&Event::TrustChanged));
    assert_eq!(shell(&core), Effect::Ask, "the allow stops allowing");
    assert!(!core.loaded().config.instructions.contains(&url));
    assert!(core.loaded().remote_instructions.is_empty());
    assert!(core.references().is_empty(), "the checkout is not used");
    assert!(core.resolve_reference("lib").is_none());
    let fetched = (index(), gateway.requests_to("/proj-rules.md").len());
    runtime.block_on(core.reload());
    assert_eq!(
        (index(), gateway.requests_to("/proj-rules.md").len()),
        fetched,
        "nothing is downloaded any more"
    );
    // The session already open: its next turn is told, and asks for the shell again.
    runtime
        .block_on(core.prompt(&session.id, prompt("two"), TurnOptions::default()))
        .unwrap();
    let body = gateway.messages().last().unwrap().body.clone();
    assert!(
        // It was the only instruction file: the whole set goes.
        body.contains("Previously loaded instructions no longer apply."),
        "{body}"
    );
}

/// Taken back while a turn runs: the step whose request is already out keeps the rules it
/// started with; the next step asks again.
#[test]
fn taking_an_allow_back_mid_turn_applies_from_the_next_step() {
    use anysola_core::session::gate::Reply as Answer;
    use anysola_testkit::sse::Stream;
    use std::sync::{Arc, Mutex, OnceLock};
    let core_cell: Arc<OnceLock<Core>> = Arc::new(OnceLock::new());
    let steps = Arc::new(Mutex::new(0));
    let gateway = FakeGateway::start({
        let core_cell = core_cell.clone();
        let steps = steps.clone();
        move |request, _| {
            if request.is_title() {
                return text(request, "title");
            }
            let step = {
                let mut steps = steps.lock().unwrap();
                *steps += 1;
                *steps
            };
            if step == 2 {
                let core = core_cell.get().unwrap();
                for item in core.trust_items() {
                    if item.kind == TrustKind::Permissions {
                        core.trust(&item.path, item.kind, &item.digest, false)
                            .unwrap();
                    }
                }
            }
            if step > 3 {
                return text(request, "done");
            }
            let model = request.json()["model"].as_str().unwrap().to_string();
            anysola_testkit::Reply::Sse {
                body: Stream::new()
                    .tool(
                        &format!("toolu_{step}"),
                        "shell",
                        json!({"command": format!("echo {step}")}),
                    )
                    .finish(&model, Some(1)),
            }
        }
    });
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "shell", "resource": "*", "effect": "ask"}]}"#,
    );
    env.home.write(
        "proj/anysola.json",
        r#"{"permissions": [{"action": "shell", "resource": "*", "effect": "allow"}]}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    for item in core.trust_items() {
        core.trust(&item.path, item.kind, &item.digest, true)
            .unwrap();
    }
    runtime.block_on(core.reload());
    let _ = core_cell.set(core.clone());
    let mut events = core.subscribe();
    let answering = core.clone();
    let asked = runtime.spawn(async move {
        let mut asked = Vec::new();
        while let Ok(event) = events.recv().await {
            if let Event::PermissionAsked {
                request_id,
                resources,
                ..
            } = &event
            {
                asked.push(resources.clone());
                answering.reply_permission(request_id, Answer::Once);
            }
            if let Event::Succeeded { .. } = event {
                break;
            }
        }
        asked
    });
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let asked = runtime.block_on(asked).unwrap();
    assert_eq!(asked, [vec!["echo 3".to_string()]], "only the step after");
}
