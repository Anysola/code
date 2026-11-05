//! File watching for live config: config files and directories, markdown definition folders,
//! skill folders, `AGENTS.md` candidates in every directory of the upward walk, and the VCS head.
//! The watcher only counts changes; the agent loop reloads before its next step once the last
//! change is 100 ms old.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{EventKind, RecursiveMode, Watcher as _};

use crate::config::Config;
use crate::paths::Paths;

const DEBOUNCE: Duration = Duration::from_millis(100);
/// Entries of a walked directory that change what loads.
const ENTRIES: [&str; 6] = [
    "opencode.json",
    "opencode.jsonc",
    "AGENTS.md",
    ".opencode",
    ".claude",
    ".agents",
];
/// Folders of a config directory whose markdown files define agents, commands and skills.
const DEFINITIONS: [&str; 8] = [
    "agent", "agents", "mode", "modes", "command", "commands", "skill", "skills",
];

/// Changes seen so far, from the watcher or the remote config refresh.
#[derive(Debug, Default)]
pub struct Changes {
    generation: AtomicU64,
    last: Mutex<Option<Instant>>,
    marked: tokio::sync::Notify,
}

impl Changes {
    pub fn mark(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.marked.notify_one();
    }

    /// Waits for the next change (one that came before the wait counts).
    pub async fn marked(&self) {
        self.marked.notified().await
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Waits until the latest change is [`DEBOUNCE`] old, so a burst of writes reloads once.
    pub async fn settled(&self) {
        loop {
            let last = *self.last.lock().unwrap_or_else(|e| e.into_inner());
            let Some(wait) = last.and_then(|l| DEBOUNCE.checked_sub(l.elapsed())) else {
                return;
            };
            tokio::time::sleep(wait).await;
        }
    }
}

/// What one watch covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Everything below a directory.
    Tree(PathBuf),
    /// These entries of a directory (not their contents).
    Entries(PathBuf, Vec<String>),
}

impl Target {
    fn root(&self) -> &Path {
        match self {
            Target::Tree(dir) | Target::Entries(dir, _) => dir,
        }
    }

    fn covers(&self, path: &Path) -> bool {
        match self {
            Target::Tree(dir) => path.starts_with(dir),
            Target::Entries(dir, names) => {
                path.parent() == Some(dir.as_path())
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| names.iter().any(|e| e == n))
            }
        }
    }
}

fn entries(dir: &Path, names: &[&str]) -> Target {
    Target::Entries(
        dir.to_path_buf(),
        names.iter().map(|n| n.to_string()).collect(),
    )
}

/// `watcher.ignore` globs (`watcher` is the config key), matched against the whole path and
/// each of its components.
pub fn ignore_set(watcher: Option<&serde_json::Value>) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    let patterns = watcher
        .and_then(|w| w.get("ignore"))
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    for pattern in patterns.iter().filter_map(|p| p.as_str()) {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

fn ignored(set: &GlobSet, path: &Path) -> bool {
    set.is_match(path) || path.components().any(|c| set.is_match(c.as_os_str()))
}

/// Everything that changes what [`crate::config::load`], agents, commands, skills and
/// instructions produce for `directory`.
pub fn targets(
    paths: &Paths,
    config: &Config,
    directory: &Path,
    skills: &[PathBuf],
) -> Vec<Target> {
    let mut out = Vec::new();
    let mut add = |target: Target| {
        if !out.contains(&target) {
            out.push(target);
        }
    };
    for dir in crate::config::upward(directory, &paths.home) {
        add(entries(&dir, &ENTRIES));
    }
    let mut config_dirs = vec![paths.legacy_config_dir(), paths.data.clone()];
    config_dirs.extend(config.directories().cloned());
    for dir in &config_dirs {
        add(entries(
            dir,
            &[
                "opencode.json",
                "opencode.jsonc",
                "config.json",
                "AGENTS.md",
            ],
        ));
        add(entries(dir, &DEFINITIONS));
        for name in DEFINITIONS {
            add(Target::Tree(dir.join(name)));
        }
    }
    for root in skills {
        add(Target::Tree(root.clone()));
    }
    if let Some(root) = crate::project::vcs_root(directory) {
        add(entries(&root.join(".git"), &["HEAD"]));
        add(entries(&root.join(".hg"), &["branch"]));
    }
    out
}

fn disabled() -> bool {
    [
        "OPENCODE_DISABLE_FILEWATCHER",
        "OPENCODE_FILEWATCHER_DISABLE",
    ]
    .iter()
    .any(|name| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0" && v != "false"))
}

/// The live watch; [`Watch::arm`] replaces what it covers after every reload.
#[derive(Default)]
pub struct Watch {
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

impl Watch {
    pub fn arm(&self, targets: Vec<Target>, ignore: GlobSet, changes: std::sync::Arc<Changes>) {
        let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        if disabled() {
            return;
        }
        let targets: Vec<Target> = targets
            .into_iter()
            .filter(|t| !ignored(&ignore, t.root()) && t.root().is_dir())
            .collect();
        let rules = targets.clone();
        let handler = move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let relevant = event
                .paths
                .iter()
                .any(|path| !ignored(&ignore, path) && rules.iter().any(|rule| rule.covers(path)));
            if relevant {
                changes.mark();
            }
        };
        let Ok(mut watcher) = notify::recommended_watcher(handler) else {
            crate::log::warn("cannot start the file watcher", &[]);
            return;
        };
        for target in &targets {
            let mode = match target {
                Target::Tree(_) => RecursiveMode::Recursive,
                Target::Entries(..) => RecursiveMode::NonRecursive,
            };
            let _ = watcher.watch(target.root(), mode);
        }
        *slot = Some(watcher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn coverage() {
        let entry = entries(Path::new("/p"), &ENTRIES);
        assert!(entry.covers(Path::new("/p/AGENTS.md")));
        assert!(entry.covers(Path::new("/p/.opencode")));
        assert!(!entry.covers(Path::new("/p/src/main.rs")));
        assert!(!entry.covers(Path::new("/p/notes.md")));
        assert!(
            Target::Tree("/p/.opencode/agent".into())
                .covers(Path::new("/p/.opencode/agent/a/b.md"))
        );
    }

    #[test]
    fn ignore_matches_components() {
        let set = ignore_set(Some(&serde_json::json!({"ignore": [".git", "**/tmp/**"]})));
        assert!(ignored(&set, Path::new("/p/.git/HEAD")));
        assert!(ignored(&set, Path::new("/p/tmp/x")));
        assert!(!ignored(&set, Path::new("/p/AGENTS.md")));
    }

    #[tokio::test]
    async fn debounce_waits_for_quiet() {
        let changes = Changes::default();
        changes.settled().await;
        changes.mark();
        let start = Instant::now();
        changes.settled().await;
        assert!(start.elapsed() >= Duration::from_millis(90));
        assert_eq!(changes.generation(), 1);
    }

    #[tokio::test]
    async fn edits_to_agents_md_mark_a_change_and_other_files_do_not() {
        if disabled() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        let changes = Arc::new(Changes::default());
        let watch = Watch::default();
        watch.arm(
            vec![entries(&dir, &ENTRIES)],
            GlobSet::empty(),
            changes.clone(),
        );
        std::fs::write(dir.join("main.rs"), "x").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        std::fs::write(dir.join("AGENTS.md"), "rules").unwrap();
        while changes.generation() == 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(changes.generation() > 0);
        // One write can arrive as several events (create, modify, close).
        tokio::time::sleep(Duration::from_millis(200)).await;
        let seen = changes.generation();
        std::fs::write(dir.join("main.rs"), "y").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(changes.generation(), seen);
    }
}
