//! Snapshots: the project's files captured as git trees in a shadow repository
//! (`~/.anysola/snapshot/<projectID>/<sha1(worktree)>`, driven with the system `git` and
//! `--git-dir`/`--work-tree`), so a session can show what a step changed and undo it. The store
//! borrows the project's objects through `alternates` and its ignore rules through the worktree.
//! [`Store::at`] opens any store directory, so 0.5.x stores can be attached read-only by import.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha1::{Digest, Sha1};

const MAX_UNTRACKED_BYTES: u64 = 2 * 1024 * 1024;
const PACK_INTERVAL: Duration = Duration::from_secs(10 * 60);

pub struct Store {
    git_dir: PathBuf,
    work_tree: PathBuf,
    /// The capture scope: the session directory relative to the worktree (`.` for the root).
    scope: String,
    last_pack: Mutex<Option<Instant>>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

impl Store {
    /// The store for the git project whose worktree holds `directory`; `None` outside git.
    pub fn open(data: &Path, project_id: &str, directory: &Path) -> Option<Store> {
        let top = git_output(directory, &["rev-parse", "--show-toplevel"])?;
        let work_tree = crate::paths::canonicalize(&top).ok()?;
        let source = crate::paths::canonicalize(
            directory.join(git_output(directory, &["rev-parse", "--git-common-dir"])?),
        )
        .ok()?;
        let digest = hex(&Sha1::digest(work_tree.display().to_string().as_bytes()));
        let git_dir = data.join("snapshot").join(project_id).join(digest);
        let directory = crate::paths::canonicalize(directory).ok()?;
        let store = Store::at(git_dir, work_tree, &directory)?;
        if !store.git_dir.join("HEAD").exists() {
            store.create(&source)?;
        }
        Some(store)
    }

