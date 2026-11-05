//! Path resolution and file permissions shared by the file and search tools.
//!
//! A path inside the session directory (or inside the project worktree, unless that is a filesystem
//! root) is internal: its permission resource is the directory-relative path with `/` separators.
//! Anything else is external: the resource is the absolute path, and the call first needs the
//! `external_directory` permission for `<dir>/*`.

use std::path::{Path, PathBuf};

use crate::permission::PermissionRequest;
use crate::project;

use super::{ToolContext, ToolError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct External {
    pub directory: PathBuf,
    pub resource: String,
    pub save: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub absolute: PathBuf,
    /// Directory-relative for internal paths, absolute for external ones (always `/`-separated).
    pub resource: String,
    pub external: Option<External>,
}

/// Rewrites MSYS/Cygwin/WSL drive paths (`/c/x`, `/cygdrive/c/x`, `/mnt/c/x`) to `C:/x` on Windows.
pub fn windows_path(input: &str) -> String {
    if cfg!(windows) {
        drive_path(input)
    } else {
        input.to_string()
    }
}

fn drive_path(input: &str) -> String {
    for prefix in ["/cygdrive/", "/mnt/", "/"] {
        let Some(rest) = input.strip_prefix(prefix) else {
            continue;
        };
        let mut chars = rest.chars();
        let Some(drive) = chars.next().filter(char::is_ascii_alphabetic) else {
            continue;
        };
        let tail = chars.as_str();
        let tail = if prefix == "/" {
            tail.strip_prefix(':').unwrap_or(tail)
        } else {
            tail
        };
        if tail.is_empty() || tail.starts_with('/') || tail.starts_with('\\') {
            let tail = tail.trim_start_matches(['/', '\\']);
            return format!("{}:/{}", drive.to_ascii_uppercase(), tail);
        }
    }
    input.to_string()
}

/// Expands `~` and resolves against `directory` (lexically, like `path.resolve`).
pub fn resolve_path(ctx: &ToolContext, input: &str) -> PathBuf {
    resolve_path_at(ctx, &ctx.directory, input)
}

fn resolve_path_at(ctx: &ToolContext, base: &Path, input: &str) -> PathBuf {
    let normalized = windows_path(input);
    let expanded = ctx.paths.expand_home(&normalized);
    crate::paths::normalize(&base.join(expanded))
}

pub fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn contains(parent: &Path, child: &Path) -> bool {
    child.starts_with(parent)
}

pub fn is_internal(ctx: &ToolContext, absolute: &Path) -> bool {
    let worktree = &ctx.worktree;
    contains(&ctx.directory, absolute)
        || (worktree.parent().is_some() && contains(worktree, absolute))
}

pub fn resolve(ctx: &ToolContext, input: &str, kind: Option<Kind>) -> Target {
    resolve_at(ctx, &ctx.directory, input, kind)
}

/// [`resolve`] with relative paths taken from `base` instead of the session directory.
pub fn resolve_at(ctx: &ToolContext, base: &Path, input: &str, kind: Option<Kind>) -> Target {
    let absolute = resolve_path_at(ctx, base, input);
    if is_internal(ctx, &absolute) {
        let relative = absolute
            .strip_prefix(&ctx.directory)
            .ok()
            .map(slash)
            .filter(|r| !r.is_empty());
        let resource = match relative {
            Some(relative) => relative,
            None if absolute == ctx.directory => ".".to_string(),
            None => relative_path(&ctx.directory, &absolute),
        };
        return Target {
            absolute,
            resource,
            external: None,
        };
    }
    let kind = kind.or_else(|| match std::fs::metadata(&absolute) {
        Ok(meta) if meta.is_dir() => Some(Kind::Directory),
        Ok(_) => Some(Kind::File),
        Err(_) => None,
    });
    let directory = if kind == Some(Kind::Directory) {
        absolute.clone()
    } else {
        absolute
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| absolute.clone())
    };
    let root = project::vcs_root(&directory).unwrap_or_else(|| directory.clone());
    Target {
        resource: slash(&absolute),
        external: Some(External {
            resource: slash(&directory.join("*")),
            save: slash(&root.join("*")),
            directory,
        }),
        absolute,
    }
}

