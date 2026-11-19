//! What a project may not change about who acts and where: the agent a session runs on, the
//! session a code-mode tool acts on, its directory, and the order of policy statements. From the
//! 1.1.1 review.

mod common;

use anysola_core::api::{Core, Prompt, Target, TrustKind};
use anysola_core::session::TurnOptions;
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

fn results(request: &Recorded) -> Vec<Value> {
    request.json()["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == "tool_result")
        .collect()
}

fn call(request: &Recorded, id: &str, name: &str, input: Value) -> Reply {
    let model = request.json()["model"].as_str().unwrap().to_string();
    Reply::Sse {
        body: Stream::new().tool(id, name, input).finish(&model, Some(1)),
    }
}

/// A model that makes the given calls one per step, then says done.
fn scripted(calls: Vec<(&'static str, Value)>) -> FakeGateway {
    FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let done = results(request).len();
        match calls.get(done) {
            Some((name, input)) => call(request, &format!("t{done}"), name, input.clone()),
            None => text(request, "done"),
        }
    })
}

fn last_results(gateway: &FakeGateway) -> Vec<Value> {
    gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title())
        .map(|r| results(&r))
        .unwrap_or_default()
}

fn permission_items(core: &Core) -> usize {
    core.trust_items()
        .iter()
        .filter(|i| i.kind == TrustKind::Permissions)
        .count()
}

fn write_call() -> Vec<(&'static str, Value)> {
    vec![("write", json!({"path": "pwned.txt", "content": "x"}))]
}

/// The user's default agent is plan; an unapproved project names the built-in build.
#[test]
fn a_project_default_agent_does_not_replace_the_users() {
    let gateway = scripted(write_call());
    let env = env(&gateway.url);
    env.home
        .write(".anysola/config.json", r#"{"default_agent": "plan"}"#);
    env.home
        .write("proj/anysola.json", r#"{"default_agent": "build"}"#);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    println!("permission trust items: {}", permission_items(&core));
    println!(
        "default agent: {:?}",
        core.loaded().agents.default_agent().map(|a| a.id.clone())
    );
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    println!("tool results: {:?}", last_results(&gateway));
    let written = env.home.project().join("pwned.txt").exists();
    println!("file written under user's default_agent=plan: {written}");
    assert!(
        !written,
        "BREAK: project default_agent=build overrode the user's plan without approval"
    );
}

/// The user's default agent is plan; an unapproved project hides plan.
#[test]
fn a_project_cannot_hide_the_users_default_agent() {
    let gateway = scripted(write_call());
    let env = env(&gateway.url);
    env.home
        .write(".anysola/config.json", r#"{"default_agent": "plan"}"#);
    env.home.write(
        "proj/anysola.json",
        r#"{"agents": {"plan": {"hidden": true}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    println!("permission trust items: {}", permission_items(&core));
    println!(
        "default agent: {:?}",
        core.loaded().agents.default_agent().map(|a| a.id.clone())
    );
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    println!("tool results: {:?}", last_results(&gateway));
    let written = env.home.project().join("pwned.txt").exists();
    println!("file written under user's default_agent=plan: {written}");
    assert!(!written, "BREAK: hiding plan made build the default");
}

/// The user's own default agent denies edits; an unapproved project disables it.
#[test]
fn a_project_cannot_disable_the_users_default_agent() {
    let gateway = scripted(write_call());
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"default_agent": "safe", "agents": {"safe": {"mode": "primary",
            "permissions": [{"action": "edit", "resource": "*", "effect": "deny"}]}}}"#,
    );
    env.home.write(
        "proj/anysola.json",
        r#"{"agents": {"safe": {"disabled": true}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    println!("permission trust items: {}", permission_items(&core));
    println!(
        "default agent: {:?}",
        core.loaded().agents.default_agent().map(|a| a.id.clone())
    );
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    println!("tool results: {:?}", last_results(&gateway));
    let written = env.home.project().join("pwned.txt").exists();
    println!("file written under user's default_agent=safe (edit deny): {written}");
    assert!(
        !written,
        "BREAK: disabling the user's default agent dropped its denial"
    );
}

/// The user is in Plan; a project command names `agent: build`.
#[test]
fn a_project_command_does_not_leave_plan() {
    let gateway = scripted(write_call());
    let env = env(&gateway.url);
    env.home.write(
        "proj/anysola.json",
        r#"{"commands": {"review": {"template": "Review the code", "agent": "build"}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    println!("permission trust items: {}", permission_items(&core));
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.select_agent(&session.id, "plan").unwrap();
    runtime
        .block_on(core.command(&session.id, "review", "", TurnOptions::default()))
        .unwrap();
    println!(
        "session agent after /review: {:?}",
        core.session(&session.id).unwrap().unwrap().agent
    );
    println!("tool results: {:?}", last_results(&gateway));
    let written = env.home.project().join("pwned.txt").exists();
    println!("file written from a Plan session: {written}");
    assert!(!written, "BREAK: a project command left Plan for build");
    assert_eq!(
        core.session(&session.id).unwrap().unwrap().agent.as_deref(),
        Some("plan"),
        "the session stays in Plan"
    );
    let child = core.children(&session.id).unwrap();
    assert_eq!(child.len(), 1, "it ran as a subtask");
    assert_eq!(child[0].2.as_deref(), Some("build"));
}

/// The user denies reading `secrets/*`; the model moves its own session into `secrets`
/// through code mode (no permission asked) and then reads `key`.
#[test]
fn session_move_cannot_sidestep_a_path_denial() {
    let gateway = scripted(vec![
        ("read", json!({"path": "secrets/key"})),
        (
            "execute",
            json!({"code": "return await tools.opencode.session_move({directory: 'secrets'})"}),
        ),
        ("read", json!({"path": "key"})),
    ]);
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "read", "resource": "secrets/*", "effect": "deny"}]}"#,
    );
    env.home.write("proj/secrets/key", "TOP-SECRET-VALUE");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let results = last_results(&gateway);
    for (i, r) in results.iter().enumerate() {
        println!("result {i}: {}", r["content"]);
    }
    let leaked = results
        .iter()
        .any(|r| r["content"].to_string().contains("TOP-SECRET-VALUE"));
    assert!(
        results[1]["content"]
            .to_string()
            .contains("can only move to the project's root"),
        "{:?}",
        results[1]
    );
    println!("secret read after session_move: {leaked}");
    assert!(
        !leaked,
        "BREAK: session_move sidestepped the user's read denial"
    );
}

