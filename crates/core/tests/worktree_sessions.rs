//! Sessions in their own git worktree: where they may write, what they see, their snapshots,
//! the wait while the worktree is made, forks with files put back, apply-continue, and cleanup.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anysola_core::api::{Core, Fallback, Prompt, Release, Target};
use anysola_core::events::Event;
use anysola_core::session::TurnOptions;
use anysola_core::session::gate::{Floor, Reply as Answer};
use anysola_core::storage::{Db, WorktreeState};
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Default::default()
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
        .status
        .success()
}

fn results(request: &Recorded) -> Vec<Value> {
    request.json()["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == "tool_result")
        .collect()
}

/// User prompts (not tool results) in the request.
fn prompts(request: &Recorded) -> usize {
    request.json()["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["role"] == "user")
        .filter(|m| {
            m["content"].as_array().is_some_and(|c| {
                c.iter().any(|b| b["type"] == "text")
                    && !c.iter().any(|b| b["type"] == "tool_result")
            }) || m["content"].is_string()
        })
        .count()
}

/// A model that makes each turn's calls (one per step), then says done.
fn turns(script: Vec<Vec<(&'static str, Value)>>) -> FakeGateway {
    FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let turn = prompts(request).saturating_sub(1);
        let before: usize = script.iter().take(turn).map(Vec::len).sum();
        let done = results(request).len().saturating_sub(before);
        match script.get(turn).and_then(|calls| calls.get(done)) {
            Some((name, input)) => {
                let model = request.json()["model"].as_str().unwrap().to_string();
                Reply::Sse {
                    body: Stream::new()
                        .tool(&format!("t{turn}_{done}"), name, input.clone())
                        .finish(&model, Some(1)),
                }
            }
            None => text(request, "done"),
        }
    })
}

fn last_results(gateway: &FakeGateway) -> Vec<Value> {
    gateway
        .messages()
        .into_iter()
        .rfind(|r| !r.is_title())
        .map(|r| results(&r))
        .unwrap_or_default()
}

fn model_calls(gateway: &FakeGateway) -> usize {
    gateway.messages().iter().filter(|r| !r.is_title()).count()
}

/// A git project with `files` committed.
fn project(env: &Env, files: &[(&str, &str)]) -> PathBuf {
    let project = env.home.project();
    env.home.write("proj/README.md", "hi");
    for (path, content) in files {
        env.home.write(&format!("proj/{path}"), content);
    }
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "init"]);
    project
}

fn db(env: &Env) -> Db {
    Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap()
}

/// Answers every permission ask with `answer(action, resources)`, recording the asks.
/// `(action, resources, save)` of each ask.
type Asked = Arc<Mutex<Vec<(String, Vec<String>, Vec<String>)>>>;
fn answering(
    runtime: &tokio::runtime::Runtime,
    core: &Core,
    answer: impl Fn(&str, &[String]) -> Answer + Send + 'static,
) -> Asked {
    let asked: Asked = Arc::default();
    let mut events = core.subscribe();
    let (core, record) = (core.clone(), asked.clone());
    runtime.spawn(async move {
        while let Ok(event) = events.recv().await {
            if let Event::PermissionAsked {
                request_id,
                action,
                resources,
                save,
                ..
            } = event
            {
                record
                    .lock()
                    .unwrap()
                    .push((action.clone(), resources.clone(), save));
                core.reply_permission(&request_id, answer(&action, &resources));
            }
        }
    });
    asked
}

/// A new session isolated in its own worktree and marked ready: `(session id, worktree top)`.
fn isolated(runtime: &tokio::runtime::Runtime, core: &Core) -> (String, PathBuf) {
    let (session, _) = core.open(&Target::New, false).unwrap();
    let row = runtime.block_on(core.isolate(&session.id)).unwrap();
    assert_eq!(row.state, WorktreeState::Preparing);
    assert!(
        core.set_worktree_state(&session.id, WorktreeState::Preparing, WorktreeState::Ready)
            .unwrap()
    );
    (session.id, PathBuf::from(row.directory))
}

/// A file's text with line endings as written: Git for Windows checks files out with CRLF
/// (`core.autocrlf`), which is not what these tests are about.
fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.replace("\r\n", "\n"))
}

/// The path as the file system spells it (8.3 short names and symlinks resolved), so two
/// spellings of one folder compare equal.
fn real(path: &Path) -> PathBuf {
    anysola_core::config::real_path(path)
}

fn user_messages(env: &Env, session: &str) -> Vec<String> {
    db(env)
        .messages(session)
        .unwrap()
        .into_iter()
        .filter(|m| m.kind == "user")
        .map(|m| m.id)
        .collect()
}

#[test]
fn a_session_gets_its_own_worktree_and_branch() {
    let gateway = turns(vec![]);
    let env = env(&gateway.url);
    let project = project(&env, &[]);
    env.home.write("proj/uncommitted.txt", "not carried");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (id, top) = isolated(&runtime, &core);
    let row = core.session_worktree(&id).unwrap().unwrap();
    assert!(row.branch.starts_with("anysola/"), "{}", row.branch);
    assert_eq!(row.base_branch.as_deref(), Some("main"));
    assert_eq!(row.base_commit, git(&project, &["rev-parse", "HEAD"]));
    assert_eq!(
        git(&top, &["rev-parse", "--abbrev-ref", "HEAD"]),
        row.branch
    );
    assert!(
        real(&top).starts_with(real(&env.home.home().join(".anysola/worktrees"))),
        "{top:?}"
    );
    assert!(top.join("README.md").exists());
    assert!(
        !top.join("uncommitted.txt").exists(),
        "main-folder edits stay out"
    );
    let session = core.session(&id).unwrap().unwrap();
    assert_eq!(real(Path::new(&session.directory)), real(&top));
    let checkout = core.checkout(&id).unwrap();
    assert!(checkout.pinned);
    assert_eq!(real(&checkout.root), real(&top));
    assert_eq!(real(&checkout.main), real(&project));
    // Its subagents' sessions work there too, pinned by their root.
    let child = core.create_child(&id, None).unwrap();
    assert_eq!(core.checkout(&child.id).unwrap(), checkout);
    // Listed with its main folder, and among the project's worktrees.
    let folder = project.display().to_string();
    let listed = db(&env).list_sessions(Some(&folder), None, 10).unwrap();
    assert!(listed.iter().any(|s| s.id == id));
    assert!(
        core.worktrees()
            .iter()
            .any(|w| real(Path::new(&w.directory)) == real(&top)
                && w.strategy.as_deref() == Some("session"))
    );
    // Twice, or after it ran: refused.
    assert!(runtime.block_on(core.isolate(&id)).is_err());
}