/// `path.relative(from, to)` for lexically normalized absolute paths.
pub fn relative_path(from: &Path, to: &Path) -> String {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_string(), from.len() - common).collect();
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// Asks `external_directory` once for every distinct external directory among `targets`.
pub async fn authorize_external(ctx: &ToolContext, targets: &[&Target]) -> Result<(), ToolError> {
    let mut resources: Vec<String> = Vec::new();
    let mut save = Vec::new();
    for external in targets.iter().filter_map(|t| t.external.as_ref()) {
        if !resources.contains(&external.resource) {
            resources.push(external.resource.clone());
            save.push(external.save.clone());
        }
    }
    if resources.is_empty() {
        return Ok(());
    }
    ctx.ask(PermissionRequest {
        action: "external_directory".into(),
        resources,
        save,
        ..Default::default()
    })
    .await
}

/// Resolves a path to read, asking `external_directory` (when outside) and then `read`.
pub async fn authorize_read(ctx: &ToolContext, input: &str) -> Result<Target, ToolError> {
    let target = resolve(ctx, input, None);
    authorize_external(ctx, &[&target]).await?;
    ctx.ask(PermissionRequest {
        action: "read".into(),
        resources: vec![target.resource.clone()],
        save: vec!["*".into()],
        ..Default::default()
    })
    .await?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::test_support::context;

    #[test]
    fn drive_paths() {
        for (input, expected) in [
            ("/c/x/y", "C:/x/y"),
            ("/c:/x", "C:/x"),
            ("/cygdrive/d/x", "D:/x"),
            ("/mnt/c/x\\y", "C:/x\\y"),
            ("/c", "C:/"),
            ("/cd/x", "/cd/x"),
            ("/mnt/data/x", "/mnt/data/x"),
            ("relative/c", "relative/c"),
        ] {
            assert_eq!(drive_path(input), expected, "{input}");
        }
    }

    #[test]
    fn internal_and_external_targets() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        let sub = project.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let test = context(&sub);
        let mut ctx = test.ctx.clone();
        ctx.worktree = project.clone();

        let home = resolve(&ctx, "~/notes.txt", None);
        assert_eq!(home.absolute, sub.join(".home/notes.txt"));
        assert_eq!(home.resource, ".home/notes.txt");
        assert_eq!(resolve(&ctx, ".", None).resource, ".");
        assert_eq!(resolve(&ctx, "a/../b.txt", None).resource, "b.txt");
        // Inside the worktree but outside the session directory: still internal, relative.
        let sibling = resolve(&ctx, "../other/x.txt", None);
        assert_eq!(
            (sibling.resource.as_str(), sibling.external),
            ("../other/x.txt", None)
        );

        let outside = dir.path().join("elsewhere");
        let repo = outside.join("repo");
        std::fs::create_dir_all(repo.join("deep")).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        let file = resolve(&ctx, &slash(&repo.join("deep/f.txt")), None);
        let external = file.external.unwrap();
        assert_eq!(file.resource, slash(&repo.join("deep/f.txt")));
        assert_eq!(external.resource, slash(&repo.join("deep/*")));
        assert_eq!(external.save, slash(&repo.join("*")));
        let folder = resolve(&ctx, &slash(&outside), Some(Kind::Directory));
        let external = folder.external.unwrap();
        assert_eq!(
            (external.resource, external.save),
            (slash(&outside.join("*")), slash(&outside.join("*")))
        );
    }

    #[tokio::test]
    async fn external_directories_are_asked_once_per_directory() {
        let dir = tempfile::tempdir().unwrap();
        let test = context(&dir.path().join("p"));
        let outside = dir.path().join("o");
        let a = resolve(&test.ctx, &slash(&outside.join("a")), Some(Kind::File));
        let b = resolve(&test.ctx, &slash(&outside.join("b")), Some(Kind::File));
        let inside = resolve(&test.ctx, "c", None);
        authorize_external(&test.ctx, &[&a, &inside, &b])
            .await
            .unwrap();
        let asked = test.gate.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].action, "external_directory");
        assert_eq!(asked[0].resources, vec![slash(&outside.join("*"))]);
        assert_eq!(asked[0].save, vec![slash(&outside.join("*"))]);
    }
}
