//! Importing the rich 0.5.x fixture (and variants of it) in-process. The process takes on the
//! test home's environment, so the tests run one at a time.
// The legacy fixture is a Linux 0.5.1 capture whose home path is spliced into JSON text as is.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anysola_core::paths::Paths;
use anysola_core::storage::Db;
use anysola_testkit::{Legacy, TestHome};
use rusqlite::Connection;
use serde_json::{Value, json};

static ENV: Mutex<()> = Mutex::new(());

struct Env {
    home: TestHome,
    legacy: Legacy,
    paths: Paths,
    _guard: MutexGuard<'static, ()>,
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rich")
}

fn env(extra_sql: Option<&str>) -> Env {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = TestHome::new("http://127.0.0.1:9");
    for (name, value) in home.env() {
        // SAFETY: tests in this binary hold `ENV` while they touch the environment.
        unsafe { std::env::set_var(name, value) };
    }
    let legacy = home.install_legacy(&fixture(), extra_sql);
    Env {
        paths: Paths::from_env(),
        home,
        legacy,
        _guard: guard,
    }
}

fn rows(conn: &Connection, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = conn.prepare(sql).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |r| (0..width).map(|i| r.get(i)).collect())
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn text(value: &rusqlite::types::Value) -> String {
    match value {
        rusqlite::types::Value::Text(t) => t.clone(),
        other => panic!("not text: {other:?}"),
    }
}

/// Every file under the 0.5.x roots with its bytes and modification time.
fn old_files(home: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let modified = entry.metadata().unwrap().modified().unwrap();
                out.insert(path.clone(), (std::fs::read(&path).unwrap(), modified));
            }
        }
    }
    let mut out = BTreeMap::new();
    for root in [
        ".local/share/opencode",
        ".config/opencode",
        ".local/state/opencode",
    ] {
        walk(&home.join(root), &mut out);
    }
    out
}