#[test]
fn isolating_needs_a_commit() {
    let gateway = turns(vec![]);
    let env = env(&gateway.url);
    git(&env.home.project(), &["init", "-q"]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let refused = runtime.block_on(core.isolate(&session.id)).unwrap_err();
    assert!(refused.to_string().contains("commit"), "{refused}");
    assert!(core.session_worktree(&session.id).unwrap().is_none());
}

/// Relative writes land in the worktree; an absolute path into the main folder is outside and
/// asks `external_directory` (rejected here), so the main folder stays as it was.
#[test]
fn a_pinned_session_writes_in_its_worktree_only() {
    let gateway = turns(vec![vec![
        ("write", json!({"path": "inside.txt", "content": "x"})),
        (
            "write",
            json!({"path": "~/proj/outside.txt", "content": "x"}),
        ),
    ]]);
    let env = env(&gateway.url);
    let project = project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let asked = answering(&runtime, &core, |action, _| match action {
        "external_directory" => Answer::Reject(Some("no".into())),
        _ => Answer::Once,
    });
    let (id, top) = isolated(&runtime, &core);
    // The desktop's Edits mode: work outside the project asks.
    core.set_permission_floor(&id, Floor::Commands).unwrap();
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(top.join("inside.txt").exists());
    assert!(!project.join("inside.txt").exists());
    assert!(
        !project.join("outside.txt").exists(),
        "{:?}",
        last_results(&gateway)
    );
    // Refused outright, not asked: no answer could open the main folder to a worktree session.
    let text = last_results(&gateway)[1]["content"].to_string();
    assert!(text.contains("works in its own worktree"), "{text}");
    let asked = asked.lock().unwrap();
    assert!(
        !asked.iter().any(|(a, _, _)| a == "external_directory"),
        "{asked:?}"
    );
    assert_eq!(git(&project, &["status", "--porcelain"]), "");
}

/// Auto mode never asks about outside folders, and a saved "always" for a parent folder allows
/// them; a pinned session still can't write or edit the main folder or another session's
/// worktree. Reading the main folder stays possible.
#[test]
fn a_pinned_session_never_writes_the_main_folder_in_auto_mode() {
    let paths = Arc::new(Mutex::new((String::new(), String::new())));
    let shared = paths.clone();
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let (main, other) = shared.lock().unwrap().clone();
        let calls = [
            (
                "write",
                json!({"path": format!("{main}/new.txt"), "content": "x"}),
            ),
            (
                "edit",
                json!({"path": format!("{main}/README.md"), "oldString": "hi", "newString": "HACKED"}),
            ),
            (
                "write",
                json!({"path": format!("{other}/planted.txt"), "content": "x"}),
            ),
            (
                "write",
                json!({"path": ".git", "content": "gitdir: /tmp/elsewhere\n"}),
            ),
            ("read", json!({"path": format!("{main}/README.md")})),
            ("write", json!({"path": "mine.txt", "content": "x"})),
        ];
        match calls.get(results(request).len()) {
            Some((name, input)) => {
                let model = request.json()["model"].as_str().unwrap().to_string();
                Reply::Sse {
                    body: Stream::new()
                        .tool(&format!("t{}", results(request).len()), name, input.clone())
                        .finish(&model, Some(1)),
                }
            }
            None => text(request, "done"),
        }
    });
    let env = env(&gateway.url);
    let home = env.home.home().display().to_string().replace('\\', "/");
    // A saved "always" for the whole home folder (a parent of the main folder and the worktrees).
    env.home.write(
        ".anysola/config.json",
        &format!(
            r#"{{"permissions": [{{"action": "external_directory", "resource": "{home}/*", "effect": "allow"}}]}}"#
        ),
    );
    let project = project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let asked = answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    let (_, other) = isolated(&runtime, &core);
    *paths.lock().unwrap() = (project.display().to_string(), other.display().to_string());
    // Auto: no floor, the default agent allows everything.
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let results = last_results(&gateway);
    assert_eq!(results.len(), 6, "{results:?}");
    for r in &results[..3] {
        let text = r["content"].to_string();
        assert!(text.contains("works in its own worktree"), "BREAK: {text}");
    }
    let text = results[3]["content"].to_string();
    assert!(text.contains("Git data"), "BREAK: {text}");
    assert!(
        read(&top.join(".git"))
            .is_some_and(|g| g.starts_with("gitdir: ") && !g.contains("elsewhere")),
        "BREAK: the worktree's .git was rewritten"
    );
    assert!(
        results[4]["content"].to_string().contains("hi"),
        "{results:?}"
    );
    assert!(top.join("mine.txt").exists());
    assert!(
        !project.join("new.txt").exists(),
        "BREAK: wrote the main folder"
    );
    assert_eq!(read(&project.join("README.md")).as_deref(), Some("hi"));
    assert!(
        !other.join("planted.txt").exists(),
        "BREAK: wrote another worktree"
    );
    assert_eq!(git(&project, &["status", "--porcelain"]), "");
    assert!(
        !asked.lock().unwrap().iter().any(|(a, _, _)| a == "edit"),
        "refused before any ask"
    );
}

/// A main-folder session in Auto mode: writing the repository's own data (`.git/hooks`,
/// `.git/config`) always asks; refused, nothing is written.
#[test]
fn writing_git_data_always_asks() {
    let gateway = turns(vec![vec![
        (
            "write",
            json!({"path": ".git/hooks/pre-commit", "content": "#!/bin/sh\nexit 0\n"}),
        ),
        (
            "edit",
            json!({"path": ".git/config", "oldString": "[core]", "newString": "[core]\n\tfsmonitor = x"}),
        ),
    ]]);
    let env = env(&gateway.url);
    let project = project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let asked = answering(&runtime, &core, |_, _| Answer::Reject(Some("no".into())));
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(
        !project.join(".git/hooks/pre-commit").exists(),
        "BREAK: wrote a git hook"
    );
    assert!(
        !read(&project.join(".git/config"))
            .unwrap()
            .contains("fsmonitor"),
        "BREAK: edited git config"
    );
    let asked = asked.lock().unwrap();
    assert_eq!(
        asked.iter().filter(|(a, _, _)| a == "edit").count(),
        2,
        "{asked:?}"
    );
}

