//! Where the agent's skills, commands, agents and instructions come from for a folder, every
//! place that is looked in (found or not), the personal `AGENTS.md`, and showing a file in the
//! system's file manager.

use std::path::{Path, PathBuf};

use anysola_core::api::Core;
use anysola_core::command::Template;
use anysola_core::config::{self, NAMES};
use anysola_core::instructions::{InstructionFile, Observed, RemoteStatus};
use anysola_core::paths::Paths;
use anysola_core::reference::{self, Remote, Source};
use serde_json::{Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, core_for, text};

/// What a place or file is, by where it is (`Origin` in the window).
pub(crate) fn origin(path: &Path, paths: &Paths) -> &'static str {
    let within = |dir: &Path| path.starts_with(dir);
    let has = |name: &str| path.components().any(|c| c.as_os_str() == name);
    if within(&paths.data.join("cache").join("skills")) {
        "url"
    } else if has(".claude") {
        "claude"
    } else if has(".agents") {
        "agents"
    } else if within(&paths.data) {
        "personal"
    } else if within(&paths.legacy_config_dir()) {
        "legacy"
    } else if config::DOT_DIRS.iter().any(|d| has(d))
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| NAMES.contains(&n) || n == "AGENTS.md")
    {
        "project"
    } else {
        "config"
    }
}

/// A config document's origin: a file, or `ANYSOLA_CONFIG_CONTENT` (not a file).
fn document_origin(source: &str, paths: &Paths) -> &'static str {
    let path = Path::new(source);
    if path.is_absolute() {
        origin(path, paths)
    } else {
        "config"
    }
}

struct Places {
    paths: Paths,
    list: Vec<Value>,
}

impl Places {
    fn add(&mut self, path: PathBuf, holds: &str, always: bool) {
        let exists = path.exists();
        if !(always || exists) {
            return;
        }
        let text = path.display().to_string();
        if self.list.iter().any(|p| p["path"] == text.as_str()) {
            return;
        }
        let origin = origin(&path, &self.paths);
        self.list
            .push(json!({"path": text, "holds": holds, "origin": origin, "exists": exists}));
    }
}

/// Every place read for `directory`: the global ones always, the folder's own where they exist.
/// Built from the same lists the core reads, so it can't drift from what loads.
fn places(core: &Core, paths: &Paths) -> Vec<Value> {
    let loaded = core.loaded();
    let mut places = Places {
        paths: paths.clone(),
        list: Vec::new(),
    };
    let legacy = paths.legacy_config_dir();
    places.add(paths.config_file(), "config", true);
    for name in NAMES {
        places.add(
            legacy.join(name),
            "config",
            name.starts_with("opencode.json"),
        );
    }
    for (source, _) in loaded.config.documents() {
        let path = Path::new(source);
        if path.is_absolute() {
            places.add(path.to_path_buf(), "config", false);
        }
    }
    places.add(paths.data.join("AGENTS.md"), "instructions", true);
    places.add(legacy.join("AGENTS.md"), "instructions", true);
    for dir in config::upward(core.directory(), &paths.home) {
        places.add(dir.join("AGENTS.md"), "instructions", false);
    }
    for root in [&paths.data, &legacy] {
        places.add(root.join("skills"), "skills", true);
        places.add(root.join("skill"), "skills", false);
        for folder in anysola_core::command::FOLDERS {
            places.add(root.join(folder), "commands", folder == "commands");
        }
        for folder in anysola_core::agent::FOLDERS {
            places.add(root.join(folder), "agents", folder == "agents");
        }
    }
    places.add(paths.home.join(".claude").join("skills"), "skills", true);
    places.add(paths.home.join(".agents").join("skills"), "skills", true);
    for root in anysola_core::skill::local_roots(paths, &loaded.config, core.directory()) {
        places.add(root, "skills", false);
    }
    for dir in loaded.config.directories() {
        for folder in anysola_core::command::FOLDERS {
            places.add(dir.join(folder), "commands", false);
        }
        for folder in anysola_core::agent::FOLDERS {
            places.add(dir.join(folder), "agents", false);
        }
    }
    places.list
}

