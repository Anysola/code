//! What the agent may do, end to end through config loading and real turns: a project's rules
//! never weaken the user's denials (even approved), and a subagent never does more than the
//! sessions above it.

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::{Core, Prompt, Target, TrustKind};
use anysola_core::events::Event;
use anysola_core::permission::{Effect, evaluate, wholly_disabled};
use anysola_core::session::TurnOptions;
use anysola_core::session::gate::Reply as Answer;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Default::default()
    }
}

fn last(request: &Recorded) -> String {
    request.json()["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(Value::to_string)
        .unwrap_or_default()
}

fn answered(request: &Recorded) -> bool {
    last(request).contains("tool_result")
}

fn call(request: &Recorded, name: &str, input: Value) -> Reply {
    let model = request.json()["model"].as_str().unwrap().to_string();
    Reply::Sse {
        body: Stream::new()
            .tool("toolu_1", name, input)
            .finish(&model, Some(1)),
    }
}

fn is_child(request: &Recorded) -> bool {
    request.header("x-opencode-parent-session-id").is_some()
}

fn tool_names(request: &Recorded) -> Vec<String> {
    request.json()["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

fn model_requests(gateway: &FakeGateway) -> Vec<Recorded> {
    gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect()
}

/// The tool results the children got back, in order.
fn child_results(gateway: &FakeGateway) -> Vec<String> {
    model_requests(gateway)
        .into_iter()
        .filter(|r| is_child(r) && answered(r))
        .map(|r| last(&r))
        .collect()
}

fn subagent(agent: &str, prompt: &str) -> Value {
    json!({"agent": agent, "description": "Do it", "prompt": prompt})
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn approve_permissions(runtime: &tokio::runtime::Runtime, core: &Core) {
    for item in core.trust_items() {
        if item.kind == TrustKind::Permissions {
            core.trust(&item.path, item.kind, &item.digest, true)
                .unwrap();
        }
    }
    runtime.block_on(core.reload());
}

/// The whole 1.1.1 table through config loading: a user document and a project document (with
/// agent-level rules on both sides and a project agent markdown file), everything approved.
#[test]
fn a_project_cannot_weaken_the_users_denials_even_approved() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let rule = |action: &str, resource: &str, effect: &str| json!({"action": action, "resource": resource, "effect": effect});
    env.home.write(
        ".anysola/config.json",
        &json!({
            "permissions": [
                rule("shell", "rm *", "deny"),
                rule("shell", "git push*", "ask"),
                rule("shell", "ls*", "allow"),
                rule("webfetch", "*", "deny"),
                rule("subagent", "explore", "deny"),
            ],
            "agents": {"build": {"permissions": [rule("edit", "*.lock", "deny")]}},
        })
        .to_string(),
    );
    env.home.write(
        "proj/anysola.json",
        &json!({
            "permissions": [
                // Trying to pass as the user's: the loader decides, not the document.
                {"action": "shell", "resource": "rm *", "effect": "ask", "project": false},
                rule("shell", "rm -rf*", "allow"),
                rule("shell", "git push*", "allow"),
                rule("shell", "ls*", "ask"),
                rule("webfetch", "https://docs.example/*", "allow"),
                rule("subagent", "*", "allow"),
                rule("read", "*.pem", "deny"),
            ],
            "agents": {"build": {"permissions": [rule("edit", "*", "allow")]}},
        })
        .to_string(),
    );
    env.home.write(
        "proj/.anysola/agent/build.md",
        "---\npermission:\n  edit:\n    \"*.lock\": allow\n  bash:\n    \"rm *\": allow\n---\nBuild things.\n",
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let check = |core: &Core| {
        let loaded = core.loaded();
        let build = &loaded.agents.get("build").unwrap().permissions;
        let effect = |action: &str, resource: &str| evaluate(action, resource, build);
        assert_eq!(effect("shell", "rm x"), Effect::Deny, "deny / ask");
        assert_eq!(effect("shell", "rm -rf x"), Effect::Deny, "deny / allow");
        assert_eq!(
            effect("edit", "Cargo.lock"),
            Effect::Deny,
            "agent-level deny"
        );
        assert_eq!(effect("edit", "src/a.rs"), Effect::Allow);
        assert_eq!(effect("shell", "ls -la"), Effect::Ask, "allow / ask");
        assert_eq!(effect("read", "key.pem"), Effect::Deny, "allow / deny");
        assert!(
            wholly_disabled("webfetch", build),
            "stays out of the request"
        );
        let subagents: Vec<String> = loaded
            .agents
            .subagents_for(build)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert!(!subagents.contains(&"explore".to_string()), "{subagents:?}");
    };
    check(&core);
    assert_eq!(
        evaluate(
            "shell",
            "git push",
            &core.loaded().agents.get("build").unwrap().permissions
        ),
        Effect::Ask,
        "an unapproved allow is withheld"
    );
    approve_permissions(&runtime, &core);
    assert!(
        core.trust_items()
            .iter()
            .filter(|i| i.kind == TrustKind::Permissions)
            .all(|i| i.decision == Some(true))
    );
    check(&core);
    // The user's ask meets the project's approved allow: the user approved that exact rule.
    assert_eq!(
        evaluate(
            "shell",
            "git push",
            &core.loaded().agents.get("build").unwrap().permissions
        ),
        Effect::Allow
    );
}

/// Through a real turn: the gate refuses what the user denies although an approved project rule
/// allows it, without asking, and the wholly denied tool is not offered.
#[test]
fn the_gate_refuses_what_the_user_denies_after_approval() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() || answered(request) {
            return text(request, "done");
        }
        call(request, "shell", json!({"command": "rm -f victim"}))
    });
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "shell", "resource": "rm *", "effect": "deny"},
                            {"action": "webfetch", "resource": "*", "effect": "deny"}]}"#,
    );
    env.home.write(
        "proj/anysola.json",
        r#"{"permissions": [{"action": "shell", "resource": "*", "effect": "allow"},
                            {"action": "webfetch", "resource": "*", "effect": "allow"}]}"#,
    );
    let victim = env.home.write("proj/victim", "keep me");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    approve_permissions(&runtime, &core);
    let mut events = core.subscribe();
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(victim.exists());
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, Event::PermissionAsked { .. }),
            "denied without asking"
        );
    }
    let requests = model_requests(&gateway);
    assert!(last(&requests[1]).contains("Permission denied: shell"));
    assert!(!tool_names(&requests[0]).contains(&"webfetch".to_string()));
    assert!(tool_names(&requests[0]).contains(&"shell".to_string()));
}

