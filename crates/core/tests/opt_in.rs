//! What only the user's own config turns on (1.4.2): the shell sandbox and LSP diagnostics. Both
//! are off by default; a project document can't turn either on, and can only turn LSP off.

mod common;

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn results(request: &Recorded) -> Vec<Value> {
    request.json()["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == "tool_result")
        .collect()
}

/// A model that makes one call, then says done.
fn once(name: &'static str, input: Value) -> FakeGateway {
    FakeGateway::start(move |request, _| {
        if request.is_title() || !results(request).is_empty() {
            return text(request, "done");
        }
        let model = request.json()["model"].as_str().unwrap().to_string();
        Reply::Sse {
            body: Stream::new()
                .tool("t0", name, input.clone())
                .finish(&model, Some(1)),
        }
    })
}

fn run_turn(core: &Core, runtime: &tokio::runtime::Runtime) {
    let (session, _) = core.open(&Target::New, false).unwrap();
    let prompt = Prompt {
        text: "go".into(),
        ..Default::default()
    };
    runtime
        .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
        .unwrap();
}

fn tool_result(gateway: &FakeGateway) -> String {
    gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title())
        .and_then(|r| results(&r).into_iter().next())
        .map(|r| r.to_string())
        .unwrap_or_default()
}

fn warned(core: &Core, key: &str) -> bool {
    core.loaded()
        .config
        .warnings
        .iter()
        .any(|w| w.contains(&format!("Ignoring \"{key}\"")))
}

#[cfg(target_os = "linux")]
fn sandbox_starts() -> bool {
    anysola_core::tool::shell::sandbox::bwrap_path().is_some()
}

#[cfg(not(target_os = "linux"))]
fn sandbox_starts() -> bool {
    false
}

/// The user asked for the sandbox and a project says `false`: the project is ignored (with a
/// warning), and where bwrap can't start the command is refused rather than run unconfined.
#[test]
fn a_sandbox_the_user_asked_for_is_never_skipped() {
    let gateway = once(
        "shell",
        json!({"command": "echo \"[$ANYSOLA_SANDBOX]\" > ran.txt"}),
    );
    let env = env(&gateway.url);
    env.home
        .write(".anysola/config.json", r#"{"sandbox": true}"#);
    env.home.write("proj/anysola.json", r#"{"sandbox": false}"#);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(
        warned(&core, "sandbox"),
        "{:?}",
        core.loaded().config.warnings
    );
    run_turn(&core, &runtime);
    let wrote = std::fs::read_to_string(env.home.project().join("ran.txt"));
    let ran = wrote.is_ok();
    if sandbox_starts() {
        // Written, and from inside the sandbox (not because the project's `false` won).
        assert_eq!(wrote.unwrap_or_default().trim(), "[1]");
    } else {
        let result = tool_result(&gateway);
        assert!(
            !ran,
            "BREAK: ran without the sandbox the user asked for: {result}"
        );
        assert!(result.contains("was not run"), "{result}");
    }
}

/// A project's `"sandbox": true` turns nothing on: the command runs as it would without it.
/// (Unix: the command reads a variable the POSIX way.)
#[cfg(unix)]
#[test]
fn a_project_cannot_turn_the_sandbox_on() {
    let gateway = once(
        "shell",
        json!({"command": "echo \"[$ANYSOLA_SANDBOX]\" > ran.txt"}),
    );
    let env = env(&gateway.url);
    env.home.write("proj/anysola.json", r#"{"sandbox": true}"#);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(warned(&core, "sandbox"));
    run_turn(&core, &runtime);
    let ran = std::fs::read_to_string(env.home.project().join("ran.txt"));
    assert_eq!(ran.unwrap_or_default().trim(), "[]");
}

/// A fake `typescript-language-server` first on `PATH` that only notes it was started.
#[cfg(unix)]
struct FakeServer {
    marker: std::path::PathBuf,
    old_path: Option<std::ffi::OsString>,
}

#[cfg(unix)]
impl FakeServer {
    fn install(env: &Env) -> FakeServer {
        use std::os::unix::fs::PermissionsExt;
        let bin = env.home.home().join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let marker = env.home.home().join("lsp-started");
        let server = bin.join("typescript-language-server");
        std::fs::write(
            &server,
            format!("#!/bin/sh\necho started >> '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        let old_path = std::env::var_os("PATH");
        let mut dirs = vec![bin];
        dirs.extend(old_path.iter().flat_map(std::env::split_paths));
        // SAFETY: the test holds the environment lock (`env`) until this is dropped.
        unsafe { std::env::set_var("PATH", std::env::join_paths(dirs).unwrap()) };
        FakeServer { marker, old_path }
    }

    fn started(&self) -> bool {
        self.marker.exists()
    }
}

#[cfg(unix)]
impl Drop for FakeServer {
    fn drop(&mut self) {
        // SAFETY: as in `install`.
        unsafe {
            match &self.old_path {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
    }
}

/// Edits a `.ts` file with the given user and project config; whether a server was started.
#[cfg(unix)]
fn lsp_starts(user: Option<&str>, project: Option<&str>, warning: bool) -> bool {
    let gateway = once(
        "write",
        json!({"path": "a.ts", "content": "const a: number = 1\n"}),
    );
    let env = env(&gateway.url);
    let server = FakeServer::install(&env);
    if let Some(user) = user {
        env.home.write(".anysola/config.json", user);
    }
    if let Some(project) = project {
        env.home.write("proj/anysola.json", project);
    }
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert_eq!(
        warned(&core, "lsp"),
        warning,
        "{:?}",
        core.loaded().config.warnings
    );
    run_turn(&core, &runtime);
    assert!(env.home.project().join("a.ts").exists());
    let started = server.started();
    drop(core);
    started
}

#[cfg(unix)]
#[test]
fn lsp_is_off_unless_the_user_turns_it_on() {
    assert!(!lsp_starts(None, None, false), "BREAK: on by default");
    assert!(
        !lsp_starts(None, Some(r#"{"lsp": true}"#), true),
        "BREAK: a project turned LSP on"
    );
    assert!(
        !lsp_starts(Some(r#"{"lsp": "yes"}"#), None, false),
        "BREAK: a non-boolean value turned LSP on"
    );
    assert!(
        !lsp_starts(Some(r#"{"lsp": true}"#), Some(r#"{"lsp": false}"#), false),
        "a project may turn it off"
    );
    // The control: the user's own `true` does start it.
    assert!(lsp_starts(Some(r#"{"lsp": true}"#), None, false));
}