#[test]
fn imports_everything_once_without_touching_old_files() {
    let env = env(None);
    let before = old_files(&env.home.home());
    let counts = anysola_migrate::run(&env.paths).unwrap().expect("imported");
    assert_eq!((counts.sessions, counts.projects), (4, 1));
    assert_eq!(old_files(&env.home.home()), before, "0.5.x files unchanged");
    assert!(
        !env.legacy.database.with_extension("db-shm").exists(),
        "no -shm created next to the old database"
    );

    let old = Connection::open(&env.legacy.database).unwrap();
    let new = Connection::open(env.paths.database()).unwrap();
    let ours = rows(
        &new,
        "SELECT id, session_id, seq, type, data FROM message ORDER BY session_id, seq",
    );
    let theirs = rows(
        &old,
        "SELECT id, session_id, seq, type, data FROM session_message ORDER BY session_id, seq",
    );
    assert_eq!(ours.len(), theirs.len());
    for (a, b) in ours.iter().zip(&theirs) {
        assert_eq!(a[..4], b[..4]);
        let a: Value = serde_json::from_str(&text(&a[4])).unwrap();
        let b: Value = serde_json::from_str(&text(&b[4])).unwrap();
        assert_eq!(a, b, "message data round-trips");
    }
    let sessions = "SELECT id, project_id, parent_id, fork_session_id, fork_boundary, slug, directory, title, \
                    version, agent, model, cost, tokens_input, tokens_output, revert, permission, time_created, \
                    time_updated, time_archived, idle_outcome FROM ";
    assert_eq!(
        rows(&new, &format!("{sessions} session ORDER BY id")),
        rows(&old, &format!("{sessions} session_v2 ORDER BY id"))
    );
    let instruction = "SELECT session_id, epoch_start, through_seq, initial_values, current_values FROM instruction_state ORDER BY session_id";
    assert_eq!(rows(&new, instruction), rows(&old, instruction));
    assert_eq!(
        rows(&new, "SELECT id, worktree, vcs FROM project"),
        rows(&old, "SELECT id, worktree, vcs FROM project")
    );
    assert_eq!(
        rows(&new, "SELECT project_id, action, resource FROM permission"),
        vec![vec![
            rows(&old, "SELECT id FROM project")[0][0].clone(),
            "shell".to_string().into(),
            "git status *".to_string().into()
        ]]
    );

    let db = Db::open(&env.paths.database()).unwrap();
    assert_eq!(
        db.anysola_login().unwrap().as_deref(),
        Some("asc_FAKEtoken0000000000")
    );
    assert_eq!(
        db.kv_get("anysola:budget:default").unwrap(),
        Some(json!(25))
    );
    assert_eq!(db.kv_get("anysola:session:ses_orphan").unwrap(), None);
    let root = text(
        &rows(
            &old,
            "SELECT id FROM session_v2 WHERE title = 'Rich storage test'",
        )[0][0],
    );
    assert_eq!(
        db.kv_get(&format!("anysola:session:{root}")).unwrap(),
        Some(json!({"total": 16, "turns": 1, "budget": null}))
    );
    let config = db.kv_get("anysola:config").unwrap().expect("cached config");
    let body: Value =
        serde_json::from_str(config.as_str().expect("kept as the response text")).unwrap();
    assert_eq!(body["model"], "anysola/claude-sonnet-5-5");
    let marker = db.kv_get(anysola_migrate::MARKER).unwrap().expect("marker");
    assert_eq!(marker["sessions"], 4);

    let data = &env.paths.data;
    assert_eq!(
        std::fs::read(data.join("tui.json")).unwrap(),
        std::fs::read(fixture().join("home/cli.json")).unwrap()
    );
    let state = env.home.home().join(".local/state/opencode");
    for (old, new) in [
        ("prompt-history.jsonl", "prompt-history.jsonl"),
        ("prompt-stash.jsonl", "prompt-stash.jsonl"),
        ("model.json", "model.json"),
        ("latest/tui/tabs.json", "tui/tabs.json"),
    ] {
        assert_eq!(
            std::fs::read(state.join(old)).unwrap(),
            std::fs::read(env.paths.state_dir().join(new)).unwrap(),
            "{new}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(data.join("tui.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    assert_eq!(
        anysola_migrate::run(&env.paths).unwrap(),
        None,
        "imported once"
    );
}

#[test]
fn imported_overlay_legacy_blob_and_interrupted_rows() {
    let synthetic = std::fs::read_to_string(fixture().join("synthetic.sql")).unwrap();
    let env = env(Some(&synthetic));
    anysola_migrate::run(&env.paths).unwrap().expect("imported");
    let db = Db::open(&env.paths.database()).unwrap();
    let old = Connection::open(&env.legacy.database).unwrap();
    let fork = text(
        &rows(
            &old,
            "SELECT id FROM session_v2 WHERE fork_session_id IS NOT NULL",
        )[0][0],
    );
    let messages = db.messages(&fork).unwrap();
    assert!(
        messages.iter().all(|m| m.kind != "compaction"),
        "running compaction dropped"
    );
    let steps: Vec<&Value> = messages
        .iter()
        .filter(|m| m.kind == "assistant")
        .map(|m| &m.data)
        .collect();
    assert_eq!(
        steps[0]["content"][0]["type"], "reasoning",
        "parts merged from session_part"
    );
    assert_eq!(
        steps[0]["content"][0]["native"]["signature"]
            .as_str()
            .map(|s| s.starts_with("SIG_")),
        Some(true)
    );
    let reasoning = &steps[1]["content"][1];
    assert!(reasoning["native"]["signature"].is_string() && reasoning.get("state").is_none());
    assert!(steps[1]["content"][0]["native"]["redactedData"].is_string());
    let user = messages
        .iter()
        .find(|m| m.kind == "user" && m.data["files"].as_array().is_some_and(|f| !f.is_empty()))
        .unwrap();
    assert_eq!(
        user.data["files"][0],
        json!({"mime": "image/png", "source": {"type": "inline"}, "name": "pixel.png",
               "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="})
    );
    assert!(
        steps.last().unwrap()["time"].get("completed").is_none(),
        "kept as streamed"
    );
}

#[test]
fn revert_of_an_imported_message_restores_through_the_old_store() {
    let env = env(None);
    anysola_migrate::run(&env.paths).unwrap().expect("imported");
    let alternates = std::fs::read_dir(env.paths.data.join("snapshot"))
        .unwrap()
        .flatten()
        .flat_map(|p| std::fs::read_dir(p.path()).unwrap().flatten())
        .map(|s| std::fs::read_to_string(s.path().join("objects/info/alternates")).unwrap())
        .next()
        .expect("a new store");
    assert_eq!(alternates.lines().count(), 2, "{alternates}");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let core = runtime
        .block_on(anysola_core::api::Core::start(&env.legacy.project))
        .unwrap();
    let db = Db::open(&env.paths.database()).unwrap();
    let old = Connection::open(&env.legacy.database).unwrap();
    let root = text(
        &rows(
            &old,
            "SELECT id FROM session_v2 WHERE title = 'Rich storage test'",
        )[0][0],
    );
    let messages = db.messages(&root).unwrap();
    let edit = messages
        .iter()
        .position(|m| m.data["content"].to_string().contains("\"name\":\"edit\""))
        .unwrap();
    // Reverting to the step before the edit undoes the edit.
    let before = &messages[edit - 1];
    core.revert(&root, &before.id).unwrap();
    assert_eq!(
        std::fs::read_to_string(env.legacy.project.join("notes.md")).unwrap(),
        "# Notes\n\nfirst line\n"
    );
}

#[test]
fn untitled_sessions_import_with_a_default_title() {
    let env = env(Some(
        "UPDATE session_v2 SET title = NULL WHERE title = 'Rich storage test';",
    ));
    let old = Connection::open(&env.legacy.database).unwrap();
    let untitled = rows(
        &old,
        "SELECT id, time_created FROM session_v2 WHERE title IS NULL AND parent_id IS NULL",
    );
    assert!(!untitled.is_empty());
    anysola_migrate::run(&env.paths).unwrap().expect("imported");
    let new = Connection::open(env.paths.database()).unwrap();
    for row in untitled {
        let id = text(&row[0]);
        let rusqlite::types::Value::Integer(created) = row[1] else {
            panic!("time_created");
        };
        let title = rows(
            &new,
            &format!("SELECT title FROM session WHERE id = '{id}'"),
        );
        assert_eq!(
            text(&title[0][0]),
            format!("New session - {}", anysola_core::time::iso(created as u64))
        );
        let messages = rows(
            &new,
            &format!("SELECT count(*) FROM message WHERE session_id = '{id}'"),
        );
        assert_ne!(messages[0][0], rusqlite::types::Value::Integer(0));
    }
}

#[test]
fn outdated_or_damaged_databases_import_nothing() {
    let outdated = env(Some(
        "DELETE FROM migration WHERE id = '20261030054400_job';",
    ));
    let error = anysola_migrate::run(&outdated.paths).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Run Anysola Code 0.5.x once to upgrade its database, then start 1.0 again."
    );
    drop(outdated);

    let env = env(None);
    std::fs::write(&env.legacy.database, b"SQLite format 3\0 but not really").unwrap();
    assert!(anysola_migrate::run(&env.paths).is_err());
    let db = Db::open(&env.paths.database()).unwrap();
    assert_eq!(db.kv_get(anysola_migrate::MARKER).unwrap(), None);
    assert!(!env.paths.data.join("tui.json").exists());
}

#[test]
fn v1_tables_are_skipped() {
    let env = env(Some(
        "CREATE TABLE session (id TEXT PRIMARY KEY); CREATE TABLE message (id TEXT PRIMARY KEY);
         CREATE TABLE part (id TEXT PRIMARY KEY); INSERT INTO session VALUES ('v1');",
    ));
    let counts = anysola_migrate::run(&env.paths).unwrap().unwrap();
    assert_eq!(counts.sessions, 4);
}