/// Claude Code's documented hook form, `"$CLAUDE_PROJECT_DIR"/.claude/hooks/x.sh`, runs as
/// written: in a main-folder session from the main folder, in a worktree session from the
/// worktree (where the desktop copied the ignored script), with the worktree as project folder.
#[test]
fn the_documented_hook_form_runs_in_both_kinds_of_session() {
    // Each session's first turn writes one.txt.
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "one.txt", "content": "1"}),
    )]]);
    let env = env(&gateway.url);
    let log = env.home.home().join("hook.log");
    // The form as Claude Code documents it for a POSIX shell; falls back to PowerShell syntax
    // only on Windows machines without Git Bash.
    let hook_shell = anysola_core::tool::shell::hook_shell(None);
    let is_pwsh = matches!(
        anysola_core::tool::shell::shell_name(Some(&hook_shell)).as_str(),
        "pwsh" | "powershell"
    );
    let (command, name, script) = if is_pwsh {
        (
            r#"& "$env:CLAUDE_PROJECT_DIR/.claude/hooks/x.cmd""#.to_string(),
            "x.cmd",
            format!(
                "@echo %CD%^|%CLAUDE_PROJECT_DIR%>> \"{}\"\r\n@exit /b 0\r\n",
                log.display()
            ),
        )
    } else if cfg!(windows) {
        (
            r#"sh "$CLAUDE_PROJECT_DIR/.claude/hooks/x.sh""#.to_string(),
            "x.sh",
            format!(
                "#!/bin/sh\necho \"$(pwd -W)|$CLAUDE_PROJECT_DIR\" >> '{}'\nexit 0\n",
                log.display().to_string().replace('\\', "/")
            ),
        )
    } else {
        (
            r#""$CLAUDE_PROJECT_DIR"/.claude/hooks/x.sh"#.to_string(),
            "x.sh",
            format!(
                "#!/bin/sh\necho \"$PWD|$CLAUDE_PROJECT_DIR\" >> '{}'\nexit 0\n",
                log.display()
            ),
        )
    };
    let config = json!({"hooks": {"PreToolUse": [{"matcher": "Write", "hooks": [{"type": "command",
        "command": command}]}]}});
    env.home.write(".anysola/config.json", &config.to_string());
    let project = project(&env, &[(".gitignore", ".claude/hooks/\n")]);
    env.home
        .write(&format!("proj/.claude/hooks/{name}"), &script);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            project.join(".claude/hooks").join(name),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    // A main-folder session.
    let (main_session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&main_session.id, prompt("one"), TurnOptions::default()))
        .unwrap();
    assert!(
        project.join("one.txt").exists(),
        "the hook let the write through"
    );
    // A worktree session, its ignored script copied in as the desktop does.
    let (id, top) = isolated(&runtime, &core);
    std::fs::create_dir_all(top.join(".claude/hooks")).unwrap();
    std::fs::copy(
        project.join(".claude/hooks").join(name),
        top.join(".claude/hooks").join(name),
    )
    .unwrap();
    runtime
        .block_on(core.prompt(&id, prompt("two"), TurnOptions::default()))
        .unwrap();
    assert!(top.join("one.txt").exists());
    let lines: Vec<String> = read(&log)
        .unwrap()
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    let real = |p: &str| real(Path::new(p));
    let (pwd, dir) = lines[0].split_once('|').unwrap();
    assert_eq!(
        (real(pwd), real(dir)),
        (
            real(&project.display().to_string()),
            real(&project.display().to_string())
        )
    );
    let (pwd, dir) = lines[1].split_once('|').unwrap();
    assert_eq!(
        (real(pwd), real(dir)),
        (
            real(&top.display().to_string()),
            real(&top.display().to_string())
        )
    );
}

/// A hook that writes its data argument (`prettier --write "$CLAUDE_PROJECT_DIR/README.md"`)
/// from a worktree session writes the worktree's file, never the main folder's.
#[test]
fn a_formatting_hook_in_a_worktree_leaves_the_main_folder_alone() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a"}),
    )]]);
    let env = env(&gateway.url);
    // A formatter writing the file named by the project folder (POSIX when Git Bash is present).
    let hook_shell = anysola_core::tool::shell::hook_shell(None);
    let is_pwsh = matches!(
        anysola_core::tool::shell::shell_name(Some(&hook_shell)).as_str(),
        "pwsh" | "powershell"
    );
    let command = if is_pwsh {
        r#"Set-Content -LiteralPath "$env:CLAUDE_PROJECT_DIR/README.md" -Value formatted"#
    } else {
        r#"sh -c 'echo formatted > "$1"' prettier "$CLAUDE_PROJECT_DIR/README.md""#
    };
    let config = json!({"hooks": {"PostToolUse": [{"matcher": "Write", "hooks": [{"type": "command",
        "command": command}]}]}});
    env.home.write(".anysola/config.json", &config.to_string());
    let project = project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(read(&top.join("README.md")).as_deref(), Some("formatted\n"));
    assert_eq!(
        read(&project.join("README.md")).as_deref(),
        Some("hi"),
        "BREAK: wrote F"
    );
}

/// The user's root-relative denial names the main folder's file reached by absolute path from a
/// worktree, and the worktree's own copy, even with the outside folder approved.
#[test]
fn a_root_relative_denial_holds_for_the_main_folder_and_the_worktree() {
    let gateway = turns(vec![vec![
        ("read", json!({"path": "~/proj/secrets/key"})),
        ("read", json!({"path": "secrets/key"})),
    ]]);
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "read", "resource": "secrets/*", "effect": "deny"}]}"#,
    );
    project(&env, &[("secrets/key", "COMMITTED-SECRET")]);
    env.home.write("proj/secrets/key", "MAIN-SECRET");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    assert!(top.join("secrets/key").exists());
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let results = last_results(&gateway);
    assert_eq!(results.len(), 2, "{results:?}");
    for r in &results {
        let text = r["content"].to_string();
        assert!(!text.contains("SECRET"), "BREAK: {text}");
        assert!(text.contains("Permission denied"), "{text}");
    }
}

/// Another session's worktree is outside too, and a root-relative rule names its files.
#[test]
fn another_sessions_worktree_is_outside_and_its_files_keep_their_names() {
    let env_gateway = Arc::new(Mutex::new(String::new()));
    let target = env_gateway.clone();
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let path = target.lock().unwrap().clone();
        match results(request).len() {
            0 => {
                let model = request.json()["model"].as_str().unwrap().to_string();
                Reply::Sse {
                    body: Stream::new()
                        .tool("t0", "read", json!({"path": path}))
                        .finish(&model, Some(1)),
                }
            }
            _ => text(request, "done"),
        }
    });
    let env = env(&gateway.url);
    env.home.write(
        ".anysola/config.json",
        r#"{"permissions": [{"action": "read", "resource": ".env", "effect": "deny"}]}"#,
    );
    project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let asked = answering(&runtime, &core, |_, _| Answer::Once);
    let (first, _) = isolated(&runtime, &core);
    core.set_permission_floor(&first, Floor::Commands).unwrap();
    let (_, other) = isolated(&runtime, &core);
    std::fs::write(other.join(".env"), "OTHER-SECRET").unwrap();
    *env_gateway.lock().unwrap() = other.join(".env").display().to_string();
    runtime
        .block_on(core.prompt(&first, prompt("go"), TurnOptions::default()))
        .unwrap();
    let results = last_results(&gateway);
    let text = results[0]["content"].to_string();
    assert!(!text.contains("OTHER-SECRET"), "BREAK: {text}");
    let asked = asked.lock().unwrap();
    assert!(
        asked.iter().any(|(a, _, _)| a == "external_directory"),
        "{asked:?}"
    );
}

