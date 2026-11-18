//! A project's MCP servers and formatters, like its hooks, apply only once the user approved
//! them for their current content; user documents never ask. (A real server process not being
//! started is covered in `crates/mcp/tests/trust.rs`.)

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::{Core, Target, TrustKind};
use anysola_core::events::Event;
use common::*;
use serde_json::{Value, json};

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn server(core: &Core, name: &str) -> Option<Value> {
    core.loaded()
        .config
        .mcp_servers()
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, s)| s)
}

fn item(core: &Core, kind: TrustKind) -> anysola_core::api::TrustItem {
    core.trust_items()
        .into_iter()
        .find(|i| i.kind == kind)
        .expect("an item of that kind")
}

#[test]
fn a_project_mcp_server_waits_for_approval_of_its_content() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let file = env.home.write(
        "proj/anysola.json",
        &json!({"mcp": {"servers": {"proj": {"type": "local", "command": ["proj-server"]}}}})
            .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(
        server(&core, "proj").is_none(),
        "not applied before approval"
    );
    let waiting = item(&core, TrustKind::Mcp);
    assert_eq!(std::path::Path::new(&waiting.path), file);
    assert_eq!(waiting.decision, None);
    assert_eq!(waiting.entries[0].label, "proj");
    assert_eq!(waiting.entries[0].command, "proj-server");
    let mut events = core.subscribe();
    core.trust(&waiting.path, TrustKind::Mcp, &waiting.digest, true)
        .unwrap();
    wait_for("the approved server", || server(&core, "proj").is_some());
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert!(seen.contains(&Event::TrustChanged));
    assert_eq!(item(&core, TrustKind::Mcp).decision, Some(true));
    // A changed entry is not applied and waits again.
    std::fs::write(
        &file,
        json!({"mcp": {"servers": {"proj": {"type": "local", "command": ["other-server"]}}}})
            .to_string(),
    )
    .unwrap();
    runtime.block_on(core.reload());
    assert!(server(&core, "proj").is_none());
    let changed = item(&core, TrustKind::Mcp);
    assert_eq!(changed.decision, None);
    // Refused: stays off.
    core.trust(&changed.path, TrustKind::Mcp, &changed.digest, false)
        .unwrap();
    runtime.block_on(core.reload());
    assert!(server(&core, "proj").is_none());
    assert_eq!(item(&core, TrustKind::Mcp).decision, Some(false));
}

#[test]
fn a_project_cannot_change_a_user_server_without_approval() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        &json!({"mcp": {"servers": {
            "shared": {"type": "local", "command": ["mine"]},
            "other": {"type": "remote", "url": "https://mine/mcp"},
        }}})
        .to_string(),
    );
    env.home.write(
        "proj/anysola.json",
        &json!({"mcp": {"servers": {
            "shared": {"type": "local", "command": ["theirs"]},
            "other": {"type": "remote", "url": "https://mine/mcp", "disabled": true},
        }}})
        .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert_eq!(server(&core, "shared").unwrap()["command"], json!(["mine"]));
    assert!(server(&core, "other").unwrap().get("disabled").is_none());
    let waiting = item(&core, TrustKind::Mcp);
    core.trust(&waiting.path, TrustKind::Mcp, &waiting.digest, true)
        .unwrap();
    runtime.block_on(core.reload());
    assert_eq!(
        server(&core, "shared").unwrap()["command"],
        json!(["theirs"])
    );
    assert_eq!(server(&core, "other").unwrap()["disabled"], true);
}

#[test]
fn project_formatters_wait_for_approval_and_the_users_do_not() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home
        .write("proj/anysola.json", r#"{"formatter": true}"#);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(
        core.loaded().formatters.is_none(),
        "the built-in formatters stay off"
    );
    let waiting = item(&core, TrustKind::Formatter);
    assert_eq!(waiting.entries[0].command, "the built-in formatters");
    core.trust(&waiting.path, TrustKind::Formatter, &waiting.digest, true)
        .unwrap();
    runtime.block_on(core.reload());
    assert!(core.loaded().formatters.is_some());
    drop(core);
    // The user's own formatter needs nothing.
    std::fs::remove_file(env.home.project().join("anysola.json")).unwrap();
    env.home
        .write(".anysola/config.json", r#"{"formatter": true}"#);
    let core = runtime.block_on(common::core(&env));
    assert!(core.loaded().formatters.is_some());
    assert!(core.trust_items().is_empty());
}

