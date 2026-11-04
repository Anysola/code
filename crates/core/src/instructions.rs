//! System block 2: the Code Mode catalog, instruction files (`AGENTS.md` and configured
//! `instructions`), today's date and the environment block, joined by blank lines.

mod state;

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::paths::Paths;

pub use state::{Observed, Prepared, Source, prepare};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Environment {
    pub directory: PathBuf,
    pub worktree: PathBuf,
    pub git: bool,
}

pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// The `<env>` block as the environment source stores it.
pub fn environment(env: &Environment) -> String {
    let temp = Paths::temp_dir();
    [
        "<env>".to_string(),
        format!("  Working directory: {}", env.directory.display()),
        format!("  Workspace root folder: {}", env.worktree.display()),
        format!("  Is directory a git repo: {}", if env.git { "yes" } else { "no" }),
        format!("  Platform: {}", platform()),
        format!(
            "  Prefer {} over generic system temporary directories such as /tmp; it is pre-created and approved for external access.",
            temp.display()
        ),
        "</env>".into(),
    ]
    .join("\n")
}

fn files_of(value: &Value) -> Option<Vec<InstructionFile>> {
    value
        .as_array()?
        .iter()
        .map(|f| {
            Some(InstructionFile {
                path: f.get("path")?.as_str()?.to_string(),
                content: f.get("content")?.as_str()?.to_string(),
            })
        })
        .collect()
}

