//! Favourite and recently used models, kept in `~/.anysola/state/model.json`.

use std::path::PathBuf;

use serde_json::{Value, json};

const RECENT: usize = 10;

pub struct Models {
    path: PathBuf,
    pub recent: Vec<String>,
    pub favorite: Vec<String>,
}

fn ids(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

impl Models {
    pub fn load(path: PathBuf) -> Models {
        let value: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        Models {
            recent: ids(&value, "recent"),
            favorite: ids(&value, "favorite"),
            path,
        }
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let value = json!({"recent": self.recent, "favorite": self.favorite});
        let _ = std::fs::write(&self.path, format!("{value:#}\n"));
    }

    /// Records a choice as the most recent (at most ten).
    pub fn used(&mut self, id: &str) {
        self.recent.retain(|r| r != id);
        self.recent.insert(0, id.to_string());
        self.recent.truncate(RECENT);
        self.save();
    }

    pub fn toggle_favorite(&mut self, id: &str) {
        if self.favorite.iter().any(|f| f == id) {
            self.favorite.retain(|f| f != id);
        } else {
            self.favorite.push(id.to_string());
        }
        self.save();
    }
}
