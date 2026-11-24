//! Worktree sessions against real repositories in a temporary folder: the session's diff, the
//! apply (clean, onto a moved base, conflict, dirty main folder, staged, detached, failing hook,
//! nothing to apply), bringing the base in, putting a file back, env files and the setup command.

use super::*;

/// Repositories in a temporary folder with a git config of their own (as in `git.rs`).
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!(
            "anysola-wt-{name}-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let shared = std::env::temp_dir().join(format!("anysola-git-{}", std::process::id()));
        std::fs::create_dir_all(&shared).unwrap();
        let global = shared.join("gitconfig");
        if !global.exists() {
            std::fs::write(
                &global,
                "[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n[commit]\n\tgpgsign = false\n",
            )
            .unwrap();
        }
        crate::git::TEST_ENV.get_or_init(|| {
            vec![
                ("GIT_CONFIG_GLOBAL".into(), global.display().to_string()),
                ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                ("GIT_AUTHOR_DATE".into(), "2026-01-01T00:00:00Z".into()),
            ]
        });
        // Without Windows' verbatim prefix, which git can't take (as the app's own paths).
        Sandbox(anysola_core::paths::canonicalize(root).unwrap())
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        run_git(dir, args).unwrap_or_else(|f| panic!("git {args:?}: {f:?}"))
    }

    /// A main folder with `a.txt`, `b.txt` committed on `main`, and a session worktree of it.
    fn session(&self) -> (PathBuf, SessionWorktreeRow) {
        let main = self.0.join("main");
        std::fs::create_dir_all(&main).unwrap();
        self.git(&main, &["init", "-q", "-b", "main"]);
        write(&main, "a.txt", "one\ntwo\nthree\n");
        write(&main, "b.txt", "bee\n");
        write(&main, ".gitignore", ".env*\n!.env.example\nnode_modules/\n");
        write(&main, ".env.example", "TRACKED=1\n");
        self.git(&main, &["add", "-A"]);
        self.git(&main, &["commit", "-q", "-m", "init"]);
        let head = self.git(&main, &["rev-parse", "HEAD"]);
        let worktree = self.0.join("worktrees/main-abc123/s");
        let wt = worktree.display().to_string();
        self.git(
            &main,
            &["worktree", "add", "-q", "-b", "anysola/s", &wt, &head],
        );
        let row = SessionWorktreeRow {
            session_id: "ses_1".into(),
            project_id: "p".into(),
            folder: main.display().to_string(),
            directory: wt,
            branch: "anysola/s".into(),
            base_branch: Some("main".into()),
            base_commit: head,
            snapshot_dir: None,
            state: WorktreeState::Ready,
            error: None,
            applied_commit: None,
            apply_from: None,
            apply_tree: None,
            restored: None,
            leftover: false,
            time_created: 1,
            time_updated: 1,
            base_pending: None,
            apply_stash: None,
        };
        (main, row)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The file's text with `\r\n` read as `\n`: Git for Windows writes files it checks out or puts
/// back from a stash with CRLF (`core.autocrlf`), which is the same text.
fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

fn w(row: &SessionWorktreeRow) -> PathBuf {
    PathBuf::from(&row.directory)
}

fn always(_: &str, _: &str) -> bool {
    true
}

fn never(_: &str, _: &str) -> bool {
    false
}

fn nothing(_: &str) {}

fn marks() -> Marks<'static> {
    Marks {
        start: &always,
        stashed: &nothing,
    }
}

fn never_marks() -> Marks<'static> {
    Marks {
        start: &never,
        stashed: &nothing,
    }
}

/// The main folder's `HEAD` and status, to compare before and after.
fn state(sandbox: &Sandbox, main: &Path) -> (String, String) {
    (
        sandbox.git(main, &["rev-parse", "HEAD"]),
        sandbox.git(main, &["status", "--porcelain", "--untracked-files=all"]),
    )
}

fn reason(refusal: &Value) -> &str {
    refusal["reason"].as_str().unwrap()
}

/// What the core's `continue_after_apply` does to the worktree.
fn continue_after(sandbox: &Sandbox, row: &mut SessionWorktreeRow, commit: &str) {
    sandbox.git(&w(row), &["reset", "--hard", "-q", commit]);
    row.base_commit = commit.to_string();
}

