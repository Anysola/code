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

fn child_request(marker: &str, capture: &str) {
    let gateway = gateway_logger(String::new());
    let home = anysola_testkit::TestHome::new(&gateway.url);
    let dir = my_app(&home);
    assert_eq!(run(&home, &dir, &["run", marker, "go"]).code, 0);
    let child = gateway
        .messages()
        .into_iter()
        .find(|r| r.header("x-opencode-parent-session-id").is_some())
        .expect("a child request");
    assert_ne!(
        child.header("x-opencode-session-id"),
        child.header("x-opencode-session"),
        "the child shares its parent's affinity"
    );
    assert_request(
        capture,
        &child,
        &read_json(capture),
        &our_places(&home, &dir),
    );
}

#[test]
fn explore_subagent_request() {
    child_request(
        "SUBEXPLORE",
        "tools/_captured-request-explore-subagent.json",
    );
}

#[test]
fn general_subagent_request() {
    child_request(
        "SUBGENERAL",
        "tools/_captured-request-general-subagent.json",
    );
}
