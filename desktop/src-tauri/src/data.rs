//! Permissions and data in Settings: saved "always allow" answers and the configured rules, what
//! `~/.anysola` holds and clearing its caches, deleting every session, exporting one, and the
//! app's version.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anysola_core::api::ConfiguredRule;
use anysola_core::paths::Paths;
use anysola_core::permission::Effect;
use anysola_core::storage::Db;
use serde_json::{Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, core_for, text};

/// The marker the one-time import of 0.5.x data leaves (`anysola_migrate::MARKER`).
const IMPORT_MARKER: &str = "migration:0.5.x";
/// A folder's size stops counting after this many entries or this long: an estimate beats a
/// window that waits on a huge tree.
const WALK_ENTRIES: usize = 200_000;
const WALK_TIME: Duration = Duration::from_secs(2);

fn saved(db: &Db) -> Answer {
    let list: Vec<Value> = db
        .all_saved_permissions()
        .map_err(text)?
        .into_iter()
        .map(|p| {
            json!({"projectId": p.project_id, "folder": p.worktree,
                   "action": p.action, "resource": p.resource})
        })
        .collect();
    Ok(Value::Array(list))
}

/// A configured rule as the window lists it: where it applies (`agent` null for every agent),
/// its file, and whether it is in effect (`applied` false: a project allow rule waiting for that
/// file's `permissions` approval).
fn rule_json(rule: &ConfiguredRule) -> Value {
    let effect = match rule.effect {
        Effect::Allow => "allow",
        Effect::Ask => "ask",
        Effect::Deny => "deny",
    };
    json!({
        "action": rule.action,
        "resource": rule.resource,
        "effect": effect,
        "agent": rule.agent,
        "source": rule.source,
        "project": rule.project,
        "applied": rule.applied,
    })
}

/// Every project's saved answers, and every rule the folder's config documents and agent files
/// set (for all agents or one, in effect or waiting), with their files.
#[tauri::command]
pub async fn permissions(handle: AppHandle, app: State<'_, App>, directory: String) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let rules: Vec<Value> = core
        .configured_rules()
        .iter()
        // A rule from no file (an agent defined without one) has nothing to open or approve.
        .filter(|r| !r.source.is_empty())
        .map(rule_json)
        .collect();
    Ok(json!({"saved": saved(app.db()?)?, "rules": rules}))
}

/// Forgets one saved answer; the next matching request asks again.
#[tauri::command]
pub async fn permission_forget(
    app: State<'_, App>,
    project_id: String,
    action: String,
    resource: String,
) -> Answer {
    let db = app.db()?;
    db.forget_permission(&project_id, &action, &resource)
        .map_err(text)?;
    saved(db)
}

#[tauri::command]
pub async fn permission_forget_all(app: State<'_, App>, project_id: String) -> Answer {
    let db = app.db()?;
    db.forget_permissions(&project_id).map_err(text)?;
    saved(db)
}

