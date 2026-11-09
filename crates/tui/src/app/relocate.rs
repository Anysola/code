//! Moving the app to another directory (`/cd`, `/open` on a project) and picking up a new
//! sign-in (`/connect`): the core is bound to one directory and one credential, so the app
//! starts a fresh core in this process and swaps it in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anysola_core::api::Core;
use futures_util::future::BoxFuture;

use super::{App, Msg};
use crate::state::{Route, Variant};

/// Starts a core (with its MCP runtime) for a directory; supplied by the binary.
pub type Restart = Arc<dyn Fn(PathBuf) -> BoxFuture<'static, Result<Core, String>> + Send + Sync>;

/// A move that waits for running turns to finish.
pub(super) struct Pending {
    pub directory: PathBuf,
    pub session: Option<String>,
    /// The session moves into the new directory (`/cd`) instead of being opened where it is.
    pub adopt: bool,
}

/// `/cd` argument → an existing directory (`~`, `~/…`, relative to the current one).
pub fn resolve(argument: &str, current: &Path, home: &Path) -> Option<PathBuf> {
    let argument = argument.trim();
    let path = match argument.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => current.join(argument),
    };
    let path = path.canonicalize().ok()?;
    path.is_dir().then_some(path)
}

/// `/cd` recents per project in `~/.anysola/state/tui/directory-recents.json`
/// (`{"projects": {<id>: [{directory, usedAt}]}}`, newest first, at most 10).
pub(super) struct Recents(PathBuf);

impl Recents {
    pub fn new(data: &Path) -> Recents {
        Recents(
            data.join("state")
                .join("tui")
                .join("directory-recents.json"),
        )
    }

    fn read(&self) -> serde_json::Value {
        std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::json!({"projects": {}}))
    }

    pub fn list(&self, project: &str) -> Vec<PathBuf> {
        let mut items: Vec<(u64, PathBuf)> = self.read()["projects"][project]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| {
                Some((
                    i["usedAt"].as_u64()?,
                    PathBuf::from(i["directory"].as_str()?),
                ))
            })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.0));
        items.into_iter().map(|(_, d)| d).collect()
    }

    pub fn touch(&self, project: &str, directory: &Path, now: u64) {
        let mut value = self.read();
        let directory = directory.display().to_string();
        let mut list: Vec<serde_json::Value> =
            vec![serde_json::json!({"directory": directory, "usedAt": now})];
        list.extend(
            value["projects"][project]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|i| i["directory"].as_str() != Some(&directory))
                .cloned(),
        );
        list.truncate(10);
        if !value["projects"].is_object() {
            value["projects"] = serde_json::json!({});
        }
        value["projects"][project] = serde_json::Value::Array(list);
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.0, value.to_string());
    }
}

impl Recents {
    pub fn remove(&self, project: &str, directory: &Path) {
        let mut value = self.read();
        let directory = directory.display().to_string();
        if let Some(list) = value["projects"][project].as_array_mut() {
            list.retain(|i| i["directory"].as_str() != Some(&directory));
            let _ = std::fs::write(&self.0, value.to_string());
        }
    }
}

impl App {
    /// `ctrl+d` twice on a `recent` `/cd` row forgets that directory.
    pub(super) fn remove_recent(&mut self) {
        let Some(popup) = self.state.popup.as_ref() else {
            return;
        };
        let Some(row) = popup.rows.get(popup.selected) else {
            return;
        };
        if row.description != "recent" {
            return;
        }
        if !self.state.popup_confirm {
            self.state.popup_confirm = true;
            return;
        }
        let (shown, selected, count) = (row.display.clone(), popup.selected, popup.rows.len());
        let home = PathBuf::from(&self.state.home);
        let project = self.core.project_id().to_string();
        if let Some(directory) = self
            .recents
            .list(&project)
            .into_iter()
            .find(|d| crate::complete::recent_value(d, &home) == shown)
        {
            self.recents.remove(&project, &directory);
        }
        self.state.popup_confirm = false;
        self.refresh_popup();
        if let Some(popup) = self.state.popup.as_mut() {
            popup.selected = selected.min(count.saturating_sub(2));
        }
    }

