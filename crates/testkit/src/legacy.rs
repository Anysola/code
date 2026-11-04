//! A 0.5.x home recreated from a fixture of `crates/migrate/tests/fixtures/`: `opencode.db` from
//! the SQL dump, `cli.json`, the state files, the shadow snapshot store and the `proj` checkout,
//! all with the fixture's `@HOME@` pointing at this home.
//!
//! Stored instruction values embed the home and the date, so their hashes are recomputed (sha256
//! of canonical JSON, as 0.5.1 hashes them) and the date is today's: a session continued today is
//! then in the state 0.5.1 saw when the fixture's captures were taken.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha1::Digest;

use crate::TestHome;

/// A 0.5.x home installed into a [`TestHome`].
pub struct Legacy {
    pub database: PathBuf,
    pub project: PathBuf,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn canonical(value: &Value) -> String {
    match value {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let entries: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical(&map[k])))
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        other => other.to_string(),
    }
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}");
}

fn copy_dir(from: &Path, to: &Path, home: &str) {
    std::fs::create_dir_all(to).expect("dir");
    for entry in std::fs::read_dir(from).expect("fixture dir").flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target, home);
            continue;
        }
        let bytes = std::fs::read(entry.path()).expect("fixture file");
        let bytes = match String::from_utf8(bytes) {
            Ok(text) => text.replace("@HOME@", home).into_bytes(),
            Err(binary) => binary.into_bytes(),
        };
        std::fs::write(target, bytes).expect("write");
    }
}

impl TestHome {
    /// Installs the fixture directory `fixture` as this home's 0.5.x data; `extra_sql` runs on the
    /// database after it is loaded.
    pub fn install_legacy(&self, fixture: &Path, extra_sql: Option<&str>) -> Legacy {
        let home = self.home();
        let home_text = home.display().to_string();
        let files = fixture.join("home");

        let project = home.join("proj");
        std::fs::create_dir_all(&project).expect("project");
        std::fs::write(project.join("README.md"), "# Fixture\n").expect("write");
        std::fs::write(project.join("note.txt"), "attached note\n").expect("write");
        std::fs::copy(files.join("pixel.png"), project.join("pixel.png")).expect("pixel");
        git(&project, &["init", "-q"]);
        git(
            &project,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/anysola/m3-fixture.git",
            ],
        );
        git(&project, &["add", "-A"]);
        git(&project, &["commit", "-qm", "init"]);
        std::fs::write(project.join("hello.txt"), "hello\n").expect("write");
        std::fs::write(project.join("notes.md"), "# Notes\n\nsecond line\n").expect("write");

        let data = home.join(".local/share/opencode");
        std::fs::create_dir_all(&data).expect("data dir");
        let database = data.join("opencode.db");
        let sql = std::fs::read_to_string(fixture.join("opencode.sql")).expect("dump");
        let conn = rusqlite::Connection::open(&database).expect("db");
        conn.execute_batch(&sql.replace("@HOME@", &home_text))
            .expect("load dump");
        if let Some(extra) = extra_sql {
            conn.execute_batch(extra).expect("extra sql");
        }
        rehash(&conn);
        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .expect("wal");
        drop(conn);

        let config = home.join(".config/opencode");
        std::fs::create_dir_all(&config).expect("config dir");
        std::fs::copy(files.join("cli.json"), config.join("cli.json")).expect("cli.json");
        copy_dir(
            &files.join("state"),
            &home.join(".local/state/opencode"),
            &home_text,
        );
        let worktree = std::fs::canonicalize(&project).expect("canonical");
        let digest = hex(&sha1::Sha1::digest(
            worktree.display().to_string().as_bytes(),
        ));
        for store in std::fs::read_dir(files.join("snapshot"))
            .expect("snapshots")
            .flatten()
        {
            let target = data.join("snapshot").join(store.file_name()).join(&digest);
            copy_dir(&store.path(), &target, &home_text);
        }
        Legacy { database, project }
    }
}

/// Sets `core/date` to today and recomputes every instruction blob's hash after the home moved.
fn rehash(conn: &rusqlite::Connection) {
    let today = chrono::Local::now().format("%a %b %d %Y").to_string();
    let mut statement = conn
        .prepare("SELECT hash, value FROM instruction_blob")
        .expect("blobs");
    let blobs: Vec<(String, String)> = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("blobs")
        .map(|r| r.expect("blob"))
        .collect();
    let states: Vec<(String, String, String)> = conn
        .prepare("SELECT session_id, initial_values, current_values FROM instruction_state")
        .expect("states")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("states")
        .map(|r| r.expect("state"))
        .collect();
    let date_hashes: Vec<String> = states
        .iter()
        .flat_map(|(_, initial, current)| [initial, current])
        .filter_map(|values| {
            serde_json::from_str::<Value>(values).ok()?["core/date"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    let mut renamed = HashMap::new();
    conn.execute("DELETE FROM instruction_blob", [])
        .expect("clear");
    for (hash, value) in blobs {
        let value: Value = if date_hashes.contains(&hash) {
            Value::String(today.clone())
        } else {
            serde_json::from_str(&value).expect("blob json")
        };
        let new = hex(&sha2::Sha256::digest(canonical(&value).as_bytes()));
        conn.execute(
            "INSERT OR IGNORE INTO instruction_blob (hash, value) VALUES (?1, ?2)",
            [&new, &value.to_string()],
        )
        .expect("blob");
        renamed.insert(hash, new);
    }
    let rename = |values: &str| {
        let mut map: serde_json::Map<String, Value> = serde_json::from_str(values).expect("values");
        for value in map.values_mut() {
            if let Some(new) = value.as_str().and_then(|h| renamed.get(h)) {
                *value = Value::String(new.clone());
            }
        }
        Value::Object(map).to_string()
    };
    for (session, initial, current) in &states {
        conn.execute(
            "UPDATE instruction_state SET initial_values = ?2, current_values = ?3 WHERE session_id = ?1",
            [session, &rename(initial), &rename(current)],
        )
        .expect("state");
    }
}
