//! Git worktrees of a project, as the `opencode.worktree_*` tools manage them. The project's own
//! checkout is an unowned row; worktrees created here are `git` rows under
//! `~/.anysola/worktree/<project id prefix>/<name>` unless config `worktree.directory` names
//! another base.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::storage::Db;

const STRATEGY: &str = "git";
const MAX_SUFFIX: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub directory: String,
    pub strategy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoveError {
    #[error("the directory does not exist or is not a directory")]
    Unavailable,
    #[error("it is not a worktree of this project")]
    NotWorktree,
    #[error("it has modified or untracked files")]
    Dirty,
    #[error("{0}")]
    Failed(String),
}

/// Where new worktrees go: config `worktree.directory` (`~/` is home, relative is the project
/// root), else `<data>/worktree/<first 6 of the project id>`.
pub fn base(
    config: &crate::config::Config,
    paths: &crate::paths::Paths,
    root: &Path,
    project_id: &str,
) -> PathBuf {
    let configured = config
        .get("worktree")
        .and_then(|w| w.get("directory"))
        .and_then(serde_json::Value::as_str);
    match configured {
        Some(directory) => match directory.strip_prefix("~/") {
            Some(rest) => paths.home.join(rest),
            None => crate::paths::normalize(&root.join(directory)),
        },
        None => paths
            .data
            .join("worktree")
            .join(project_id.chars().take(6).collect::<String>()),
    }
}

pub fn list(db: &Db, project_id: &str) -> Vec<Worktree> {
    db.worktrees(project_id)
        .unwrap_or_default()
        .into_iter()
        .map(|(directory, strategy)| Worktree {
            directory,
            strategy,
        })
        .collect()
}

/// The directory resolved and canonical, when it is an existing directory.
fn canonical(directory: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(directory).ok().filter(|p| p.is_dir())
}

async fn git(cwd: &Path, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "Git failed".into()
    })
}

async fn toplevel(directory: &Path) -> Option<PathBuf> {
    let out = git(
        directory,
        &["rev-parse".as_ref(), "--show-toplevel".as_ref()],
    )
    .await
    .ok()?;
    Some(PathBuf::from(out.trim()))
}

/// Creates a detached worktree of the project checkout at `HEAD` under `base`; `-2`…`-10`
/// suffixes avoid existing directories.
pub async fn create(
    db: &Db,
    base: &Path,
    project_id: &str,
    root: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    let source = canonical(root).ok_or("project directory unavailable")?;
    let known = list(db, project_id)
        .iter()
        .any(|w| Path::new(&w.directory) == source);
    if !known {
        return Err("project directory is not a known worktree".into());
    }
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let mut target = base.join(name);
    let mut suffix = 1;
    while target.exists() {
        suffix += 1;
        if suffix > MAX_SUFFIX {
            return Err(format!("{} already exists", target.display()));
        }
        target = base.join(format!("{name}-{suffix}"));
    }
    let repository = toplevel(&source)
        .await
        .ok_or("project is not a git repository")?;
    git(
        &repository,
        &[
            "worktree".as_ref(),
            "add".as_ref(),
            "--detach".as_ref(),
            "--".as_ref(),
            target.as_os_str(),
            "HEAD".as_ref(),
        ],
    )
    .await?;
    let created = canonical(&target).ok_or("created worktree could not be opened")?;
    db.put_worktree(project_id, &created.display().to_string(), STRATEGY)
        .map_err(|e| e.to_string())?;
    Ok(created)
}

/// Removes a worktree this project created. `directory` must already be absolute.
pub async fn remove(
    db: &Db,
    project_id: &str,
    directory: &Path,
    force: bool,
) -> Result<(), RemoveError> {
    let directory = canonical(directory).ok_or(RemoveError::Unavailable)?;
    let text = directory.display().to_string();
    let owned = list(db, project_id)
        .into_iter()
        .any(|w| w.directory == text && w.strategy.is_some());
    if !owned {
        return Err(RemoveError::NotWorktree);
    }
    let common = git(
        &directory,
        &[
            "rev-parse".as_ref(),
            "--path-format=absolute".as_ref(),
            "--git-common-dir".as_ref(),
        ],
    )
    .await
    .map_err(|_| RemoveError::Unavailable)?;
    let mut args: Vec<&std::ffi::OsStr> = vec!["worktree".as_ref(), "remove".as_ref()];
    if force {
        args.push("--force".as_ref());
    }
    args.push(directory.as_os_str());
    git(Path::new(common.trim()), &args)
        .await
        .map_err(|message| {
            let lower = message.to_lowercase();
            if lower.contains("contains modified or untracked files") || lower.contains("is dirty")
            {
                RemoveError::Dirty
            } else {
                RemoveError::Failed(message)
            }
        })?;
    db.delete_worktree(project_id, &text)
        .map_err(|e| RemoveError::Failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn repo(dir: &Path) {
        for args in [
            vec!["init", "-q", "."],
            vec![
                "-c",
                "user.email=a@b",
                "-c",
                "user.name=a",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        ] {
            assert!(
                StdCommand::new("git")
                    .args(&args)
                    .current_dir(dir)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }

    #[tokio::test]
    async fn create_list_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let project = root.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        repo(&project);
        let base = root.join("home/.anysola/worktree/abc");
        let db = Db::open(&root.join("db.sqlite")).unwrap();
        db.upsert_project(&crate::project::identify(&project))
            .unwrap();
        let id = crate::project::identify(&project).id;

        let first = create(&db, &base, &id, &project, "wt").await.unwrap();
        let second = create(&db, &base, &id, &project, "wt").await.unwrap();
        assert!(first.ends_with("wt"));
        assert!(second.ends_with("wt-2"));
        let listed = list(&db, &id);
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[2].directory, project.display().to_string());
        assert_eq!(listed[2].strategy, None);

        assert_eq!(
            remove(&db, &id, &project, false).await,
            Err(RemoveError::NotWorktree)
        );
        assert_eq!(
            remove(&db, &id, &root.join("missing"), false).await,
            Err(RemoveError::Unavailable)
        );
        std::fs::write(first.join("dirty.txt"), "x").unwrap();
        assert_eq!(
            remove(&db, &id, &first, false).await,
            Err(RemoveError::Dirty)
        );
        remove(&db, &id, &first, true).await.unwrap();
        assert!(!first.exists());
        assert_eq!(list(&db, &id).len(), 2);
    }
}
