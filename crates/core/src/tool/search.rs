//! What `grep` and `glob` share: walking a directory the way 0.5.1's ripgrep did and the common
//! texts. The walk uses the `ignore` crate ripgrep is built on, configured with rg's defaults
//! (`.gitignore` inside git repositories, `.ignore`, `.rgignore`, no symlink following, errors
//! skipped as with `--no-messages`) plus the exact `--glob` list 0.5.1 passed. Unlike rg, entries
//! come in path order instead of rg's parallel-walk order, so results are deterministic.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ignore::WalkBuilder;
use ignore::overrides::{Override, OverrideBuilder};

use super::ToolError;

/// `FileSystem.DEFAULT_SEARCH_LIMIT`.
pub const DEFAULT_LIMIT: usize = 100;
const TIMEOUT: Duration = Duration::from_secs(30);

/// Excludes `.git` contents even when hidden files are searched (`--glob=!**/.git/**`).
pub const EXCLUDE_GIT: &str = "!**/.git/**";

pub fn truncation_notice(shown: usize) -> String {
    format!(
        "(Results are truncated: showing first {shown} results. Consider using a more specific path or pattern.)"
    )
}

/// rg `--glob` overrides rooted at `root`: later globs win, `!` excludes, and a positive glob both
/// restricts the results and re-includes ignored or hidden entries it matches. `None` when a glob
/// does not parse: rg then exits with an error and no output, which 0.5.1 read as no results.
pub fn overrides(root: &Path, globs: &[&str]) -> Option<Override> {
    let mut builder = OverrideBuilder::new(root);
    for glob in globs {
        builder.add(glob).ok()?;
    }
    builder.build().ok()
}

/// Files under the directory `root` in path order, until `stop` is set.
pub fn files<'a>(
    root: &Path,
    overrides: Override,
    hidden: bool,
    stop: &'a AtomicBool,
) -> impl Iterator<Item = PathBuf> + 'a {
    WalkBuilder::new(root)
        .hidden(!hidden)
        .add_custom_ignore_filename(".rgignore")
        .overrides(overrides)
        .sort_by_file_path(|a, b| a.cmp(b))
        .build()
        .take_while(|_| !stop.load(Ordering::Relaxed))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(ignore::DirEntry::into_path)
}

/// Sets the flag when dropped, so an abandoned blocking search winds down.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Runs `search` on the blocking pool for at most 30 seconds. The search must poll the flag it is
/// given: it is set when the call times out or its future is dropped (an interrupted turn).
pub async fn run<T: Send + 'static>(
    search: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> Result<T, ToolError> {
    let stop = Arc::new(AtomicBool::new(false));
    let _guard = StopOnDrop(stop.clone());
    let task = tokio::task::spawn_blocking(move || search(&stop));
    match tokio::time::timeout(TIMEOUT, task).await {
        Ok(joined) => {
            Ok(joined.unwrap_or_else(|error| std::panic::resume_unwind(error.into_panic())))
        }
        Err(_) => Err(ToolError::execution(format!(
            "Search timed out after {} seconds. Consider using a more specific path or pattern.",
            TIMEOUT.as_secs()
        ))),
    }
}

#[cfg(test)]
pub mod fixture {
    //! The project 0.5.1's grep and glob captures ran in.

    use std::fs;
    use std::path::Path;

    pub fn project(dir: &Path) {
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join(".hid")).unwrap();
        fs::create_dir_all(dir.join("ign")).unwrap();
        fs::write(dir.join("src/x.ts"), "foo one\nbar\nfoo two\n").unwrap();
        fs::write(dir.join(".hid/h.ts"), "foo\n").unwrap();
        fs::write(dir.join("ign/i.ts"), "foo\n").unwrap();
        fs::write(dir.join(".gitignore"), "ign\n").unwrap();
        fs::write(dir.join("a.txt"), "plain\n").unwrap();
        fs::write(dir.join(".git/HEAD"), "foo\n").unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(root: &Path, globs: &[&str], hidden: bool) -> Vec<String> {
        let stop = AtomicBool::new(false);
        let overrides = overrides(root, globs).unwrap();
        files(root, overrides, hidden, &stop)
            .map(|path| super::super::access::relative_path(root, &path))
            .collect()
    }

    #[test]
    fn walk_follows_rg_rules() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        assert_eq!(
            list(dir.path(), &[EXCLUDE_GIT], true),
            [".gitignore", ".hid/h.ts", "a.txt", "src/x.ts"]
        );
        assert_eq!(
            list(dir.path(), &[EXCLUDE_GIT], false),
            ["a.txt", "src/x.ts"]
        );
        // A positive glob matching the ignored directory re-includes it.
        assert_eq!(
            list(dir.path(), &["**/*", "!**/.*", EXCLUDE_GIT], false),
            ["a.txt", "ign/i.ts", "src/x.ts"]
        );
    }

    #[test]
    fn gitignore_needs_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        fixture::project(dir.path());
        std::fs::remove_dir_all(dir.path().join(".git")).unwrap();
        assert_eq!(
            list(dir.path(), &[], false),
            ["a.txt", "ign/i.ts", "src/x.ts"]
        );
    }

    #[test]
    fn invalid_glob_is_none() {
        assert!(overrides(Path::new("/"), &["a{b"]).is_none());
    }

    #[tokio::test]
    async fn run_returns_the_value() {
        assert!(!run(|stop| stop.load(Ordering::Relaxed)).await.unwrap());
    }
}
