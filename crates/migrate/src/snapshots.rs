//! 0.5.x shadow repositories stay where they are, read-only: each project's new store borrows
//! their objects through a second `alternates` line, so reverting an imported message restores
//! files from trees 0.5.x recorded.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anysola_core::paths::Paths;
use anysola_core::snapshot::Store;

use crate::source::Source;

/// The worktree a 0.5.x shadow repository captured, from its `core.worktree`.
fn worktree(old: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("config")
        .arg("--file")
        .arg(old.join("config"))
        .arg("core.worktree")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
}

fn attach(paths: &Paths, project: &str, old: &Path) -> std::io::Result<()> {
    let Some(worktree) = worktree(old).filter(|w| w.is_dir()) else {
        anysola_core::log::info(
            "snapshot store not attached: its worktree is gone",
            &[("store", &old.display().to_string())],
        );
        return Ok(());
    };
    let Some(store) = Store::open(&paths.data, project, &worktree) else {
        return Ok(());
    };
    let alternates = store.git_dir().join("objects/info/alternates");
    let line = old.join("objects").display().to_string();
    let current = std::fs::read_to_string(&alternates).unwrap_or_default();
    if current.lines().any(|l| l == line) {
        return Ok(());
    }
    std::fs::write(&alternates, format!("{current}{line}\n"))
}

pub fn attach_all(source: &Source, paths: &Paths) -> std::io::Result<()> {
    let Ok(projects) = std::fs::read_dir(source.data.join("snapshot")) else {
        return Ok(());
    };
    for project in projects.flatten() {
        let Ok(stores) = std::fs::read_dir(project.path()) else {
            continue;
        };
        let id = project.file_name().to_string_lossy().into_owned();
        for store in stores.flatten() {
            if store.path().join("HEAD").is_file() {
                attach(paths, &id, &store.path())?;
            }
        }
    }
    Ok(())
}