    /// `/cd <dir>`: on home the next session starts there; in a session the session moves.
    pub(super) fn change_directory(&mut self, argument: &str) {
        let current = PathBuf::from(&self.state.directory);
        let home = PathBuf::from(&self.state.home);
        let Some(directory) = resolve(argument, &current, &home) else {
            return self.toast(
                Some("Failed to change directory".into()),
                format!("No such directory: {}", argument.trim()),
                Variant::Error,
            );
        };
        let session = match &self.state.route {
            Route::Session(id) => Some(id.clone()),
            Route::Home => None,
        };
        self.relocate(directory, session, true);
    }

    /// Opens a session, first moving the app to its directory when it lives elsewhere.
    pub(super) fn open_elsewhere(&mut self, id: &str) {
        let directory = self
            .core
            .session(id)
            .ok()
            .flatten()
            .map(|row| row.directory);
        match directory {
            Some(directory) if Path::new(&directory) != self.core.directory() => {
                self.relocate(directory.into(), Some(id.to_string()), false)
            }
            _ => self.open_session(id),
        }
    }

    /// Restarts the core for `directory`, once no turn is running.
    pub(super) fn relocate(&mut self, directory: PathBuf, session: Option<String>, adopt: bool) {
        if self.restarter.is_none() {
            return self.toast(
                Some("Failed to change directory".into()),
                "Not available in this mode".into(),
                Variant::Error,
            );
        }
        self.relocating = Some(Pending {
            directory,
            session,
            adopt,
        });
        if !self.state.sessions.values().any(|s| s.busy) {
            self.resume_relocate();
        }
    }

    /// Runs a waiting move; called again whenever a turn ends.
    pub(super) fn resume_relocate(&mut self) {
        if self.state.sessions.values().any(|s| s.busy) {
            return;
        }
        let (Some(pending), Some(restart)) = (self.relocating.take(), self.restarter.clone())
        else {
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let started = restart(pending.directory.clone()).await;
            let apply = move |app: &mut App| match started {
                Ok(core) => app.swap_core(core, pending.session, pending.adopt),
                Err(error) => app.toast(
                    Some("Failed to change directory".into()),
                    error,
                    Variant::Error,
                ),
            };
            let _ = tx.send(Msg::Apply(Box::new(apply)));
        });
    }

    fn swap_core(&mut self, core: Core, session: Option<String>, adopt: bool) {
        if core.directory() != self.core.directory() {
            self.recents
                .touch(self.core.project_id(), core.directory(), super::now());
        }
        let old = std::mem::replace(&mut self.core, core);
        old.shutdown();
        self.resubscribe = Some(self.core.subscribe());
        let directory = self.core.directory().to_path_buf();
        let _ = std::env::set_current_dir(&directory);
        let home = PathBuf::from(&self.state.home);
        self.state.location = super::location(&directory, &home);
        self.state.directory = directory.display().to_string();
        self.files = None;
        self.files_walked = false;
        self.refresh_loaded();
        self.reload_all();
        self.mcp_seen.clear();
        let rows = self.mcp_rows();
        self.count_mcp(&rows);
        self.refresh_recent();
        self.fetch_account();
        match session {
            Some(id) => {
                if adopt && let Err(error) = self.core.adopt_session(&id) {
                    self.toast(
                        Some("Failed to move session".into()),
                        error.to_string(),
                        Variant::Error,
                    );
                }
                self.open_session(&id);
            }
            None => {
                self.state.route = Route::Home;
                if std::mem::take(&mut self.submit_after_move) {
                    self.submit(false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recents_newest_first_per_project() {
        let dir = tempfile::tempdir().unwrap();
        let recents = Recents::new(dir.path());
        recents.touch("p", Path::new("/a"), 1);
        recents.touch("p", Path::new("/b"), 2);
        recents.touch("p", Path::new("/a"), 3);
        recents.touch("q", Path::new("/c"), 4);
        assert_eq!(
            recents.list("p"),
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
        recents.remove("p", Path::new("/a"));
        assert_eq!(recents.list("p"), [PathBuf::from("/b")]);
        assert_eq!(recents.list("q"), [PathBuf::from("/c")]);
    }

    #[test]
    fn resolves_home_relative_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        assert_eq!(resolve("sub", &root, &root), Some(root.join("sub")));
        assert_eq!(
            resolve("~/sub", Path::new("/"), &root),
            Some(root.join("sub"))
        );
        assert_eq!(resolve("~", Path::new("/"), &root), Some(root.clone()));
        assert_eq!(resolve("sub/..", &root, &root), Some(root.clone()));
        assert_eq!(resolve("missing", &root, &root), None);
    }
}
