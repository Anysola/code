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
    shell_as_asked();
    Env {
        home,
        _guard: guard,
    }
}

/// `SHELL` as [`again_with_shell`] asked for it (`ANYSOLA_TEST_SHELL`; `-`: none at all, as for
/// the app started from the Start menu or a desktop launcher). Nothing otherwise.
fn shell_as_asked() {
    // SAFETY: the callers hold `ENV`.
    match std::env::var("ANYSOLA_TEST_SHELL").ok().as_deref() {
        None => {}
        Some("-") => unsafe { std::env::remove_var("SHELL") },
        Some(shell) => unsafe { std::env::set_var("SHELL", shell) },
    }
}

/// Runs this binary's test `name` again in a process of its own whose `SHELL` is `shell` (`None`:
/// no `SHELL`). The shell is chosen once per process, so a test that sets `SHELL` in this one
/// would see the first choice; and CI runs the tests from a shell that sets it, unlike a user
/// starting the app. Nothing when this process is such a child already.
pub fn again_with_shell(name: &str, shell: Option<&str>) {
    if std::env::var_os("ANYSOLA_TEST_SHELL").is_some() {
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env("ANYSOLA_TEST_SHELL", shell.unwrap_or("-"))
        .env_remove("SHELL")
        .envs(shell.map(|shell| ("SHELL", shell)))
        .output()
        .expect("the test binary runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{name} with SHELL={shell:?}:\n{stdout}\n{stderr}"
    );
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