#[test]
fn versions_and_status_counts() {
    assert_eq!(parse_version("git version 2.43.0"), Some((2, 43)));
    assert_eq!(parse_version("git version 2.39.2.windows.1"), Some((2, 39)));
    assert_eq!(parse_version("nothing"), None);
    assert!((2, 39) < APPLY_GIT && (2, 40) >= APPLY_GIT && (3, 0) > APPLY_GIT);
    assert_eq!(status_count(" M a\0R  new\0old\0?? c\0"), 3);
    assert_eq!(status_count(""), 0);
}

#[test]
fn the_diff_is_the_sessions_own_and_applies_as_one_commit_again_and_again() {
    let sandbox = Sandbox::new("apply");
    let (main, mut row) = sandbox.session();
    let wt = w(&row);
    write(&wt, "a.txt", "one\nTWO\nthree\n");
    write(&wt, "c.txt", "new\n");
    // An unrelated commit on main meanwhile: not part of the session's diff.
    write(&main, "b.txt", "bee\nmore\n");
    sandbox.git(&main, &["commit", "-qam", "main moves on"]);
    let diffs = worktree_changes(&row).unwrap();
    let files: Vec<&str> = diffs.iter().map(|d| d["file"].as_str().unwrap()).collect();
    assert_eq!(files, ["a.txt", "c.txt"]);
    assert_eq!(diffs[1]["status"], "added");
    // The worktree's own index is not touched by reading it.
    assert_eq!(sandbox.git(&wt, &["diff", "--cached", "--name-only"]), "");
    let plan = plan(&row, false).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(
        (plan.onto.as_str(), plan.files, plan.behind),
        ("main", 2, 1)
    );
    assert!(plan.dirty.is_empty());
    let before = sandbox.git(&main, &["rev-parse", "HEAD"]);
    let (commit, left) = apply(
        &marks(),
        &row,
        "Session work\n\nDetails",
        "main",
        false,
        false,
    )
    .unwrap();
    assert_eq!(left, None);
    assert_eq!(sandbox.git(&main, &["rev-parse", "HEAD"]), commit);
    assert_eq!(sandbox.git(&main, &["rev-parse", "HEAD^"]), before);
    assert_eq!(
        sandbox.git(&main, &["log", "-1", "--format=%B"]),
        "Session work\n\nDetails"
    );
    assert_eq!(read(&main, "a.txt"), "one\nTWO\nthree\n");
    assert_eq!(read(&main, "c.txt"), "new\n");
    assert_eq!(
        read(&main, "b.txt"),
        "bee\nmore\n",
        "main's own commit stays"
    );
    assert_eq!(sandbox.git(&main, &["status", "--porcelain"]), "");
    // The session goes on from the applied commit and applies again.
    continue_after(&sandbox, &mut row, &commit);
    assert!(worktree_changes(&row).unwrap().is_empty());
    assert_eq!(reason(&plan_err(&row)), "empty");
    write(&wt, "c.txt", "new\nagain\n");
    let (second, _) = apply(&marks(), &row, "More", "main", false, false).unwrap();
    assert_eq!(sandbox.git(&main, &["rev-parse", "HEAD^"]), commit);
    assert_eq!(sandbox.git(&main, &["rev-parse", "HEAD"]), second);
    assert_eq!(read(&main, "c.txt"), "new\nagain\n");
}

fn plan_err(row: &SessionWorktreeRow) -> Value {
    match plan(row, false) {
        Ok(_) => panic!("the apply would go"),
        Err(refusal) => refusal,
    }
}