#[test]
fn trust_items_agree_with_hook_documents() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        "proj/anysola.json",
        &json!({
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "true", "timeout": 5}]}]},
            "mcp": {"servers": {"s": {"type": "remote", "url": "https://s/mcp"}}},
        })
        .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let items = core.trust_items();
    assert_eq!(
        items.iter().map(|i| i.kind).collect::<Vec<_>>(),
        [TrustKind::Hooks, TrustKind::Mcp]
    );
    let hooks = &core.hook_documents()[0];
    assert_eq!(
        (&items[0].path, &items[0].digest, items[0].decision),
        (&hooks.path, &hooks.digest, hooks.decision)
    );
    assert_eq!(items[0].entries[0].label, "Stop");
    assert_eq!(items[0].entries[0].detail.as_deref(), Some("timeout 5 s"));
    core.trust(&items[0].path, TrustKind::Hooks, &items[0].digest, true)
        .unwrap();
    assert_eq!(core.hook_documents()[0].decision, Some(true));
    assert_eq!(core.trust_items()[0].decision, Some(true));
    assert_eq!(core.trust_items()[1].decision, None, "each kind on its own");
    let (_session, _) = core.open(&Target::New, false).unwrap();
}

#[cfg(unix)]
#[test]
fn a_project_formatter_command_runs_only_once_approved() {
    use anysola_core::api::Prompt;
    use anysola_core::session::TurnOptions;
    let gateway = anysola_testkit::FakeGateway::start(|request, _| {
        let answered = request.body.contains("tool_result");
        if request.is_title() || answered {
            return text(request, "done");
        }
        let model = request.json()["model"].as_str().unwrap().to_string();
        anysola_testkit::Reply::Sse {
            body: anysola_testkit::sse::Stream::new()
                .tool("toolu_1", "write", json!({"path": "a.txt", "content": "x"}))
                .finish(&model, Some(1)),
        }
    });
    let env = env(&gateway.url);
    let marker = env.home.home().join("formatted");
    let formatter = |marker: &std::path::Path| {
        json!({"formatter": {"mark": {
            "command": ["sh", "-c", format!("touch '{}'", marker.display())],
            "extensions": [".txt"],
        }}})
    };
    env.home
        .write("proj/anysola.json", &formatter(&marker).to_string());
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let write = |core: &Core| {
        let (session, _) = core.open(&Target::New, false).unwrap();
        let prompt = Prompt {
            text: "go".into(),
            ..Default::default()
        };
        runtime
            .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
            .unwrap();
    };
    write(&core);
    assert!(!marker.exists(), "not run before approval");
    let waiting = item(&core, TrustKind::Formatter);
    core.trust(&waiting.path, TrustKind::Formatter, &waiting.digest, true)
        .unwrap();
    runtime.block_on(core.reload());
    write(&core);
    assert!(marker.exists(), "runs once approved");
    // The user's own formatter needs no approval.
    drop(core);
    std::fs::remove_file(&marker).unwrap();
    std::fs::remove_file(env.home.project().join("anysola.json")).unwrap();
    env.home
        .write(".anysola/config.json", &formatter(&marker).to_string());
    let core = runtime.block_on(common::core(&env));
    write(&core);
    assert!(marker.exists());
}

/// A shell that leaves a marker, then runs the command with `/bin/sh`.
#[cfg(unix)]
fn marking_shell(env: &Env, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let marker = env.home.home().join(format!("{name}-ran"));
    let script = env.home.write(
        &format!("bin/{name}"),
        &format!(
            "#!/bin/sh\ntouch '{}'\nexec /bin/sh \"$@\"\n",
            marker.display()
        ),
    );
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, marker)
}

#[cfg(unix)]
#[test]
fn a_project_cannot_choose_the_shell() {
    use anysola_core::api::Prompt;
    use anysola_core::session::TurnOptions;
    let gateway = anysola_testkit::FakeGateway::start(|request, _| {
        if request.is_title() || request.body.contains("tool_result") {
            return text(request, "done");
        }
        let model = request.json()["model"].as_str().unwrap().to_string();
        anysola_testkit::Reply::Sse {
            body: anysola_testkit::sse::Stream::new()
                .tool("toolu_1", "shell", json!({"command": "echo hi"}))
                .finish(&model, Some(1)),
        }
    });
    let env = env(&gateway.url);
    let (project_shell, project_marker) = marking_shell(&env, "projsh");
    env.home.write(
        "proj/anysola.json",
        &json!({"shell": project_shell.display().to_string()}).to_string(),
    );
    let runtime = runtime();
    let run = |core: &Core| {
        let (session, _) = core.open(&Target::New, false).unwrap();
        let prompt = Prompt {
            text: "go".into(),
            ..Default::default()
        };
        runtime
            .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
            .unwrap();
        runtime
            .block_on(core.run_shell(&session.id, "echo bang"))
            .unwrap();
    };
    let core = runtime.block_on(common::core(&env));
    assert_eq!(core.loaded().config.shell(), None);
    assert!(
        core.loaded()
            .config
            .warnings
            .iter()
            .any(|w| w.contains("\"shell\"") && w.contains("your own config"))
    );
    run(&core);
    assert!(!project_marker.exists(), "the project's shell never ran");
    drop(core);
    // The user's own shell is used, by the tool and by `!` commands.
    let (user_shell, user_marker) = marking_shell(&env, "usersh");
    env.home.write(
        ".anysola/config.json",
        &json!({"shell": user_shell.display().to_string()}).to_string(),
    );
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.run_shell(&session.id, "echo bang"))
        .unwrap();
    assert!(
        user_marker.exists(),
        "a `!` command ran through the user's shell"
    );
    std::fs::remove_file(&user_marker).unwrap();
    run(&core);
    assert!(user_marker.exists());
    assert!(!project_marker.exists());
}

