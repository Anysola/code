//! Hooks on every platform, Windows included: whatever shell the `shell` tool picks there runs
//! a hook command, and its exit code decides. No script file, no Unix tools. Which shell ran is
//! proved with commands only one kind of shell can run, also without `SHELL` (as the app runs
//! when started from the Start menu), which the CI's own shell always sets.

mod common;

use anysola_core::api::{Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn last(request: &Recorded) -> String {
    request.json()["messages"]
        .as_array()
        .and_then(|m| m.last())
        .map(Value::to_string)
        .unwrap_or_default()
}

/// A hook handler running `command`, in `shell` when given.
fn handler(command: &str, shell: Option<&str>) -> Value {
    let mut handler = json!({"type": "command", "command": command, "timeout": 60});
    if let Some(shell) = shell {
        handler["shell"] = json!(shell);
    }
    handler
}

fn hooks(handler: Value) -> Value {
    json!({"hooks": {"PreToolUse": [{"matcher": "Glob", "hooks": [handler]}]}})
}

/// The tool result the model got for one `glob` call under a PreToolUse hook `handler` of the
/// user's config, or (`project`) of the project's approved one; with the config's warnings.
fn glob_run(handler: Value, project: bool) -> (String, Vec<String>) {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() || last(request).contains("tool_result") {
            return text(request, "done");
        }
        let model = request.json()["model"].as_str().unwrap().to_string();
        Reply::Sse {
            body: Stream::new()
                .tool("toolu_1", "glob", json!({"pattern": "*.md"}))
                .finish(&model, Some(1)),
        }
    });
    let env = env(&gateway.url);
    let file = if project {
        "proj/anysola.json"
    } else {
        ".anysola/config.json"
    };
    env.home.write(file, &hooks(handler).to_string());
    env.home.write("proj/notes.md", "x");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    if project {
        let document = core.hook_documents().remove(0);
        core.trust_hooks(&document.path, &document.digest, true)
            .unwrap();
    }
    let (session, _) = core.open(&Target::New, false).unwrap();
    let prompt = Prompt {
        text: "go".into(),
        ..Default::default()
    };
    runtime
        .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
        .unwrap();
    let requests: Vec<Recorded> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    assert_eq!(requests.len(), 2);
    (last(&requests[1]), core.loaded().config.warnings.clone())
}

fn glob_result(command: &str) -> String {
    glob_run(handler(command, None), false).0
}

/// POSIX shells only: arithmetic expansion into a variable, then stderr. PowerShell and cmd
/// can't print `posix=42` to stderr from it.
const POSIX: &str = r#"x=$((6*7)); echo "posix=$x" >&2; exit 2"#;
/// PowerShell only: a POSIX shell stops at the syntax, without expanding the variable.
const POWERSHELL: &str = "[Console]::Error.WriteLine('ps=' + $PSVersionTable.PSEdition); exit 2";

/// The shell hooks without a `shell` of their own run in here.
fn default_shell() -> String {
    let shell = anysola_core::tool::shell::hook_shell(None);
    anysola_core::tool::shell::shell_name(Some(&shell))
}

#[test]
fn exit_two_blocks_and_exit_zero_lets_the_call_run() {
    let blocked = glob_result("exit 2");
    assert!(
        blocked.contains("A PreToolUse hook blocked this call"),
        "{blocked}"
    );
    assert!(!blocked.contains("notes.md"), "{blocked}");
    let ran = glob_result("exit 0");
    assert!(!ran.contains("blocked this call"), "{ran}");
    assert!(ran.contains("notes.md"), "{ran}");
}

/// Without a `shell` of its own a hook runs in the default: a POSIX shell on Linux, Git Bash on
/// Windows (the CI image has Git), even when `SHELL` is unset or names PowerShell.
#[test]
fn a_hook_without_a_shell_runs_in_the_default() {
    let (result, _) = glob_run(handler(POSIX, None), false);
    assert!(result.contains("posix=42"), "{result}");
    if cfg!(windows) {
        assert_eq!(default_shell(), "bash", "Windows CI has Git Bash");
    }
}

/// A hook's own `shell` decides where it runs: PowerShell for PowerShell syntax (Windows; where
/// PowerShell 7 is installed elsewhere), and on Unix a shell of the user's (a wrapper that notes
/// it ran).
#[test]
fn a_hook_runs_in_the_shell_it_names() {
    let powershell = if cfg!(windows) {
        Some("powershell")
    } else {
        anysola_core::which::which("pwsh").map(|_| "pwsh")
    };
    if let Some(shell) = powershell {
        let (result, _) = glob_run(handler(POWERSHELL, Some(shell)), false);
        assert!(
            result.contains("ps=Desktop") || result.contains("ps=Core"),
            "{result}"
        );
    }
    #[cfg(unix)]
    {
        let (wrapper, marker) = wrapper_shell();
        let (result, _) = glob_run(handler(POSIX, Some(&wrapper)), false);
        assert!(result.contains("posix=42"), "{result}");
        assert!(marker.exists(), "BREAK: the hook's own shell didn't run it");
    }
}

/// A project document's hook can't choose its shell (as it can't choose the config's): the
/// field is dropped with a warning and the approved hook runs in the default.
#[test]
fn a_project_hook_cannot_choose_its_shell() {
    #[cfg(unix)]
    let (shell, marker) = wrapper_shell();
    #[cfg(not(unix))]
    let shell = "powershell".to_string();
    let (result, warnings) = glob_run(handler(POSIX, Some(&shell)), true);
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("Ignoring the \"shell\" of hooks in")),
        "{warnings:?}"
    );
    assert!(result.contains("posix=42"), "{result}");
    #[cfg(unix)]
    assert!(!marker.exists(), "BREAK: a project chose the hook's shell");
}

/// A shell that notes it ran, then runs `sh` with its arguments: `(path, marker file)`.
#[cfg(unix)]
fn wrapper_shell() -> (String, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("hook-shell-{}", anysola_core::ids::part()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("ran");
    let shell = dir.join("myshell");
    std::fs::write(
        &shell,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nexec /bin/sh \"$@\"\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
    (shell.display().to_string(), marker)
}

/// The same proofs in a process without `SHELL`, and with `SHELL` naming PowerShell (Windows'
/// "Git Bash even then" path).
#[test]
fn hook_shells_hold_without_shell_set() {
    for name in [
        "a_hook_without_a_shell_runs_in_the_default",
        "a_hook_runs_in_the_shell_it_names",
        "a_project_hook_cannot_choose_its_shell",
    ] {
        again_with_shell(name, None);
        again_with_shell(name, Some("powershell.exe"));
    }
}
