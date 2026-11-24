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
    /// The same for the folder the path really is in, when a symlink makes that another one:
    /// `(resource, save)`.
    pub real: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub absolute: PathBuf,
    /// Directory-relative for internal paths, absolute for external ones (always `/`-separated).
    pub resource: String,
    pub external: Option<External>,
    /// The path's other names a rule may use: relative to the project's root and to the root of
    /// the checkout the session works in, where those contain it and differ from `resource`.
    pub also: Vec<String>,
}

impl Target {
    /// Every name of the path, for a permission request: a rule on any of them decides (the
    /// gate takes the strictest), so moving the session never stops a rule from matching.
    pub fn resources(&self) -> Vec<String> {
        std::iter::once(self.resource.clone())
            .chain(self.also.iter().cloned())
            .collect()
    }
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

/// The places a path's permission names are taken from: the session directory, the root of the
/// checkout the session works in (the project's root, or its own worktree), the root of the
/// checkout the session directory is in (a worktree, or the project itself for a subfolder),
/// and the project's main root. Only the first two hold what is *inside*: for a session pinned
/// to its own worktree the main folder is outside, but a rule written relative to the project
/// still names its files.
struct Places {
    directory: PathBuf,
    worktree: PathBuf,
    checkout: Option<PathBuf>,
    main: PathBuf,
    /// Where sessions' own worktrees live (`<data>/worktrees`): a path in one is also named
    /// relative to that worktree's top, so a root-relative rule names its files too.
    sessions: PathBuf,
}

impl Places {
    /// As written (lexically).
    fn written(ctx: &ToolContext) -> Places {
        Places {
            directory: ctx.directory.clone(),
            worktree: ctx.worktree.clone(),
            checkout: project::vcs_root(&ctx.directory),
            main: ctx.main.clone(),
            sessions: ctx.paths.data.join("worktrees"),
        }
    }

    /// Where they really are, symlinks resolved.
    fn real(ctx: &ToolContext) -> Places {
        let real = crate::config::real_path;
        let written = Places::written(ctx);
        Places {
            directory: real(&written.directory),
            worktree: real(&written.worktree),
            checkout: written.checkout.as_deref().map(real),
            main: real(&written.main),
            sessions: real(&written.sessions),
        }
    }

    /// The top of the session worktree holding `path` (`<sessions>/<project>/<session>`).
    fn session_top(&self, path: &Path) -> Option<PathBuf> {
        let rest = path.strip_prefix(&self.sessions).ok()?;
        let mut parts = rest.components();
        let (project, session) = (parts.next()?, parts.next()?);
        Some(self.sessions.join(project).join(session))
    }

    fn inside(&self, path: &Path) -> bool {
        contains(&self.directory, path)
            || (self.worktree.parent().is_some() && contains(&self.worktree, path))
    }

