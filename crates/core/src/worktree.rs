//! Git worktrees of a project, as the `opencode.worktree_*` tools manage them. The project's own
//! checkout is an unowned row; worktrees created here are `git` rows under
//! `~/.anysola/worktree/<project id prefix>/<name>` unless config `worktree.directory` names
//! another base.

use std::path::{Path, PathBuf};

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
    #[error("it is a session's own worktree; apply or discard that session's changes instead")]
    Bound,
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

/// Folders Anysola Code loads the user's own definitions or config from: a worktree there would
/// make the repository's files load as the user's.
fn loaded_folders(paths: &crate::paths::Paths) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = crate::agent::FOLDERS
        .iter()
        .chain(crate::command::FOLDERS.iter())
        .chain(["skill", "skills"].iter())
        .map(|folder| paths.data.join(folder))
        .collect();
    out.push(paths.data.join("config.json"));
    out.push(paths.data.join("AGENTS.md"));
    out.push(paths.legacy_config_dir());
    out.push(crate::config::system_dir());
    out.push(paths.home.join(".claude"));
    out.push(paths.home.join(".agents"));
    out
}

/// Whether a worktree named `name` may be created under `base`: the name is one plain folder
/// name, and the worktree is neither inside nor around a folder the app loads definitions from.
pub fn place(paths: &crate::paths::Paths, base: &Path, name: &str) -> Result<PathBuf, String> {
    let plain = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
        && Path::new(name).components().count() == 1;
    if !plain {
        return Err(format!(
            "\"{name}\" is not a worktree name: use a plain folder name."
        ));
    }
    let target = crate::paths::normalize(&base.join(name));
    // Also where both really are: another spelling of a folder (a symlink, a Windows 8.3 short
    // name) is the same folder.
    let real_target = crate::config::real_path(&target);
    let near = |folder: &Path| {
        let folder = crate::paths::normalize(folder);
        let real_folder = crate::config::real_path(&folder);
        target.starts_with(&folder)
            || folder.starts_with(&target)
            || real_target.starts_with(&real_folder)
            || real_folder.starts_with(&real_target)
    };
    if let Some(folder) = loaded_folders(paths).iter().find(|f| near(f)) {
        return Err(format!(
            "A worktree can't go in {}: Anysola Code loads your own agents, commands, skills or config from {}. Set \"worktree.directory\" in your config to another folder.",
            target.display(),
            folder.display()
        ));
    }
    Ok(target)
}

/// Strategy of the rows sessions' own worktrees have in table `worktree`.
pub const SESSION_STRATEGY: &str = "session";

/// Where sessions' own worktrees of the main folder `folder` go:
/// `<data>/worktrees/<folder name>-<first 6 of the project id>`. A fixed place (config
/// `worktree.directory` is for the agent's `worktree_create`), still checked by [`place`].
pub fn session_base(
    paths: &crate::paths::Paths,
    folder: &Path,
    project_id: &str,
) -> Result<PathBuf, String> {
    let name: String = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('.');
    let name = if name.is_empty() { "project" } else { name };
    let id: String = project_id.chars().take(6).collect();
    place(
        paths,
        &paths.data.join("worktrees"),
        &format!("{name}-{id}"),
    )
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

/// Whether `directory` is (in) a session's own worktree that session still works in.
pub fn bound(db: &Db, directory: &Path) -> bool {
    let target = crate::paths::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    db.session_worktrees()
        .unwrap_or_default()
        .iter()
        .filter(|row| !row.state.released())
        .any(|row| {
            let top = Path::new(&row.directory);
            let top = crate::paths::canonicalize(top).unwrap_or_else(|_| top.to_path_buf());
            target.starts_with(&top)
        })
}

/// The directory resolved and canonical, when it is an existing directory.
fn canonical(directory: &Path) -> Option<PathBuf> {
    crate::paths::canonicalize(directory)
        .ok()
        .filter(|p| p.is_dir())
}

pub(crate) async fn git(cwd: &Path, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = crate::console::tokio_command("git")
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

/// `git` run synchronously in `cwd`, with extra environment; stdout on success, else git's words.
pub(crate) fn git_sync(cwd: &Path, args: &[&str], env: &[(&str, &Path)]) -> Result<String, String> {
    let mut command = crate::console::command("git");
    command
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        "Git failed".into()
    } else {
        stderr
    })
}