fn root_and_child(
    root: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
    child: impl Fn(&Recorded) -> Reply + Send + Sync + 'static,
) -> FakeGateway {
    FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if is_child(request) {
            child(request)
        } else {
            root(request)
        }
    })
}

/// A root that delegates once to `agent`, and a child that makes one call.
fn delegating(agent: &'static str, name: &'static str, input: Value) -> FakeGateway {
    root_and_child(
        move |request| {
            if answered(request) {
                return text(request, "done");
            }
            call(request, "subagent", subagent(agent, "do it"))
        },
        move |request| {
            if answered(request) {
                return text(request, "child done");
            }
            call(request, name, input.clone())
        },
    )
}

fn run(runtime: &tokio::runtime::Runtime, core: &Core, agent: &str, text: &str) -> String {
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.select_agent(&session.id, agent).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
        .unwrap();
    session.id
}

#[test]
fn a_general_child_of_plan_cannot_edit_outside_the_plan_dir() {
    let gateway = delegating(
        "general",
        "write",
        json!({"path": "made-by-child.txt", "content": "x"}),
    );
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    run(&runtime, &core, "plan", "go");
    assert!(!env.home.project().join("made-by-child.txt").exists());
    let results = child_results(&gateway);
    assert!(
        results[0].contains("Permission denied: edit"),
        "{results:?}"
    );
    // Under build the same child may write.
    let core2 = core.clone();
    run(&runtime, &core2, "build", "go");
    assert!(env.home.project().join("made-by-child.txt").exists());
}

