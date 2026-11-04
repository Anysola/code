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
    if !cfg!(windows) {
        return input.to_string();
    }
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
    let normalized = windows_path(input);
    let expanded = ctx.paths.expand_home(&normalized);
    crate::paths::normalize(&ctx.directory.join(expanded))
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
    let absolute = resolve_path(ctx, input);
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
