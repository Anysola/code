//! A session export (Settings → Data in the desktop app) keeps 0.5.1's `{info, messages}` shape,
//! and the sanitized form redacts its text and paths.

mod common;
mod replay;

use anysola_core::session::Outcome;
use anysola_core::storage::Db;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use replay::Run;
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
    }
}

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
    let env = common::env(&gateway.url);
    let home = &env.home;
    assert_eq!(
        Run::default().go(&home.project(), "hello there"),
        Ok(Outcome::Succeeded)
    );
    let db = Db::open(&home.home().join(".anysola/anysola.db")).unwrap();
    let (id, _, _) = db
        .all_root_sessions()
        .unwrap()
        .into_iter()
        .next()
        .expect("a session");
    let ours = db.export_session(&id).unwrap().expect("exported");
    let captured: Value =
        serde_json::from_str(include_str!("replay/session-export-0.5.1.json")).unwrap();
    assert_eq!(
        anysola_core::json::pretty(&normalized(ours.clone())),
        anysola_core::json::pretty(&normalized(captured))
    );

    let mut sanitized = ours;
    anysola_core::storage::sanitize(&mut sanitized);
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
    assert_eq!(db.export_session("ses_nope").unwrap(), None);
}
