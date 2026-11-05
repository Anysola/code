//! Continuing sessions imported from 0.5.x: the first start imports the fixture home, then `run -s` /
//! `run -c` must send what 0.5.1 sent for the same continuation of the same database
//! (`crates/migrate/tests/fixtures/rich/capture-*.json`, made by `testing/m3-fixtures.sh`).

mod common;

use std::path::PathBuf;

use anysola_testkit::TestHome;
use common::*;
use serde_json::Value;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../migrate/tests/fixtures/rich")
}

/// The home the fixture captures were taken in.
fn fixture_places() -> Places {
    let home = "/home/ubuntu/anysola-code/rust/testing/runs/m3-fix/home";
    Places(vec![
        (format!("{home}/proj"), "<PROJECT>"),
        (home.to_string(), "<HOME>"),
    ])
}

fn continuation(capture: &str, extra_sql: Option<&str>, args: &[&str]) {
    let gateway = gateway_logger(String::new());
    let mut home = TestHome::new(&gateway.url);
    // The imported device login signs the requests, as it did for 0.5.1.
    home.remove_env("ANYSOLA_TOKEN");
    let legacy = home.install_legacy(&fixture(), extra_sql);
    install_captured_skills(&home);
    let output = run(&home, &legacy.project, args);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(
        output
            .stderr
            .starts_with("Imported 4 sessions from Anysola Code 0.5.x.\n"),
        "stderr: {}",
        output.stderr
    );
    let request = gateway
        .messages()
        .into_iter()
        .find(|r| !r.is_title())
        .expect("a main request");
    let captured: Value = serde_json::from_str(
        &std::fs::read_to_string(fixture().join(format!("capture-{capture}.json"))).unwrap(),
    )
    .unwrap();
    let expected = normalize_body(&captured["body"], &fixture_places(), true);
    // Both sides: stored history keeps 0.5.1's argv quoting (§B1), only the new prompt differs.
    let actual = normalize_body(&request.json(), &our_places(&home, &legacy.project), true);
    let expected = serde_json::to_string_pretty(&expected).unwrap();
    let actual = serde_json::to_string_pretty(&actual).unwrap();
    if expected != actual {
        let (e, a): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
        let first = e
            .iter()
            .zip(&a)
            .position(|(x, y)| x != y)
            .unwrap_or(e.len().min(a.len()));
        let window = |lines: &[&str]| {
            lines[first.saturating_sub(3)..(first + 6).min(lines.len())].join("\n")
        };
        panic!(
            "{capture}: request body differs at line {first}\n--- 0.5.1\n{}\n+++ ours\n{}",
            window(&e),
            window(&a)
        );
    }
    assert_eq!(
        normalize_headers(&capture_headers(&captured)),
        normalize_headers(&request.headers),
        "{capture}: headers"
    );
}

/// The skills 0.5.1 listed when the fixture was captured (it walked up past the fixture home to
/// the capturing user's `~/.claude/skills`), so the continuation sees the same skill guidance.
fn install_captured_skills(home: &TestHome) {
    let sql = std::fs::read_to_string(fixture().join("opencode.sql")).unwrap();
    let listing = sql
        .lines()
        .filter_map(|l| l.strip_prefix("INSERT INTO instruction_blob VALUES('"))
        .filter_map(|l| l.split_once("','"))
        .filter_map(|(_, rest)| rest.strip_suffix("');"))
        .find_map(|value| {
            let value: Value = serde_json::from_str(&value.replace("''", "'")).ok()?;
            value.as_array()?.first()?.get("description")?;
            Some(value)
        })
        .expect("a stored skill listing");
    for skill in listing.as_array().unwrap() {
        let field = |key: &str| serde_json::to_string(&skill[key]).unwrap();
        home.write(
            &format!(".claude/skills/{}/SKILL.md", skill["id"].as_str().unwrap()),
            &format!(
                "---\nname: {}\ndescription: {}\n---\n",
                field("name"),
                field("description")
            ),
        );
    }
}

fn session(kind: &str) -> String {
    let sql = std::fs::read_to_string(fixture().join("opencode.sql")).unwrap();
    let pattern = match kind {
        "root" => "Rich storage test',",
        _ => "(fork #1)',",
    };
    let line = sql
        .lines()
        .find(|l| l.starts_with("INSERT INTO session_v2") && l.contains(pattern))
        .expect("session row");
    line.split('\'').nth(1).unwrap().to_string()
}

#[test]
fn continue_imported_session_by_id() {
    let id = session("root");
    continuation(
        "continue-id",
        None,
        &["run", "-s", &id, "continue imported"],
    );
}

#[test]
fn continue_last_imported_session_after_compaction() {
    continuation("continue-last", None, &["run", "-c", "after compaction"]);
}

#[test]
fn continue_imported_overlay_legacy_blob_and_interrupted_rows() {
    let synthetic = std::fs::read_to_string(fixture().join("synthetic.sql")).unwrap();
    let id = session("fork");
    continuation(
        "continue-synthetic",
        Some(&synthetic),
        &["run", "-s", &id, "continue synthetic"],
    );
}

#[test]
fn failed_import_warns_and_runs_again_next_start() {
    let gateway = gateway_logger(String::new());
    let home = TestHome::new(&gateway.url);
    let legacy = home.install_legacy(&fixture(), None);
    assert_eq!(run(&home, &legacy.project, &["--version"]).code, 0);
    assert!(
        !home.home().join(".anysola/anysola.db").exists(),
        "--version imports nothing"
    );

    let good = std::fs::read(&legacy.database).unwrap();
    std::fs::write(&legacy.database, b"SQLite format 3\0damaged").unwrap();
    let output = run(&home, &legacy.project, &["run", "hello"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let first = output.stderr.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("Could not import data from Anysola Code 0.5.x (")
            && first.ends_with("); it will be tried again next start."),
        "stderr: {}",
        output.stderr
    );

    std::fs::write(&legacy.database, good).unwrap();
    let output = run(&home, &legacy.project, &["run", "hello"]);
    assert!(
        output
            .stderr
            .starts_with("Imported 4 sessions from Anysola Code 0.5.x.\n")
    );
    let output = run(&home, &legacy.project, &["run", "hello"]);
    assert!(
        !output.stderr.contains("Anysola Code 0.5.x"),
        "stderr: {}",
        output.stderr
    );
}