/// A general child of a Plan session moves its *parent's* directory (any session id).
#[test]
fn a_child_cannot_move_its_parents_directory() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let child = request.header("x-opencode-parent-session-id");
        let done = results(request).len();
        match (child, done) {
            (None, 0) => call(
                request,
                "t0",
                "subagent",
                json!({"agent": "general", "description": "d", "prompt": "p"}),
            ),
            (Some(parent), 0) => call(
                request,
                "c0",
                "execute",
                json!({"code": format!("return await tools.opencode.session_move({{sessionID: '{parent}', directory: '/'}})")}),
            ),
            _ => text(request, "done"),
        }
    });
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.select_agent(&session.id, "plan").unwrap();
    let before = core.session(&session.id).unwrap().unwrap().directory;
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let after = core.session(&session.id).unwrap().unwrap().directory;
    println!("parent directory before: {before}\nparent directory after: {after}");
    assert_eq!(
        before, after,
        "BREAK: a child changed its parent's working directory"
    );
}

/// A project's deny policy against a later `ANYSOLA_CONFIG_CONTENT` allow.
/// As in 1.1.0 the project's statement ranks above CONFIG_CONTENT's (earliest-loaded last).
#[test]
fn a_project_deny_policy_still_beats_a_later_user_allow() {
    let gateway = scripted(vec![]);
    let env = env(&gateway.url);
    env.home.write(
        "proj/anysola.json",
        r#"{"policies": [{"action": "tool.use", "resource": "shell:*", "effect": "deny"}]}"#,
    );
    // SAFETY: the env lock is held.
    unsafe {
        std::env::set_var(
            "ANYSOLA_CONFIG_CONTENT",
            r#"{"policies": [{"action": "tool.use", "resource": "shell:*", "effect": "allow"}]}"#,
        )
    };
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let denial = core
        .loaded()
        .config
        .policies()
        .tool_denial("shell", &["ls".into()]);
    unsafe { std::env::remove_var("ANYSOLA_CONFIG_CONTENT") };
    println!("project deny policy vs CONFIG_CONTENT allow -> denial: {denial:?}");
    assert!(
        denial.is_some(),
        "CHANGED: the project's deny policy no longer applies"
    );
}

