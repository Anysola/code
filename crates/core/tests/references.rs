//! Config `references`: the `<available_references>` source in block 2 (after the Code Mode
//! catalog, before the instruction files), its update when a reference changes, and `@` mentions.

mod common;

use std::path::PathBuf;

use anysola_core::api::{Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::FakeGateway;
use common::*;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        files: Vec::new(),
    }
}

#[test]
fn references_in_block_two_and_mentions() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let project = env.home.project();
    env.home.write(
        "proj/opencode.json",
        r#"{"references": {"docs": {"path": "./docs", "description": "Project docs"}, "quiet": "./quiet"}}"#,
    );
    env.home.write("proj/AGENTS.md", "Rules.");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert_eq!(
        core.resolve_reference("docs/guide.md"),
        Some(project.join("docs/guide.md"))
    );
    assert_eq!(
        core.references()
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["docs", "quiet"]
    );
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("one"), TurnOptions::default()))
        .unwrap();

    env.home.write(
        "proj/opencode.json",
        r#"{"references": {"docs": {"path": "./docs", "description": "All docs"}}}"#,
    );
    runtime.block_on(core.refresh());
    let reloaded = std::time::Instant::now();
    while core.references().len() != 1 && reloaded.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        runtime.block_on(core.refresh());
    }
    runtime
        .block_on(core.prompt(&session.id, prompt("two"), TurnOptions::default()))
        .unwrap();

    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let block2 = main[0].json()["system"][2]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let docs = PathBuf::from(&project).join("docs").display().to_string();
    let listing = format!(
        "Project references provide additional directories that can be accessed when relevant.\n<available_references>\n  <reference>\n    <name>docs</name>\n    <path>{docs}</path>\n    <description>Project docs</description>\n  </reference>\n</available_references>"
    );
    let (before, after) = block2.split_once(&listing).expect("listing in block 2");
    assert!(before.starts_with("# Code Mode"), "{before}");
    assert!(before.ends_with("\n\n"));
    assert!(after.starts_with("\n\nInstructions from: "), "{after}");

    let update = transcript(&main[1])
        .into_iter()
        .find(|t| t.contains("project references have changed"))
        .expect("reference update");
    assert_eq!(
        update,
        format!(
            "system: The following project references have changed. Each entry replaces the previously listed reference with the same name:\n  <reference>\n    <name>docs</name>\n    <path>{docs}</path>\n    <description>All docs</description>\n  </reference>"
        )
    );
}