#[test]
fn a_conflict_leaves_the_main_folder_alone_until_main_is_brought_in() {
    let sandbox = Sandbox::new("conflict");
    let (main, mut row) = sandbox.session();
    let wt = w(&row);
    write(&wt, "a.txt", "one\nsession\nthree\n");
    write(&main, "a.txt", "one\nmain\nthree\n");
    sandbox.git(&main, &["commit", "-qam", "main edits a"]);
    write(&main, "b.txt", "user edit\n");
    let before = state(&sandbox, &main);
    let refusal = plan_err(&row);
    assert_eq!(reason(&refusal), "conflict");
    assert_eq!(refusal["files"], json!(["a.txt"]));
    assert!(apply(&marks(), &row, "x", "main", false, false).is_err());
    assert_eq!(
        state(&sandbox, &main),
        before,
        "BREAK: the main folder changed"
    );
    // Bring main in: the conflict stays in the worktree as markers.
    let (conflicts, tip) = bring_base(&row).unwrap();
    assert_eq!(conflicts, ["a.txt"]);
    assert_eq!(tip, sandbox.git(&main, &["rev-parse", "main"]));
    // The base moves only once the merge is committed.
    row.base_pending = Some(tip);
    assert!(read(&wt, "a.txt").contains("<<<<<<<"));
    let refusal = plan_err(&row);
    assert_eq!(reason(&refusal), "unresolved");
    assert_eq!(refusal["files"], json!(["a.txt"]));
    // During the merge Review shows only the session's side against main.
    let files: Vec<String> = worktree_changes(&row)
        .unwrap()
        .iter()
        .map(|d| d["file"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(files, ["a.txt"]);
    // Resolved (by the agent): the apply finishes the merge and lands on main's tip.
    write(&wt, "a.txt", "one\nmain and session\nthree\n");
    let (commit, _) = apply(&marks(), &row, "Resolved", "main", true, false).unwrap();
    assert_eq!(read(&main, "a.txt"), "one\nmain and session\nthree\n");
    assert_eq!(
        read(&main, "b.txt"),
        "user edit\n",
        "an untouched edit stays"
    );
    assert_eq!(
        sandbox.git(&main, &["rev-parse", "HEAD^"]),
        before.0,
        "one commit on main"
    );
    assert_eq!(sandbox.git(&main, &["rev-parse", "HEAD"]), commit);
}

#[test]
fn a_clean_merge_of_main_moves_the_base() {
    let sandbox = Sandbox::new("bring");
    let (main, mut row) = sandbox.session();
    let wt = w(&row);
    write(&wt, "a.txt", "one\nsession\nthree\n");
    write(&main, "b.txt", "main\n");
    sandbox.git(&main, &["commit", "-qam", "main edits b"]);
    let (conflicts, tip) = bring_base(&row).unwrap();
    assert!(conflicts.is_empty());
    row.base_commit = tip;
    assert_eq!(read(&wt, "b.txt"), "main\n");
    let files: Vec<String> = worktree_changes(&row)
        .unwrap()
        .iter()
        .map(|d| d["file"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(files, ["a.txt"], "main's change is not the session's");
    let plan = plan(&row, false).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!((plan.files, plan.behind), (1, 0));
}

#[test]
fn main_folder_edits_in_touched_files_are_set_aside_and_back() {
    let sandbox = Sandbox::new("dirty");
    let (main, row) = sandbox.session();
    let wt = w(&row);
    write(&wt, "a.txt", "one\ntwo\nthree\nsession\n");
    write(&main, "a.txt", "user\none\ntwo\nthree\n");
    write(&main, "b.txt", "untouched edit\n");
    let plan = plan(&row, false).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(plan.dirty, ["a.txt"]);
    assert!(apply(&marks(), &row, "x", "main", false, false).is_err());
    assert_eq!(read(&main, "a.txt"), "user\none\ntwo\nthree\n");
    let (_, left) = apply(&marks(), &row, "With stash", "main", true, false).unwrap();
    assert_eq!(left, None);
    assert_eq!(read(&main, "a.txt"), "user\none\ntwo\nthree\nsession\n");
    assert_eq!(read(&main, "b.txt"), "untouched edit\n");
    assert_eq!(
        sandbox.git(&main, &["show", "HEAD:a.txt"]),
        "one\ntwo\nthree\nsession"
    );
    assert_eq!(sandbox.git(&main, &["stash", "list"]), "");
}

#[test]
fn a_stash_that_cannot_come_back_cleanly_is_kept() {
    let sandbox = Sandbox::new("stashleft");
    let (main, row) = sandbox.session();
    write(&w(&row), "a.txt", "one\nsession\nthree\n");
    write(&main, "a.txt", "one\nuser\nthree\n");
    let (commit, left) = apply(&marks(), &row, "Overlap", "main", true, false).unwrap();
    assert_eq!(left.as_deref(), Some("stash@{0}"));
    assert_eq!(sandbox.git(&main, &["rev-parse", "HEAD"]), commit);
    // No markers in the main folder; the user's edit is in the stash.
    assert_eq!(read(&main, "a.txt"), "one\nsession\nthree\n");
    assert_eq!(sandbox.git(&main, &["status", "--porcelain"]), "");
    assert!(
        sandbox
            .git(&main, &["stash", "show", "-p", "stash@{0}"])
            .contains("+user")
    );
}

/// Names git would quote (`core.quotePath`) still match the main folder's edits.
#[test]
fn file_names_beyond_ascii_compare() {
    let sandbox = Sandbox::new("names");
    let (main, row) = sandbox.session();
    write(&main, "şema.txt", "bir\n");
    sandbox.git(&main, &["add", "-A"]);
    sandbox.git(&main, &["commit", "-qm", "şema"]);
    // The session starts from before it: bring main in, then change it on both sides.
    let (_, tip) = bring_base(&row).unwrap();
    let row = SessionWorktreeRow {
        base_commit: tip,
        ..row
    };
    write(&w(&row), "şema.txt", "bir\niki\n");
    write(&main, "şema.txt", "sıfır\nbir\n");
    let plan = plan(&row, false).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(plan.dirty, ["şema.txt"]);
    let files: Vec<String> = worktree_changes(&row)
        .unwrap()
        .iter()
        .map(|d| d["file"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(files, ["şema.txt"]);
    let (_, left) = apply(&marks(), &row, "Şema", "main", true, false).unwrap();
    assert_eq!(left, None);
    assert_eq!(read(&main, "şema.txt"), "sıfır\nbir\niki\n");
}

#[test]
fn staged_detached_and_nothing_to_apply_are_refused() {
    let sandbox = Sandbox::new("refusals");
    let (main, row) = sandbox.session();
    assert_eq!(reason(&plan_err(&row)), "empty");
    write(&w(&row), "a.txt", "changed\n");
    write(&main, "b.txt", "staged\n");
    sandbox.git(&main, &["add", "b.txt"]);
    assert_eq!(reason(&plan_err(&row)), "staged");
    sandbox.git(&main, &["reset", "-q"]);
    sandbox.git(&main, &["checkout", "-q", "--detach"]);
    assert_eq!(reason(&plan_err(&row)), "detached");
    sandbox.git(&main, &["checkout", "-q", "main"]);
    // The main folder on another branch: the apply names it, and must be asked for by name.
    sandbox.git(&main, &["checkout", "-q", "-b", "other"]);
    let plan = plan(&row, false).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(plan.onto, "other");
    assert!(
        apply(&marks(), &row, "x", "main", false, false)
            .unwrap_err()
            .contains("is on other now")
    );
}

#[test]
fn a_failing_commit_hook_puts_the_main_folder_back() {
    let sandbox = Sandbox::new("hook");
    let (main, row) = sandbox.session();
    write(&w(&row), "a.txt", "session\n");
    write(&w(&row), "new.txt", "new\n");
    let hooks = main.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    write(
        &hooks,
        "pre-commit",
        "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            hooks.join("pre-commit"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let before = state(&sandbox, &main);
    let error = apply(&marks(), &row, "Blocked", "main", false, false).unwrap_err();
    assert!(error.contains("lint failed"), "{error}");
    assert_eq!(
        state(&sandbox, &main),
        before,
        "BREAK: the main folder kept the files"
    );
    assert!(!main.join("new.txt").exists());
    // The session's work is untouched.
    assert_eq!(read(&w(&row), "a.txt"), "session\n");
}

#[test]
fn an_apply_refused_by_the_core_leaves_everything() {
    let sandbox = Sandbox::new("mark");
    let (main, row) = sandbox.session();
    write(&w(&row), "a.txt", "session\n");
    write(&main, "a.txt", "one\ntwo\nthree\nuser\n");
    let before = state(&sandbox, &main);
    assert!(apply(&never_marks(), &row, "x", "main", true, false).is_err());
    assert_eq!(state(&sandbox, &main), before);
    assert_eq!(
        sandbox.git(&main, &["stash", "list"]),
        "",
        "the stash came back"
    );
}

#[test]
fn a_file_goes_back_to_the_base() {
    let sandbox = Sandbox::new("restore");
    let (_, row) = sandbox.session();
    let wt = w(&row);
    write(&wt, "a.txt", "changed\n");
    write(&wt, "added.txt", "new\n");
    put_back_file(&row, "a.txt").unwrap();
    put_back_file(&row, "added.txt").unwrap();
    assert_eq!(read(&wt, "a.txt"), "one\ntwo\nthree\n");
    assert!(!wt.join("added.txt").exists());
    assert!(worktree_changes(&row).unwrap().is_empty());
    assert!(put_back_file(&row, "../main/a.txt").is_err());
}

#[test]
fn ignored_env_files_are_copied_and_nothing_else() {
    let sandbox = Sandbox::new("env");
    let (main, row) = sandbox.session();
    let wt = w(&row);
    write(&main, ".env", "SECRET=1\n");
    write(&main, "apps/web/.env.local", "LOCAL=1\n");
    write(&main, "node_modules/pkg/.env", "SKIP=1\n");
    write(&main, ".env.example", "CHANGED=1\n");
    write(&main, "a/b/c/d/e/.env", "TOO_DEEP=1\n");
    let mut copied = copy_env(&main, &wt, &|| true);
    copied.sort();
    assert_eq!(copied, [".env", "apps/web/.env.local"]);
    assert_eq!(read(&wt, ".env"), "SECRET=1\n");
    assert_eq!(
        read(&wt, ".env.example"),
        "TRACKED=1\n",
        "tracked: never overwritten"
    );
    assert!(!wt.join("node_modules").exists());
    // Already there: left alone.
    write(&main, ".env", "SECRET=2\n");
    assert!(copy_env(&main, &wt, &|| true).is_empty());
    assert_eq!(read(&wt, ".env"), "SECRET=1\n");
}

#[cfg(unix)]
fn quiet_out(id: &str) -> Out {
    Out {
        handle: None,
        folder: "/main".into(),
        session_id: id.into(),
        slot: slot(id),
    }
}

#[cfg(unix)]
fn lines(out: &Out) -> Vec<(String, String)> {
    out.slot
        .run
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| {
            r.lines
                .iter()
                .map(|(s, t)| (s.to_string(), t.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(unix)]
#[test]
fn the_setup_command_runs_is_stopped_and_times_out() {
    let dir = std::env::temp_dir();
    let out = quiet_out("ses_setup_a");
    out.begin(Some("x".into()));
    let ended = run_setup(
        &out,
        Some("/bin/sh"),
        "echo hi; echo \"$ANYSOLA_WORKTREE\"; echo oops >&2; exit 3",
        &dir,
        "/wt",
        Duration::from_secs(30),
    );
    assert_eq!(ended, ("failed", Some(3)));
    let got = lines(&out);
    assert!(got.contains(&("out".into(), "hi".into())), "{got:?}");
    assert!(got.contains(&("out".into(), "/wt".into())), "{got:?}");
    assert!(got.contains(&("err".into(), "oops".into())), "{got:?}");
    assert!(!setup_running("ses_setup_a"));

    let out = quiet_out("ses_setup_b");
    let stopper = out.slot.clone();
    let started = Instant::now();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        stopper.stop.store(true, Ordering::SeqCst);
    });
    let ended = run_setup(
        &out,
        Some("/bin/sh"),
        "sleep 30",
        &dir,
        "/wt",
        Duration::from_secs(30),
    );
    thread.join().unwrap();
    assert_eq!(ended, ("stopped", None));
    assert!(started.elapsed() < Duration::from_secs(10));

    let out = quiet_out("ses_setup_c");
    let ended = run_setup(
        &out,
        Some("/bin/sh"),
        "sleep 30",
        &dir,
        "/wt",
        Duration::from_millis(300),
    );
    assert_eq!(ended, ("timeout", None));
}

fn hook_fails(main: &Path) {
    let hooks = main.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    write(
        &hooks,
        "pre-commit",
        "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            hooks.join("pre-commit"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
}

/// An ignored file of the main folder the session also has (tracked there with `add -f`): the
/// apply never overwrites it, and a failed commit never deletes it.
#[test]
fn an_ignored_main_folder_file_is_never_overwritten_or_lost() {
    let sandbox = Sandbox::new("ignored");
    let (main, row) = sandbox.session();
    let wt = w(&row);
    write(&main, ".env", "MAIN=1\n");
    write(&wt, ".env", "SESSION=1\n");
    sandbox.git(&wt, &["add", "-f", ".env"]);
    let plan = plan(&row, false).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(plan.dirty, [".env"]);
    assert!(apply(&marks(), &row, "x", "main", false, false).is_err());
    assert_eq!(
        read(&main, ".env"),
        "MAIN=1\n",
        "BREAK: refused apply touched it"
    );
    // A failing hook: everything goes back, the user's .env included.
    hook_fails(&main);
    let before = state(&sandbox, &main);
    assert!(apply(&marks(), &row, "x", "main", true, false).is_err());
    assert_eq!(
        read(&main, ".env"),
        "MAIN=1\n",
        "BREAK: the rollback lost the user's .env"
    );
    assert_eq!(state(&sandbox, &main), before);
    assert_eq!(sandbox.git(&main, &["stash", "list"]), "");
    // Applied with it set aside: the commit has the session's .env, the user's is kept.
    std::fs::remove_file(main.join(".git/hooks/pre-commit")).unwrap();
    let (_, left) = apply(&marks(), &row, "With env", "main", true, false).unwrap();
    assert_eq!(sandbox.git(&main, &["show", "HEAD:.env"]), "SESSION=1");
    let left = left.expect("the user's .env is kept in the stash");
    let kept = sandbox.git(&main, &["show", &format!("{left}^3:.env")]);
    assert_eq!(kept, "MAIN=1");
}

/// Conflict markers in a changed file are never applied, merge or not.
#[test]
fn conflict_markers_are_never_applied() {
    let sandbox = Sandbox::new("markers");
    let (_, row) = sandbox.session();
    write(
        &w(&row),
        "a.txt",
        "one\n<<<<<<< HEAD\ntwo\n=======\nTWO\n>>>>>>> other\nthree\n",
    );
    let refusal = plan_err(&row);
    assert_eq!(reason(&refusal), "unresolved");
    assert_eq!(refusal["files"], json!(["a.txt"]));
    // Lines that only start alike are not markers.
    write(&w(&row), "a.txt", "<<<<<<<<<< title\n>>>>>>>>>>\n");
    assert!(plan(&row, false).is_ok());
}

/// A worktree whose `.git` was pointed elsewhere is not run git in.
#[test]
fn a_redirected_worktree_is_refused() {
    let sandbox = Sandbox::new("redirect");
    let (_, row) = sandbox.session();
    let wt = w(&row);
    write(&wt, "a.txt", "changed\n");
    assert!(verify_worktree(&row).is_ok());
    let elsewhere = sandbox.0.join("elsewhere");
    sandbox.git(
        &sandbox.0,
        &["init", "-q", &elsewhere.display().to_string()],
    );
    write(
        &wt,
        ".git",
        &format!("gitdir: {}/.git\n", elsewhere.display()),
    );
    assert!(verify_worktree(&row).is_err());
    assert!(worktree_changes(&row).is_err());
    assert_eq!(reason(&plan_err(&row)), "git");
    assert!(bring_base(&row).is_err());
    assert!(put_back_file(&row, "a.txt").is_err());
}

/// A preparation let go stops copying files.
#[test]
fn a_cancelled_preparation_copies_nothing() {
    let sandbox = Sandbox::new("cancel");
    let (main, row) = sandbox.session();
    write(&main, ".env", "SECRET=1\n");
    assert!(copy_env(&main, &w(&row), &|| false).is_empty());
    assert!(!w(&row).join(".env").exists());
}

/// A merge of main with conflicts, then the worktree made again from its files without the
/// merge state (the folder was deleted): Review and Apply never undo main's commit.
#[test]
fn an_unfinished_merge_made_again_never_undoes_main() {
    let sandbox = Sandbox::new("unfinished");
    let (main, row) = sandbox.session();
    let wt = w(&row);
    write(&wt, "a.txt", "one\nsession\nthree\n");
    write(&main, "a.txt", "one\nmain\nthree\n");
    write(&main, "new.txt", "main's new file\n");
    sandbox.git(&main, &["add", "-A"]);
    sandbox.git(&main, &["commit", "-qm", "main moves on"]);
    let (conflicts, _) = bring_base(&row).unwrap();
    assert_eq!(conflicts, ["a.txt"]);
    // Resolved in the folder, not committed; then the folder goes and is made again from the
    // branch (the wip commit) with the last files laid over, merge state lost.
    write(&wt, "a.txt", "one\nmain and session\nthree\n");
    let saved: Vec<(String, String)> = ["a.txt", "b.txt", "new.txt"]
        .iter()
        .map(|f| (f.to_string(), read(&wt, f)))
        .collect();
    let wt_text = row.directory.clone();
    sandbox.git(
        &main,
        &["worktree", "remove", "--force", "--force", &wt_text],
    );
    sandbox.git(&main, &["worktree", "add", "-q", &wt_text, &row.branch]);
    for (file, text) in &saved {
        write(&wt, file, text);
    }
    let before = state(&sandbox, &main);
    // Review: never main's own change read backwards.
    for diff in worktree_changes(&row).unwrap() {
        assert!(
            !diff["patch"].as_str().unwrap().contains("-main's new file"),
            "BREAK: Review undoes main's file: {diff}"
        );
        assert!(
            !(diff["file"] == "a.txt" && diff["patch"].as_str().unwrap().contains("-one\nmain")),
            "{diff}"
        );
    }
    // Apply: whatever it decides, main's file and change stay.
    let _ = apply(&marks(), &row, "x", "main", true, false);
    assert_eq!(
        read(&main, "new.txt"),
        "main's new file\n",
        "BREAK: main's file undone"
    );
    assert!(
        read(&main, "a.txt").contains("main"),
        "BREAK: main's change undone"
    );
    let _ = before;
}

/// An ignored file of the main folder where the session has a folder (`cache` vs
/// `cache/data`) is in the way like any other.
#[test]
fn an_ignored_file_in_place_of_a_folder_is_in_the_way() {
    let sandbox = Sandbox::new("clash");
    let (main, row) = sandbox.session();
    write(
        &main,
        ".gitignore",
        ".env*\n!.env.example\nnode_modules/\ncache\n",
    );
    sandbox.git(&main, &["commit", "-qam", "ignore cache"]);
    sandbox.git(&w(&row), &["merge", "-q", "main"]);
    let row = SessionWorktreeRow {
        base_commit: sandbox.git(&main, &["rev-parse", "HEAD"]),
        ..row
    };
    write(&main, "cache", "user's cache file\n");
    write(&w(&row), "cache/data", "session data\n");
    sandbox.git(&w(&row), &["add", "-f", "cache/data"]);
    let plan = plan(&row, false).unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(plan.dirty, ["cache"]);
    assert_eq!(plan.ignored, ["cache/data"]);
    assert!(apply(&marks(), &row, "x", "main", false, false).is_err());
    assert_eq!(read(&main, "cache"), "user's cache file\n");
}

/// "Apply anyway": conflict markers committed as they are, once asked for.
#[test]
fn markers_apply_only_when_asked() {
    let sandbox = Sandbox::new("anyway");
    let (main, row) = sandbox.session();
    let text = "<<<<<<< HEAD\nexample\n=======\nfixture\n>>>>>>> other\n";
    write(&w(&row), "docs/merge.md", text);
    assert!(apply(&marks(), &row, "x", "main", false, false).is_err());
    apply(&marks(), &row, "Docs", "main", false, true).unwrap();
    assert_eq!(read(&main, "docs/merge.md"), text);
}

/// "Apply anyway" never commits the markers of an open merge: that merge must be finished.
#[test]
fn markers_of_an_open_merge_are_never_applied_anyway() {
    let sandbox = Sandbox::new("anyway-merge");
    let (main, row) = sandbox.session();
    write(&w(&row), "a.txt", "one\nsession\nthree\n");
    write(&main, "a.txt", "one\nmain\nthree\n");
    sandbox.git(&main, &["commit", "-qam", "main edits a"]);
    let (conflicts, _) = bring_base(&row).unwrap();
    assert_eq!(conflicts, ["a.txt"]);
    let refusal = plan_err(&row);
    assert_eq!(refusal["anyway"], false);
    let before = state(&sandbox, &main);
    assert!(apply(&marks(), &row, "x", "main", false, true).is_err());
    assert_eq!(state(&sandbox, &main), before);
}

/// Local hook and config files under `.claude/` and `.anysola/` that Git ignores (in the main
/// folder and in the worktree) come along and never show as the session's change; untracked
/// files that aren't ignored, tracked ones and `node_modules` don't.
#[test]
fn ignored_local_hook_files_are_copied() {
    let sandbox = Sandbox::new("local");
    let (main, row) = sandbox.session();
    write(
        &main,
        ".git/info/exclude",
        ".claude/settings.local.json\n.claude/hooks/\n.anysola/node_modules/\n",
    );
    write(&main, ".claude/settings.json", "{}\n");
    sandbox.git(&main, &["add", ".claude/settings.json"]);
    sandbox.git(&main, &["commit", "-qm", "settings"]);
    write(&main, ".claude/settings.json", "{\"changed\": true}\n");
    write(&main, ".claude/settings.local.json", "{\"local\": true}\n");
    write(&main, ".claude/hooks/x.sh", "#!/bin/sh\nexit 0\n");
    write(&main, ".anysola/notes.json", "{\"not ignored\": 1}\n");
    write(&main, ".anysola/node_modules/pkg/index.js", "x\n");
    let mut copied = copy_local(&main, &w(&row), &|| true);
    copied.sort();
    assert_eq!(
        copied,
        [".claude/hooks/x.sh", ".claude/settings.local.json"]
    );
    assert!(
        !w(&row).join(".anysola/notes.json").exists(),
        "untracked, not ignored"
    );
    assert!(!w(&row).join(".anysola/node_modules").exists());
    assert!(
        worktree_changes(&row).unwrap().is_empty(),
        "never the session's change"
    );
    // Already there, or let go: nothing.
    assert!(copy_local(&main, &w(&row), &|| true).is_empty());
    assert!(copy_local(&main, &w(&row), &|| false).is_empty());
}

/// A `.claude` that is a link (to the home folder's, say) is never copied from.
#[cfg(unix)]
#[test]
fn a_linked_local_folder_is_skipped() {
    let sandbox = Sandbox::new("linked");
    let (main, row) = sandbox.session();
    let home_claude = sandbox.0.join("home/.claude");
    write(&home_claude, ".credentials.json", "secret\n");
    std::os::unix::fs::symlink(&home_claude, main.join(".claude")).unwrap();
    write(&main, ".git/info/exclude", ".claude\n");
    assert!(copy_local(&main, &w(&row), &|| true).is_empty());
    assert!(!w(&row).join(".claude").exists());
}

#[test]
fn undo_apply_reverts_head_and_keeps_worktree_changes() {
    let sandbox = Sandbox::new("undo-apply");
    let (main, mut row) = sandbox.session();
    let before_head = read_git(&main, &["rev-parse", "HEAD"]).unwrap();
    write(&w(&row), "undo_new.txt", "created in worktree\n");
    let (applied, _) = apply(&marks(), &row, "apply worktree", "main", false, false).unwrap();
    assert_eq!(
        read_git(&main, &["rev-parse", "HEAD"]).as_deref(),
        Some(applied.as_str())
    );
    assert_eq!(
        std::fs::read_to_string(main.join("undo_new.txt")).unwrap(),
        "created in worktree\n"
    );
    row.applied_commit = Some(applied.clone());
    row.base_commit = applied.clone();

    let parent = undo_apply(&row).unwrap();
    assert_eq!(parent, before_head);
    assert_eq!(
        read_git(&main, &["rev-parse", "HEAD"]).as_deref(),
        Some(before_head.as_str())
    );
    assert!(!main.join("undo_new.txt").exists());
    // Worktree files remain intact so the user can edit or apply again.
    assert_eq!(
        std::fs::read_to_string(w(&row).join("undo_new.txt")).unwrap(),
        "created in worktree\n"
    );
}

#[test]
fn undo_apply_refuses_when_main_moved_or_dirty() {
    let sandbox = Sandbox::new("undo-refuse");
    let (main, mut row) = sandbox.session();
    write(&w(&row), "a.txt", "v1\n");
    let (applied, _) = apply(&marks(), &row, "apply v1", "main", false, false).unwrap();
    row.applied_commit = Some(applied.clone());
    row.base_commit = applied.clone();

    // Uncommitted edit to an applied file blocks undo without touching it.
    write(&main, "a.txt", "local edit\n");
    assert!(undo_apply(&row).is_err());
    assert_eq!(
        std::fs::read_to_string(main.join("a.txt")).unwrap(),
        "local edit\n"
    );

    // A newer commit on main also blocks undo.
    sandbox.git(&main, &["add", "a.txt"]);
    sandbox.git(&main, &["commit", "-qm", "later commit"]);
    let err = undo_apply(&row).unwrap_err();
    assert!(err.contains("moved past"), "{err}");
}