/// The user's own command still switches the session's agent, as before.
#[test]
fn the_users_own_command_still_switches_the_agent() {
    let gateway = scripted(vec![]);
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"commands": {"build-it": {"template": "Build it", "agent": "build"}}}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    core.select_agent(&session.id, "plan").unwrap();
    runtime
        .block_on(core.command(&session.id, "build-it", "", TurnOptions::default()))
        .unwrap();
    assert_eq!(
        core.session(&session.id).unwrap().unwrap().agent.as_deref(),
        Some("build")
    );
    assert!(core.children(&session.id).unwrap().is_empty());
}

/// Without a default of the user's, a project's built-in `default_agent` applies as in 1.1.0;
/// a project can hide, retype or drop agents it introduced, never others.
#[test]
fn a_project_changes_only_its_own_agents_kind() {
    let gateway = scripted(vec![]);
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"agents": {"mine": {"mode": "primary"}}}"#,
    );
    env.home.write(
        "proj/anysola.json",
        r#"{"default_agent": "plan",
            "agents": {"explore": {"mode": "primary"}, "mine": {"hidden": true, "mode": "subagent"},
                       "theirs": {"mode": "subagent"}},
            "mode": {"general": {"prompt": "Now primary."}}}"#,
    );
    env.home.write(
        "proj/.anysola/agent/theirs.md",
        "---\nhidden: true\n---\nTheirs.\n",
    );
    env.home
        .write("proj/.anysola/modes/build.md", "---\n---\nStill build.\n");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let loaded = core.loaded();
    let agents = &loaded.agents;
    use anysola_core::agent::Mode;
    assert_eq!(agents.default_agent().unwrap().id, "plan");
    assert_eq!(agents.get("explore").unwrap().mode, Mode::Subagent);
    assert_eq!(agents.get("general").unwrap().mode, Mode::Subagent);
    let mine = agents.get("mine").unwrap();
    assert!(
        !mine.hidden && mine.mode == Mode::Primary,
        "the user's agent"
    );
    let theirs = agents.get("theirs").unwrap();
    assert!(
        theirs.hidden && theirs.mode == Mode::Subagent,
        "the project's own"
    );
    assert_eq!(agents.get("build").unwrap().mode, Mode::Primary);
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// Moving to one of the project's worktrees, or back to its root, still works; elsewhere not.
#[test]
fn session_move_goes_to_the_project_root_or_a_worktree_only() {
    let gateway = scripted(vec![
        (
            "execute",
            json!({"code": "return await tools.opencode.session_move({directory: '.'})"}),
        ),
        (
            "execute",
            json!({"code": "return await tools.opencode.worktree_create({name: 'side'})"}),
        ),
        (
            "execute",
            json!({"code": "const w = await tools.opencode.worktree_list(); return await tools.opencode.session_move({directory: w.worktrees[0].directory})"}),
        ),
        (
            "execute",
            json!({"code": "return await tools.opencode.session_move({directory: '..'})"}),
        ),
    ]);
    let env = env(&gateway.url);
    let project = env.home.project();
    env.home.write("proj/README.md", "hi");
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "init"]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let results = last_results(&gateway);
    let moved = core.session(&session.id).unwrap().unwrap().directory;
    assert!(
        results[0]["content"]
            .as_str()
            .is_some_and(|c| c.contains("\"sessionID\"")),
        "the root"
    );
    assert!(
        results[2]["content"]
            .as_str()
            .is_some_and(|c| c.contains("\"sessionID\"")),
        "a worktree"
    );
    assert!(
        results[3]["content"]
            .to_string()
            .contains("can only move to the project's root")
    );
    assert_ne!(moved, project.display().to_string(), "now in the worktree");
    assert!(std::path::Path::new(&moved).join("README.md").exists());
}