fn render(files: &[InstructionFile]) -> String {
    files
        .iter()
        .map(|f| format!("Instructions from: {}\n{}", f.path, f.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_files(value: &Value) -> Option<String> {
    files_of(value).map(|files| render(&files))
}

/// Removed, added, then changed files; a change shows as a diff when that is shorter than the
/// new file.
fn render_files_update(previous: &Value, current: &Value) -> Option<String> {
    let (before, after) = (files_of(previous)?, files_of(current)?);
    let removed = before
        .iter()
        .filter(|b| !after.iter().any(|a| a.path == b.path))
        .map(|f| format!("The instructions from {} no longer apply.", f.path));
    let added = after
        .iter()
        .filter(|a| !before.iter().any(|b| b.path == a.path))
        .map(|f| {
            format!(
                "New instructions apply from:\n{}",
                render(std::slice::from_ref(f))
            )
        });
    let changed = after.iter().filter_map(|a| {
        let b = before
            .iter()
            .find(|b| b.path == a.path && b.content != a.content)?;
        let hunks = crate::tool::edit::diff::hunks_text(&b.content, &a.content, 3);
        let diff = format!(
            "The instructions from {} changed. Here's the diff:\n```diff\n{}\n```",
            a.path,
            hunks.trim_end()
        );
        let replacement = format!(
            "The instructions changed:\n{}",
            render(std::slice::from_ref(a))
        );
        Some(if diff.chars().count() < replacement.chars().count() {
            diff
        } else {
            replacement
        })
    });
    Some(
        removed
            .chain(added)
            .chain(changed)
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

fn text(value: &Value) -> Option<&str> {
    value.as_str()
}

fn render_catalog(value: &Value) -> Option<String> {
    let summary: crate::codemode::catalog::Summary = serde_json::from_value(value.clone()).ok()?;
    Some(crate::codemode::catalog::render(&summary))
}

fn observed<T>(value: Option<T>, encode: impl FnOnce(T) -> Value) -> Observed {
    value.map_or(Observed::Removed, |v| Observed::Value(encode(v)))
}

/// Block 2's sources in 0.5.1's order (most shared first, so the baseline stays a reusable
/// prompt-cache prefix): the Code Mode catalog (its summary), skill guidance, instruction files,
/// the date, the environment. A changed catalog or skill listing is shown whole again.
pub fn sources(
    catalog: Option<&crate::codemode::catalog::Summary>,
    skills: Option<String>,
    files: Observed,
    date: &str,
    env: &Environment,
) -> Vec<Source> {
    vec![
        Source {
            key: "core/codemode",
            observed: observed(catalog, |s| serde_json::to_value(s).unwrap_or_default()),
            initial: render_catalog,
            changed: |_, v| render_catalog(v),
            removed: |_| {
                Some("Code Mode tools are no longer available. Do not use any previously listed Code Mode tools.".into())
            },
        },
        Source {
            key: "core/skill-guidance",
            observed: observed(skills, Value::String),
            initial: |v| text(v).map(str::to_string),
            changed: |_, v| text(v).map(str::to_string),
            removed: state::never,
        },
        Source {
            key: "core/instructions",
            observed: files,
            initial: render_files,
            changed: render_files_update,
            removed: |_| Some("Previously loaded instructions no longer apply.".into()),
        },
        Source {
            key: "core/date",
            observed: Observed::Value(Value::String(date.to_string())),
            initial: |v| text(v).map(|d| format!("Today's date: {d}")),
            changed: |_, v| text(v).map(|d| format!("Today's date is now: {d}")),
            removed: state::never,
        },
        Source {
            key: "core/environment",
            observed: Observed::Value(Value::String(environment(env))),
            initial: |v| {
                text(v).map(|e| {
                    format!("Here is some useful information about the environment you are running in:\n{e}")
                })
            },
            changed: |_, v| {
                text(v).map(|e| format!("The environment you are running in is now:\n{e}"))
            },
            removed: state::never,
        },
    ]
}

fn has_glob(text: &str) -> bool {
    text.contains(['*', '?', '[', '{'])
}

/// Adds the file unless it is missing or already listed. `Err` when it exists but cannot be read.
fn push(out: &mut Vec<InstructionFile>, path: &Path) -> Result<(), ()> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return Err(()),
        Err(_) => return Ok(()),
    };
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !out.iter().any(|f| {
        std::fs::canonicalize(&f.path).unwrap_or_else(|_| PathBuf::from(&f.path)) == canonical
    }) {
        out.push(InstructionFile {
            path: path.display().to_string(),
            content,
        });
    }
    Ok(())
}

fn glob_files(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let Ok(matcher) = globset::Glob::new(pattern).map(|g| g.compile_matcher()) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = ignore::WalkBuilder::new(base)
        .standard_filters(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| {
            e.path()
                .strip_prefix(base)
                .is_ok_and(|rel| matcher.is_match(rel))
        })
        .map(|e| e.into_path())
        .collect();
    found.sort();
    found
}

fn is_url(entry: &str) -> bool {
    entry.starts_with("http://") || entry.starts_with("https://")
}

/// The configured `instructions` entries that are URLs, fetched once when the config loads.
pub async fn fetch_remote(
    gateway: &crate::gateway::Gateway,
    configured: &[String],
) -> Vec<InstructionFile> {
    let urls: Vec<&String> = configured.iter().filter(|e| is_url(e)).collect();
    let fetched = futures_util::stream::iter(urls)
        .map(|url| async move {
            let content = gateway
                .fetch_text(url, std::time::Duration::from_secs(5))
                .await;
            if content.is_none() {
                crate::log::warn("failed to fetch instruction url", &[("url", url)]);
            }
            content.map(|content| InstructionFile {
                path: url.clone(),
                content,
            })
        })
        .buffered(4)
        .collect::<Vec<_>>()
        .await;
    fetched.into_iter().flatten().collect()
}

/// `AGENTS.md` files (global, then nearest project file first within the discovery bound) and the
/// configured `instructions` entries (`remote` holds the fetched URLs). An unreadable project
/// `AGENTS.md` makes the whole set unavailable.
pub fn discover(
    paths: &Paths,
    directory: &Path,
    worktree: &Path,
    configured: &[String],
    remote: &[InstructionFile],
) -> Observed {
    let mut out = Vec::new();
    let _ = push(&mut out, &paths.legacy_config_dir().join("AGENTS.md"));
    let _ = push(&mut out, &paths.data.join("AGENTS.md"));
    if directory.starts_with(worktree) || directory.starts_with(&paths.home) {
        for dir in crate::config::upward(directory, &paths.home) {
            if push(&mut out, &dir.join("AGENTS.md")).is_err() {
                return Observed::Unavailable;
            }
        }
    }
    for entry in configured {
        if is_url(entry) {
            out.extend(remote.iter().filter(|f| &f.path == entry).cloned());
            continue;
        }
        let expanded = paths.expand_home(entry);
        let mut found = Vec::new();
        if expanded.is_absolute() {
            if has_glob(entry) {
                let text = expanded.display().to_string();
                let root =
                    Path::new(&text[..text.find(['*', '?', '[', '{']).unwrap_or(text.len())])
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                let pattern = expanded
                    .strip_prefix(&root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                found = glob_files(&root, &pattern);
            } else {
                found.push(expanded);
            }
        } else {
            for dir in directory.ancestors() {
                if has_glob(entry) {
                    found.extend(glob_files(dir, entry));
                } else if dir.join(entry).is_file() {
                    found.push(dir.join(entry));
                }
                if dir == worktree {
                    break;
                }
            }
            found.sort();
        }
        for path in found {
            let _ = push(&mut out, &path);
        }
    }
    if out.is_empty() {
        return Observed::Removed;
    }
    Observed::Value(Value::Array(
        out.into_iter()
            .map(|f| json!({"path": f.path, "content": f.content}))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_contents(observed: &Observed) -> Vec<String> {
        match observed {
            Observed::Value(value) => files_of(value)
                .unwrap()
                .into_iter()
                .map(|f| f.content)
                .collect(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn discovery_order() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        std::fs::create_dir_all(project.join("sub")).unwrap();
        std::fs::create_dir_all(project.join("docs")).unwrap();
        std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
        std::fs::create_dir_all(home.join(".anysola")).unwrap();
        std::fs::write(home.join(".config/opencode/AGENTS.md"), "global\n").unwrap();
        std::fs::write(home.join(".anysola/AGENTS.md"), "anysola\n").unwrap();
        std::fs::write(project.join("AGENTS.md"), "project\n").unwrap();
        std::fs::write(project.join("sub/AGENTS.md"), "sub\n").unwrap();
        std::fs::write(project.join("docs/rules.md"), "rules\n").unwrap();
        let paths = Paths::isolated(home.clone());
        let remote = [InstructionFile {
            path: "https://example.com/rules.md".into(),
            content: "remote\n".into(),
        }];
        let files = discover(
            &paths,
            &project.join("sub"),
            &project,
            &[
                "docs/rules.md".into(),
                "https://example.com/rules.md".into(),
                "https://example.com/down.md".into(),
            ],
            &remote,
        );
        assert_eq!(
            file_contents(&files),
            vec![
                "global\n",
                "anysola\n",
                "sub\n",
                "project\n",
                "rules\n",
                "remote\n"
            ]
        );
        let empty = tempfile::tempdir().unwrap();
        let nothing = discover(
            &Paths::isolated(empty.path().into()),
            empty.path(),
            empty.path(),
            &[],
            &[],
        );
        assert_eq!(nothing, Observed::Removed);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_project_file_is_unavailable() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("AGENTS.md");
        std::fs::write(&file, "secret").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&file).is_ok() {
            return; // running as root
        }
        let paths = Paths::isolated(root.path().join("home"));
        assert_eq!(
            discover(&paths, root.path(), root.path(), &[], &[]),
            Observed::Unavailable
        );
    }

    #[test]
    fn file_updates() {
        let files = |list: &[(&str, &str)]| {
            Value::Array(
                list.iter()
                    .map(|(p, c)| json!({"path": p, "content": c}))
                    .collect(),
            )
        };
        let long: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let edited = long.replace("line 15\n", "line fifteen\n");
        let before = files(&[("/a", &long), ("/gone", "x\n")]);
        let after = files(&[("/a", &edited), ("/new", "n\n")]);
        assert_eq!(
            render_files_update(&before, &after).unwrap(),
            "The instructions from /gone no longer apply.\n\nNew instructions apply from:\nInstructions from: /new\nn\n\n\nThe instructions from /a changed. Here's the diff:\n```diff\n@@ -12,7 +12,7 @@\n line 12\n line 13\n line 14\n-line 15\n+line fifteen\n line 16\n line 17\n line 18\n```"
        );
        let short =
            render_files_update(&files(&[("/a", "x\n")]), &files(&[("/a", "y\n")])).unwrap();
        assert_eq!(
            short,
            "The instructions changed:\nInstructions from: /a\ny\n"
        );
    }
}