    /// An existing or new store directory over `work_tree`, capturing `directory`.
    pub fn at(git_dir: PathBuf, work_tree: PathBuf, directory: &Path) -> Option<Store> {
        let scope = directory
            .strip_prefix(&work_tree)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        Some(Store {
            git_dir,
            work_tree,
            scope: if scope.is_empty() { ".".into() } else { scope },
            last_pack: Mutex::new(None),
        })
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.work_tree)
            .args(args)
            .current_dir(&self.work_tree)
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&str]) -> Option<Vec<u8>> {
        let output = self.command(args).stderr(Stdio::null()).output().ok()?;
        output.status.success().then_some(output.stdout)
    }

    fn run_with_input(&self, args: &[&str], input: &[u8]) -> Option<()> {
        use std::io::Write;
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(input).ok()?;
        child.wait().ok()?.success().then_some(())
    }

    fn create(&self, source: &Path) -> Option<()> {
        std::fs::create_dir_all(&self.git_dir).ok()?;
        self.run(&["init", "--quiet"])?;
        for (key, value) in [
            ("core.autocrlf", "false"),
            ("core.longpaths", "true"),
            ("core.symlinks", "true"),
            ("core.fsmonitor", "false"),
        ] {
            self.run(&["config", key, value])?;
        }
        let info = self.git_dir.join("objects/info");
        std::fs::create_dir_all(&info).ok()?;
        std::fs::write(
            info.join("alternates"),
            format!("{}\n", source.join("objects").display()),
        )
        .ok()?;
        if let Ok(exclude) = std::fs::read(source.join("info/exclude")) {
            let _ = std::fs::create_dir_all(self.git_dir.join("info"));
            let _ = std::fs::write(self.git_dir.join("info/exclude"), exclude);
        }
        Some(())
    }

    fn list(&self, args: &[&str]) -> Vec<String> {
        self.run(args)
            .map(|out| {
                out.split(|b| *b == 0)
                    .filter(|p| !p.is_empty())
                    .map(|p| String::from_utf8_lossy(p).into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The current files of the scope as a tree id. Untracked files over 2 MiB are left out.
    pub fn capture(&self) -> Option<String> {
        let scope = self.scope.as_str();
        let _ = self.run(&["update-index", "-q", "--refresh"]);
        let mut paths = self.list(&["ls-files", "-z", "--modified", "--deleted", "--", scope]);
        paths.extend(
            self.list(&[
                "ls-files",
                "-z",
                "--others",
                "--exclude-standard",
                "--",
                scope,
            ])
            .into_iter()
            .filter(|p| {
                std::fs::symlink_metadata(self.work_tree.join(p))
                    .is_ok_and(|m| !m.is_file() || m.len() <= MAX_UNTRACKED_BYTES)
            }),
        );
        if !paths.is_empty() {
            let input: Vec<u8> = paths
                .iter()
                .flat_map(|p| [p.as_bytes(), b"\0"].concat())
                .collect();
            self.run_with_input(
                &[
                    "update-index",
                    "--add",
                    "--remove",
                    "--replace",
                    "-z",
                    "--stdin",
                ],
                &input,
            )?;
        }
        let tree = String::from_utf8(self.run(&["write-tree"])?).ok()?;
        self.pack_when_due();
        Some(tree.trim().to_string())
    }

    fn pack_when_due(&self) {
        let mut last = self.last_pack.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < PACK_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
        // Packing takes git's own lock; a slow pack never blocks a capture.
        let _ = self
            .command(&["gc", "--auto", "--quiet"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    /// Worktree-relative paths that differ between two trees.
    pub fn files(&self, from: &str, to: &str) -> Vec<String> {
        self.list(&[
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-renames",
            from,
            to,
        ])
    }

    /// A unified patch between two trees (the `/diff` viewer's `Last turn`).
    pub fn patch(&self, from: &str, to: &str) -> Option<String> {
        let out = self.run(&["diff", "--no-color", "--no-ext-diff", from, to])?;
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    /// A file's content in a tree; `None` when the tree does not have it.
    pub fn read(&self, tree: &str, path: &str) -> Option<Vec<u8>> {
        self.run(&["cat-file", "blob", &format!("{tree}:{path}")])
    }

    /// Puts each path back as its tree has it; a path the tree lacks is removed.
    pub fn restore(&self, files: &[(String, String)]) -> Result<(), String> {
        for (path, tree) in files {
            let present = !self
                .list(&["ls-tree", "-z", "--name-only", tree, "--", path])
                .is_empty();
            if present {
                self.run(&["checkout", tree, "--", path])
                    .ok_or_else(|| format!("Cannot restore {path}"))?;
            } else {
                let target = self.work_tree.join(path);
                if target.symlink_metadata().is_ok() {
                    std::fs::remove_file(&target)
                        .map_err(|e| format!("Cannot remove {path}: {e}"))?;
                }
            }
        }
        Ok(())
    }

    /// The worktree root paths are relative to.
    pub fn work_tree(&self) -> &Path {
        &self.work_tree
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn capture_diff_restore() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("p");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q"]);
        std::fs::write(project.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(project.join("a.txt"), "one\n").unwrap();
        std::fs::write(project.join("ignored.txt"), "x").unwrap();
        std::fs::write(project.join("big.bin"), vec![0u8; 3 * 1024 * 1024]).unwrap();
        git(&project, &["add", "a.txt", ".gitignore"]);
        git(&project, &["commit", "-q", "-m", "init"]);
        let data = root.path().join("data");
        let store = Store::open(&data, "pid", &project).unwrap();
        let start = store.capture().unwrap();
        assert!(store.read(&start, "a.txt").is_some());
        assert!(store.read(&start, "ignored.txt").is_none());
        assert!(
            store.read(&start, "big.bin").is_none(),
            "large untracked files are skipped"
        );

        std::fs::write(project.join("a.txt"), "two\n").unwrap();
        std::fs::write(project.join("new.txt"), "new\n").unwrap();
        let end = store.capture().unwrap();
        assert_eq!(store.files(&start, &end), vec!["a.txt", "new.txt"]);
        let patch = store.patch(&start, &end).unwrap();
        assert!(patch.contains("diff --git a/a.txt b/a.txt") && patch.contains("+two"));
        store
            .restore(&[
                ("a.txt".into(), start.clone()),
                ("new.txt".into(), start.clone()),
            ])
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(project.join("a.txt")).unwrap(),
            "one\n"
        );
        assert!(!project.join("new.txt").exists());
        store.restore(&[("new.txt".into(), end.clone())]).unwrap();
        assert_eq!(
            std::fs::read_to_string(project.join("new.txt")).unwrap(),
            "new\n"
        );
        let reopened = Store::open(&data, "pid", &project.join(".")).unwrap();
        assert_eq!(reopened.git_dir, store.git_dir);
        assert!(
            Store::open(&data, "pid", root.path()).is_none(),
            "not a git project"
        );
    }
}
