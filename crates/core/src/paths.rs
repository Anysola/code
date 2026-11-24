//! Where Anysola Code keeps its files: everything lives under `~/.anysola/`, which is also the
//! global config directory (`config.json`, `AGENTS.md`, `agent/`, `command/`, `skills/`, `themes/`).
//! The old OpenCode XDG locations are only read (global config, import sources).

use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    /// `~/.anysola`
    pub data: PathBuf,
    /// The 0.5.x global config directory, still read: `ANYSOLA_CONFIG_DIR` (or `OPENCODE_…`), else
    /// `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`.
    pub legacy_config: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let home = home_dir();
        Paths::under(home)
    }

    /// The layout for a given home, with the legacy config dir taken from the environment.
    pub fn under(home: PathBuf) -> Self {
        let data = home.join(".anysola");
        let legacy_config =
            if let Some(dir) = crate::env::var_os("CONFIG_DIR").filter(|v| !v.is_empty()) {
                PathBuf::from(dir)
            } else {
                match env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
                    Some(dir) => PathBuf::from(dir).join("opencode"),
                    None => home.join(".config").join("opencode"),
                }
            };
        Paths {
            home,
            data,
            legacy_config,
        }
    }

    /// The layout for tests: everything under `home`.
    pub fn isolated(home: PathBuf) -> Self {
        Paths {
            data: home.join(".anysola"),
            legacy_config: home.join(".config").join("opencode"),
            home,
        }
    }

    pub fn database(&self) -> PathBuf {
        self.data.join("anysola.db")
    }

    pub fn config_file(&self) -> PathBuf {
        self.data.join("config.json")
    }

    /// The desktop app's own preferences.
    pub fn desktop_prefs(&self) -> PathBuf {
        self.data.join("desktop.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data.join("log").join("anysola.log")
    }

    pub fn shell_dir(&self, project_id: &str) -> PathBuf {
        self.data.join("shell").join(project_id)
    }

    pub fn tool_output_dir(&self) -> PathBuf {
        self.data.join("tool-output")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.data.join("state")
    }

    pub fn legacy_config_dir(&self) -> PathBuf {
        self.legacy_config.clone()
    }

    /// Checkouts of git `references`.
    pub fn repos_dir(&self) -> PathBuf {
        self.data.join("repos")
    }

    /// Where plan mode writes plans: `~/.anysola/plan`.
    pub fn plan_dir(&self) -> PathBuf {
        self.data.join("plan")
    }

    /// Where 0.5.x and 1.0.0 wrote plans; plan mode may still edit them.
    pub fn legacy_plan_dir(&self) -> PathBuf {
        self.home.join(".opencode").join("plan")
    }

    /// `<os tmp>/anysola`, named in the system prompt as approved scratch space.
    pub fn temp_dir() -> PathBuf {
        env::temp_dir().join("anysola")
    }

    /// Expands `~` and `~/…` (and `~\…` on Windows) against home.
    pub fn expand_home(&self, input: &str) -> PathBuf {
        if input == "~" {
            return self.home.clone();
        }
        let rest = input.strip_prefix("~/").or_else(|| {
            if cfg!(windows) {
                input.strip_prefix("~\\")
            } else {
                None
            }
        });
        match rest {
            Some(rest) => self.home.join(rest),
            None => PathBuf::from(input),
        }
    }
}

fn home_dir() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// [`std::fs::canonicalize`] without the Windows verbatim prefix (`\\?\C:\…` → `C:\…`,
/// `\\?\UNC\h\s` → `\\h\s`), whatever its length: git can't take verbatim paths (`worktree add`
/// fails on them), nor can shells and users read them. Rust's own file calls put the prefix
/// back for an over-long path, and git runs with `core.longpaths` for the worktrees.
pub fn canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    Ok(plain(&std::fs::canonicalize(path)?))
}

/// A path without Windows' verbatim prefix (`\\?\C:\x` → `C:\x`, `\\?\UNC\h\s` → `\\h\s`),
/// whatever its length, so it compares with and reads like the paths users write.
pub fn plain(path: &Path) -> PathBuf {
    PathBuf::from(plain_text(&path.to_string_lossy()))
}

pub(crate) fn plain_text(text: &str) -> String {
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
        _ => text.to_string(),
    }
}

/// The most read from a markdown or instruction file (agents, commands, skills, `AGENTS.md`).
pub const TEXT_CAP: u64 = 4 << 20;
/// The most read from a project's config document.
pub const DOCUMENT_CAP: u64 = 1 << 20;

/// Why [`read_text`] skipped a file.
#[derive(Debug)]
pub enum Unread {
    /// Not a regular file after following symlinks (a device, a pipe, a folder).
    NotAFile,
    TooLarge,
    Io(std::io::Error),
}

/// A text file that must be a regular file (symlinks followed) of at most `cap` bytes: a
/// repository's `AGENTS.md` linked to `/dev/zero` must not exhaust memory.
pub fn read_text(path: &Path, cap: u64) -> Result<String, Unread> {
    use std::io::Read;
    let meta = std::fs::metadata(path).map_err(Unread::Io)?;
    if !meta.is_file() {
        return Err(Unread::NotAFile);
    }
    if meta.len() > cap {
        return Err(Unread::TooLarge);
    }
    let file = std::fs::File::open(path).map_err(Unread::Io)?;
    let mut bytes = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(Unread::Io)?;
    if bytes.len() as u64 > cap {
        return Err(Unread::TooLarge);
    }
    String::from_utf8(bytes)
        .map_err(|e| Unread::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
}

/// [`read_text`] with [`TEXT_CAP`]; a file it skips for its type or size is logged once.
pub fn read_markdown(path: &Path) -> Option<String> {
    match read_text(path, TEXT_CAP) {
        Ok(text) => Some(text),
        Err(Unread::Io(_)) => None,
        Err(reason) => {
            crate::log::warn(
                "skipped a file that is not a regular file of at most 4 MiB",
                &[
                    ("path", &path.display().to_string()),
                    ("reason", &format!("{reason:?}")),
                ],
            );
            None
        }
    }
}

/// Lexically normalizes `.` and `..` without touching the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Node's `path.join(base, rest)`: joined, then `.`/`..` and repeated separators resolved, a trailing
/// separator kept.
pub fn node_join(base: &str, rest: &str) -> String {
    if cfg!(windows) && !base.starts_with('/') {
        return normalize(&Path::new(base).join(rest)).display().to_string();
    }
    let joined = if rest.is_empty() {
        base.to_string()
    } else if base.is_empty() {
        rest.to_string()
    } else {
        format!("{base}/{rest}")
    };
    let absolute = joined.starts_with('/');
    let trailing = joined.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut out = parts.join("/");
    if absolute {
        out.insert(0, '/');
    }
    if out.is_empty() {
        out.push('.');
    }
    if trailing && !out.ends_with('/') {
        out.push('/');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_prefixes_go_whatever_the_length() {
        let long = format!(r"\\?\C:\{}\x", "d".repeat(300));
        assert_eq!(plain_text(&long), &long[4..]);
        assert_eq!(plain_text(r"\\?\C:\Users\u"), r"C:\Users\u");
        assert_eq!(plain_text(r"\\?\UNC\host\share\x"), r"\\host\share\x");
        assert_eq!(plain_text(r"\\?\Volume{abc}\x"), r"\\?\Volume{abc}\x");
        assert_eq!(plain_text("/home/u/x"), "/home/u/x");
    }
}
