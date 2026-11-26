//! Opt-in and project-trust boundaries: the shell sandbox is off by default and controlled only by
//! user/managed config; LSP diagnostics are enabled by default ONLY for projects the user approves
//! (`TrustKind::Lsp`), and `"lsp": false` disables them globally or per-project without asking.

mod common;

#[cfg(unix)]
use anysola_core::api::TrustKind;
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

/// Template ``!`cmd` `` honors `"sandbox": true` (refused when unavailable; runs inside sandbox
/// with `ANYSOLA_SANDBOX=1` when bwrap starts), and deleting a session removes its sandbox cache.
#[cfg(unix)]
#[test]
fn template_interpolation_honors_sandbox_and_delete_cleans_cache() {
    let gateway = anysola_testkit::FakeGateway::start(|request, _| text(request, "done"));
    let env = env(&gateway.url);
    env.home
        .write(".anysola/config.json", r#"{"sandbox": true}"#);
    env.home.write(
        ".anysola/commands/probe.md",
        "---\ndescription: probe\n---\nResult: !`echo [$ANYSOLA_SANDBOX]`\n",
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let cache = core.paths().sandbox_cache_dir(&session.id).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("marker"), "1").unwrap();

    let outcome = runtime
        .block_on(core.command(&session.id, "probe", "", TurnOptions::default()))
        .unwrap();
    if sandbox_starts() {
        assert!(
            matches!(outcome, anysola_core::session::Outcome::Succeeded),
            "{outcome:?}"
        );
        let req = gateway
            .messages()
            .into_iter()
            .rfind(|r| !r.is_title())
            .expect("model request");
        assert!(
            req.json().to_string().contains("Result: [1]"),
            "{}",
            req.json()
        );
    } else {
        match outcome {
            anysola_core::session::Outcome::Failed(err) => {
                assert!(err.message.contains("was not run"), "{}", err.message);
            }
            other => panic!("expected sandbox refusal, got {other:?}"),
        }
    }

    assert!(core.delete_session(&session.id).unwrap());
    assert!(
        !cache.exists(),
        "deleting a session must remove its sandbox cache directory"
    );
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

#[cfg(unix)]
fn approve_lsp(core: &Core, runtime: &tokio::runtime::Runtime) -> bool {
    let Some(item) = core
        .trust_items()
        .into_iter()
        .find(|i| i.kind == TrustKind::Lsp)
    else {
        return false;
    };
    core.trust(&item.path, TrustKind::Lsp, &item.digest, true)
        .unwrap();
    runtime.block_on(core.reload());
    true
}

/// Edits a `.ts` file with the given user/project config and optional LSP approval; returns
/// whether a server was started.
#[cfg(unix)]
fn lsp_starts(user: Option<&str>, project: Option<&str>, approve: bool, warning: bool) -> bool {
    let gateway = once(
        "write",
        json!({"path": "a.ts", "content": "const a: number = 1\n"}),
    );
    let env = env(&gateway.url);
    let server = FakeServer::install(&env);
    env.home.write("proj/package.json", r#"{"name": "p"}"#);
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
    if approve {
        assert!(
            approve_lsp(&core, &runtime),
            "expected a TrustKind::Lsp item to approve"
        );
    }
    run_turn(&core, &runtime);
    assert!(env.home.project().join("a.ts").exists());
    let started = server.started();
    drop(core);
    started
}

#[cfg(unix)]
#[test]
fn lsp_waits_for_project_approval_and_honors_disable() {
    // Default config, unapproved project: never starts.
    assert!(
        !lsp_starts(None, None, false, false),
        "BREAK: started in an unapproved project"
    );
    // Explicit user `"lsp": true`, still unapproved: never starts.
    assert!(
        !lsp_starts(Some(r#"{"lsp": true}"#), None, false, false),
        "BREAK: user true bypassed project approval"
    );
    // Project `"lsp": true` warns and never overrides user `"lsp": false`.
    assert!(
        !lsp_starts(
            Some(r#"{"lsp": false}"#),
            Some(r#"{"lsp": true}"#),
            false,
            true
        ),
        "BREAK: project true overrode user false"
    );
    // Non-boolean `"lsp": "yes"` is treated as disabled.
    assert!(
        !lsp_starts(Some(r#"{"lsp": "yes"}"#), None, false, false),
        "BREAK: non-boolean value enabled LSP"
    );
    // Default config + project approval: starts without needing `"lsp": true` in user config.
    assert!(
        lsp_starts(None, None, true, false),
        "expected approved project to start LSP by default"
    );
    // User `"lsp": false` or project `"lsp": false` suppresses the TrustKind::Lsp item and blocks
    // LSP even if a previous decision existed in the database.
    for (user, project) in [
        (Some(r#"{"lsp": false}"#), None),
        (None, Some(r#"{"lsp": false}"#)),
    ] {
        let gateway = once(
            "write",
            json!({"path": "a.ts", "content": "const a: number = 1\n"}),
        );
        let env = env(&gateway.url);
        let server = FakeServer::install(&env);
        env.home.write("proj/package.json", r#"{"name": "p"}"#);
        let runtime = runtime();
        // First open & approve.
        let core = runtime.block_on(common::core(&env));
        assert!(approve_lsp(&core, &runtime));
        drop(core);
        // Now disable via user or project config and reopen.
        if let Some(u) = user {
            env.home.write(".anysola/config.json", u);
        }
        if let Some(p) = project {
            env.home.write("proj/anysola.json", p);
        }
        let core = runtime.block_on(common::core(&env));
        assert!(
            core.trust_items().iter().all(|i| i.kind != TrustKind::Lsp),
            "disabled LSP must not ask for approval"
        );
        run_turn(&core, &runtime);
        assert!(
            !server.started(),
            "BREAK: LSP started despite disable ({user:?}, {project:?})"
        );
        drop(core);
    }
}

/// Revoking `TrustKind::Lsp` stops an already-running language server, and adding a new language
/// ecosystem (`Cargo.toml` + `rust-analyzer`) invalidates the digest so the new server waits.
#[cfg(unix)]
#[test]
fn revoking_lsp_stops_running_server_and_new_ecosystem_reprompts() {
    use std::os::unix::fs::PermissionsExt;
    let gateway = once("write", json!({"path": "a.ts", "content": "const a = 1\n"}));
    let env = env(&gateway.url);
    let mut server = ScriptedServer::install(&env, "");
    env.home.write("proj/package.json", r#"{"name": "p"}"#);
    // Put a fake rust-analyzer on PATH alongside typescript-language-server.
    let fake_bin = env.home.home().join("fake-bin");
    let rust_marker = env.home.home().join("rust-lsp-started");
    let rust_bin = fake_bin.join("rust-analyzer");
    std::fs::write(
        &rust_bin,
        format!("#!/bin/sh\necho started >> '{}'\n", rust_marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&rust_bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let item = core
        .trust_items()
        .into_iter()
        .find(|i| i.kind == TrustKind::Lsp)
        .expect("LSP trust item");
    assert_eq!(item.entries.len(), 1);
    assert_eq!(item.entries[0].label, "TypeScript / JavaScript");
    core.trust(&item.path, TrustKind::Lsp, &item.digest, true)
        .unwrap();
    runtime.block_on(core.reload());
    run_turn(&core, &runtime);
    assert_eq!(core.loaded().config.approved_lsp(), &["typescript"]);

    // Revoking immediately shuts down the running TypeScript language server.
    core.trust(&item.path, TrustKind::Lsp, &item.digest, false)
        .unwrap();
    runtime.block_on(core.reload());
    assert!(core.loaded().config.approved_lsp().is_empty());

    // Re-approve TypeScript, then add a Cargo.toml: digest changes so decision resets to None.
    core.trust(&item.path, TrustKind::Lsp, &item.digest, true)
        .unwrap();
    runtime.block_on(core.reload());
    env.home.write(
        "proj/Cargo.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n",
    );
    runtime.block_on(core.reload());
    let updated = core
        .trust_items()
        .into_iter()
        .find(|i| i.kind == TrustKind::Lsp)
        .expect("updated LSP trust item");
    assert_ne!(updated.digest, item.digest);
    assert_eq!(updated.decision, None);
    assert_eq!(updated.entries.len(), 2);
    assert!(core.loaded().config.approved_lsp().is_empty());

    core.shutdown();
    drop(core);
    drop(runtime);
    server.finish();
    assert!(server.lines("shutdown") >= 1, "{}", server.kept);
    assert!(server.lines("exit") >= 1, "{}", server.kept);
    assert!(!rust_marker.exists());
}

/// Editing a file outside the project root never runs LSP diagnostics on it (even when the
/// opened project has approved LSP) and never adds its language to the project's trust digest.
#[cfg(unix)]
#[test]
fn external_file_edit_never_starts_lsp_or_pollutes_project_trust() {
    let outside = tempfile::tempdir().unwrap();
    let external_ts = outside.path().join("untrusted.ts");
    let gateway = once(
        "write",
        json!({"path": external_ts.display().to_string(), "content": "const x: number = 1\n"}),
    );
    let env = env(&gateway.url);
    let server = FakeServer::install(&env);
    env.home.write("proj/package.json", r#"{"name": "p"}"#);
    // Allow external_directory + write so the edit itself succeeds.
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "external_directory", "resource": "*", "effect": "allow"}]}"#,
    );
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(approve_lsp(&core, &runtime));
    run_turn(&core, &runtime);
    assert!(external_ts.exists());
    assert!(
        !server.started(),
        "BREAK: editing an external file started LSP for it"
    );
    drop(core);
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
    env.home.write("proj/package.json", r#"{"name": "p"}"#);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    assert!(approve_lsp(&core, &runtime));
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
