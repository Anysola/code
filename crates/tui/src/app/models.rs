//! Favourite and recently used models, kept in `~/.anysola/state/model.json`.

use std::path::PathBuf;

use serde_json::{Value, json};

const RECENT: usize = 10;

pub struct Models {
    path: PathBuf,
    pub recent: Vec<String>,
    pub favorite: Vec<String>,
    /// The variant last chosen per model; `default` is stored explicitly.
    variants: serde_json::Map<String, Value>,
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
            variants: value
                .get("variant")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
            path,
        }
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let value = json!({
            "recent": self.recent,
            "favorite": self.favorite,
            "variant": self.variants,
        });
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

    /// `Some(None)` when the model was last used with its default variant.
    pub fn variant(&self, id: &str) -> Option<Option<String>> {
        let value = self.variants.get(id)?.as_str()?;
        Some((value != "default").then(|| value.to_string()))
    }

    pub fn set_variant(&mut self, id: &str, variant: Option<&str>) {
        let value = variant.unwrap_or("default");
        self.variants.insert(id.to_string(), json!(value));
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_remembered_per_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.json");
        let mut models = Models::load(path.clone());
        models.set_variant("a", Some("high"));
        models.set_variant("b", None);
        let again = Models::load(path);
        assert_eq!(again.variant("a"), Some(Some("high".into())));
        assert_eq!(again.variant("b"), Some(None));
        assert_eq!(again.variant("c"), None);
    }
}
