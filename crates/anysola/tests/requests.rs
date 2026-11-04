//! Request replay: each scenario of `spec/verbatim/session/requests/` driven through `anysola run`
//! against the fake gateway; body and headers must equal the 0.5.1 capture after normalization.

mod common;

use common::*;

fn scenario(
    capture: &str,
    project: fn(&anysola_testkit::TestHome) -> std::path::PathBuf,
    args: &[&str],
    pick: usize,
) {
    let gateway = gateway_logger(String::new());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = project(&home);
    let output = run(&home, &dir, args);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_request(
        capture,
        &main[pick],
        &read_json(capture),
        &our_places(&home, &dir),
    );
}

#[test]
fn hello_sonnet() {
    scenario(
        "session/requests/primary-hello-sonnet.json",
        my_app,
        &["run", "--standalone", "hello"],
        0,
    );
}

#[test]
fn build_tool_list() {
    scenario(
        "tools/_captured-request-build-sonnet.json",
        my_app,
        &["run", "--standalone", "say", "hi"],
        0,
    );
}

#[test]
fn title_request() {
    let gateway = gateway_logger(String::new());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = my_app(&home);
    assert_eq!(run(&home, &dir, &["run", "hello"]).code, 0);
    let title = gateway
        .messages()
        .into_iter()
        .find(|r| r.is_title())
        .expect("a title request");
    assert_request(
        "title",
        &title,
        &read_json("session/requests/title-hello.json"),
        &our_places(&home, &dir),
    );
}

#[test]
fn auto_model() {
    scenario(
        "session/requests/primary-auto.json",
        my_app,
        &["run", "-m", "anysola/auto", "hi", "auto"],
        0,
    );
}

#[test]
fn economy_variant() {
    scenario(
        "session/requests/primary-economy.json",
        my_app,
        &[
            "run",
            "-m",
            "anysola/claude-sonnet-5-5#economy",
            "hi",
            "economy",
        ],
        0,
    );
}

#[test]
fn high_variant() {
    scenario(
        "session/requests/primary-variant-high.json",
        my_app,
        &["run", "-m", "anysola/claude-sonnet-5-5#high", "hi", "there"],
        0,
    );
}

#[test]
fn plan_agent() {
    scenario(
        "session/requests/primary-plan-agent.json",
        my_app,
        &["run", "--agent", "plan", "hi", "plan"],
        0,
    );
}

#[test]
fn explore_agent() {
    scenario(
        "session/requests/primary-explore-agent.json",
        proj,
        &["run", "--agent", "explore", "hi", "explore"],
        0,
    );
}

#[test]
fn project_instructions() {
    scenario(
        "session/requests/primary-project-with-instructions.json",
        proj,
        &["run", "hello"],
        0,
    );
}

#[test]
fn file_attachments() {
    let gateway = gateway_logger(String::new());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = proj(&home);
    let png = include_bytes!("fixtures/red.png");
    std::fs::write(dir.join("red.png"), png).unwrap();
    let output = run(
        &home,
        &dir,
        &[
            "run",
            "--file",
            "README.md",
            "--file",
            "red.png",
            "describe",
            "files",
        ],
    );
    assert_eq!(output.code, 0, "{}", output.stderr);
    let main = gateway
        .messages()
        .into_iter()
        .find(|r| !r.is_title())
        .unwrap();
    let capture = read_json("session/requests/run-file-attachments.json");
    assert_request("files", &main, &capture, &our_places(&home, &dir));
}

#[test]
fn thinking_replayed_next_turn() {
    let gateway = gateway_logger(String::new());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = proj(&home);
    assert_eq!(run(&home, &dir, &["run", "THINKTEST", "please"]).code, 0);
    assert_eq!(
        run(&home, &dir, &["run", "-c", "followup", "question"]).code,
        0
    );
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let capture = read_json("session/requests/thinking-replay-next-turn.json");
    assert_request("thinking", &main[1], &capture, &our_places(&home, &dir));
}