fn shell_effect(core: &Core) -> anysola_core::permission::Effect {
    let loaded = core.loaded();
    let build = loaded.agents.get("build").unwrap();
    anysola_core::permission::evaluate("shell", "echo hi", &build.permissions)
}

fn approve_all(runtime: &tokio::runtime::Runtime, core: &Core, kind: TrustKind) {
    for item in core.trust_items().into_iter().filter(|i| i.kind == kind) {
        core.trust(&item.path, kind, &item.digest, true).unwrap();
    }
    runtime.block_on(core.reload());
}

#[test]
fn a_project_only_tightens_permissions_until_approved() {
    use anysola_core::permission::Effect;
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "shell", "resource": "*", "effect": "ask"}]}"#,
    );
    // Each with different rules: a decision is per content.
    let allow =
        |resource: &str| json!([{"action": "shell", "resource": resource, "effect": "allow"}]);
    let documents: [(&str, String); 4] = [
        (
            "proj/anysola.json",
            json!({"permissions": allow("*")}).to_string(),
        ),
        (
            "proj/anysola.json",
            json!({"permission": {"bash": {"echo *": "allow"}}}).to_string(),
        ),
        (
            "proj/anysola.json",
            json!({"agents": {"build": {"permissions": allow("echo hi")}}}).to_string(),
        ),
        (
            "proj/.anysola/agent/build.md",
            "---\npermission:\n  bash: allow\n---\nBuild things.\n".to_string(),
        ),
    ];
    let runtime = runtime();
    for (file, text) in &documents {
        let _ = std::fs::remove_file(env.home.project().join("anysola.json"));
        let _ = std::fs::remove_dir_all(env.home.project().join(".anysola"));
        let written = env.home.write(file, text);
        let core = runtime.block_on(common::core(&env));
        assert_eq!(shell_effect(&core), Effect::Ask, "{file}: still asks");
        let items: Vec<_> = core
            .trust_items()
            .into_iter()
            .filter(|i| i.kind == TrustKind::Permissions)
            .collect();
        assert_eq!(items.len(), 1, "{file}");
        assert_eq!(std::path::Path::new(&items[0].path), written, "{file}");
        approve_all(&runtime, &core, TrustKind::Permissions);
        assert_eq!(shell_effect(&core), Effect::Allow, "{file}: approved");
    }
}

#[test]
fn a_project_s_deny_and_ask_rules_apply_at_once_and_ask_rules_ask_nothing() {
    use anysola_core::permission::{Effect, evaluate};
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let rules = |extra: Value| {
        let mut list = vec![
            json!({"action": "read", "resource": "*.secret", "effect": "deny"}),
            json!({"action": "shell", "resource": "npm test*", "effect": "allow"}),
        ];
        list.push(extra);
        json!({"permissions": list}).to_string()
    };
    let file = env.home.write(
        "proj/anysola.json",
        &rules(json!({"action": "webfetch", "resource": "*", "effect": "ask"})),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let effect = |action: &str, resource: &str| {
        let loaded = core.loaded();
        evaluate(
            action,
            resource,
            &loaded.agents.get("build").unwrap().permissions,
        )
    };
    assert_eq!(
        effect("read", "a.secret"),
        Effect::Deny,
        "a deny applies at once"
    );
    assert_eq!(
        effect("webfetch", "https://x"),
        Effect::Ask,
        "an ask applies at once"
    );
    let item = item(&core, TrustKind::Permissions);
    assert_eq!(item.entries.len(), 1, "only the allow waits");
    assert_eq!(item.entries[0].label, "all agents");
    assert_eq!(item.entries[0].command, "allow shell: npm test*");
    approve_all(&runtime, &core, TrustKind::Permissions);
    // Another ask rule: no new question.
    std::fs::write(
        &file,
        rules(json!({"action": "edit", "resource": "*", "effect": "ask"})),
    )
    .unwrap();
    runtime.block_on(core.reload());
    assert_eq!(item_decision(&core, TrustKind::Permissions), Some(true));
}

fn item_decision(core: &Core, kind: TrustKind) -> Option<bool> {
    item(core, kind).decision
}

#[test]
fn a_project_policy_cannot_switch_off_a_user_server_without_approval() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"mcp": {"servers": {"mine": {"type": "remote", "url": "https://mine/mcp"}}}}"#,
    );
    env.home.write(
        "proj/anysola.json",
        r#"{"policies": [{"action": "mcp.use", "resource": "mine", "effect": "deny"}]}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(server(&core, "mine").is_some());
    let waiting = item(&core, TrustKind::Permissions);
    assert_eq!(waiting.entries[0].label, "policy");
    assert_eq!(waiting.entries[0].command, "deny mcp.use: mine");
    approve_all(&runtime, &core, TrustKind::Permissions);
    assert!(server(&core, "mine").is_none());
}

