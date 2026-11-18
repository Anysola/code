//! Compaction against the fake gateway, one headless turn at a time: the overflow path replays 0.5.1's captured summary request
//! and the checkpoint request after it; auto compaction runs at the ceiling.

mod common;
mod replay;

use anysola_core::session::Outcome;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use replay::*;
use serde_json::json;

fn summary() -> String {
    let capture = read_json("session/requests/after-compaction-checkpoint.json");
    let text = capture["body"]["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let start = text.find("<summary>\n").unwrap() + "<summary>\n".len();
    let end = text.find("\n</summary>").unwrap();
    text[start..end].to_string()
}

fn sse(request: &Recorded, text: &str) -> Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Reply::Sse {
        body: Stream::new()
            .usage(json!({"input_tokens": 10, "output_tokens": 1}))
            .text(text)
            .finish(&model, Some(1)),
    }
}

fn too_long() -> Reply {
    Reply::Json {
        status: 400,
        body: json!({"type": "error", "error": {"type": "invalid_request_error", "message": "prompt is too long: 1000001 tokens > 1000000 maximum"}}),
        headers: vec![],
    }
}

#[test]
fn overflow_compaction() {
    let summary = summary();
    let gateway = FakeGateway::start(move |request, _| {
        let body = request.body.clone();
        if request.is_title() {
            sse(request, "Fake title")
        } else if body.contains("You MUST summarize") {
            sse(request, &summary)
        } else if body.contains("<conversation-checkpoint>") {
            sse(request, "Continued.")
        } else {
            too_long()
        }
    });
    let env = common::env(&gateway.url);
    let home = &env.home;
    let dir = proj(home);
    assert_eq!(
        Run::default().go(&dir, "OVERFLOW test"),
        Ok(Outcome::Succeeded)
    );
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 3);
    let places = our_places(home, &dir);
    assert_request(
        "summary",
        &main[1],
        &read_json("session/requests/compaction-overflow-request.json"),
        &places,
    );
    assert_request(
        "checkpoint",
        &main[2],
        &read_json("session/requests/after-compaction-checkpoint.json"),
        &places,
    );
}

#[test]
fn second_overflow_surfaces() {
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            sse(request, "Fake title")
        } else if request.body.contains("You MUST summarize") {
            sse(request, "## Objective\n- x")
        } else {
            too_long()
        }
    });
    let env = common::env(&gateway.url);
    let home = &env.home;
    let dir = proj(home);
    match Run::default().go(&dir, "big") {
        Ok(Outcome::Failed(error)) => {
            assert!(error.message.contains("prompt is too long"), "{error:?}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn auto_compaction_at_the_ceiling() {
    let gateway = FakeGateway::start(move |request, _| {
        let body = request.body.clone();
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if request.is_title() {
            return sse(request, "Fake title");
        }
        if body.contains("You MUST summarize") {
            return sse(request, "## Objective\n- keep going");
        }
        // The first answer reports a prompt just past Sonnet's 850,000-token ceiling.
        Reply::Sse {
            body: Stream::new()
                .usage(json!({"input_tokens": 850_001, "output_tokens": 1}))
                .text("Big answer.")
                .finish(&model, Some(1)),
        }
    });
    let env = common::env(&gateway.url);
    let home = &env.home;
    let dir = proj(home);
    assert_eq!(Run::default().go(&dir, "first"), Ok(Outcome::Succeeded));
    let resume = Run {
        resume: true,
        ..Run::default()
    };
    assert_eq!(resume.go(&dir, "second"), Ok(Outcome::Succeeded));
    let main: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(main.len(), 3);
    let summary = main[1].json();
    assert!(
        summary.get("tools").is_some(),
        "the unchanged request keeps its tools"
    );
    let last = summary["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert!(
        last["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("You MUST summarize")
    );
    let after = main[2].json();
    let first = after["messages"][0]["content"][0]["text"].as_str().unwrap();
    assert!(first.starts_with("<conversation-checkpoint>"));
    assert!(first.contains("<summary>\n## Objective\n- keep going\n</summary>"));
    assert!(first.contains("\n\n<recent-context>\n[User]: second\n</recent-context>\n"));
    assert_eq!(after["messages"].as_array().unwrap().len(), 1);
}