/// A pinned session cannot move out; nobody moves into a session's worktree or removes it.
#[test]
fn pinned_sessions_stay_and_their_worktrees_are_theirs() {
    let target = Arc::new(Mutex::new(String::new()));
    let shared = target.clone();
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let done = results(request).len();
        let other = shared.lock().unwrap().clone();
        let calls = [
            json!({"code": "return await tools.opencode.session_move({directory: '~/proj'})"}),
            json!({"code": format!("return await tools.opencode.session_move({{directory: '{other}'}})")}),
            json!({"code": format!("return await tools.opencode.worktree_remove({{directory: '{other}', force: true}})")}),
        ];
        match calls.get(done) {
            Some(input) => {
                let model = request.json()["model"].as_str().unwrap().to_string();
                Reply::Sse {
                    body: Stream::new()
                        .tool(&format!("t{done}"), "execute", input.clone())
                        .finish(&model, Some(1)),
                }
            }
            None => text(request, "done"),
        }
    });
    let env = env(&gateway.url);
    project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (pinned, top) = isolated(&runtime, &core);
    let (_, other) = isolated(&runtime, &core);
    *target.lock().unwrap() = other.display().to_string();
    // The pinned session tries all three.
    runtime
        .block_on(core.prompt(&pinned, prompt("go"), TurnOptions::default()))
        .unwrap();
    let results = last_results(&gateway);
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(
        results[0]["content"]
            .to_string()
            .contains("its own worktree"),
        "{results:?}"
    );
    assert_eq!(
        PathBuf::from(core.session(&pinned).unwrap().unwrap().directory),
        top
    );
    assert!(other.exists(), "BREAK: another session's worktree removed");
    // A main-folder session can't move into one either.
    let (plain, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&plain.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let results = last_results(&gateway);
    assert!(
        results[1]["content"].to_string().contains("Unable to move"),
        "{results:?}"
    );
    assert_eq!(
        core.session(&plain.id).unwrap().unwrap().directory,
        env.home.project().display().to_string()
    );
    assert!(other.exists());
    // Nor does the app's own remove.
    assert!(
        runtime
            .block_on(core.remove_worktree(&other, true))
            .is_err()
    );
    assert!(core.adopt_session(&pinned).is_err());
}

/// Review, rewind and restore of a worktree session act on the worktree's files.
#[test]
fn snapshots_are_the_worktrees() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "two\n"}),
    )]]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "one\n")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("two\n"));
    assert!(core.restores_files(&id));
    let changes = core.session_changes(&id).unwrap().unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0]["file"], "a.txt");
    let patch = core.last_turn_patch(&id).unwrap();
    assert!(patch.contains("+two"), "{patch}");
    // The step recorded the worktree's HEAD.
    let rows = db(&env).messages(&id).unwrap();
    let step = rows.iter().find(|r| r.kind == "assistant").unwrap();
    assert_eq!(
        step.data["snapshot"]["head"].as_str(),
        Some(git(&top, &["rev-parse", "HEAD"]).as_str())
    );
    let user = user_messages(&env, &id)[0].clone();
    core.revert(&id, &user).unwrap();
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("one\n"));
    core.unrevert(&id).unwrap();
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("two\n"));
    core.restore_file(&id, "a.txt").unwrap();
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("one\n"));
    assert_eq!(read(&project.join("a.txt")).as_deref(), Some("one\n"));
    assert_eq!(git(&project, &["status", "--porcelain"]), "");
}

/// A session the agent moved into a worktree it made snapshots that worktree, not the main
/// folder (before, its rewind and Review looked at the main folder).
#[test]
fn a_session_moved_into_a_worktree_snapshots_that_worktree() {
    let gateway = turns(vec![
        vec![
            ("write", json!({"path": "a.txt", "content": "a1"})),
            (
                "execute",
                json!({"code": "const w = await tools.opencode.worktree_create({name: 'side'}); return await tools.opencode.session_move({directory: w.directory})"}),
            ),
        ],
        vec![("write", json!({"path": "moved.txt", "content": "x"}))],
    ]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0"), ("keep.txt", "k")]);
    // The main folder's files differ from the worktree's, so their snapshot trees do too.
    env.home.write("proj/only-main.txt", "m");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (session, _) = core.open(&Target::New, false).unwrap();
    // The move's note to the model counts as the second prompt of the script.
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let side = PathBuf::from(core.session(&session.id).unwrap().unwrap().directory);
    assert_ne!(side, project, "{:?}", last_results(&gateway));
    assert!(side.join("moved.txt").exists());
    let changes = core.session_changes(&session.id).unwrap().unwrap();
    assert!(
        changes.iter().any(|c| c["file"] == "moved.txt"),
        "{changes:?}"
    );
    let user = user_messages(&env, &session.id)[0].clone();
    // The rewind reads trees of both stores, and deletes nothing it can't read.
    core.revert(&session.id, &user).unwrap();
    assert!(
        !side.join("moved.txt").exists(),
        "rewind acts on the worktree"
    );
    assert_eq!(read(&side.join("a.txt")).as_deref(), Some("a0"));
    assert_eq!(read(&side.join("keep.txt")).as_deref(), Some("k"));
    assert_eq!(read(&project.join("a.txt")).as_deref(), Some("a1"));
    assert!(project.join("only-main.txt").exists());
}

/// Nothing above the worktree is read as project instructions; the worktree's own AGENTS.md
/// and the user's above the main folder are.
#[test]
fn instructions_stop_at_the_worktree() {
    let gateway = turns(vec![vec![]]);
    let env = env(&gateway.url);
    project(&env, &[("AGENTS.md", "PROJECT-RULES")]);
    env.home.write("AGENTS.md", "HOME-RULES");
    env.home
        .write(".anysola/worktrees/AGENTS.md", "PLANTED-ABOVE");
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (id, top) = isolated(&runtime, &core);
    std::fs::write(top.parent().unwrap().join("AGENTS.md"), "PLANTED-BESIDE").unwrap();
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let request = gateway
        .messages()
        .into_iter()
        .find(|r| !r.is_title())
        .unwrap();
    let body = request.body.clone();
    assert!(body.contains("PROJECT-RULES"), "the worktree's own");
    assert!(
        body.contains("HOME-RULES"),
        "the user's above the main folder"
    );
    assert!(!body.contains("PLANTED"), "BREAK: read above the worktree");
}

/// While the worktree is made a prompt waits; ready, it runs. Other states refuse.
#[test]
fn prompts_wait_while_preparing_and_refuse_when_gone() {
    let gateway = turns(vec![vec![], vec![]]);
    let env = env(&gateway.url);
    project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let row = runtime.block_on(core.isolate(&session.id)).unwrap();
    let outcome = runtime
        .block_on(core.prompt(&session.id, prompt("first"), TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, anysola_core::session::Outcome::Succeeded);
    assert_eq!(model_calls(&gateway), 0, "no turn while preparing");
    assert_eq!(core.pending(&session.id).unwrap().len(), 1);
    let mut events = core.subscribe();
    assert!(
        core.set_worktree_state(&session.id, WorktreeState::Preparing, WorktreeState::Ready)
            .unwrap()
    );
    // The same change twice: the second finds it done.
    assert!(
        !core
            .set_worktree_state(&session.id, WorktreeState::Preparing, WorktreeState::Ready)
            .unwrap()
    );
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if let Ok(Event::Succeeded { session_id }) = events.recv().await
                    && session_id == session.id
                {
                    return;
                }
            }
        })
        .await
        .expect("the waiting prompt runs once ready")
    });
    assert_eq!(model_calls(&gateway), 1);
    // Gone: refused with a plain sentence.
    std::fs::remove_dir_all(&row.directory).unwrap();
    assert!(
        core.set_worktree_error(
            &session.id,
            WorktreeState::Ready,
            WorktreeState::Missing,
            ""
        )
        .unwrap()
    );
    let refused = runtime
        .block_on(core.prompt(&session.id, prompt("again"), TurnOptions::default()))
        .unwrap_err();
    assert!(refused.to_string().contains("folder is gone"), "{refused}");
}