#[test]
fn a_project_downloads_nothing_until_approved() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    gateway.serve("/proj-rules.md", "PROJECT RULES FROM THE NETWORK");
    let env = env(&gateway.url);
    let url = format!("{}/proj-rules.md", gateway.url);
    let repository = format!("{}/lib.git", gateway.url);
    std::fs::create_dir_all(env.home.project().join("docs")).unwrap();
    env.home.write("proj/NOTES.md", "local notes");
    env.home.write(
        "proj/anysola.json",
        &json!({
            "instructions": [url, "NOTES.md"],
            "references": {"lib": {"repository": repository, "branch": "main"}, "docs": "./docs"},
        })
        .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let names =
        |core: &Core| -> Vec<String> { core.references().into_iter().map(|r| r.name).collect() };
    assert_eq!(names(&core), ["docs"], "a local reference needs nothing");
    assert!(
        core.loaded()
            .config
            .instructions
            .contains(&"NOTES.md".to_string())
    );
    assert!(!core.loaded().config.instructions.contains(&url));
    assert!(
        gateway.requests_to("/proj-rules.md").is_empty(),
        "not fetched"
    );
    assert!(
        gateway
            .requests()
            .iter()
            .all(|r| !r.path.contains("lib.git")),
        "not cloned"
    );
    let waiting = item(&core, TrustKind::Remote);
    let commands: Vec<&str> = waiting.entries.iter().map(|e| e.command.as_str()).collect();
    assert_eq!(
        commands,
        [url.clone(), format!("{repository} (branch main)")]
    );
    assert_eq!(waiting.entries[1].label, "lib");
    approve_all(&runtime, &core, TrustKind::Remote);
    assert!(core.loaded().config.instructions.contains(&url));
    assert!(
        !gateway.requests_to("/proj-rules.md").is_empty(),
        "fetched once approved"
    );
    assert!(names(&core).contains(&"lib".to_string()));
}

#[test]
fn trust_items_list_every_kind_of_a_document_in_order() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        "proj/anysola.json",
        &json!({
            "references": {"r": "https://example.com/r.git"},
            "permissions": [{"action": "edit", "resource": "*", "effect": "allow"}],
            "formatter": true,
            "mcp": {"servers": {"s": {"type": "remote", "url": "https://s/mcp"}}},
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "true"}]}]},
        })
        .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert_eq!(
        core.trust_items()
            .iter()
            .map(|i| i.kind)
            .collect::<Vec<_>>(),
        TrustKind::ALL
    );
}

#[cfg(unix)]
#[test]
fn a_project_file_placeholder_never_reads_a_device() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write("proj/name.txt", "notes-from-file");
    env.home.write(
        "proj/anysola.json",
        r#"{"instructions": ["{file:name.txt}"], "default_agent": "{file:/dev/zero}"}"#,
    );
    let runtime = runtime();
    let started = Instant::now();
    let core = runtime.block_on(common::core(&env));
    assert!(started.elapsed() < Duration::from_secs(10));
    let config = core.loaded().config.clone();
    assert_eq!(config.default_agent(), Some("{file:/dev/zero}"));
    assert!(config.instructions.contains(&"notes-from-file".to_string()));
    assert!(
        config
            .warnings
            .iter()
            .any(|w| w.contains("{file:/dev/zero}") && w.contains("not a regular file")),
        "{:?}",
        config.warnings
    );
}
