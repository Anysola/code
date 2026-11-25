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

/// A model that makes these calls one after another in one turn, then says done.
#[cfg(unix)]
fn calls(script: Vec<(&'static str, Value)>) -> FakeGateway {
    FakeGateway::start(move |request, _| {
        let done = results(request).len();
        if request.is_title() || done >= script.len() {
            return text(request, "done");
        }
        let (name, input) = &script[done];
        let model = request.json()["model"].as_str().unwrap().to_string();
        Reply::Sse {
            body: Stream::new()
                .tool(&format!("t{done}"), name, input.clone())
                .finish(&model, Some(1)),
        }
    })
}

/// The fake language server (`tests/fake-lsp/server.py`, run by `python3`) installed first on
/// `PATH` as `typescript-language-server`, with its `mode` flags; its log tells what it did.
#[cfg(unix)]
struct ScriptedServer {
    log: std::path::PathBuf,
    old_path: Option<std::ffi::OsString>,
    /// The log as it was when the run ended (its folder goes with the test's home).
    kept: String,
    /// Servers `Core::shutdown_lsp_root` stopped for the project after the turn.
    stopped: usize,
}

#[cfg(unix)]
impl ScriptedServer {
    fn install(env: &Env, mode: &str) -> ScriptedServer {
        use std::os::unix::fs::PermissionsExt;
        let script =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-lsp/server.py");
        let python = anysola_core::which::which("python3").expect("python3 runs the fake server");
        let bin = env.home.home().join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = env.home.home().join("lsp.log");
        let server = bin.join("typescript-language-server");
        std::fs::write(
            &server,
            format!(
                "#!/bin/sh\nexec '{}' '{}' '{mode}' '{}'\n",
                python.display(),
                script.display(),
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        let old_path = std::env::var_os("PATH");
        let mut dirs = vec![bin];
        dirs.extend(old_path.iter().flat_map(std::env::split_paths));
        // SAFETY: the test holds the environment lock (`env`) until this is dropped.
        unsafe { std::env::set_var("PATH", std::env::join_paths(dirs).unwrap()) };
        env.home.write(".anysola/config.json", r#"{"lsp": true}"#);
        ScriptedServer {
            log,
            old_path,
            kept: String::new(),
            stopped: 0,
        }
    }

    /// Keeps the log and puts `PATH` back, while the test still holds the environment lock.
    fn finish(&mut self) {
        self.kept = std::fs::read_to_string(&self.log).unwrap_or_default();
        restore_path(self.old_path.take());
    }

    fn lines(&self, what: &str) -> usize {
        self.kept.lines().filter(|l| l.starts_with(what)).count()
    }
}

#[cfg(unix)]
impl Drop for ScriptedServer {
    fn drop(&mut self) {
        // A test that failed before `finish` (dropped before its environment lock).
        if self.old_path.is_some() {
            restore_path(self.old_path.take());
        }
    }
}

#[cfg(unix)]
fn restore_path(old: Option<std::ffi::OsString>) {
    // SAFETY: called while the test holds the environment lock.
    unsafe {
        match old {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
}

/// Every tool result of the turn, in order (with a deadline: an edit must never hang).
#[cfg(unix)]
fn edit_results(mode: &str, writes: Vec<(&'static str, String)>) -> (Vec<String>, ScriptedServer) {
    let script = writes
        .into_iter()
        .map(|(path, content)| ("write", json!({"path": path, "content": content})))
        .collect();
    let gateway = calls(script);
    let env = env(&gateway.url);
    let server = ScriptedServer::install(&env, mode);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let mut server = server;
    let (session, _) = core.open(&Target::New, false).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let turn = {
        let core = core.clone();
        let id = session.id.clone();
        runtime.spawn(async move {
            let prompt = Prompt {
                text: "go".into(),
                ..Default::default()
            };
            let _ = core.prompt(&id, prompt, TurnOptions::default()).await;
            let _ = tx.send(());
        })
    };
    let finished = rx.recv_timeout(std::time::Duration::from_secs(60));
    if finished.is_err() {
        turn.abort();
        panic!("BREAK: an edit hung on the language server");
    }
    let results = gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title())
        .map(|r| results(&r).iter().map(Value::to_string).collect())
        .unwrap_or_default();
    server.stopped = runtime.block_on(core.shutdown_lsp_root(&env.home.project()));
    core.shutdown();
    drop(core);
    drop(runtime);
    server.finish();
    drop(env);
    (results, server)
}

/// Diagnostics that don't arrive in time are not guessed: the second edit fixed the error and
/// the server says nothing for it, so nothing comes back, never the first edit's error. With and
/// without versions in the server's reports.
#[cfg(unix)]
#[test]
fn late_diagnostics_never_show_an_earlier_versions() {
    for mode in ["silent", "silent,versionless"] {
        let (results, _) = edit_results(
            mode,
            vec![
                ("a.ts", "const broken\n".to_string()),
                ("a.ts", "const fixed = 1\n".to_string()),
            ],
        );
        assert_eq!(results.len(), 2, "{results:?}");
        assert!(
            results[0].contains("text=const broken"),
            "{mode}: {}",
            results[0]
        );
        assert!(
            !results[1].contains("diagnostics"),
            "BREAK ({mode}): the first edit's error shown for the second: {}",
            results[1]
        );
    }
}

/// Each edited file is closed again after its diagnostics: the server reads every other file
/// from disk, and open documents don't pile up.
#[cfg(unix)]
#[test]
fn edited_files_are_closed_after_their_diagnostics() {
    let (results, server) = edit_results(
        "",
        vec![
            ("a.ts", "a\n".to_string()),
            ("b.ts", "b\n".to_string()),
            ("c.ts", "c\n".to_string()),
        ],
    );
    assert_eq!(results.len(), 3, "{results:?}");
    for result in &results {
        assert!(
            result.contains("open=1 "),
            "BREAK: documents left open: {result}"
        );
    }
    assert_eq!(server.lines("close"), 3);
    // The project's root going stops its server, asked first.
    assert_eq!(server.stopped, 1);
    assert_eq!(server.lines("shutdown"), 1);
    assert_eq!(server.lines("exit"), 1);
}

/// A server that stops reading its input never holds up an edit (a write bigger than the pipe
/// holds), and it is dropped.
#[cfg(unix)]
#[test]
fn a_server_that_stops_reading_never_hangs_an_edit() {
    let big = format!("{}\n", "x".repeat(300_000));
    let (results, server) = edit_results(
        "deaf",
        vec![("a.ts", big.clone()), ("b.ts", big.clone()), ("c.ts", big)],
    );
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(server.lines("deaf") >= 1);
}

/// A server whose output can't be read is dropped and started again on the next edit; one that
/// dies on every message rests after two tries instead of restarting for each edit.
#[cfg(unix)]
#[test]
fn a_broken_server_restarts_once_then_rests() {
    let (results, server) = edit_results(
        "garbage",
        vec![
            ("a.ts", "a\n".to_string()),
            ("b.ts", "b\n".to_string()),
            ("c.ts", "c\n".to_string()),
        ],
    );
    assert!(results[0].contains("text=a"), "{}", results[0]);
    assert!(
        results[1].contains("text=b"),
        "BREAK: not started again: {}",
        results[1]
    );
    assert_eq!(
        server.lines("start"),
        2,
        "BREAK: restarted after every report"
    );

    let writes = (0..4).map(|i| ("a.ts", format!("v{i}\n"))).collect();
    let (results, server) = edit_results("crash", writes);
    assert_eq!(results.len(), 4);
    assert_eq!(
        server.lines("start"),
        2,
        "BREAK: a server that always crashes was restarted for each edit"
    );
}