/// The checkout's current files (tracked and untracked, not ignored) as a tree, written with a
/// copy of its index so the checkout's own index is untouched. With the checkout's git dir.
pub(crate) fn current_tree(checkout: &Path) -> Option<(PathBuf, String)> {
    let git_dir = PathBuf::from(
        git_sync(
            checkout,
            &[
                "-c",
                "core.fsmonitor=false",
                "rev-parse",
                "--absolute-git-dir",
            ],
            &[],
        )
        .ok()?,
    );
    let index = git_dir.join(format!("anysola-{}.index", crate::ids::part()));
    let _ = std::fs::copy(git_dir.join("index"), &index);
    let env = [("GIT_INDEX_FILE", index.as_path())];
    let tree = git_sync(
        checkout,
        &["-c", "core.fsmonitor=false", "add", "-A", "--", "."],
        &env,
    )
    .and_then(|_| {
        git_sync(
            checkout,
            &["-c", "core.fsmonitor=false", "write-tree"],
            &env,
        )
    });
    let _ = std::fs::remove_file(&index);
    Some((git_dir, tree.ok()?))
}

/// Creates a detached worktree of the project checkout at `HEAD` under `base`; `-2`…`-10`
/// suffixes avoid existing directories.
pub async fn create(
    db: &Db,
    paths: &crate::paths::Paths,
    base: &Path,
    project_id: &str,
    root: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    place(paths, base, name)?;
    let source = canonical(root).ok_or("project directory unavailable")?;
    // The checkout's row holds the folder as the project was opened (on Windows maybe an 8.3
    // short name): compare where both really are.
    let known = list(db, project_id)
        .iter()
        .any(|w| canonical(Path::new(&w.directory)).as_ref() == Some(&source));
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
    place(
        paths,
        base,
        &target.file_name().unwrap_or_default().to_string_lossy(),
    )?;
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
    if bound(db, &directory) {
        return Err(RemoveError::Bound);
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

/// Checks, without running git in it, that the worktree `dir` still leads to its own
/// administrative folder in the repository whose top is `main_top`: git run in a worktree reads
/// its config and hooks from wherever its `.git` points, and that file is in a folder the agent
/// writes. Relative pointers (git's `worktree.useRelativePaths`) are read as git reads them.
pub fn verify_checkout(main_top: &Path, dir: &Path) -> Result<(), String> {
    let changed = || {
        "This session's worktree no longer points to its repository (its .git was changed). \
         Discard it, or recreate it."
            .to_string()
    };
    let real = crate::config::real_path;
    let dot_git = dir.join(".git");
    let meta = std::fs::symlink_metadata(&dot_git).map_err(|_| changed())?;
    if !meta.is_file() {
        return Err(changed());
    }
    let text = std::fs::read_to_string(&dot_git).map_err(|_| changed())?;
    let admin = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("gitdir:"))
        .map(str::trim)
        .ok_or_else(changed)?;
    let admin_raw = dir.join(admin);
    let admin_meta = std::fs::symlink_metadata(&admin_raw).map_err(|_| changed())?;
    if !admin_meta.is_dir() || admin_meta.file_type().is_symlink() {
        return Err(changed());
    }
    let admin = real(&admin_raw);
    let common =
        git_sync(main_top, &["rev-parse", "--git-common-dir"], &[]).map_err(|_| changed())?;
    let common = real(&main_top.join(common));
    if admin.parent() != Some(common.join("worktrees").as_path()) {
        return Err(changed());
    }
    // The admin folder names this worktree back (absolute, or relative to the admin folder).
    let back_path = admin.join("gitdir");
    if !std::fs::symlink_metadata(&back_path)
        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
    {
        return Err(changed());
    }
    let back = std::fs::read_to_string(&back_path).map_err(|_| changed())?;
    if real(&admin.join(back.trim())) != real(&dot_git) {
        return Err(changed());
    }
    // And `commondir` still leads back to the repository's own common dir (not a spoofed config).
    let common_path = admin.join("commondir");
    if !std::fs::symlink_metadata(&common_path)
        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
    {
        return Err(changed());
    }
    let commondir = std::fs::read_to_string(&common_path).map_err(|_| changed())?;
    if real(&admin.join(commondir.trim())) != common {
        return Err(changed());
    }
    Ok(())
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

    #[test]
    fn worktrees_stay_out_of_the_folders_definitions_load_from() {
        let paths = crate::paths::Paths::isolated("/home/u".into());
        let data = paths.data.clone();
        let default = data.join("worktree").join("abc");
        assert_eq!(place(&paths, &default, "wt").unwrap(), default.join("wt"));
        for name in ["", ".", "..", "a/b", "a\\b", "/abs", "c:x"] {
            assert!(place(&paths, &default, name).is_err(), "{name:?}");
        }
        for (base, name) in [
            (data.join("agent"), "x"),
            (data.join("skills").join("deep"), "x"),
            (data.clone(), "agent"),
            (data.clone(), "commands"),
            (paths.home.clone(), ".claude"),
            (paths.home.clone(), ".anysola"),
            (paths.home.join(".config"), "opencode"),
            (paths.legacy_config_dir(), "x"),
            (paths.home.join(".agents").join("skills"), "x"),
        ] {
            let refused = place(&paths, &base, name).unwrap_err();
            assert!(
                refused.contains("loads your own"),
                "{base:?} {name}: {refused}"
            );
        }
        // Another spelling of a definition folder is the same folder.
        #[cfg(unix)]
        {
            let tmp = tempfile::tempdir().unwrap();
            let home = crate::paths::canonicalize(tmp.path()).unwrap();
            let paths = crate::paths::Paths::isolated(home.clone());
            std::fs::create_dir_all(paths.data.join("agent")).unwrap();
            std::os::unix::fs::symlink(paths.data.join("agent"), home.join("short")).unwrap();
            let refused = place(&paths, &home.join("short"), "x").unwrap_err();
            assert!(refused.contains("loads your own"), "{refused}");
        }
        // Next to the definition folders is fine.
        assert!(place(&paths, &data, "trees").is_ok());
        assert!(place(&paths, &paths.home, "code").is_ok());
    }

    /// The project opened under another spelling of its folder (a Windows 8.3 short name; here
    /// a symlink) still creates worktrees.
    #[cfg(unix)]
    #[tokio::test]
    async fn create_from_a_project_opened_under_another_spelling() {
        let tmp = tempfile::tempdir().unwrap();
        let root = crate::paths::canonicalize(tmp.path()).unwrap();
        let project = root.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        repo(&project);
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        let db = Db::open(&root.join("db.sqlite")).unwrap();
        let opened = crate::project::identify(&alias);
        db.upsert_project(&opened).unwrap();
        let paths = crate::paths::Paths::isolated(root.join("home"));
        let base = root.join("home/.anysola/worktree/abc");
        let created = create(&db, &paths, &base, &opened.id, &alias, "wt")
            .await
            .unwrap();
        assert!(created.ends_with("wt"));
    }

    #[tokio::test]
    async fn create_list_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let root = crate::paths::canonicalize(tmp.path()).unwrap();
        let project = root.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        repo(&project);
        let base = root.join("home/.anysola/worktree/abc");
        let db = Db::open(&root.join("db.sqlite")).unwrap();
        db.upsert_project(&crate::project::identify(&project))
            .unwrap();
        let id = crate::project::identify(&project).id;

        let paths = crate::paths::Paths::isolated(root.join("home"));
        let first = create(&db, &paths, &base, &id, &project, "wt")
            .await
            .unwrap();
        let second = create(&db, &paths, &base, &id, &project, "wt")
            .await
            .unwrap();
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

    #[tokio::test]
    async fn verify_checkout_rejects_tampered_commondir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = crate::paths::canonicalize(tmp.path()).unwrap();
        let project = root.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        repo(&project);
        let base = root.join("home/.anysola/worktree/abc");
        let db = Db::open(&root.join("db.sqlite")).unwrap();
        let info = crate::project::identify(&project);
        db.upsert_project(&info).unwrap();
        let paths = crate::paths::Paths::isolated(root.join("home"));
        let wt = create(&db, &paths, &base, &info.id, &project, "wt")
            .await
            .unwrap();
        assert!(verify_checkout(&project, &wt).is_ok());
        // Tamper admin/commondir to point at a fake git common directory.
        let fake_common = root.join("fake-git");
        std::fs::create_dir_all(&fake_common).unwrap();
        let dot_git = std::fs::read_to_string(wt.join(".git")).unwrap();
        let admin = wt.join(dot_git.trim().strip_prefix("gitdir:").unwrap().trim());
        std::fs::write(
            admin.join("commondir"),
            format!("{}\n", fake_common.display()),
        )
        .unwrap();
        assert!(verify_checkout(&project, &wt).is_err());
        drop(db);
    }
}
