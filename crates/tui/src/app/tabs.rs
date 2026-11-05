//! Session tabs persisted in `~/.anysola/state/tui/tabs.json`, as 0.5.1's `tabs` store:
//! `{"global": {"tabs": [...], "unread": {}}, "cwd": {<cwd>: {...}}}` with `{sessionID, title}`
//! tabs, one list per working directory unless `tabs.scope` is `global`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::App;
use crate::state::Tab;

pub(super) struct TabStore {
    path: PathBuf,
    /// The cwd key, or none for the global list.
    scope: Option<String>,
    /// What was last read or written, so unchanged tabs are not written again.
    saved: Vec<(String, String)>,
}

impl TabStore {
    pub fn new(data: &Path, cwd: &Path, global: bool) -> TabStore {
        TabStore {
            path: data.join("state").join("tui").join("tabs.json"),
            scope: (!global).then(|| cwd.display().to_string()),
            saved: Vec::new(),
        }
    }

    fn read(&self) -> Value {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"global": {"tabs": [], "unread": {}}, "cwd": {}}))
    }

    fn list<'a>(&self, value: &'a Value) -> &'a Value {
        match &self.scope {
            Some(cwd) => &value["cwd"][cwd]["tabs"],
            None => &value["global"]["tabs"],
        }
    }

    /// `(session ID, title)` in strip order.
    pub fn load(&mut self) -> Vec<(String, String)> {
        let value = self.read();
        let tabs: Vec<(String, String)> = self
            .list(&value)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                let id = t["sessionID"].as_str()?.to_string();
                Some((id, t["title"].as_str().unwrap_or_default().to_string()))
            })
            .collect();
        self.saved = tabs.clone();
        tabs
    }

    pub fn save(&mut self, tabs: &[Tab]) {
        let current: Vec<(String, String)> = tabs
            .iter()
            .map(|t| (t.session_id.clone(), t.title.clone()))
            .collect();
        if current == self.saved {
            return;
        }
        let mut value = self.read();
        let list: Vec<Value> = current
            .iter()
            .map(|(id, title)| json!({"sessionID": id, "title": title}))
            .collect();
        let state = json!({"tabs": list, "unread": {}});
        match &self.scope {
            Some(cwd) => {
                if !value["cwd"].is_object() {
                    value["cwd"] = json!({});
                }
                value["cwd"][cwd] = state;
            }
            None => value["global"] = state,
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&self.path, value.to_string()).is_ok() {
            self.saved = current;
        }
    }
}

impl App {
    /// `session.tab.next`/`previous` (`_unread`: only tabs with the unread marker), wrapping;
    /// from home the first or last tab.
    pub(super) fn cycle_tab(&mut self, step: isize, unread: bool) {
        let tabs = &self.state.tabs;
        if tabs.is_empty() {
            return;
        }
        let count = tabs.len() as isize;
        let active = self.state.session().map(|s| s.id.as_str());
        let start = match tabs
            .iter()
            .position(|t| Some(t.session_id.as_str()) == active)
        {
            Some(at) => at as isize,
            None if step > 0 => -1,
            None => 0,
        };
        let target = (1..=count)
            .map(|offset| &tabs[(start + step * offset).rem_euclid(count) as usize])
            .find(|t| !unread || t.unread)
            .map(|t| t.session_id.clone());
        if let Some(id) = target {
            self.open_session(&id);
        }
    }

    /// `session.tab.select.N` (1-based).
    pub(super) fn select_tab(&mut self, number: usize) {
        let id = number
            .checked_sub(1)
            .and_then(|at| self.state.tabs.get(at))
            .map(|t| t.session_id.clone());
        if let Some(id) = id {
            self.open_session(&id);
        }
    }

    /// The last run's tabs whose sessions still exist.
    pub(super) fn restore_tabs(&mut self) {
        for (id, title) in self.tab_store.load() {
            let Ok(Some(row)) = self.core.session(&id) else {
                continue;
            };
            if self.state.tabs.iter().all(|t| t.session_id != id) {
                let title = if row.title.is_empty() {
                    title
                } else {
                    row.title
                };
                self.state.tabs.push(Tab {
                    session_id: id,
                    title,
                    ..Tab::default()
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str) -> Tab {
        Tab {
            session_id: id.into(),
            title: format!("{id} title"),
            ..Tab::default()
        }
    }

    #[test]
    fn lists_per_cwd_and_global() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = TabStore::new(dir.path(), Path::new("/a"), false);
        a.save(&[tab("s1"), tab("s2")]);
        let mut b = TabStore::new(dir.path(), Path::new("/b"), false);
        b.save(&[tab("s3")]);
        let mut global = TabStore::new(dir.path(), Path::new("/a"), true);
        assert!(global.load().is_empty());
        let mut again = TabStore::new(dir.path(), Path::new("/a"), false);
        assert_eq!(
            again.load(),
            [
                ("s1".to_string(), "s1 title".to_string()),
                ("s2".to_string(), "s2 title".to_string())
            ]
        );
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("state/tui/tabs.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(raw["cwd"]["/b"]["tabs"][0]["sessionID"], "s3");
        assert_eq!(raw["cwd"]["/a"]["unread"], json!({}));
    }
}
