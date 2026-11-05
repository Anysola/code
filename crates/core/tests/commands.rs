//! Slash commands through the core API: markdown templates with arguments and shell output,
//! the built-ins, and subagent commands that report back to the parent.

mod common;

use anysola_core::api::{Prompt, Target};
use anysola_core::session::{Outcome, TurnOptions};
use anysola_testkit::FakeGateway;
use common::*;

fn user_texts(gateway: &FakeGateway) -> Vec<String> {
    gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .map(|r| transcript(&r).last().cloned().unwrap_or_default())
        .collect()
}

#[test]
fn markdown_command_with_arguments_and_shell() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        "proj/.opencode/command/hello.md",
        "---\ndescription: Greets\n---\nSay $1 to !`echo world`\n",
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert_eq!(
        core.commands().get("hello").unwrap().description.as_deref(),
        Some("Greets")
    );
    let (session, _) = core.open(&Target::New, false).unwrap();
    let outcome = runtime
        .block_on(core.command(&session.id, "hello", "hi there", TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let init = runtime
        .block_on(core.command(&session.id, "init", "be brief", TurnOptions::default()))
        .unwrap();
    assert_eq!(init, Outcome::Succeeded);
    let texts = user_texts(&gateway);
    assert_eq!(texts[0], "user: Say hi there to world\n");
    let project = env.home.project().display().to_string();
    assert!(texts[1].starts_with("user: Create or update `AGENTS.md` for this repository."));
    assert!(texts[1].contains(&project), "{}", texts[1]);
    assert!(texts[1].contains("be brief"));
    assert!(!texts[1].contains("$ARGUMENTS"));
}

#[test]
fn subagent_command_reports_to_the_parent() {
    let gateway = FakeGateway::start(|request, _| {
        if request
            .body
            .contains("You are a subagent spawned by another session.")
        {
            text(request, "child result")
        } else {
            text(request, "parent ok")
        }
    });
    let env = env(&gateway.url);
    env.home.write(
        "proj/.opencode/command/scan.md",
        "---\ndescription: Scan things\nsubagent: true\n---\nScan $ARGUMENTS\n",
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(
            &session.id,
            Prompt {
                text: "start".into(),
                files: vec![],
                skills: Vec::new(),
            },
            TurnOptions::default(),
        ))
        .unwrap();
    let outcome = runtime
        .block_on(core.command(&session.id, "scan", "the repo", TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let texts = user_texts(&gateway);
    assert_eq!(
        texts[1],
        "user: You are a subagent spawned by another session.\nScan the repo"
    );
    let completion = &texts[2];
    assert!(
        completion.starts_with("user: <subagent sessionID=\"ses_"),
        "{completion}"
    );
    assert!(completion.ends_with(
        "\" state=\"completed\" description=\"Scan things\">\nchild result\n</subagent>"
    ));
}
