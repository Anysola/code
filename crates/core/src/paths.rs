//! Where Anysola Code keeps its files: everything lives under `~/.anysola/`. The old OpenCode XDG
//! locations are only read (global config, import sources).

use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    /// `~/.anysola`
    pub data: PathBuf,
    /// The 0.5.x global config directory, still read: `OPENCODE_CONFIG_DIR`, else
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
            if let Some(dir) = env::var_os("OPENCODE_CONFIG_DIR").filter(|v| !v.is_empty()) {
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

    /// Plan files stay where 0.5.1 named them in the plan-mode prompt.
    /// Checkouts of git `references`.
    pub fn repos_dir(&self) -> PathBuf {
        self.data.join("repos")
    }

    pub fn plan_dir(&self) -> PathBuf {
        self.home.join(".opencode").join("plan")
    }

    /// `<os tmp>/opencode`, named in the system prompt as approved scratch space.
    pub fn temp_dir() -> PathBuf {
        env::temp_dir().join("opencode")
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
