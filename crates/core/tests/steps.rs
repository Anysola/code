//! Step boundaries: what is selected again before each step, and how a step ends.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::FakeGateway;
use anysola_testkit::sse::Stream;
use common::*;
use serde_json::json;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        files: vec![],
    }
}

fn sse(request: &anysola_testkit::Recorded, stream: Stream) -> anysola_testkit::Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    anysola_testkit::Reply::Sse {
        body: stream.finish(&model, Some(1)),
        before: Some(100),
    }
}

#[test]
fn config_changes_apply_at_the_next_step() {
    let core: Arc<OnceLock<Core>> = Arc::default();
    let project: Arc<OnceLock<std::path::PathBuf>> = Arc::default();
    let (handle, dir) = (core.clone(), project.clone());
    let steps = AtomicUsize::new(0);
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if steps.fetch_add(1, Ordering::SeqCst) == 0 {
            let dir = dir.get().unwrap();
            std::fs::write(
                dir.join("opencode.json"),
                r#"{"agents": {"build": {"system": "Reloaded prompt."}}}"#,
            )
            .unwrap();
            std::fs::write(dir.join("AGENTS.md"), "Reloaded rules.\n").unwrap();
            handle.get().unwrap().reload();
            return sse(
                request,
                Stream::new().tool("toolu_1", "glob", json!({"pattern": "*.md"})),
            );
        }
        text(request, "done")
    });
    let env = env(&gateway.url);
    project.set(env.home.project()).unwrap();
    let runtime = runtime();
    let started = runtime.block_on(common::core(&env));
    let _ = core.set(started.clone());
    let (session, _) = started.open(&Target::New, false).unwrap();
    runtime
        .block_on(started.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let requests: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(requests.len(), 2);
    let first = requests[0].json();
    let second = requests[1].json();
    assert_ne!(first["system"][0]["text"], "Reloaded prompt.");
    assert_eq!(second["system"][0]["text"], "Reloaded prompt.");
    let last = transcript(&requests[1]);
    assert!(
        last.iter()
            .any(|t| t.starts_with("system: Instructions from: ")
                && t.ends_with("Reloaded rules.\n")),
        "{last:?}"
    );
}