#[test]
fn the_users_rules_for_build_ask_in_its_general_child() {
    let gateway = delegating(
        "general",
        "shell",
        json!({"command": "git push origin main"}),
    );
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"agents": {"build": {"permissions": [{"action": "shell", "resource": "git push*", "effect": "ask"}]}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let mut events = core.subscribe();
    let answering = core.clone();
    let asked = runtime.spawn(async move {
        loop {
            if let Ok(Event::PermissionAsked {
                session_id,
                request_id,
                action,
                resources,
                ..
            }) = events.recv().await
            {
                answering.reply_permission(&request_id, Answer::Reject(Some("not now".into())));
                return (session_id, action, resources);
            }
        }
    });
    let root = run(&runtime, &core, "build", "go");
    let (session_id, action, resources) = runtime.block_on(asked).unwrap();
    let child = core.children(&root).unwrap();
    assert_eq!(session_id, child[0].0, "the child asked");
    assert_eq!(action, "shell");
    assert_eq!(resources, ["git push origin main"]);
}

/// A custom primary agent that may not run commands or fetch: its `general` child neither (and is
/// not offered the shell), not through code mode either.
#[test]
fn a_child_of_an_agent_without_the_shell_has_none_either() {
    let fetched = "/fetched-by-child";
    let gateway = root_and_child(
        |request| {
            if answered(request) {
                return text(request, "done");
            }
            call(request, "subagent", subagent("general", "do it"))
        },
        move |request| {
            let done = request.json()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
                .filter(|p| p["type"] == "tool_result")
                .count();
            let url = format!(
                "http://{}{fetched}",
                request.header("host").unwrap_or("127.0.0.1")
            );
            match done {
                0 => call(request, "shell", json!({"command": "touch child-made"})),
                1 => call(
                    request,
                    "execute",
                    json!({"code": format!("return await (await fetch('{url}')).text()")}),
                ),
                _ => text(request, "child done"),
            }
        },
    );
    gateway.serve(fetched, "fetched");
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        &json!({"agents": {"locked": {"mode": "primary", "permissions": [
            {"action": "shell", "resource": "*", "effect": "deny"},
            {"action": "webfetch", "resource": "*", "effect": "deny"},
        ]}}})
        .to_string(),
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    run(&runtime, &core, "locked", "go");
    assert!(!env.home.project().join("child-made").exists());
    assert!(gateway.requests_to(fetched).is_empty(), "code mode too");
    let results = child_results(&gateway);
    assert!(
        results[0].contains("No tool named") && results[0].contains("shell"),
        "not even offered: {results:?}"
    );
    assert!(
        results[1].contains("Permission denied: webfetch"),
        "{results:?}"
    );
    let child_request = model_requests(&gateway).into_iter().find(is_child).unwrap();
    let tools = tool_names(&child_request);
    assert!(!tools.contains(&"shell".to_string()), "{tools:?}");
    assert!(tools.contains(&"execute".to_string()), "{tools:?}");
}

#[test]
fn two_levels_down_the_root_still_decides() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        if answered(request) {
            return text(request, "done");
        }
        if !is_child(request) {
            return call(request, "subagent", subagent("middle", "delegate"));
        }
        if request.body.contains("MIDDLE-AGENT") {
            return call(request, "subagent", subagent("general", "write it"));
        }
        call(
            request,
            "write",
            json!({"path": "deep.txt", "content": "x"}),
        )
    });
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"experimental": {"subagent_depth": 2},
            "agents": {"middle": {"mode": "subagent", "system": "MIDDLE-AGENT"}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let root = run(&runtime, &core, "plan", "go");
    let middle = core.children(&root).unwrap();
    assert_eq!(middle.len(), 1);
    assert_eq!(
        core.children(&middle[0].0).unwrap().len(),
        1,
        "two levels ran"
    );
    assert!(!env.home.project().join("deep.txt").exists());
    assert!(
        child_results(&gateway)
            .iter()
            .any(|r| r.contains("Permission denied: edit"))
    );
}

#[test]
fn a_background_child_works_under_its_parent_too() {
    let gateway = root_and_child(
        |request| {
            if answered(request) {
                return text(request, "done");
            }
            let mut input = subagent("general", "write it");
            input["background"] = json!(true);
            call(request, "subagent", input)
        },
        |request| {
            if answered(request) {
                return text(request, "child done");
            }
            call(request, "write", json!({"path": "bg.txt", "content": "x"}))
        },
    );
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let root = run(&runtime, &core, "plan", "go");
    wait_for("the background child", || {
        core.children(&root)
            .unwrap()
            .first()
            .is_some_and(|c| !core.child_running(&c.0))
            && !child_results(&gateway).is_empty()
    });
    assert!(!env.home.project().join("bg.txt").exists());
    assert!(child_results(&gateway)[0].contains("Permission denied: edit"));
}

/// The child is created under build, then the user turns Plan on and the parent continues it:
/// the parent's rules as of the child's turn apply.
#[test]
fn a_continued_child_works_under_its_parent_as_it_is_now() {
    let gateway = root_and_child(
        |request| {
            if answered(request) {
                return text(request, "done");
            }
            if !last(request).contains("second") {
                return call(request, "subagent", subagent("general", "say hi"));
            }
            let body = request.body.clone();
            let at = body.find("sessionID=\\\"ses_").expect("the child's id") + 12;
            let id: String = body[at..].chars().take_while(|c| *c != '\\').collect();
            let mut input = subagent("general", "WRITE NOW");
            input["sessionID"] = json!(id);
            call(request, "subagent", input)
        },
        |request| {
            if answered(request) || !last(request).contains("WRITE NOW") {
                return text(request, "hi");
            }
            call(
                request,
                "write",
                json!({"path": "later.txt", "content": "x"}),
            )
        },
    );
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let root = run(&runtime, &core, "build", "first");
    assert_eq!(core.children(&root).unwrap().len(), 1);
    core.select_agent(&root, "plan").unwrap();
    runtime
        .block_on(core.prompt(&root, prompt("second"), TurnOptions::default()))
        .unwrap();
    assert_eq!(core.children(&root).unwrap().len(), 1, "the same child");
    assert!(!env.home.project().join("later.txt").exists());
    assert!(
        child_results(&gateway)
            .iter()
            .any(|r| r.contains("Permission denied: edit"))
    );
}