fn instructions(core: &Core, paths: &Paths) -> Vec<Value> {
    let loaded = core.loaded();
    // A URL that failed stays in its place in the list, with why: it stands in empty here.
    let failed: Vec<&RemoteStatus> = loaded
        .remote_instruction_status
        .iter()
        .filter(|s| s.error.is_some())
        .collect();
    let mut remote = loaded.remote_instructions.clone();
    remote.extend(failed.iter().map(|s| InstructionFile {
        path: s.url.clone(),
        content: String::new(),
    }));
    let found = anysola_core::instructions::discover(
        paths,
        core.directory(),
        core.worktree(),
        &loaded.config.instructions,
        &remote,
    );
    let Observed::Value(Value::Array(files)) = found else {
        return Vec::new();
    };
    files
        .iter()
        .filter_map(|file| {
            let path = file["path"].as_str()?;
            let url = path.starts_with("http://") || path.starts_with("https://");
            let origin = if url {
                "url"
            } else {
                origin(Path::new(path), paths)
            };
            let error = failed
                .iter()
                .find(|s| s.url == path)
                .and_then(|s| s.error.clone());
            Some(json!({
                "path": path,
                "bytes": file["content"].as_str().map_or(0, str::len),
                "origin": origin,
                "error": error,
            }))
        })
        .collect()
}

/// How each configured `skills` URL's index download went, in config order.
fn skill_urls(status: &[RemoteStatus]) -> Vec<Value> {
    status
        .iter()
        .map(|s| json!({"url": s.url, "error": s.error}))
        .collect()
}

/// The core keeps git's first line, which for a clone is its progress line, not why it failed.
fn git_reason(error: &str) -> String {
    if error.starts_with("Cloning into ") {
        "Git couldn’t clone it".to_string()
    } else {
        error.to_string()
    }
}

/// The git references and how each checkout stands, in config order.
fn repositories(core: &Core) -> Vec<Value> {
    let references = core.references();
    core.reference_status()
        .into_iter()
        .map(|s| {
            let (repository, branch) = references
                .iter()
                .find(|r| r.name == s.name)
                .and_then(|r| match &r.source {
                    Source::Git { repository, branch } => {
                        Some((repository.clone(), branch.clone()))
                    }
                    Source::Local(_) => None,
                })
                .unzip();
            json!({
                "name": s.name,
                "repository": repository,
                "branch": branch.flatten(),
                "cloned": s.cloned,
                "attemptedAt": s.attempted_at,
                "error": s.error.as_deref().map(git_reason),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn sources(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let paths = core.paths().clone();
    let loaded = core.loaded();
    let skills: Vec<Value> = loaded
        .skills
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "path": s.path.display().to_string(),
                "origin": origin(&s.path, &paths),
            })
        })
        .collect();
    let commands: Vec<Value> = core
        .commands()
        .list()
        .iter()
        .map(|c| {
            let (origin, source) = match (&c.template, &c.source) {
                (Template::Mcp(prompt), _) => ("mcp", Some(prompt.server.clone())),
                (_, Some(source)) => (document_origin(source, &paths), Some(source.clone())),
                (_, None) => ("builtin", None),
            };
            json!({"name": c.name, "description": c.description, "origin": origin, "source": source})
        })
        .collect();
    let agents: Vec<Value> = loaded
        .agents
        .list
        .iter()
        .filter(|a| !a.hidden)
        .map(|a| {
            let origin = a
                .source
                .as_deref()
                .map_or("builtin", |s| document_origin(s, &paths));
            json!({"id": a.id, "name": a.name, "description": a.description,
                   "origin": origin, "source": a.source})
        })
        .collect();
    let (instructions, places) = {
        let core = core.clone();
        let paths = paths.clone();
        tauri::async_runtime::spawn_blocking(move || {
            (instructions(&core, &paths), places(&core, &paths))
        })
        .await
        .map_err(text)?
    };
    Ok(json!({
        "skills": skills,
        "commands": commands,
        "agents": agents,
        "instructions": instructions,
        "places": places,
        "skillUrls": skill_urls(&loaded.skill_status),
        "repositories": repositories(&core),
    }))
}

/// Tries the downloads that failed again: instruction URLs and skill indexes at a reload of the
/// folder's config, a repository that never cloned by cloning it now. Then lists the sources.
#[tauri::command]
pub async fn sources_retry(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    core.reload().await;
    let failed: Vec<(Remote, Option<String>)> = core
        .reference_status()
        .iter()
        .filter(|s| !s.cloned && s.error.is_some())
        .filter_map(|s| core.references().into_iter().find(|r| r.name == s.name))
        .filter_map(|r| match r.source {
            Source::Git { repository, branch } => Some((reference::parse(&repository)?, branch)),
            Source::Local(_) => None,
        })
        .collect();
    if !failed.is_empty() {
        let (db, root) = (core.db().clone(), core.paths().repos_dir());
        tauri::async_runtime::spawn_blocking(move || {
            for (remote, branch) in failed {
                // Its reason is kept with the attempt, for the status read below.
                let _ = reference::ensure(&db, &root, &remote, branch.as_deref());
            }
        })
        .await
        .map_err(text)?;
    }
    sources(handle, app, directory).await
}

#[tauri::command]
pub async fn personal_instructions(app: State<'_, App>) -> Answer {
    let path = app.paths.data.join("AGENTS.md");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Can’t read {}: {e}", path.display())),
    };
    Ok(json!({"path": path.display().to_string(), "text": text}))
}