#[test]
fn tool_error_replayed_with_thinking() {
    let gateway = gateway_logger("<PROJECT>/README.md".into());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = proj(&home);
    assert_eq!(
        run(&home, &dir, &["run", "--auto", "TOOLTEST", "please"]).code,
        0
    );
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let capture = read_json("session/requests/tool-error-replay-with-thinking.json");
    let ours = anysola_testkit::Recorded {
        body: main[1]
            .body
            .replace("<PROJECT>", &dir.display().to_string()),
        ..main[1].clone()
    };
    assert_request("tool error", &ours, &capture, &our_places(&home, &dir));
}

#[test]
fn parallel_tools_replayed() {
    scenario(
        "session/requests/parallel-tools-replay.json",
        proj,
        &["run", "--auto", "TWOTOOLS", "go"],
        1,
    );
}

#[test]
fn max_steps_final_step() {
    scenario(
        "session/requests/max-steps-final-step.json",
        proj,
        &["run", "--auto", "--agent", "reviewer", "LONGSTEPS", "go"],
        2,
    );
}

#[test]
fn effort_switch_mid_session() {
    let gateway = gateway_logger(String::new());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = proj(&home);
    let sonnet = "anysola/claude-sonnet-5-5";
    assert_eq!(run(&home, &dir, &["run", "-m", sonnet, "first"]).code, 0);
    let high = format!("{sonnet}#high");
    assert_eq!(
        run(&home, &dir, &["run", "-c", "-m", &high, "second"]).code,
        0
    );
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let body = main[1].json();
    assert!(
        body.get("output_config").is_none(),
        "top-level effort stays the first one"
    );
    assert_eq!(
        body["thinking"]["display"], "summarized",
        "the request settings follow the variant"
    );
    assert_eq!(
        body["messages"][2],
        serde_json::json!({"role": "system", "content": [], "output_config": {"effort": "high"}})
    );
    assert_eq!(body["messages"][3]["content"][0]["text"], "second");
    let beta = main[1]
        .headers
        .iter()
        .find(|(k, _)| k == "anthropic-beta")
        .map(|(_, v)| v.as_str());
    assert_eq!(
        beta,
        Some(
            "interleaved-thinking-2025-05-14,thinking-binding-controls-2026-08-01,mid-conversation-output-config-2026-07-01"
        )
    );
}

/// Runs `first`, rewrites the project's `AGENTS.md`, then continues with `second`; returns the
/// second main request.
fn instruction_update(
    model: &str,
    before: &str,
    after: &str,
    turns: [&str; 2],
) -> (anysola_testkit::Recorded, Places) {
    let gateway = gateway_logger(String::new());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = proj(&home);
    std::fs::write(dir.join("AGENTS.md"), before).unwrap();
    assert_eq!(run(&home, &dir, &["run", "-m", model, turns[0]]).code, 0);
    std::fs::write(dir.join("AGENTS.md"), after).unwrap();
    let output = run(&home, &dir, &["run", "-c", "-m", model, turns[1]]);
    assert_eq!(output.code, 0, "{}", output.stderr);
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    (main[1].clone(), our_places(&home, &dir))
}

#[test]
fn instruction_update_native_system() {
    let rule = "Project rule: always answer in English.\n";
    let (request, places) = instruction_update(
        "anysola/claude-sonnet-5-5",
        rule,
        &format!("{rule}New line added.\n"),
        ["hello", "next turn"],
    );
    let capture = read_json("session/requests/instruction-update-native-system.json");
    assert_request("native update", &request, &capture, &places);
}

#[test]
fn instruction_update_wrapped_for_auto() {
    let rule = "Project rule: always answer in English.\n";
    let (request, places) = instruction_update(
        "anysola/auto",
        &format!("{rule}New line added.\n"),
        &format!("{rule}Changed again.\n"),
        ["first", "second"],
    );
    let capture = read_json("session/requests/instruction-update-wrapped-auto.json");
    assert_request("wrapped update", &request, &capture, &places);
}