/// Settings' data: every rule with the agent it applies to, its file, and whether it is in
/// effect or waits for approval, agent-level and markdown-agent rules included.
#[test]
fn configured_rules_list_agent_level_rules_applied_and_waiting() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let user = env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "shell", "resource": "*", "effect": "ask"}],
            "agents": {"build": {"permissions": [{"action": "edit", "resource": "*.lock", "effect": "deny"}]}}}"#,
    );
    let project = env.home.write(
        "proj/anysola.json",
        r#"{"permissions": [{"action": "read", "resource": "*.pem", "effect": "deny"}],
            "agents": {"build": {"permissions": [{"action": "shell", "resource": "npm test*", "effect": "allow"}]}}}"#,
    );
    let file = env.home.write(
        "proj/.anysola/agent/review.md",
        "---\nmode: subagent\npermission:\n  edit: deny\n  bash:\n    \"git diff*\": allow\n---\nReview.\n",
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let shown = |core: &Core| -> Vec<(String, Option<String>, std::path::PathBuf, bool, bool)> {
        core.configured_rules()
            .into_iter()
            .map(|r| {
                (
                    format!("{:?} {}: {}", r.effect, r.action, r.resource),
                    r.agent,
                    std::path::PathBuf::from(r.source),
                    r.project,
                    r.applied,
                )
            })
            .collect()
    };
    // Compared as paths: the test writes `proj/x` with `/` on every OS.
    let path = |p: &std::path::Path| p.to_path_buf();
    let build = Some("build".to_string());
    let review = Some("review".to_string());
    let expected = |approved: bool| {
        vec![
            ("Ask shell: *".to_string(), None, path(&user), false, true),
            (
                "Deny edit: *.lock".into(),
                build.clone(),
                path(&user),
                false,
                true,
            ),
            ("Deny read: *.pem".into(), None, path(&project), true, true),
            (
                "Allow shell: npm test*".into(),
                build.clone(),
                path(&project),
                true,
                approved,
            ),
            (
                "Deny edit: *".into(),
                review.clone(),
                path(&file),
                true,
                true,
            ),
            (
                "Allow shell: git diff*".into(),
                review.clone(),
                path(&file),
                true,
                approved,
            ),
        ]
    };
    assert_eq!(shown(&core), expected(false));
    approve_permissions(&runtime, &core);
    assert_eq!(shown(&core), expected(true));
}

/// Disabling an agent in a project and defining it again (in another project file) does not
/// start it afresh: its built-in and user rules stay. And an approved project allow cannot lift
/// a built-in denial either (Plan's edits).
#[test]
fn a_project_cannot_reset_an_agent_to_drop_its_rules() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"agents": {"build": {"permissions": [{"action": "shell", "resource": "rm *", "effect": "deny"}]}}}"#,
    );
    env.home.write(
        "proj/anysola.json",
        r#"{"agents": {"build": {"disabled": true}, "plan": {"disabled": true}}}"#,
    );
    env.home.write(
        "proj/.anysola/anysola.json",
        r#"{"agents": {"build": {"description": "Again"},
                       "plan": {"description": "Again",
                                "permissions": [{"action": "edit", "resource": "*", "effect": "allow"}]}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    approve_permissions(&runtime, &core);
    let loaded = core.loaded();
    let build = &loaded.agents.get("build").unwrap();
    assert_eq!(build.description.as_deref(), Some("Again"));
    assert_eq!(evaluate("shell", "rm x", &build.permissions), Effect::Deny);
    let plan = &loaded.agents.get("plan").unwrap().permissions;
    assert_eq!(evaluate("edit", "src/a.rs", plan), Effect::Deny);
}

/// A partial denial of the parent's reaches the child's gate (the tool is offered, the call is
/// refused).
#[test]
fn a_child_is_refused_what_its_parent_is_denied() {
    let gateway = delegating("general", "shell", json!({"command": "rm -f victim"}));
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"agents": {"careful": {"mode": "primary", "permissions": [
            {"action": "shell", "resource": "rm *", "effect": "deny"}]}}}"#,
    );
    let victim = env.home.write("proj/victim", "keep me");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    run(&runtime, &core, "careful", "go");
    assert!(victim.exists());
    assert!(child_results(&gateway)[0].contains("Permission denied: shell"));
}
