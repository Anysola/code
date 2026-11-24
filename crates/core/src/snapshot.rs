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
    let output = crate::console::command("git")
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

/// A scratch index file, removed when dropped.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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
        let mut command = crate::console::command("git");
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

    /// The `HEAD` commit of the checkout the store captures; `None` on an unborn branch.
    pub fn head(&self) -> Option<String> {
        git_output(
            &self.work_tree,
            &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
        )
        .filter(|h| !h.is_empty())
    }

    /// The capture scope, relative to the worktree (`.` for all of it).
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Whether `spec` (`<tree>^{tree}`, `<commit>^{commit}`) is readable from this store, the
    /// project's objects included.
    pub fn has(&self, spec: &str) -> bool {
        self.run(&["cat-file", "-e", spec]).is_some()
    }

    /// Lets this store read another store's objects too (a fork's store reads the trees its
    /// copied history names, which live in the source's store).
    pub fn borrow_objects(&self, other_git_dir: &Path) {
        let file = self.git_dir.join("objects/info/alternates");
        let line = other_git_dir.join("objects").display().to_string();
        let mut text = std::fs::read_to_string(&file).unwrap_or_default();
        if other_git_dir == self.git_dir || text.lines().any(|l| l == line) {
            return;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&line);
        text.push('\n');
        let _ = std::fs::write(&file, text);
    }

    /// Starts the store's index from the checkout's own (a fresh worktree's): the first capture
    /// then only refreshes it instead of hashing every file. A store that has an index keeps it.
    pub fn seed_index(&self, index: &Path) {
        let own = self.git_dir.join("index");
        if own.exists() || !index.is_file() {
            return;
        }
        if std::fs::copy(index, &own).is_err() || self.run(&["ls-files", "-z"]).is_none() {
            // An index this git cannot read is no head start.
            let _ = std::fs::remove_file(&own);
        }
    }

    /// Puts the scope's files in `target` (another checkout of the same repository) as `tree`
    /// has them; see [`overlay`].
    pub fn overlay(&self, tree: &str, target: &Path) -> Result<Vec<String>, String> {
        overlay(&self.git_dir, &self.scope, tree, target)
    }

    /// Puts each path in `target` back as its tree has it (a path the tree lacks is removed),
    /// like [`Store::restore`] does in the store's own checkout.
    pub fn restore_into(&self, files: &[(String, String)], target: &Path) -> Result<(), String> {
        for (path, tree) in files {
            let file = target.join(path);
            match self.read(tree, path) {
                Some(content) => {
                    if let Some(parent) = file.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("Cannot restore {path}: {e}"))?;
                    }
                    std::fs::write(&file, content)
                        .map_err(|e| format!("Cannot restore {path}: {e}"))?;
                }
                None => {
                    if file.symlink_metadata().is_ok() {
                        std::fs::remove_file(&file)
                            .map_err(|e| format!("Cannot remove {path}: {e}"))?;
                    }
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

/// Puts the files under `scope` in `target` (a checkout of the repository `git_dir` reaches the
/// objects of) as `tree` has them: files the tree has are written, files tracked in `target` that it lacks are
/// removed, unless they are over 2 MiB (never captured), which are left as they are and
/// returned. Nothing outside the scope is touched.
pub fn overlay(
    git_dir: &Path,
    scope: &str,
    tree: &str,
    target: &Path,
) -> Result<Vec<String>, String> {
    let index = TempIndex(git_dir.join(format!("overlay-{}.index", crate::ids::part())));
    let in_target = |args: &[&str]| {
        let mut command = crate::console::command("git");
        command
            .arg("--git-dir")
            .arg(&git_dir)
            .arg("--work-tree")
            .arg(target)
            .args(args)
            .current_dir(target)
            .env("GIT_INDEX_FILE", &index.0)
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    };
    let run = |args: &[&str], input: Option<&[u8]>| -> Result<Vec<u8>, String> {
        use std::io::Write;
        let mut child = in_target(args).spawn().map_err(|e| e.to_string())?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .ok_or("no stdin")?
                .write_all(input)
                .map_err(|e| e.to_string())?;
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    };
    let split = |out: Vec<u8>| -> Vec<String> {
        out.split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect()
    };
    run(&["read-tree", tree], None)?;
    let wanted = split(run(&["ls-files", "-z", "--", scope], None)?);
    if !wanted.is_empty() {
        let input: Vec<u8> = wanted
            .iter()
            .flat_map(|p| [p.as_bytes(), b"\0"].concat())
            .collect();
        run(&["checkout-index", "-f", "-z", "--stdin"], Some(&input))?;
    }
    // What the target's own checkout tracks there and the tree lacks.
    let tracked = split(
        crate::console::command("git")
            .args(["ls-files", "-z", "--", scope])
            .current_dir(target)
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?
            .stdout,
    );
    let wanted: std::collections::HashSet<String> = wanted.into_iter().collect();
    let mut kept = Vec::new();
    for path in tracked {
        if wanted.contains(&path) {
            continue;
        }
        let file = target.join(&path);
        match std::fs::symlink_metadata(&file) {
            Ok(meta) if meta.is_file() && meta.len() > MAX_UNTRACKED_BYTES => kept.push(path),
            Ok(_) => {
                std::fs::remove_file(&file).map_err(|e| format!("Cannot remove {path}: {e}"))?
            }
            Err(_) => {}
        }
    }
    Ok(kept)
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