    /// The path's names: directory-relative inside, absolute outside, then relative to each
    /// checkout root that holds it.
    fn names(&self, path: &Path) -> Vec<String> {
        let first = if self.inside(path) {
            match path.strip_prefix(&self.directory).ok().map(slash) {
                Some(relative) if !relative.is_empty() => relative,
                _ if path == self.directory => ".".to_string(),
                _ => relative_path(&self.directory, path),
            }
        } else {
            slash(path)
        };
        let mut out = vec![first];
        let session_top = self.session_top(path);
        let roots = std::iter::once(&self.worktree)
            .chain(self.checkout.as_ref())
            .chain(std::iter::once(&self.main))
            .chain(session_top.as_ref());
        for root in roots {
            if root.parent().is_none() {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let name = match slash(relative) {
                empty if empty.is_empty() => ".".to_string(),
                name => name,
            };
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }
}

pub fn is_internal(ctx: &ToolContext, absolute: &Path) -> bool {
    Places::written(ctx).inside(absolute)
}

pub fn resolve(ctx: &ToolContext, input: &str, kind: Option<Kind>) -> Target {
    resolve_at(ctx, &ctx.directory, input, kind)
}

/// [`resolve`] with relative paths taken from `base` instead of the session directory.
///
/// Decisions are made on the file the path really reaches (symlinks resolved, also of a file
/// not created yet or a dangling link, so `/proc/self/root/…` and in-repository links name what
/// they point at): it is inside or outside the project as that file is, and the target carries
/// that file's names besides those of the path as written, so a rule on either decides.
pub fn resolve_at(ctx: &ToolContext, base: &Path, input: &str, kind: Option<Kind>) -> Target {
    let absolute = resolve_path_at(ctx, base, input);
    let real = crate::config::real_path(&absolute);
    let real_places = Places::real(ctx);
    let mut names = Places::written(ctx).names(&absolute);
    let resource = names.remove(0);
    let mut also = names;
    let mut real_names = real_places.names(&real);
    if !real_places.inside(&real) {
        // Rules written with `~` are expanded against the home folder as configured.
        let home = crate::config::real_path(&ctx.paths.home);
        if let Ok(rest) = real.strip_prefix(&home) {
            real_names.push(slash(&ctx.paths.home.join(rest)));
        }
    }
    for name in real_names {
        if name != resource && !also.contains(&name) {
            also.push(name);
        }
    }
    let real_inside = real_places.inside(&real);
    let written_inside = is_internal(ctx, &absolute);
    if real_inside && written_inside {
        return Target {
            absolute,
            resource,
            external: None,
            also,
        };
    }
    let kind = kind.or_else(|| match std::fs::metadata(&real) {
        Ok(meta) if meta.is_dir() => Some(Kind::Directory),
        Ok(_) => Some(Kind::File),
        Err(_) => None,
    });
    let folder = |path: &Path| {
        let directory = if kind == Some(Kind::Directory) {
            path.to_path_buf()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.to_path_buf())
        };
        let root = project::vcs_root(&directory).unwrap_or_else(|| directory.clone());
        (directory, slash(&root.join("*")))
    };
    let (real_directory, real_save) = folder(&real);
    let real_resource = slash(&real_directory.join("*"));
    // A path written outside is asked about with its folder's name as in 1.1.0, even when it
    // really leads into the project (so the user's rules and saved answers still match); the
    // folder it really reaches is asked about too when that is outside and differs.
    let external = if written_inside {
        External {
            directory: real_directory,
            resource: real_resource,
            save: real_save,
            real: None,
        }
    } else {
        let (directory, save) = folder(&absolute);
        let resource = slash(&directory.join("*"));
        let real =
            (!real_inside && resource != real_resource).then_some((real_resource, real_save));
        External {
            directory,
            resource,
            save,
            real,
        }
    };
    Target {
        also,
        resource,
        external: Some(external),
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

/// The sentence a pinned session gets for a write outside its worktree that would land in the
/// main folder or another session's worktree.
pub const FOREIGN_WRITE: &str =
    "This session works in its own worktree; apply its changes to bring them to the main folder.";

/// A session pinned to its own worktree never writes into the main folder or another session's
/// worktree, in any mode and whatever was saved for a parent folder: such a write is refused
/// before anything is asked. Reads are not affected, and other outside folders are asked about
/// as usual. Both the path as written and the file it really reaches are checked, against both
/// spellings of each root (symlinks, Windows short names).
pub fn refuse_foreign_write(ctx: &ToolContext, target: &Target) -> Result<(), ToolError> {
    if !ctx.pinned() || target.external.is_none() {
        return Ok(());
    }
    let real = crate::config::real_path;
    let written = Places::written(ctx);
    let places = Places::real(ctx);
    let paths = [target.absolute.clone(), real(&target.absolute)];
    let mains = [written.main.clone(), places.main.clone()];
    let own = [written.worktree.clone(), places.worktree.clone()];
    let bases = [written.sessions.clone(), places.sessions.clone()];
    let foreign = paths.iter().any(|path| {
        let in_main = mains
            .iter()
            .any(|m| m.parent().is_some() && contains(m, path));
        let in_own = own.iter().any(|w| contains(w, path));
        let in_other = !in_own && bases.iter().any(|b| contains(b, path));
        (in_main && !in_own) || in_other
    });
    if foreign {
        return Err(ToolError {
            kind: super::ToolErrorKind::Blocked,
            message: FOREIGN_WRITE.into(),
        });
    }
    Ok(())
}

/// Asks `external_directory` once for every distinct external directory among `targets`.
pub async fn authorize_external(ctx: &ToolContext, targets: &[&Target]) -> Result<(), ToolError> {
    let mut resources: Vec<String> = Vec::new();
    let mut save = Vec::new();
    for external in targets.iter().filter_map(|t| t.external.as_ref()) {
        let pairs = std::iter::once((&external.resource, &external.save))
            .chain(external.real.as_ref().map(|(r, s)| (r, s)));
        for (resource, saved) in pairs {
            if !resources.contains(resource) {
                resources.push(resource.clone());
                save.push(saved.clone());
            }
        }
    }
    if resources.is_empty() {
        return Ok(());
    }
    // A session in its own worktree asks each time for the main folder: an "always" saved for
    // the project would let every such session write there unasked from then on.
    if ctx.pinned() {
        let main = crate::config::real_path(&ctx.main);
        let in_main = targets.iter().filter_map(|t| t.external.as_ref()).any(|e| {
            e.directory.starts_with(&ctx.main)
                || crate::config::real_path(&e.directory).starts_with(&main)
        });
        if in_main {
            save.clear();
        }
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
        resources: target.resources(),
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
        ctx.main = project.clone();

        let home = resolve(&ctx, "~/notes.txt", None);
        assert_eq!(home.absolute, sub.join(".home/notes.txt"));
        assert_eq!(home.resource, ".home/notes.txt");
        assert_eq!(resolve(&ctx, ".", None).resource, ".");
        assert_eq!(resolve(&ctx, "a/../b.txt", None).resource, "b.txt");
        // Inside the worktree but outside the session directory: still internal, relative.
        let sibling = resolve(&ctx, "../other/x.txt", None);
        assert_eq!(
            (sibling.resource.as_str(), sibling.external.clone()),
            ("../other/x.txt", None)
        );
        // Its name from the project's root too, so a root-relative rule still names it.
        assert_eq!(sibling.resources(), ["../other/x.txt", "other/x.txt"]);
        assert_eq!(
            resolve(&ctx, "y.txt", None).resources(),
            ["y.txt", "sub/y.txt"]
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
        // As written, and as the file system spells it when that differs (Windows short names).
        let folder = crate::tool::test_support::folder_spellings(&outside);
        assert_eq!(asked[0].resources, folder);
        assert_eq!(asked[0].save, folder);
    }
}
