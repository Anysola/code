//! In-process core tests: a temp home whose environment the process takes on while a test runs
//! (tests in one binary are serialized), and the fake gateway.
#![allow(dead_code)]

use std::sync::{Mutex, MutexGuard};

use anysola_core::api::Core;
use anysola_testkit::sse::Stream;
use anysola_testkit::{Recorded, Reply, TestHome};
use serde_json::json;

static ENV: Mutex<()> = Mutex::new(());

pub struct Env {
    pub home: TestHome,
    _guard: MutexGuard<'static, ()>,
}

pub fn env(gateway_url: &str) -> Env {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = TestHome::new(gateway_url);
    for (name, value) in home.env() {
        // SAFETY: tests in this binary hold `ENV` while they touch the environment.
        unsafe { std::env::set_var(name, value) };
    }
    Env {
        home,
        _guard: guard,
    }
}

/// A runtime of the test's own; the fake gateway runs on another.
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

pub async fn core(env: &Env) -> Core {
    Core::start(&env.home.project()).await.expect("core starts")
}

pub fn text(request: &Recorded, text: &str) -> Reply {
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

/// The text of every message of a request, in order (`role: text` per block).
pub fn transcript(request: &Recorded) -> Vec<String> {
    let body = request.json();
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| {
            let role = m["role"].as_str().unwrap_or_default().to_string();
            m["content"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(move |b| {
                    let text = b["text"]
                        .as_str()
                        .or(b["content"].as_str())
                        .or(b["name"].as_str())
                        .unwrap_or_default()
                        .to_string();
                    format!("{role}: {text}")
                })
        })
        .collect()
}
