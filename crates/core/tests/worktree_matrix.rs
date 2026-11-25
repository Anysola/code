//! The recreate-and-apply matrix: whatever state a session's worktree was in when its folder
//! went, recreating it and applying never undoes the main folder's own changes.
//!
//! Cells: bringing the base in {none, conflicted, resolved but not committed, committed} × {a turn
//! ran after it, not} × {the agent committed in the last step, not}. Each cell: the folder is
//! deleted, cleanup marks it missing, `recreate_worktree` makes it again, then the session is
//! applied. Every path the main folder changed that the session didn't touch must keep the main
//! folder's content on its branch after the apply, or the apply must refuse.
//!
//! The apply here is the desktop's (`desktop/src-tauri/src/worktree.rs`: `diff_base`, `plan`,
//! `apply`) written out with git: base = `MERGE_HEAD` while a merge is open (concluded first, as
//! the desktop does), else where `HEAD` and the recorded base meet when the base isn't on the
//! branch, else where `HEAD` and the base branch meet; files with conflict markers refuse; then
//! `merge-tree --merge-base` against the main folder's `HEAD`, `read-tree -m -u`, `commit`.
//! Change it with that file.

mod common;

use std::path::{Path, PathBuf};

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::session::TurnOptions;
use anysola_core::storage::{Db, SessionWorktreeRow, WorktreeState};
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn git_out(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_out(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn ok(dir: &Path, args: &[&str]) -> bool {
    git_out(dir, args).status.success()
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|t| t.replace("\r\n", "\n"))
}

fn results(request: &Recorded) -> usize {
    request.json()["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == "tool_result")
        .count()
}

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

/// Each turn's tool calls, one per step, then "done".
fn turns(script: Vec<Vec<(&'static str, Value)>>) -> FakeGateway {
    FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        let turn = prompts(request).saturating_sub(1);
        let before: usize = script.iter().take(turn).map(Vec::len).sum();
        let done = results(request).saturating_sub(before);
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

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Bring {
    None,
    Conflicted,
    Resolved,
    Committed,
}

/// The worktree's files as a tree, its index untouched.
fn files_tree(w: &Path) -> String {
    let index = std::env::temp_dir().join(format!("matrix-{}", anysola_core::ids::part()));
    let index_name = index.display().to_string();
    let with_index = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(w)
            .env("GIT_INDEX_FILE", &index_name)
            .output()
            .unwrap()
    };
    assert!(with_index(&["read-tree", "HEAD"]).status.success());
    assert!(with_index(&["add", "-A", "--", "."]).status.success());
    let tree = String::from_utf8_lossy(&with_index(&["write-tree"]).stdout)
        .trim()
        .to_string();
    let _ = std::fs::remove_file(&index);
    tree
}

/// What Review measures the session's changes from: the desktop's `diff_base` written out
/// (`MERGE_HEAD` while a merge is open, else where `HEAD` and the recorded base meet when the base
/// isn't on the branch, else where `HEAD` and the base branch meet).
fn review_base(w: &Path, row: &SessionWorktreeRow) -> String {
    if ok(w, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]) {
        return git(w, &["rev-parse", "MERGE_HEAD"]);
    }
    if !ok(
        w,
        &["merge-base", "--is-ancestor", &row.base_commit, "HEAD"],
    ) {
        return git(w, &["merge-base", "HEAD", &row.base_commit]);
    }
    git(w, &["merge-base", "HEAD", "refs/heads/main"])
}

/// The desktop's apply, written out (see the file's comment). True when it applied.
fn apply(main: &Path, row: &SessionWorktreeRow) -> bool {
    let w = Path::new(&row.directory);
    // A merge left open is concluded first, as the desktop's apply does.
    if ok(w, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]) {
        git(w, &["add", "-A"]);
        if !ok(
            w,
            &["commit", "-q", "--no-verify", "-m", "Bring the base in"],
        ) {
            return false;
        }
    }
    let tree = files_tree(w);
    let base = if !ok(
        w,
        &["merge-base", "--is-ancestor", &row.base_commit, "HEAD"],
    ) {
        git(w, &["merge-base", "HEAD", &row.base_commit])
    } else {
        git(w, &["merge-base", "HEAD", "refs/heads/main"])
    };
    // Conflict markers refuse.
    let changed = git(w, &["diff-tree", "-r", "--name-only", &base, &tree]);
    for path in changed.lines().filter(|l| !l.is_empty()) {
        let blob = git_out(w, &["cat-file", "-p", &format!("{tree}:{path}")]);
        if String::from_utf8_lossy(&blob.stdout)
            .lines()
            .any(|l| l.starts_with("<<<<<<<"))
        {
            return false;
        }
    }
    let head_w = git(w, &["rev-parse", "HEAD"]);
    let session = git(w, &["commit-tree", &tree, "-p", &head_w, "-m", "session"]);
    let head = git(main, &["rev-parse", "HEAD"]);
    let merged = git_out(
        main,
        &[
            "merge-tree",
            "--write-tree",
            &format!("--merge-base={base}"),
            &head,
            &session,
        ],
    );
    if !merged.status.success() {
        return false;
    }
    let result = String::from_utf8_lossy(&merged.stdout)
        .lines()
        .next()
        .unwrap()
        .to_string();
    if result == git(main, &["rev-parse", "HEAD^{tree}"]) {
        return false;
    }
    git(main, &["read-tree", "-m", "-u", &head, &result]);
    git(main, &["commit", "-q", "-m", "apply"]);
    true
}

fn cell(bring: Bring, turn_after: bool, agent_commit: bool) {
    let label = format!("bring {bring:?}, turn after {turn_after}, agent commit {agent_commit}");
    let commit = (
        "shell",
        json!({"command": "git add -A && git commit -qm agent", "description": "Commit"}),
    );
    let mut first = vec![
        ("write", json!({"path": "a.txt", "content": "session\n"})),
        ("write", json!({"path": "s.txt", "content": "s\n"})),
    ];
    let mut second = vec![("write", json!({"path": "t.txt", "content": "t\n"}))];
    if agent_commit {
        if turn_after {
            second.push(commit.clone());
        } else {
            first.push(commit.clone());
        }
    }
    let gateway = turns(vec![first, second]);
    let env = env(&gateway.url);
    let main = env.home.project();
    env.home.write("proj/a.txt", "a0\n");
    env.home.write("proj/c.txt", "c0\n");
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["add", "."]);
    git(&main, &["commit", "-qm", "init"]);
    let runtime = runtime();
    let core: Core = runtime.block_on(common::core(&env));
    let (session, _) = core.open(&Target::New, false).unwrap();
    let id = session.id.clone();
    let row = runtime.block_on(core.isolate(&id)).unwrap();
    assert!(
        core.set_worktree_state(&id, WorktreeState::Preparing, WorktreeState::Ready)
            .unwrap()
    );
    let w = PathBuf::from(&row.directory);
    runtime
        .block_on(core.prompt(&id, prompt("one"), TurnOptions::default()))
        .unwrap();
    // The main folder moves on: a change the session also made (a.txt, only when the base is
    // brought in) and two it didn't (c.txt, a new m.txt).
    std::fs::write(main.join("c.txt"), "c-main\n").unwrap();
    std::fs::write(main.join("m.txt"), "m\n").unwrap();
    if bring != Bring::None {
        std::fs::write(main.join("a.txt"), "a-main\n").unwrap();
    }
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-qm", "main moves on"]);
    let tip = git(&main, &["rev-parse", "HEAD"]);
    if bring != Bring::None {
        // As the desktop's Bring main in: pending work committed, then the merge.
        if !git(&w, &["status", "--porcelain"]).is_empty() {
            git(&w, &["add", "-A"]);
            git(
                &w,
                &["commit", "-qm", "Session work before bringing the base in"],
            );
        }
        assert!(!ok(&w, &["merge", "--no-edit", "-q", &tip]), "{label}");
        assert!(
            core.set_worktree_pending(&id, WorktreeState::Ready, Some(&tip))
                .unwrap()
        );
        if matches!(bring, Bring::Resolved | Bring::Committed) {
            std::fs::write(w.join("a.txt"), "resolved\n").unwrap();
        }
        if bring == Bring::Committed {
            git(&w, &["add", "-A"]);
            git(&w, &["commit", "-qm", "merged"]);
        }
    }
    if turn_after {
        runtime
            .block_on(core.prompt(&id, prompt("two"), TurnOptions::default()))
            .unwrap();
    }
    // Whether the bring-in's merge was still open when the folder went.
    let open_before = ok(&w, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    std::fs::remove_dir_all(&w).unwrap();
    let db = Db::open(&env.home.home().join(".anysola/anysola.db")).unwrap();
    anysola_core::api::cleanup_worktrees(&db);
    let made = runtime.block_on(core.recreate_worktree(&id)).unwrap();
    if made.state == WorktreeState::Preparing {
        assert!(
            core.set_worktree_state(&id, WorktreeState::Preparing, WorktreeState::Ready)
                .unwrap()
        );
    }
    let row = core.session_worktree(&id).unwrap().unwrap();
    // An unfinished bring-in is made again by git itself (its MERGE_HEAD is git's own) or
    // dropped; never kept without its merge.
    let merging = ok(&w, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    assert_eq!(row.base_pending.is_some(), merging, "{label}");
    // The last step ran while the merge was open: git makes it again, the resolution in
    // progress comes back.
    let during = open_before && turn_after;
    assert_eq!(merging, during, "{label}");
    if bring == Bring::Resolved && during {
        assert_eq!(
            read(&w.join("a.txt")).as_deref(),
            Some("resolved\n"),
            "{label}"
        );
    }
    if merging {
        assert_eq!(git(&w, &["rev-parse", "MERGE_HEAD"]), tip, "{label}");
        assert_eq!(row.base_pending.as_deref(), Some(tip.as_str()), "{label}");
    }
    // Review never shows the main folder's own files as the session's (`+m` added, c.txt
    // changed).
    let base = review_base(&w, &row);
    let changed = git(
        &w,
        &["diff-tree", "-r", "--name-only", &base, &files_tree(&w)],
    );
    for main_only in ["m.txt", "c.txt"] {
        assert!(
            !changed.lines().any(|l| l == main_only),
            "BREAK ({label}): Review shows main's {main_only} as the session's: {changed:?}"
        );
    }
    let before = git(&main, &["rev-parse", "HEAD"]);
    let applied = apply(&main, &row);
    if !applied {
        assert_eq!(git(&main, &["rev-parse", "HEAD"]), before, "{label}");
    }
    assert_eq!(
        read(&main.join("c.txt")).as_deref(),
        Some("c-main\n"),
        "BREAK ({label}): main's change to c.txt undone"
    );
    assert_eq!(
        read(&main.join("m.txt")).as_deref(),
        Some("m\n"),
        "BREAK ({label}): main's new m.txt removed"
    );
    assert_eq!(
        git(&main, &["show", "HEAD:m.txt"]),
        "m",
        "BREAK ({label}): m.txt gone from main's branch"
    );
    assert_eq!(
        git(&main, &["show", "HEAD:c.txt"]),
        "c-main",
        "BREAK ({label})"
    );
    drop(core);
}

#[test]
fn recreate_then_apply_never_undoes_the_main_folder() {
    for bring in [
        Bring::None,
        Bring::Conflicted,
        Bring::Resolved,
        Bring::Committed,
    ] {
        for turn_after in [false, true] {
            for agent_commit in [false, true] {
                cell(bring, turn_after, agent_commit);
            }
        }
    }
}
