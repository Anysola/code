//! TUI settings: `~/.anysola/tui.json` (0.5.1's `cli.json` schema) with the
//! `OPENCODE_CLI_CONFIG_CONTENT` overlay; changes are written back with temp + rename, mode 0600.

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::state::{DiffView, Settings, Verbosity};
use crate::theme::Mode;

pub struct Tui {
    pub path: PathBuf,
    /// The file's own content (what gets written back).
    file: Value,
    /// The file with the environment overlay (what is in effect).
    pub value: Value,
}

fn parse(text: &str) -> Option<Value> {
    anysola_core::config::jsonc::parse(text)
        .ok()
        .filter(Value::is_object)
}

impl Tui {
    pub fn load(path: PathBuf) -> Tui {
        let file = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| parse(&t))
            .unwrap_or_else(|| Value::Object(Map::new()));
        let mut value = file.clone();
        if let Some(overlay) = std::env::var("OPENCODE_CLI_CONFIG_CONTENT")
            .ok()
            .and_then(|t| parse(&t))
        {
            merge(&mut value, &overlay);
        }
        Tui { path, file, value }
    }

    fn get(&self, path: &[&str]) -> Option<&Value> {
        path.iter().try_fold(&self.value, |v, k| v.get(k))
    }

    /// `diffs.fullscreen`: `/diff` never docks beside the session.
    pub fn diff_fullscreen(&self) -> bool {
        self.get(&["diffs", "fullscreen"]).and_then(Value::as_bool) == Some(true)
    }

    /// `tabs.scope = "global"`: one tab list for every directory instead of one per cwd.
    pub fn global_tabs(&self) -> bool {
        self.str(&["tabs", "scope"]) == Some("global")
    }

    fn str(&self, path: &[&str]) -> Option<&str> {
        self.get(path).and_then(Value::as_str)
    }

    pub fn theme_name(&self) -> String {
        self.str(&["theme", "name"])
            .or_else(|| self.str(&["theme"]))
            .unwrap_or(crate::theme::DEFAULT)
            .to_string()
    }

    /// The colour mode the config locks, if any.
    pub fn theme_mode(&self) -> Option<Mode> {
        match self.str(&["theme", "mode"]) {
            Some("dark") => Some(Mode::Dark),
            Some("light") => Some(Mode::Light),
            _ => None,
        }
    }

    pub fn mouse(&self) -> bool {
        self.get(&["mouse"]).and_then(Value::as_bool) != Some(false)
    }

    pub fn editor_context(&self) -> bool {
        self.get(&["prompt", "editor"]).and_then(Value::as_bool) != Some(false)
    }

    pub fn notifications(&self) -> bool {
        self.get(&["attention", "notifications"])
            .and_then(Value::as_bool)
            == Some(true)
    }

    pub fn sound(&self) -> bool {
        self.get(&["attention", "sound"]).and_then(Value::as_bool) == Some(true)
    }

    pub fn volume(&self) -> f64 {
        self.get(&["attention", "volume"])
            .and_then(Value::as_f64)
            .unwrap_or(0.4)
    }

    /// `attention.sounds.<name>`: a file played instead of the built-in sound.
    pub fn sound_file(&self, name: &str) -> Option<std::path::PathBuf> {
        self.str(&["attention", "sounds", name])
            .map(std::path::PathBuf::from)
    }

    pub fn auto_accept(&self) -> bool {
        self.str(&["session", "permissions"]) == Some("autoaccept")
    }

    pub fn settings(&self) -> Settings {
        let flag = |path: &[&str], default: bool| {
            self.get(path).and_then(Value::as_bool).unwrap_or(default)
        };
        Settings {
            thinking: self.str(&["session", "thinking"]) == Some("show"),
            source_markdown: self.str(&["session", "markdown"]) == Some("source"),
            diff_view: match self.str(&["diffs", "view"]) {
                Some("split") => DiffView::Split,
                Some("unified") => DiffView::Unified,
                _ => DiffView::Auto,
            },
            sidebar: self.str(&["session", "sidebar"]) != Some("hide"),
            paste_compact: self.str(&["prompt", "paste"]) != Some("full"),
            animations: flag(&["animations"], true),
            copy_on_select: self
                .str(&["terminal", "copy"])
                .map_or(!cfg!(windows), |mode| mode == "select"),
            title: flag(&["terminal", "title"], true),
            grouping: self.str(&["session", "grouping"]) != Some("none"),
            verbosity: match self.str(&["session", "verbosity"]) {
                Some("low") => Verbosity::Low,
                Some("high") => Verbosity::High,
                _ => Verbosity::Medium,
            },
        }
    }

    /// Sets `path` to `value` in the file and in effect, then writes the file.
    pub fn set(&mut self, path: &[&str], value: Value) -> std::io::Result<()> {
        for target in [&mut self.file, &mut self.value] {
            let mut node = &mut *target;
            for key in &path[..path.len() - 1] {
                if !node.get(*key).is_some_and(Value::is_object) {
                    node[*key] = Value::Object(Map::new());
                }
                node = &mut node[*key];
            }
            node[path[path.len() - 1]] = value.clone();
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, format!("{:#}\n", self.file))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&temp, &self.path)
    }
}

fn merge(target: &mut Value, overlay: &Value) {
    match (target, overlay) {
        (Value::Object(target), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, overlay) => *target = overlay.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tui.json");
        std::fs::write(&path, "{ // comment\n \"theme\": {\"name\": \"dracula\", \"mode\": \"light\"}, \"session\": {\"thinking\": \"show\"} }").unwrap();
        let mut tui = Tui::load(path.clone());
        assert_eq!(tui.theme_name(), "dracula");
        assert_eq!(tui.theme_mode(), Some(Mode::Light));
        assert!(tui.settings().thinking);
        tui.set(&["terminal", "title"], Value::Bool(false)).unwrap();
        let again = Tui::load(path);
        assert!(!again.settings().title);
        assert_eq!(again.theme_name(), "dracula");
    }

    #[test]
    fn copy_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tui.json");
        std::fs::write(&path, r#"{"terminal": {"copy": "manual"}}"#).unwrap();
        assert!(!Tui::load(path.clone()).settings().copy_on_select);
        std::fs::write(&path, r#"{"terminal": {"copy": "select"}}"#).unwrap();
        assert!(Tui::load(path).settings().copy_on_select);
    }
}