/// `/memory`: the instruction files a note can go to for the session (`MemoryFiles` in bridge.ts).
#[tauri::command]
pub async fn memory_files(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: Option<String>,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let files = core
        .memory_files(session_id.as_deref())
        .map_err(crate::plain_error)?;
    Ok(json!({
        "project": files.project.display().to_string(),
        "personal": files.personal.display().to_string(),
    }))
}

/// `/memory <text>`: adds the note as a line at the end of one of those files.
#[tauri::command]
pub async fn add_memory(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: Option<String>,
    scope: anysola_core::api::MemoryScope,
    text: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        core.add_memory(session_id.as_deref(), scope, &text)
    })
    .await
    .map_err(crate::text)?
    .map_err(crate::plain_error)?;
    Ok(json!({"path": path.display().to_string()}))
}

/// Saves `~/.anysola/AGENTS.md`; empty text removes it. Sessions take it at their next step.
#[tauri::command]
pub async fn save_personal_instructions(app: State<'_, App>, text: String) -> Answer {
    let path = app.paths.data.join("AGENTS.md");
    if text.trim().is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Can’t remove {}: {e}", path.display())),
        }
        return Ok(Value::Null);
    }
    // In place, so a symlinked file stays a link.
    let fail = |e: std::io::Error| format!("Can’t save {}: {e}", path.display());
    std::fs::create_dir_all(&app.paths.data).map_err(fail)?;
    std::fs::write(&path, text).map_err(fail)?;
    Ok(Value::Null)
}

/// The nearest folder that exists, for a place that doesn't yet.
fn existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| p.is_dir())
}

/// Shows a file selected in its folder, or opens a folder; a place that doesn't exist yet opens
/// the nearest folder above it that does.
#[tauri::command]
pub async fn reveal_path(handle: AppHandle, path: String) -> Answer {
    use tauri_plugin_opener::OpenerExt;
    let target = Path::new(&path);
    if target.is_file() {
        handle.opener().reveal_item_in_dir(target).map_err(text)?;
        return Ok(Value::Null);
    }
    let folder = existing(target).ok_or_else(|| format!("{path} is not on this computer."))?;
    handle
        .opener()
        .open_path(folder.display().to_string(), None::<&str>)
        .map_err(text)?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_by_location() {
        let paths = Paths::isolated(PathBuf::from("/home/u"));
        let o = |p: &str| origin(Path::new(p), &paths);
        assert_eq!(o("/home/u/.claude/skills/pdf/SKILL.md"), "claude");
        assert_eq!(o("/work/shop/.claude/skills/x/SKILL.md"), "claude");
        assert_eq!(o("/work/shop/.agents/skills/x/SKILL.md"), "agents");
        assert_eq!(o("/home/u/.anysola/skills/x/SKILL.md"), "personal");
        assert_eq!(o("/home/u/.anysola/config.json"), "personal");
        assert_eq!(o("/home/u/.anysola/cache/skills/abc/x/SKILL.md"), "url");
        assert_eq!(o("/home/u/.config/opencode/command/x.md"), "legacy");
        assert_eq!(o("/work/shop/.anysola/commands/deploy.md"), "project");
        assert_eq!(o("/work/shop/.opencode/agent/r.md"), "project");
        assert_eq!(o("/work/shop/anysola.json"), "project");
        assert_eq!(o("/work/shop/AGENTS.md"), "project");
        assert_eq!(o("/work/shop/docs/rules.md"), "config");
        assert_eq!(document_origin("ANYSOLA_CONFIG_CONTENT", &paths), "config");
    }

    #[test]
    fn git_reasons() {
        assert_eq!(
            git_reason("Cloning into '/home/u/.anysola/repos/github.com/acme/x'..."),
            "Git couldn’t clone it"
        );
        assert_eq!(
            git_reason("repository 'x' does not exist"),
            "repository 'x' does not exist"
        );
    }

    #[test]
    fn nearest_existing_folder() {
        let dir = std::env::temp_dir();
        let missing = dir.join("anysola-no-such-dir").join("skills");
        assert_eq!(existing(&missing), Some(dir.as_path()));
    }
}