/// Bytes under `dir`, counted until the walk's bounds.
fn size(dir: &Path) -> u64 {
    let start = Instant::now();
    let mut total = 0;
    let mut entries = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            entries += 1;
            if entries > WALK_ENTRIES || start.elapsed() > WALK_TIME {
                return total;
            }
            // Not followed: a link to a big tree is not this folder's size.
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

fn cache_dir(paths: &Paths, id: &str) -> Option<PathBuf> {
    match id {
        "tool-output" => Some(paths.tool_output_dir()),
        "cache" => Some(paths.data.join("cache")),
        "shell" => Some(paths.data.join("shell")),
        _ => None,
    }
}

fn info(paths: &Paths, db: &Db) -> Answer {
    let database = paths.database();
    // The write-ahead log is part of the database until it is checkpointed.
    let bytes = file_size(&database)
        + file_size(&database.with_extension("db-wal"))
        + file_size(&database.with_extension("db-shm"));
    let caches: Vec<Value> = ["tool-output", "cache", "shell"]
        .iter()
        .filter_map(|id| {
            let dir = cache_dir(paths, id)?;
            Some(json!({"id": id, "path": dir.display().to_string(), "bytes": size(&dir)}))
        })
        .collect();
    let imported = db.kv_get(IMPORT_MARKER).map_err(text)?.map(|marker| {
        json!({
            "sessions": marker["sessions"].as_u64().unwrap_or(0),
            "at": marker["time"].as_u64().unwrap_or(0),
        })
    });
    Ok(json!({
        "folder": paths.data.display().to_string(),
        "database": {"path": database.display().to_string(), "bytes": bytes},
        "sessions": db.all_root_sessions().map_err(text)?.len(),
        "caches": caches,
        "log": paths.log_file().display().to_string(),
        "imported": imported,
    }))
}

#[tauri::command]
pub async fn data_info(app: State<'_, App>) -> Answer {
    let (paths, db) = (app.paths.clone(), app.db()?.clone());
    tauri::async_runtime::spawn_blocking(move || info(&paths, &db))
        .await
        .map_err(text)?
}

/// Empties everything inside `dir`, keeping the folder.
fn empty(dir: &Path) -> Result<(), String> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Can’t read {}: {e}", dir.display())),
    };
    for entry in read.flatten() {
        let path = entry.path();
        let removed = if entry.file_type().is_ok_and(|t| t.is_dir()) {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        removed.map_err(|e| format!("Can’t remove {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Empties one of the caches; never while a turn runs, which may be writing to it.
#[tauri::command]
pub async fn data_clear(app: State<'_, App>, id: String) -> Answer {
    let dir = cache_dir(&app.paths, &id).ok_or_else(|| format!("There’s no cache “{id}”."))?;
    let (paths, db) = (app.paths.clone(), app.db()?.clone());
    // Turns here, and those of a terminal session on the same data.
    let held = db.all_root_sessions().map_err(text)?.iter().any(|s| s.2);
    let cores = app.cores_running().await;
    if held || cores.iter().any(|(_, c)| c.any_busy()) {
        return Err("Wait for running sessions to finish.".into());
    }
    // A background command outlives its turn and is still writing its output there.
    if id == "shell" && cores.iter().any(|(_, c)| c.any_background_shell()) {
        return Err(
            "A background command is still running. Stop it or let it finish first.".into(),
        );
    }
    let answer = tauri::async_runtime::spawn_blocking(move || {
        empty(&dir)?;
        info(&paths, &db)
    })
    .await
    .map_err(text)??;
    // Skills from URLs live in the cache: cores fetch them again rather than point at files
    // that are gone.
    if id == "cache" {
        for (_, core) in cores {
            core.reload().await;
        }
    }
    Ok(answer)
}

/// Deletes every session of every project as deleting one does, keeping those with a turn
/// running here or in another Anysola Code process. The window asks first.
#[tauri::command]
pub async fn delete_all_sessions(app: State<'_, App>) -> Answer {
    let db = app.db()?.clone();
    let mut deleted = 0;
    let mut kept = 0;
    // Of `kept`: the ones whose changes were being applied (not a running turn).
    let mut applying = 0;
    for (id, directory, held) in db.all_root_sessions().map_err(text)? {
        // A worktree session's core is its main folder's.
        let directory = crate::worktree::folder_of(&db, &directory, &id);
        let core = app.running(&directory).await;
        if held || core.as_ref().is_some_and(|core| core.is_busy(&id)) {
            kept += 1;
            continue;
        }
        // Its worktree goes too (any terminal holding the folder open is closed first, as when
        // deleting one); one being applied is kept.
        if let Ok(Some(w)) = db.session_worktree(&id) {
            app.terminals.close_in_directory(&w.directory);
        }
        if crate::worktree::before_delete(core.as_ref(), &db, &id)
            .await
            .is_err()
        {
            kept += 1;
            applying += 1;
            continue;
        }
        // A running core also drops the session's usage it keeps in memory.
        let removed = match &core {
            Some(core) => core.delete_session(&id).map_err(text)?,
            None => db.delete_session(&id).map_err(text)?,
        };
        crate::worktree::after_delete(&db, &id).await;
        if removed {
            deleted += 1;
        }
    }
    Ok(json!({"deleted": deleted, "kept": kept, "applying": applying}))
}

/// `title.json`, with what file systems refuse taken out.
fn file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let cleaned: String = cleaned.chars().take(80).collect();
    if cleaned.is_empty() {
        "session.json".into()
    } else {
        format!("{cleaned}.json")
    }
}

/// Asks where to save the session (0.5.1's `{info, messages}` export); `null` when the
/// dialog was cancelled.
#[tauri::command]
pub async fn export_session(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    use tauri_plugin_dialog::DialogExt;
    let db = app.db()?.clone();
    let row = db
        .session(&session_id)
        .map_err(text)?
        .ok_or("This session no longer exists.")?;
    let exported = db
        .export_session(&session_id)
        .map_err(text)?
        .ok_or("This session no longer exists.")?;
    let (sender, picked) = tokio::sync::oneshot::channel();
    let mut dialog = handle
        .dialog()
        .file()
        .set_title("Export session")
        .set_file_name(file_name(&row.title))
        .add_filter("JSON", &["json"]);
    if Path::new(&directory).is_dir() {
        dialog = dialog.set_directory(&directory);
    }
    dialog.save_file(move |path| {
        let _ = sender.send(path);
    });
    let Some(path) = picked.await.ok().flatten() else {
        return Ok(Value::Null);
    };
    let path = path
        .into_path()
        .map_err(|_| "Choose a file on this computer.".to_string())?;
    let body = serde_json::to_string_pretty(&exported).map_err(text)? + "\n";
    std::fs::write(&path, body).map_err(|e| format!("Can’t save {}: {e}", path.display()))?;
    Ok(json!(path.display().to_string()))
}

#[tauri::command]
pub async fn about(handle: AppHandle) -> Answer {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    Ok(json!({
        "version": handle.package_info().version.to_string(),
        "os": os,
        "arch": std::env::consts::ARCH,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_as_the_window_reads_them() {
        let rule = |effect, agent: Option<&str>, project, applied| ConfiguredRule {
            action: "shell".into(),
            resource: "npm test*".into(),
            effect,
            agent: agent.map(str::to_string),
            source: "/p/anysola.json".into(),
            project,
            applied,
        };
        assert_eq!(
            rule_json(&rule(Effect::Allow, Some("build"), true, false)),
            json!({"action": "shell", "resource": "npm test*", "effect": "allow", "agent": "build",
                   "source": "/p/anysola.json", "project": true, "applied": false})
        );
        let all = rule_json(&rule(Effect::Deny, None, false, true));
        assert_eq!(
            (all["effect"].clone(), all["agent"].clone()),
            (json!("deny"), Value::Null)
        );
        assert_eq!(
            rule_json(&rule(Effect::Ask, None, true, true))["effect"],
            "ask"
        );
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "anysola-data-{name}-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn info_counts_and_clear_empties() {
        let home = temp("info");
        let paths = Paths::isolated(home.clone());
        std::fs::create_dir_all(paths.tool_output_dir().join("a")).unwrap();
        std::fs::write(paths.tool_output_dir().join("a/out.txt"), "12345").unwrap();
        std::fs::write(paths.tool_output_dir().join("b.txt"), "123").unwrap();
        let db = Db::open(&paths.database()).unwrap();
        db.kv_set(IMPORT_MARKER, &json!({"sessions": 4, "time": 99}))
            .unwrap();
        let info = info(&paths, &db).unwrap();
        assert_eq!(info["caches"][0]["id"], "tool-output");
        assert_eq!(info["caches"][0]["bytes"], 8);
        assert_eq!(info["caches"][1]["bytes"], 0, "a missing folder is empty");
        assert_eq!(info["imported"], json!({"sessions": 4, "at": 99}));
        assert_eq!(info["sessions"], 0);
        assert!(info["database"]["bytes"].as_u64().unwrap() > 0);
        empty(&paths.tool_output_dir()).unwrap();
        assert!(paths.tool_output_dir().is_dir(), "the folder stays");
        assert_eq!(size(&paths.tool_output_dir()), 0);
        empty(&home.join("missing")).unwrap();
        drop(db);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn export_file_names() {
        assert_eq!(file_name("Fix: a/b \"c\""), "Fix- a-b -c-.json");
        assert_eq!(file_name("  ..  "), "session.json");
        assert_eq!(file_name(&"x".repeat(200)).len(), 85);
    }
}
