//! Live config: the remote document (cache, offline, 401) and watched files reload before the next
//! step.

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::Core;
use anysola_core::gateway::config::{CACHE_KEY, SIGNED_OUT};
use anysola_testkit::{FakeGateway, Reply, default_config};
use common::*;
use serde_json::json;

fn has_model(core: &Core, id: &str) -> bool {
    core.loaded().models.iter().any(|m| m.id == id)
}

#[test]
fn remote_document_is_cached_and_refreshed() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(has_model(&core, "claude-haiku-5-5"));
    let db = anysola_core::storage::Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    assert_eq!(
        db.kv_get(CACHE_KEY).unwrap().unwrap()["model"],
        "anysola/claude-sonnet-5-5"
    );

    let mut body = default_config(&gateway.url);
    body["providers"]["anysola"]["models"]["claude-opus-5-5"] =
        json!({"name": "Claude Opus 5.5", "limit": {"context": 1000000, "output": 32000}});
    gateway.set_config(body);
    runtime.block_on(core.refresh_remote());
    assert!(
        !has_model(&core, "claude-opus-5-5"),
        "applied before the next step only"
    );
    runtime.block_on(core.refresh());
    assert!(has_model(&core, "claude-opus-5-5"));
    assert_eq!(core.connection_status(), None);

    gateway.set_config_reply(Reply::Json {
        status: 401,
        body: json!({"type": "error", "error": {"type": "authentication_error", "message": "no"}}),
        headers: Vec::new(),
    });
    runtime.block_on(core.refresh_remote());
    runtime.block_on(core.refresh());
    assert_eq!(core.connection_status(), Some(SIGNED_OUT));
    assert!(
        has_model(&core, "claude-opus-5-5"),
        "the cached document stays"
    );

    // A restart while the gateway is unreachable keeps the cached models.
    drop(core);
    drop(gateway);
    let core = runtime.block_on(common::core(&env));
    assert!(has_model(&core, "claude-opus-5-5"));
}

#[test]
fn without_a_credential_there_is_no_remote_document() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let mut env = env(&gateway.url);
    let runtime = runtime();
    drop(runtime.block_on(common::core(&env)));
    env.home.remove_env("ANYSOLA_TOKEN");
    // SAFETY: tests in this binary hold the environment lock.
    unsafe { std::env::remove_var("ANYSOLA_TOKEN") };
    let before = gateway.requests().len();
    let core = runtime.block_on(common::core(&env));
    assert!(core.loaded().models.is_empty());
    assert_eq!(
        gateway.requests().len(),
        before,
        "no /config without a credential"
    );
}

#[test]
fn watched_files_reload_before_the_next_step() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write("proj/.opencode/opencode.json", "{}");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(core.loaded().agents.get("reviewer").is_none());
    env.home.write(
        "proj/.opencode/agent/reviewer.md",
        "---\ndescription: Reviews\nmode: subagent\n---\nReview.",
    );
    env.home.write(
        "proj/opencode.json",
        r#"{"commands": {"hi": {"template": "Say hi"}}}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while core.loaded().agents.get("reviewer").is_none() && Instant::now() < deadline {
        runtime.block_on(core.refresh());
        std::thread::sleep(Duration::from_millis(50));
    }
    let reviewer = core.loaded().agents.get("reviewer").cloned().unwrap();
    assert_eq!(reviewer.description.as_deref(), Some("Reviews"));
    while core.loaded().commands.get("hi").is_none() && Instant::now() < deadline {
        runtime.block_on(core.refresh());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(core.loaded().commands.get("hi").is_some());
}

#[test]
fn policies_withhold_the_provider_and_skills() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        "proj/.claude/skills/deploy/SKILL.md",
        "---\ndescription: Deploys\n---\nDeploy.",
    );
    env.home.write(
        "proj/.claude/skills/lint/SKILL.md",
        "---\ndescription: Lints\n---\nLint.",
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert_eq!(core.loaded().skills.len(), 2);
    assert!(core.loaded().config.websearch_enabled());
    drop(core);

    env.home.write(
        "proj/opencode.json",
        r#"{"policies": [{"action": "skill.use", "resource": "dep*", "effect": "deny"}]}"#,
    );
    let core = runtime.block_on(common::core(&env));
    let skills: Vec<String> = core.loaded().skills.iter().map(|s| s.id.clone()).collect();
    assert_eq!(skills, ["lint"]);
    assert!(!core.loaded().models.is_empty());
    drop(core);

    env.home.write(
        "proj/opencode.json",
        r#"{"policies": [{"action": "provider.use", "resource": "*", "effect": "deny"}]}"#,
    );
    let core = runtime.block_on(common::core(&env));
    assert!(core.loaded().models.is_empty());
    drop(core);

    env.home.write(
        "proj/opencode.json",
        r#"{"policies": [{"action": "integration.use", "resource": "anysola", "effect": "deny"}]}"#,
    );
    let core = runtime.block_on(common::core(&env));
    assert!(core.loaded().models.is_empty());
    assert!(!core.loaded().config.websearch_enabled());
}
