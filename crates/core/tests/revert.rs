//! Snapshots and revert: a step that edits a file records its trees; reverting restores the file,
//! clearing puts the edit back, and prompting while reverted drops the reverted history.

mod common;

use anysola_core::api::{Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_core::storage::Db;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Reply};
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
fn revert_unrevert_commit() {
    let gateway = FakeGateway::start(|request, _| {
        let body = request.body.clone();
        if request.is_title() || body.contains("tool_result") || !body.contains("write it") {
            return text(request, "done");
        }
        let model = request.json()["model"].as_str().unwrap().to_string();
        let path = body
            .split("Working directory: ")
            .nth(1)
            .and_then(|rest| rest.split("\\n").next())
            .unwrap()
            .to_string();
        Reply::Sse {
            body: Stream::new()
                .tool(
                    "toolu_w",
                    "write",
                    json!({"path": format!("{path}/a.txt"), "content": "two\n"}),
                )
                .finish(&model, Some(1)),
        }
    });
    let env = env(&gateway.url);
    let project = env.home.project();
    std::fs::write(project.join("a.txt"), "one\n").unwrap();
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("write it"), TurnOptions::default()))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("a.txt")).unwrap(),
        "two\n"
    );

    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let rows = db.messages(&session.id).unwrap();
    let edit = rows.iter().find(|r| r.kind == "assistant").unwrap();
    assert_eq!(edit.data["snapshot"]["files"], json!(["a.txt"]));
    let user = rows.iter().find(|r| r.kind == "user").unwrap().id.clone();

    let revert = core.revert(&session.id, &user).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("a.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(revert["messageID"], user.as_str());
    assert_eq!(revert["files"][0]["file"], "a.txt");
    assert_eq!(revert["files"][0]["status"], "modified");

    core.unrevert(&session.id).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("a.txt")).unwrap(),
        "two\n"
    );
    assert!(db.revert(&session.id).unwrap().is_none());

    // Review: the net change per file, and one file put back on its own.
    let changes = core.session_changes(&session.id).unwrap().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["file"], "a.txt");
    assert_eq!(changes[0]["additions"], 1);
    assert_eq!(changes[0]["deletions"], 1);
    // The write's result carries its diff, as edit's does.
    assert_eq!(
        edit.data["content"][0]["state"]["metadata"]["files"][0]["file"],
        "a.txt"
    );
    core.restore_file(&session.id, "a.txt").unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("a.txt")).unwrap(),
        "one\n"
    );
    assert!(
        core.session_changes(&session.id)
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(core.restore_file(&session.id, "b.txt").is_err());
    std::fs::write(project.join("a.txt"), "two\n").unwrap();

    core.revert(&session.id, &user).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("again"), TurnOptions::default()))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("a.txt")).unwrap(),
        "one\n"
    );
    let kinds: Vec<String> = db
        .messages(&session.id)
        .unwrap()
        .into_iter()
        .map(|r| r.kind)
        .collect();
    assert!(
        !db.messages(&session.id)
            .unwrap()
            .iter()
            .any(|r| r.id == user)
    );
    assert_eq!(kinds.first().map(String::as_str), Some("user"));
    let last = gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title())
        .unwrap();
    assert_eq!(transcript(&last), vec!["user: again"]);
}

#[test]
fn a_notebook_edit_is_captured_and_reverted() {
    let gateway = FakeGateway::start(|request, _| {
        let body = request.body.clone();
        if request.is_title() || body.contains("tool_result") || !body.contains("change cell") {
            return text(request, "done");
        }
        let model = request.json()["model"].as_str().unwrap().to_string();
        Reply::Sse {
            body: Stream::new()
                .tool(
                    "toolu_n",
                    "notebook",
                    json!({"path": "n.ipynb", "index": 0, "source": "x = 2"}),
                )
                .finish(&model, Some(1)),
        }
    });
    let env = env(&gateway.url);
    let project = env.home.project();
    let original = "{\n \"cells\": [\n  {\n   \"cell_type\": \"code\",\n   \"execution_count\": 1,\n   \"metadata\": {},\n   \"outputs\": [],\n   \"source\": [\n    \"x = 1\"\n   ]\n  }\n ],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 4\n}\n";
    std::fs::write(project.join("n.ipynb"), original).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("change cell"), TurnOptions::default()))
        .unwrap();
    let changed = std::fs::read_to_string(project.join("n.ipynb")).unwrap();
    assert!(changed.contains("\"x = 2\""), "{changed}");
    assert!(changed.contains("\"execution_count\": null"));
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    let rows = db.messages(&session.id).unwrap();
    let step = rows.iter().find(|r| r.kind == "assistant").unwrap();
    assert_eq!(step.data["snapshot"]["files"], json!(["n.ipynb"]));
    let tool = &step.data["content"][0];
    assert_eq!(tool["state"]["metadata"]["files"][0]["file"], "n.ipynb");
    let user = rows.iter().find(|r| r.kind == "user").unwrap().id.clone();
    core.revert(&session.id, &user).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("n.ipynb")).unwrap(),
        original
    );
    core.unrevert(&session.id).unwrap();
    assert_eq!(
        std::fs::read_to_string(project.join("n.ipynb")).unwrap(),
        changed
    );
}