/// "Work in the main folder instead": the worktree goes and the waiting prompt runs in the
/// main folder.
#[test]
fn unisolate_moves_a_waiting_session_back() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "here.txt", "content": "x"}),
    )]]);
    let env = env(&gateway.url);
    let project = project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (session, _) = core.open(&Target::New, false).unwrap();
    let row = runtime.block_on(core.isolate(&session.id)).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    let mut events = core.subscribe();
    let moved = runtime.block_on(core.unisolate(&session.id)).unwrap();
    assert_eq!(real(Path::new(&moved.directory)), real(&project));
    assert!(!Path::new(&row.directory).exists());
    assert!(!git_ok(
        &project,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", row.branch)
        ]
    ));
    assert!(core.session_worktree(&session.id).unwrap().is_none());
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if let Ok(Event::Succeeded { .. }) = events.recv().await {
                    return;
                }
            }
        })
        .await
        .expect("the waiting prompt runs in the main folder")
    });
    assert!(project.join("here.txt").exists());
}

/// Forking before turn 2: turn 1's edit is there, turn 2's is not, a file deleted later is back,
/// and the main folder's later commit is neither in the fork nor undone by it.
#[test]
fn a_fork_puts_the_files_back_as_at_the_fork_point() {
    let gateway = turns(vec![
        vec![("write", json!({"path": "a.txt", "content": "a1"}))],
        vec![("write", json!({"path": "b.txt", "content": "b2"}))],
    ]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0"), ("b.txt", "b0"), ("c.txt", "c0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["one", "two"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    assert_eq!(read(&project.join("b.txt")).as_deref(), Some("b2"));
    std::fs::remove_file(project.join("c.txt")).unwrap();
    // The main folder moves on after the fork point.
    env.home.write("proj/later.txt", "later");
    git(&project, &["add", "later.txt"]);
    git(&project, &["commit", "-qm", "later"]);
    let second = user_messages(&env, &session.id)[1].clone();
    let (fork, restored) = runtime
        .block_on(core.fork_into_worktree(&session.id, Some(&second)))
        .unwrap();
    assert!(restored.exact, "{restored:?}");
    assert_eq!(restored.fallback, None);
    let top = PathBuf::from(core.session_worktree(&fork.id).unwrap().unwrap().directory);
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
    assert_eq!(read(&top.join("b.txt")).as_deref(), Some("b0"));
    assert_eq!(read(&top.join("c.txt")).as_deref(), Some("c0"));
    assert!(!top.join("later.txt").exists());
    let status = git(&top, &["status", "--porcelain"]);
    assert_eq!(status, "M a.txt", "only the session's own edit shows");
    assert_eq!(fork.fork_session_id.as_deref(), Some(session.id.as_str()));
    let row = core.session_worktree(&fork.id).unwrap().unwrap();
    assert_eq!(row.state, WorktreeState::Preparing);
    assert_eq!(row.restored.as_ref().unwrap()["exact"], true);
    // The fork's store reads its copied history's trees.
    core.set_worktree_state(&fork.id, WorktreeState::Preparing, WorktreeState::Ready)
        .unwrap();
    let changes = core.session_changes(&fork.id).unwrap().unwrap();
    assert!(changes.iter().any(|c| c["file"] == "a.txt"), "{changes:?}");
}

/// A whole fork has the files as they are now; a large file never captured is kept as on the
/// commit and reported.
#[test]
fn a_whole_fork_has_the_files_now_and_reports_large_ones() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a1"}),
    )]]);
    let env = env(&gateway.url);
    let project = env.home.project();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("big.bin"), vec![1u8; 3 * 1024 * 1024]).unwrap();
    let project = self::project(&env, &[("a.txt", "a0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (session, _) = core.open(&Target::New, false).unwrap();
    runtime
        .block_on(core.prompt(&session.id, prompt("go"), TurnOptions::default()))
        .unwrap();
    env.home.write("proj/b.txt", "user's own");
    let (fork, restored) = runtime
        .block_on(core.fork_into_worktree(&session.id, None))
        .unwrap();
    let top = PathBuf::from(core.session_worktree(&fork.id).unwrap().unwrap().directory);
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
    assert_eq!(read(&top.join("b.txt")).as_deref(), Some("user's own"));
    assert_eq!(restored.kept, vec!["big.bin".to_string()]);
    assert!(!restored.exact);
    assert_eq!(restored.fallback, None);
    assert_eq!(
        std::fs::metadata(top.join("big.bin")).unwrap().len(),
        3 * 1024 * 1024
    );
    assert!(project.join("big.bin").exists());
}

/// With `snapshots: false` nothing records the fork point: the fork has the files as now.
#[test]
fn without_snapshots_a_fork_has_the_files_now() {
    let gateway = turns(vec![
        vec![("write", json!({"path": "a.txt", "content": "a1"}))],
        vec![],
    ]);
    let env = env(&gateway.url);
    env.home
        .write(".anysola/config.json", r#"{"snapshots": false}"#);
    project(&env, &[("a.txt", "a0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["one", "two"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    let first = user_messages(&env, &session.id)[0].clone();
    let second = user_messages(&env, &session.id)[1].clone();
    let _ = first;
    let (fork, restored) = runtime
        .block_on(core.fork_into_worktree(&session.id, Some(&second)))
        .unwrap();
    assert_eq!(restored.fallback, Some(Fallback::Now));
    assert!(!restored.exact);
    let top = PathBuf::from(core.session_worktree(&fork.id).unwrap().unwrap().directory);
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
}

/// Steps recorded before heads were: only the session's own later edits are undone; commits
/// made outside it since stay.
#[test]
fn a_fork_of_old_history_undoes_only_the_sessions_edits() {
    let gateway = turns(vec![
        vec![("write", json!({"path": "a.txt", "content": "a1"}))],
        vec![("write", json!({"path": "b.txt", "content": "b2"}))],
    ]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0"), ("b.txt", "b0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["one", "two"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    // As 1.1.8 stored them.
    let db = db(&env);
    for row in db.messages(&session.id).unwrap() {
        if row.data["snapshot"]["head"].is_string() {
            let mut data = row.data.clone();
            data["snapshot"].as_object_mut().unwrap().remove("head");
            db.update_message(&row.id, &data).unwrap();
        }
    }
    env.home.write("proj/later.txt", "later");
    git(&project, &["add", "later.txt"]);
    git(&project, &["commit", "-qm", "later"]);
    // The user's own uncommitted work in the main folder: not the session's, not in the fork.
    env.home.write("proj/README.md", "user edit");
    env.home.write("proj/mine.txt", "user file");
    let second = user_messages(&env, &session.id)[1].clone();
    let (fork, restored) = runtime
        .block_on(core.fork_into_worktree(&session.id, Some(&second)))
        .unwrap();
    assert_eq!(restored.fallback, Some(Fallback::Edits));
    assert!(!restored.exact);
    let top = PathBuf::from(core.session_worktree(&fork.id).unwrap().unwrap().directory);
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
    assert_eq!(read(&top.join("b.txt")).as_deref(), Some("b0"));
    assert_eq!(read(&top.join("later.txt")).as_deref(), Some("later"));
    assert_eq!(read(&top.join("README.md")).as_deref(), Some("hi"));
    assert!(!top.join("mine.txt").exists());
}

/// A session whose worktree was discarded still forks exactly: from its kept snapshot store,
/// on a commit only its deleted branch had (kept by the hidden ref).
#[test]
fn a_discarded_session_forks_from_its_kept_history() {
    let gateway = turns(vec![
        vec![("write", json!({"path": "a.txt", "content": "a1"}))],
        vec![("write", json!({"path": "b.txt", "content": "b2"}))],
    ]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0"), ("b.txt", "b0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("one"), TurnOptions::default()))
        .unwrap();
    // A commit on the session's branch between the turns.
    git(&top, &["commit", "-qam", "on the branch"]);
    let branch_commit = git(&top, &["rev-parse", "HEAD"]);
    runtime
        .block_on(core.prompt(&id, prompt("two"), TurnOptions::default()))
        .unwrap();
    let row = runtime
        .block_on(core.release_worktree(&id, Release::Discarded))
        .unwrap();
    assert_eq!(row.state, WorktreeState::Discarded);
    assert!(!top.exists());
    assert!(!git_ok(
        &project,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", row.branch)
        ]
    ));
    let pinned = git(
        &project,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            &format!("refs/anysola/sessions/{id}/"),
        ],
    );
    assert!(!pinned.is_empty(), "the branch tip is kept");
    // Discarded: no more turns.
    assert!(
        runtime
            .block_on(core.prompt(&id, prompt("three"), TurnOptions::default()))
            .is_err()
    );
    let second = user_messages(&env, &id)[1].clone();
    let (fork, restored) = runtime
        .block_on(core.fork_into_worktree(&id, Some(&second)))
        .unwrap();
    assert!(restored.exact, "{restored:?}");
    let fork_top = PathBuf::from(core.session_worktree(&fork.id).unwrap().unwrap().directory);
    assert_eq!(git(&fork_top, &["rev-parse", "HEAD"]), branch_commit);
    assert_eq!(read(&fork_top.join("a.txt")).as_deref(), Some("a1"));
    assert_eq!(read(&fork_top.join("b.txt")).as_deref(), Some("b0"));
    assert_eq!(git(&fork_top, &["status", "--porcelain"]), "");
}

/// A project opened in a subfolder: the session works in the worktree's same subfolder, and a
/// fork touches only that subfolder's files (the rest is as on the commit).
#[test]
fn a_subfolder_project_works_and_forks_in_its_subfolder() {
    let gateway = turns(vec![
        vec![
            ("write", json!({"path": "in.txt", "content": "in1"})),
            ("write", json!({"path": "../out.txt", "content": "out1"})),
        ],
        vec![],
    ]);
    let env = env(&gateway.url);
    let project = project(&env, &[("sub/in.txt", "in0"), ("out.txt", "out0")]);
    let runtime = runtime();
    let core = runtime.block_on(Core::start(&project.join("sub"))).unwrap();
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    assert_eq!(
        PathBuf::from(core.session(&id).unwrap().unwrap().directory),
        top.join("sub")
    );
    for text in ["one", "two"] {
        runtime
            .block_on(core.prompt(&id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    assert_eq!(read(&top.join("sub/in.txt")).as_deref(), Some("in1"));
    assert_eq!(read(&top.join("out.txt")).as_deref(), Some("out1"));
    let second = user_messages(&env, &id)[1].clone();
    let (fork, _) = runtime
        .block_on(core.fork_into_worktree(&id, Some(&second)))
        .unwrap();
    let fork_top = PathBuf::from(core.session_worktree(&fork.id).unwrap().unwrap().directory);
    assert_eq!(read(&fork_top.join("sub/in.txt")).as_deref(), Some("in1"));
    assert_eq!(read(&fork_top.join("out.txt")).as_deref(), Some("out0"));
}

/// After an apply the session goes on from the applied commit: the branch and worktree moved
/// there, the old tip kept, `ready` again.
#[test]
fn after_apply_the_session_continues_from_the_applied_commit() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a1"}),
    )]]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    git(&top, &["commit", "-qam", "wip"]);
    let old_tip = git(&top, &["rev-parse", "HEAD"]);
    assert!(
        core.set_worktree_state(&id, WorktreeState::Ready, WorktreeState::Applying)
            .unwrap()
    );
    // A second click finds it applying.
    assert!(
        !core
            .set_worktree_state(&id, WorktreeState::Ready, WorktreeState::Applying)
            .unwrap()
    );
    let from = git(&project, &["rev-parse", "HEAD"]);
    // The shell's apply, in short.
    std::fs::write(project.join("a.txt"), "a1").unwrap();
    git(&project, &["commit", "-qam", "applied"]);
    let applied = git(&project, &["rev-parse", "HEAD"]);
    assert!(
        core.mark_apply(&id, &from, &git(&project, &["rev-parse", "HEAD^{tree}"]))
            .unwrap()
    );
    let row = core.continue_after_apply(&id, &applied).unwrap();
    assert_eq!(row.state, WorktreeState::Ready);
    assert_eq!(row.base_commit, applied);
    assert_eq!(row.applied_commit.as_deref(), Some(applied.as_str()));
    assert_eq!(git(&top, &["rev-parse", "HEAD"]), applied);
    assert_eq!(
        git(&top, &["rev-parse", "--abbrev-ref", "HEAD"]),
        row.branch
    );
    assert_eq!(git(&top, &["status", "--porcelain"]), "");
    let pinned = git(
        &project,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            &format!("refs/anysola/sessions/{id}/"),
        ],
    );
    assert!(pinned.contains(&old_tip), "{pinned}");
    // It goes on.
    assert!(core.continue_after_apply(&id, &applied).is_err());
}

/// Recreate a worktree deleted by hand: on its branch, with the last snapshot's files back.
#[test]
fn a_missing_worktree_is_recreated_with_its_files() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a1"}),
    )]]);
    let env = env(&gateway.url);
    project(&env, &[("a.txt", "a0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    std::fs::remove_dir_all(&top).unwrap();
    let cleaned = anysola_core::api::cleanup_worktrees(&db(&env));
    assert_eq!(cleaned.len(), 1, "{cleaned:?}");
    assert_eq!(cleaned[0].state, Some(WorktreeState::Missing));
    let row = runtime.block_on(core.recreate_worktree(&id)).unwrap();
    assert_eq!(row.state, WorktreeState::Preparing);
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
}

/// Recreating after the branch moved on (main brought in) keeps main's commits: the last step's
/// files are not laid over a newer branch tip.
#[test]
fn a_recreate_after_main_came_in_keeps_mains_commits() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a1"}),
    )]]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0"), ("b.txt", "b0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    // Main moves on; the session brings it in (its work committed first, as the app does).
    std::fs::write(project.join("b.txt"), "b-main").unwrap();
    git(&project, &["commit", "-qam", "main edits b"]);
    git(&top, &["add", "-A"]);
    git(&top, &["commit", "-qm", "session work"]);
    git(&top, &["merge", "-q", "--no-edit", "main"]);
    std::fs::remove_dir_all(&top).unwrap();
    anysola_core::api::cleanup_worktrees(&db(&env));
    runtime.block_on(core.recreate_worktree(&id)).unwrap();
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
    assert_eq!(
        read(&top.join("b.txt")).as_deref(),
        Some("b-main"),
        "BREAK: main's commit was undone"
    );
}

