//! `anysola session list|delete|export|import` against sessions written by `anysola run`, compared
//! with the 0.5.1 outputs in `V/cli/other-commands.txt` and a fresh 0.5.1 export.

mod common;

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Output, Recorded, Reply, TestHome};
use common::BIN;
use regex::Regex;
use serde_json::{Value, json};

fn answer(request: &Recorded, _: usize) -> Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let text = if request.is_title() {
        "Fake title"
    } else {
        "Plain answer."
    };
    Reply::Sse {
        body: Stream::new().text(text).finish(&model, Some(1)),
        before: Some(100),
    }
}

fn home(gateway: &FakeGateway) -> TestHome {
    let mut home = TestHome::new(&gateway.url);
    home.set_env("TZ", "UTC");
    home
}

fn anysola(home: &TestHome, args: &[&str]) -> Output {
    home.run(BIN, args, None, None)
}

/// Runs one prompt and returns the new session's id.
fn session(home: &TestHome) -> String {
    let out = anysola(home, &["run", "--format", "json", "hello there"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let first: Value = serde_json::from_str(out.stdout.lines().next().unwrap()).unwrap();
    first["sessionID"].as_str().unwrap().to_string()
}

#[test]
fn list_prints_tab_separated_lines_and_json() {
    let gateway = FakeGateway::start(answer);
    let home = home(&gateway);
    let out = anysola(&home, &["session", "list"]);
    assert_eq!((out.stdout.as_str(), out.code), ("", 0));
    let older = session(&home);
    let newer = session(&home);
    let out = anysola(&home, &["session", "list"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let line =
        Regex::new(r"^(ses_\w+)\tFake title\t\d{1,2}/\d{1,2}/\d{4}, \d{1,2}:\d\d:\d\d [AP]M$")
            .unwrap();
    let ids: Vec<String> = out
        .stdout
        .lines()
        .map(|l| line.captures(l).unwrap_or_else(|| panic!("{l}"))[1].to_string())
        .collect();
    assert_eq!(ids, [newer.clone(), older]);
    let out = anysola(&home, &["session", "list", "-n", "1", "--format", "json"]);
    let list: Value = serde_json::from_str(&out.stdout).unwrap();
    let entry = &list[0];
    let keys: Vec<&str> = entry
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "title",
            "updated",
            "created",
            "projectId",
            "directory"
        ]
    );
    assert_eq!(entry["id"], json!(newer));
    assert_eq!(
        entry["directory"],
        json!(home.project().display().to_string())
    );
    assert!(out.stdout.starts_with("[\n  {\n    \"id\": "));
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[test]
fn delete_removes_the_session() {
    let gateway = FakeGateway::start(answer);
    let home = home(&gateway);
    let id = session(&home);
    let out = anysola(&home, &["session", "delete", &id]);
    assert_eq!(out.stdout, format!("Session {id} deleted\n"));
    let out = anysola(&home, &["session", "delete", &id]);
    assert_eq!(
        (out.stderr.as_str(), out.code),
        (format!("Session not found: {id}\n").as_str(), 1)
    );
    assert_eq!(anysola(&home, &["session", "list"]).stdout, "");
}

/// The 0.5.1 export of the same one-prompt session, with run-specific values replaced. Also
/// left out: what the session loop records differently (session token totals that included the
/// title request, `time.streamed`, snapshots of a git project, the quoted prompt of §B1, an empty
/// `files` list).
fn normalized(mut export: Value) -> Value {
    let info = &mut export["info"];
    info["tokens"] = json!("<tokens>");
    for key in ["id", "projectID"] {
        info[key] = json!("<id>");
    }
    info["location"]["directory"] = json!("<dir>");
    for (_, time) in info["time"].as_object_mut().unwrap() {
        *time = json!(0);
    }
    for message in export["messages"].as_array_mut().unwrap() {
        message["id"] = json!("<id>");
        let object = message.as_object_mut().unwrap();
        object.shift_remove("snapshot");
        object.shift_remove("files");
        if object["type"] == "user" {
            object["text"] = json!("<prompt>");
        }
        let time = object["time"].as_object_mut().unwrap();
        time.shift_remove("streamed");
        for (_, value) in time {
            *value = json!(0);
        }
    }
    export
}

#[test]
fn export_matches_the_0_5_1_shape() {
    let gateway = FakeGateway::start(answer);
    let home = home(&gateway);
    let id = session(&home);
    let out = anysola(&home, &["session", "export", &id]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let ours: Value = serde_json::from_str(&out.stdout).unwrap();
    let captured: Value =
        serde_json::from_str(include_str!("fixtures/session-export-0.5.1.json")).unwrap();
    let ours = normalized(ours);
    let captured = normalized(captured);
    assert_eq!(
        anysola_core::json::pretty(&ours),
        anysola_core::json::pretty(&captured)
    );

    let out = anysola(&home, &["session", "export", &id, "--sanitize"]);
    let sanitized: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        sanitized["info"]["title"],
        json!(format!("[redacted:session-title:{id}]"))
    );
    assert_eq!(
        sanitized["info"]["location"]["directory"],
        json!(format!("/[redacted:session-directory:{id}]"))
    );
    let user = &sanitized["messages"][0];
    assert_eq!(
        user["text"],
        json!(format!("[redacted:text:{}]", user["id"].as_str().unwrap()))
    );
    let assistant = &sanitized["messages"][1];
    assert_eq!(
        assistant["content"][0]["text"],
        json!(format!(
            "[redacted:text:{}]",
            assistant["id"].as_str().unwrap()
        ))
    );

    let out = anysola(&home, &["session", "export", "ses_nope"]);
    assert_eq!(
        (out.stderr.as_str(), out.code),
        ("Session not found: ses_nope\n", 1)
    );
    let out = anysola(&home, &["session", "export"]);
    assert_eq!(
        (out.stderr.as_str(), out.code),
        (
            "Pass a session ID when running without an interactive terminal\n",
            1
        )
    );
}

#[test]
fn import_round_trips_an_export() {
    let gateway = FakeGateway::start(answer);
    let source = home(&gateway);
    let id = session(&source);
    let exported = anysola(&source, &["session", "export", &id]).stdout;
    let file = source.write("export.json", &exported);

    let target = home(&gateway);
    let file = file.display().to_string();
    let out = anysola(&target, &["session", "import", &file]);
    assert_eq!(
        (out.stdout.as_str(), out.code),
        (format!("Imported session: {id}\n").as_str(), 0)
    );
    let again = anysola(&target, &["session", "export", &id]).stdout;
    let mut before: Value = serde_json::from_str(&exported).unwrap();
    let mut after: Value = serde_json::from_str(&again).unwrap();
    for export in [&mut before, &mut after] {
        export["info"]["time"]["updated"] = json!(0);
        export["info"]["location"]["directory"] = json!("<dir>");
        export["info"]["projectID"] = json!("<project>");
    }
    assert_eq!(
        anysola_core::json::pretty(&before),
        anysola_core::json::pretty(&after)
    );
    let listed = anysola(&target, &["session", "list"]).stdout;
    assert!(
        listed.starts_with(&format!("{id}\tFake title\t")),
        "{listed}"
    );

    let out = anysola(&target, &["session", "import", &file]);
    assert_eq!(
        (out.stderr.as_str(), out.code),
        ("Session already exists\n", 0)
    );

    let third = home(&gateway);
    let elsewhere = third.home().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let directory = elsewhere.display().to_string();
    let out = anysola(
        &third,
        &["session", "import", &file, "--directory", "../elsewhere"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let moved: Value =
        serde_json::from_str(&anysola(&third, &["session", "export", &id]).stdout).unwrap();
    assert_eq!(moved["info"]["location"]["directory"], json!(directory));
}

#[test]
fn import_errors() {
    let gateway = FakeGateway::start(answer);
    let home = home(&gateway);
    let missing = home.home().join("missing.json").display().to_string();
    let cases = [
        (
            missing.clone(),
            format!(
                "Error: Failed to read session data: ENOENT: no such file or directory, open '{missing}'\n"
            ),
        ),
        (
            home.write("bad.json", "not json").display().to_string(),
            "Error: Expected a valid JSON string\n".to_string(),
        ),
        (
            home.write("shape.json", "{\"a\":1}").display().to_string(),
            "Error: Failed to import session: Bad Request\n".to_string(),
        ),
        (
            format!("{}/nothere", gateway.url),
            "Error: Failed to read session data: Failed to fetch session data: Not Found\n"
                .to_string(),
        ),
    ];
    for (source, expected) in cases {
        let out = anysola(&home, &["session", "import", &source]);
        assert_eq!((out.stderr.as_str(), out.code), (expected.as_str(), 1));
    }
    gateway.serve("/export.json", "{}");
    let out = anysola(
        &home,
        &["session", "import", &format!("{}/export.json", gateway.url)],
    );
    assert_eq!(out.stderr, "Error: Failed to import session: Bad Request\n");
}
