//! What a settings screen changes under a running core: the global config file written in place
//! (picked up by the file watcher) and the process-wide switch for Claude Code's skill folders.

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::Core;
use anysola_core::config::jsonc;
use anysola_testkit::FakeGateway;
use common::*;
use serde_json::json;

fn todo(core: &Core) -> bool {
    anysola_core::tool::todowrite::enabled(&core.loaded().config)
}

#[test]
fn a_running_core_sees_the_global_config_written_in_place() {
    if anysola_core::env::flag("DISABLE_FILEWATCHER") {
        return;
    }
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(!todo(&core));
    let file = env.home.home().join(".anysola/config.json");
    let text = "{\n  // mine\n  \"shell\": \"/bin/sh\"\n}\n";
    let text = jsonc::set(text, &["experimental", "todo"], json!(true)).unwrap();
    std::fs::write(&file, &text).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !todo(&core) && Instant::now() < deadline {
        // The loop refreshes before each step; this stands in for one.
        runtime.block_on(core.refresh());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(todo(&core), "the watcher saw the write");
    std::fs::write(&file, jsonc::remove(&text, &["experimental", "todo"])).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while todo(&core) && Instant::now() < deadline {
        runtime.block_on(core.refresh());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!todo(&core));
    assert_eq!(core.loaded().config.shell(), Some("/bin/sh"));
}

#[test]
fn claude_skill_folders_follow_the_switch() {
    let gateway = FakeGateway::start(|request, _| text(request, "ok"));
    let env = env(&gateway.url);
    env.home.write(
        ".claude/skills/demo/SKILL.md",
        "---\ndescription: From Claude Code\n---\nBody",
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let has_demo = |core: &Core| core.loaded().skills.iter().any(|s| s.id == "demo");
    assert!(has_demo(&core));
    anysola_core::skill::set_claude_skills(false);
    runtime.block_on(core.reload());
    let off = has_demo(&core);
    anysola_core::skill::set_claude_skills(true);
    assert!(!off, "turned off without a restart");
    runtime.block_on(core.reload());
    assert!(has_demo(&core));
}
