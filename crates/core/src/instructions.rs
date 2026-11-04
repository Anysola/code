//! System block 2: the Code Mode catalog, instruction files (`AGENTS.md` and configured
//! `instructions`), today's date and the environment block, joined by blank lines.

use std::path::{Path, PathBuf};

use crate::paths::Paths;

/// The default Code Mode catalog (the `opencode` namespace).
// M1b: rendered from the Code Mode tool inventory (MCP tools, budget rules) instead of a constant.
pub const CODE_MODE_CATALOG: &str = include_str!("prompt/codemode-catalog.txt");

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

pub fn environment_block(env: &Environment) -> String {
    let temp = Paths::temp_dir();
    [
        "Here is some useful information about the environment you are running in:".to_string(),
        "<env>".into(),
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

fn render(files: &[InstructionFile]) -> String {
    files
        .iter()
        .map(|f| format!("Instructions from: {}\n{}", f.path, f.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Joined block 2. Empty sources are left out.
pub fn block(
    code_mode: bool,
    skills: Option<&str>,
    files: &[InstructionFile],
    date: &str,
    env: &Environment,
) -> String {
    let mut sources = Vec::new();
    if code_mode {
        sources.push(CODE_MODE_CATALOG.to_string());
    }
    sources.extend(skills.map(str::to_string));
    if !files.is_empty() {
        sources.push(render(files));
    }
    sources.push(format!("Today's date: {date}"));
    sources.push(environment_block(env));
    sources.join("\n\n")
}

fn has_glob(text: &str) -> bool {
    text.contains(['*', '?', '[', '{'])
}

fn push(out: &mut Vec<InstructionFile>, path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if out.iter().any(|f| {
        std::fs::canonicalize(&f.path).unwrap_or_else(|_| PathBuf::from(&f.path)) == canonical
    }) {
        return;
    }
    out.push(InstructionFile {
        path: path.display().to_string(),
        content,
    });
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

/// `AGENTS.md` files (global, then nearest project file first within the discovery bound) and the
/// configured `instructions` entries.
// M1b: instruction URLs, the frozen per-session baseline and change deltas.
pub fn discover(
    paths: &Paths,
    directory: &Path,
    worktree: &Path,
    configured: &[String],
) -> Vec<InstructionFile> {
    let mut out = Vec::new();
    push(&mut out, &paths.legacy_config_dir().join("AGENTS.md"));
    push(&mut out, &paths.data.join("AGENTS.md"));
    if directory.starts_with(worktree) || directory.starts_with(&paths.home) {
        for dir in crate::config::upward(directory, &paths.home) {
            push(&mut out, &dir.join("AGENTS.md"));
        }
    }
    for entry in configured {
        if entry.starts_with("http://") || entry.starts_with("https://") {
            continue;
        }
        let expanded = paths.expand_home(entry);
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
                glob_files(&root, &pattern)
                    .iter()
                    .for_each(|p| push(&mut out, p));
            } else {
                push(&mut out, &expanded);
            }
            continue;
        }
        let mut found = Vec::new();
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
        found.iter().for_each(|p| push(&mut out, p));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_order() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        std::fs::create_dir_all(project.join("sub")).unwrap();
        std::fs::create_dir_all(project.join("docs")).unwrap();
        std::fs::create_dir_all(home.join(".config/opencode")).unwrap();
        std::fs::write(home.join(".config/opencode/AGENTS.md"), "global\n").unwrap();
        std::fs::write(project.join("AGENTS.md"), "project\n").unwrap();
        std::fs::write(project.join("sub/AGENTS.md"), "sub\n").unwrap();
        std::fs::write(project.join("docs/rules.md"), "rules\n").unwrap();
        let paths = Paths::isolated(home.clone());
        let files = discover(
            &paths,
            &project.join("sub"),
            &project,
            &["docs/rules.md".into()],
        );
        let contents: Vec<&str> = files.iter().map(|f| f.content.as_str()).collect();
        assert_eq!(contents, vec!["global\n", "sub\n", "project\n", "rules\n"]);
        let env = Environment {
            directory: project.clone(),
            worktree: project,
            git: false,
        };
        let text = block(false, None, &files[..1], "Wed Nov 04 2026", &env);
        assert!(text.starts_with("Instructions from: "));
        assert!(text.contains(
            "global\n\n\nToday's date: Wed Nov 04 2026\n\nHere is some useful information"
        ));
    }
}