/// A missing folder that comes back is used as it is (never deleted); something else in its
/// place is refused and left alone.
#[test]
fn a_folder_that_came_back_is_reused_and_a_stranger_is_refused() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a1"}),
    )]]);
    let env = env(&gateway.url);
    project(&env, &[("a.txt", "a0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    std::fs::write(top.join("later.txt"), "uncommitted, after the last step").unwrap();
    let away = top.with_extension("away");
    std::fs::rename(&top, &away).unwrap();
    // Found at the next turn's start, not only at the next app start.
    assert!(
        runtime
            .block_on(core.prompt(&id, prompt("again"), TurnOptions::default()))
            .is_err()
    );
    assert_eq!(
        core.session_worktree(&id).unwrap().unwrap().state,
        WorktreeState::Missing
    );
    std::fs::rename(&away, &top).unwrap();
    let row = runtime.block_on(core.recreate_worktree(&id)).unwrap();
    assert_eq!(row.state, WorktreeState::Ready);
    assert_eq!(
        read(&top.join("later.txt")).as_deref(),
        Some("uncommitted, after the last step"),
        "BREAK: the folder that came back was replaced"
    );
    // Something else in its place.
    let other = top.with_extension("other");
    std::fs::rename(&top, &other).unwrap();
    assert!(
        runtime
            .block_on(core.prompt(&id, prompt("third"), TurnOptions::default()))
            .is_err()
    );
    std::fs::create_dir_all(&top).unwrap();
    std::fs::write(top.join("mine.txt"), "not a worktree").unwrap();
    let refused = runtime.block_on(core.recreate_worktree(&id));
    assert!(refused.is_err());
    assert_eq!(
        read(&top.join("mine.txt")).as_deref(),
        Some("not a worktree")
    );
}

/// A force-added file the ignore rules match is never captured; recreating keeps it.
#[test]
fn a_recreate_keeps_force_added_ignored_files() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a1"}),
    )]]);
    let env = env(&gateway.url);
    project(&env, &[("a.txt", "a0"), (".gitignore", "cache\n")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    std::fs::write(top.join("cache"), "kept").unwrap();
    git(&top, &["add", "-f", "cache"]);
    git(&top, &["commit", "-qm", "cache"]);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    std::fs::remove_dir_all(&top).unwrap();
    anysola_core::api::cleanup_worktrees(&db(&env));
    runtime.block_on(core.recreate_worktree(&id)).unwrap();
    assert_eq!(
        read(&top.join("cache")).as_deref(),
        Some("kept"),
        "BREAK: deleted"
    );
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
}

/// Start-up cleanup: a cut-off preparation fails and its folder goes; a deleted session's
/// worktree, branch and row go; an apply cut off after its commit finishes, one cut off before
/// it leaves the main folder alone.
/// An apply cut off by a crash after its commit puts the main-folder edits it set aside back; one
/// cut off before it leaves them in the stash and says where.
#[test]
fn a_crash_mid_apply_brings_the_stashed_edits_back_or_names_them() {
    let gateway = turns(vec![]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0"), ("b.txt", "b0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let stash = |id: &str, file: &str| {
        let message = anysola_core::api::apply_stash_message(id);
        git(
            &project,
            &["stash", "push", "-q", "-m", &message, "--", file],
        );
    };
    // Finished: the edit to b.txt was set aside, the commit landed, then the crash.
    let (done, _) = isolated(&runtime, &core);
    core.set_worktree_state(&done, WorktreeState::Ready, WorktreeState::Applying)
        .unwrap();
    std::fs::write(project.join("b.txt"), "user's b").unwrap();
    stash(&done, "b.txt");
    let from = git(&project, &["rev-parse", "HEAD"]);
    std::fs::write(project.join("a.txt"), "applied").unwrap();
    git(&project, &["commit", "-qam", "applied"]);
    core.mark_apply(&done, &from, &git(&project, &["rev-parse", "HEAD^{tree}"]))
        .unwrap();
    drop(core);
    anysola_core::api::cleanup_worktrees(&db(&env));
    assert_eq!(read(&project.join("b.txt")).as_deref(), Some("user's b"));
    assert_eq!(git(&project, &["stash", "list"]), "");

    // Cut off before its commit: the stash stays, and the notice names it.
    let core = runtime.block_on(common::core(&env));
    let (half, _) = isolated(&runtime, &core);
    core.set_worktree_state(&half, WorktreeState::Ready, WorktreeState::Applying)
        .unwrap();
    std::fs::write(project.join("b.txt"), "user's b again").unwrap();
    stash(&half, "b.txt");
    let head = git(&project, &["rev-parse", "HEAD"]);
    core.mark_apply(&half, &head, "0000000000000000000000000000000000000000")
        .unwrap();
    drop(core);
    anysola_core::api::cleanup_worktrees(&db(&env));
    let row = db(&env).session_worktree(&half).unwrap().unwrap();
    let error = row.error.unwrap();
    assert!(error.contains("stash@{0}"), "{error}");
    assert!(git(&project, &["stash", "list"]).contains(&half));
}

#[test]
fn cleanup_settles_every_state() {
    let gateway = turns(vec![]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    // Cut off while preparing.
    let (cut, _) = core.open(&Target::New, false).unwrap();
    let cut_row = runtime.block_on(core.isolate(&cut.id)).unwrap();
    // Deleted session.
    let (gone, gone_top) = isolated(&runtime, &core);
    let gone_row = core.session_worktree(&gone).unwrap().unwrap();
    core.delete_session(&gone).unwrap();
    // Apply finished before the crash.
    let (done, done_top) = isolated(&runtime, &core);
    core.set_worktree_state(&done, WorktreeState::Ready, WorktreeState::Applying)
        .unwrap();
    let from = git(&project, &["rev-parse", "HEAD"]);
    std::fs::write(project.join("a.txt"), "applied").unwrap();
    git(&project, &["commit", "-qam", "applied"]);
    core.mark_apply(&done, &from, &git(&project, &["rev-parse", "HEAD^{tree}"]))
        .unwrap();
    let applied = git(&project, &["rev-parse", "HEAD"]);
    // Apply cut off before its commit: the main folder's HEAD is not on it.
    let (half, _) = isolated(&runtime, &core);
    core.set_worktree_state(&half, WorktreeState::Ready, WorktreeState::Applying)
        .unwrap();
    core.mark_apply(&half, &applied, "0000000000000000000000000000000000000000")
        .unwrap();
    std::fs::write(project.join("a.txt"), "user's edit").unwrap();
    drop(core);

    let cleaned = anysola_core::api::cleanup_worktrees(&db(&env));
    let state = |id: &str| cleaned.iter().find(|c| c.session_id == id).map(|c| c.state);
    assert_eq!(state(&cut.id), Some(Some(WorktreeState::Failed)));
    assert!(!Path::new(&cut_row.directory).exists());
    assert_eq!(state(&gone), Some(None));
    assert!(!gone_top.exists());
    assert!(!git_ok(
        &project,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", gone_row.branch)
        ]
    ));
    assert!(db(&env).session_worktree(&gone).unwrap().is_none());
    assert_eq!(state(&done), Some(Some(WorktreeState::Ready)));
    assert_eq!(git(&done_top, &["rev-parse", "HEAD"]), applied);
    let half_row = db(&env).session_worktree(&half).unwrap().unwrap();
    assert_eq!(half_row.state, WorktreeState::Ready);
    assert!(half_row.error.unwrap().contains("cut off"));
    assert_eq!(read(&project.join("a.txt")).as_deref(), Some("user's edit"));
    // Nothing left to do the second time.
    assert!(anysola_core::api::cleanup_worktrees(&db(&env)).is_empty());
}

/// A project opened under another spelling of its folder (a Windows 8.3 short name; here a
/// symlink) isolates, and its worktree's paths compare as the same folder.
#[cfg(unix)]
#[test]
fn a_project_opened_under_another_spelling_isolates() {
    let gateway = turns(vec![vec![(
        "write",
        json!({"path": "a.txt", "content": "a1"}),
    )]]);
    let env = env(&gateway.url);
    let project = project(&env, &[]);
    let alias = env.home.home().join("alias");
    std::os::unix::fs::symlink(&project, &alias).unwrap();
    let runtime = runtime();
    let core = runtime.block_on(Core::start(&alias)).unwrap();
    answering(&runtime, &core, |_, _| Answer::Once);
    let (id, top) = isolated(&runtime, &core);
    runtime
        .block_on(core.prompt(&id, prompt("go"), TurnOptions::default()))
        .unwrap();
    assert!(top.join("a.txt").exists());
    assert!(!project.join("a.txt").exists());
    let changes = core.session_changes(&id).unwrap().unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
}

/// A fork point whose snapshot is gone (pruned) falls back to the files as they are now; an
/// unreadable tree is never taken for "the file was not there".
#[test]
fn a_fork_point_that_is_gone_falls_back_to_now() {
    let gateway = turns(vec![
        vec![("write", json!({"path": "a.txt", "content": "a1"}))],
        vec![("write", json!({"path": "b.txt", "content": "b2"}))],
    ]);
    let env = env(&gateway.url);
    project(&env, &[("a.txt", "a0"), ("b.txt", "b0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    answering(&runtime, &core, |_, _| Answer::Once);
    let (session, _) = core.open(&Target::New, false).unwrap();
    for text in ["one", "two"] {
        runtime
            .block_on(core.prompt(&session.id, prompt(text), TurnOptions::default()))
            .unwrap();
    }
    let gone = "0123456789abcdef0123456789abcdef01234567";
    let db = db(&env);
    // With the recorded head (exact path), then as old history (the "edits" path).
    for strip_head in [false, true] {
        for row in db.messages(&session.id).unwrap() {
            if row.data["snapshot"].is_object() {
                let mut data = row.data.clone();
                data["snapshot"]["start"] = json!(gone);
                data["snapshot"]["end"] = json!(gone);
                if strip_head {
                    data["snapshot"].as_object_mut().unwrap().remove("head");
                }
                db.update_message(&row.id, &data).unwrap();
            }
        }
        let second = user_messages(&env, &session.id)[1].clone();
        let (fork, restored) = runtime
            .block_on(core.fork_into_worktree(&session.id, Some(&second)))
            .unwrap();
        assert_eq!(restored.fallback, Some(Fallback::Now), "{restored:?}");
        assert!(!restored.exact);
        let top = PathBuf::from(core.session_worktree(&fork.id).unwrap().unwrap().directory);
        assert_eq!(read(&top.join("a.txt")).as_deref(), Some("a1"));
        assert_eq!(
            read(&top.join("b.txt")).as_deref(),
            Some("b2"),
            "never deleted"
        );
    }
    // A rewind onto a tree that is gone refuses instead of deleting.
    let first = user_messages(&env, &session.id)[0].clone();
    assert!(core.revert(&session.id, &first).is_err());
    assert!(env.home.project().join("b.txt").exists());
}

/// A worktree being made again (after it went missing) that the app loses mid-way keeps its
/// branch and the session's hidden refs: the work committed there stays reachable.
#[test]
fn cleanup_keeps_a_branch_that_holds_work() {
    let gateway = turns(vec![]);
    let env = env(&gateway.url);
    let project = project(&env, &[("a.txt", "a0")]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (id, top) = isolated(&runtime, &core);
    std::fs::write(top.join("a.txt"), "work").unwrap();
    git(&top, &["commit", "-qam", "work"]);
    let work = git(&top, &["rev-parse", "HEAD"]);
    let branch = core.session_worktree(&id).unwrap().unwrap().branch;
    std::fs::remove_dir_all(&top).unwrap();
    anysola_core::api::cleanup_worktrees(&db(&env));
    runtime.block_on(core.recreate_worktree(&id)).unwrap();
    // The app dies while it is `preparing` again.
    drop(core);
    let cleaned = anysola_core::api::cleanup_worktrees(&db(&env));
    assert_eq!(cleaned[0].state, Some(WorktreeState::Failed));
    assert!(!top.exists());
    assert_eq!(
        git(&project, &["rev-parse", &format!("refs/heads/{branch}")]),
        work
    );
    let core = runtime.block_on(common::core(&env));
    let row = runtime.block_on(core.recreate_worktree(&id)).unwrap();
    assert_eq!(row.state, WorktreeState::Preparing);
    assert_eq!(read(&top.join("a.txt")).as_deref(), Some("work"));
}

/// A worktree folder that can't be removed yet is left over and removed at the next start.
#[cfg(unix)]
#[test]
fn a_leftover_is_removed_at_the_next_start() {
    use std::os::unix::fs::PermissionsExt;
    let gateway = turns(vec![]);
    let env = env(&gateway.url);
    let project = project(&env, &[]);
    let runtime = runtime();
    let core = runtime.block_on(common::core(&env));
    let (id, top) = isolated(&runtime, &core);
    let parent = top.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).unwrap();
    let row = runtime
        .block_on(core.release_worktree(&id, Release::Discarded))
        .unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !row.leftover {
        // A user the folder's permissions don't stop (root): nothing to retry.
        assert!(!top.exists());
        return;
    }
    assert!(top.exists());
    let cleaned = anysola_core::api::cleanup_worktrees(&db(&env));
    assert_eq!(cleaned.len(), 1, "{cleaned:?}");
    assert!(!top.exists());
    assert!(!git_ok(
        &project,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{}", row.branch)
        ]
    ));
    assert!(!db(&env).session_worktree(&id).unwrap().unwrap().leftover);
}
