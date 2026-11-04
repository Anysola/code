//! Models offered by the Anysola provider (from the gateway's `/config`) and their variants.

use serde_json::{Map, Value, json};

use crate::config::Config;

pub const PROVIDER: &str = "anysola";

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub id: String,
    /// Thinking settings merged over the default (`{"type":"adaptive"}` …).
    pub thinking: Option<Value>,
    pub effort: Option<String>,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub context: Option<u64>,
    pub output: Option<u64>,
    pub supports_binding: Option<bool>,
    pub supports_effort_updates: Option<bool>,
    /// Input modalities (`capabilities.input`, default text and image).
    pub input: Vec<String>,
    pub variants: Vec<Variant>,
}

/// A model reference `provider/model[#variant]` as the user and the session store it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub provider: String,
    pub id: String,
    pub variant: Option<String>,
}

impl ModelRef {
    pub fn parse(text: &str) -> Option<ModelRef> {
        let (base, variant) = match text.split_once('#') {
            Some((base, variant)) => (base, Some(variant.to_string()).filter(|v| !v.is_empty())),
            None => (text, None),
        };
        let (provider, id) = base.split_once('/')?;
        if provider.is_empty() || id.is_empty() {
            return None;
        }
        Some(ModelRef {
            provider: provider.into(),
            id: id.into(),
            variant,
        })
    }

    /// Stored form on sessions: `{"id","providerID","variant"?}`.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), Value::String(self.id.clone()));
        map.insert("providerID".into(), Value::String(self.provider.clone()));
        if let Some(variant) = &self.variant {
            map.insert("variant".into(), Value::String(variant.clone()));
        }
        Value::Object(map)
    }

    pub fn from_json(value: &Value) -> Option<ModelRef> {
        Some(ModelRef {
            provider: value.get("providerID")?.as_str()?.to_string(),
            id: value.get("id")?.as_str()?.to_string(),
            variant: value
                .get("variant")
                .and_then(Value::as_str)
                .filter(|v| *v != "default")
                .map(str::to_string),
        })
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }
}

/// `claude-<family>-<major>[-<minor>]` anywhere after a `/`, `.` or the start.
pub fn claude_version(id: &str) -> Option<(String, u32, u32)> {
    let lower = id.to_lowercase();
    let mut start = 0;
    while let Some(found) = lower[start..].find("claude-") {
        let at = start + found;
        start = at + 1;
        if at > 0 && !matches!(lower.as_bytes()[at - 1], b'.' | b'/') {
            continue;
        }
        let rest = &lower[at + 7..];
        let family: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase())
            .collect();
        if family.is_empty() || !rest[family.len()..].starts_with('-') {
            continue;
        }
        let after = &rest[family.len() + 1..];
        let major: String = after.chars().take_while(char::is_ascii_digit).collect();
        if major.is_empty() {
            continue;
        }
        let tail = &after[major.len()..];
        let (minor, end) = match tail.chars().next() {
            Some('.' | '-') => {
                let digits: String = tail[1..].chars().take_while(char::is_ascii_digit).collect();
                if (1..=2).contains(&digits.len()) {
                    (digits.parse().unwrap_or(0), &tail[1 + digits.len()..])
                } else {
                    (0, tail)
                }
            }
            _ => (0, tail),
        };
        if end.is_empty() || matches!(end.chars().next(), Some('-' | ':' | '@')) {
            return Some((family, major.parse().ok()?, minor));
        }
    }
    None
}

impl Model {
    fn from_config(id: &str, info: &Value) -> Model {
        let text = |path: &[&str]| {
            path.iter()
                .try_fold(info, |v, k| v.get(*k))
                .and_then(Value::as_str)
        };
        let number = |path: &[&str]| {
            path.iter()
                .try_fold(info, |v, k| v.get(*k))
                .and_then(Value::as_u64)
        };
        let flag = |key: &str| {
            info.get("compatibility")
                .and_then(|c| c.get(key))
                .and_then(Value::as_bool)
        };
        let mut variants: Vec<Variant> = info
            .get("variants")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(variant_from_config).collect())
            .unwrap_or_default();
        let efforts = info
            .get("capabilities")
            .and_then(|c| c.get("reasoning"))
            .and_then(|r| r.get("effort"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for effort in efforts.iter().filter_map(Value::as_str) {
            if !variants.iter().any(|v| v.id == effort) {
                variants.push(Variant {
                    id: effort.to_string(),
                    thinking: Some(json!({"type": "adaptive", "display": "summarized"})),
                    effort: Some(effort.to_string()),
                    headers: Vec::new(),
                });
            }
        }
        Model {
            id: id.to_string(),
            name: text(&["name"]).unwrap_or(id).to_string(),
            context: number(&["limit", "context"]),
            output: number(&["limit", "output"]),
            supports_binding: flag("supportsThinkingBlockBinding"),
            supports_effort_updates: flag("supportsEffortUpdates"),
            input: info
                .get("capabilities")
                .and_then(|c| c.get("input"))
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_else(|| vec!["text".into(), "image".into()]),
            variants,
        }
    }

    pub fn variant(&self, id: &str) -> Option<&Variant> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// Thinking block binding: Claude ≥ 5.1 ids, or the gateway's compatibility flag.
    pub fn thinking_binding(&self) -> bool {
        self.supports_binding.unwrap_or_else(|| {
            claude_version(&self.id)
                .is_some_and(|(_, major, minor)| major > 5 || (major == 5 && minor >= 1))
        })
    }

    /// Mid-conversation effort changes: Claude Opus ≥ 5 and every Claude ≥ 5.1, or the gateway's
    /// compatibility flag.
    pub fn effort_updates(&self) -> bool {
        self.supports_effort_updates.unwrap_or_else(|| {
            claude_version(&self.id).is_some_and(|(family, major, minor)| {
                (family == "opus" && major >= 5) || major > 5 || (major == 5 && minor >= 1)
            })
        })
    }

    /// Instruction updates go out as native system messages only for Claude ≥ 5 ids.
    pub fn native_system_updates(&self) -> bool {
        claude_version(&self.id).is_some_and(|(family, major, minor)| {
            if family == "opus" && major == 4 {
                minor >= 8
            } else {
                major >= 5
            }
        })
    }
}

fn variant_from_config(value: &Value) -> Option<Variant> {
    let id = value.get("id")?.as_str()?.to_string();
    let settings = value.get("settings");
    let headers = value
        .get("headers")
        .and_then(Value::as_object)
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Some(Variant {
        id,
        thinking: settings.and_then(|s| s.get("thinking")).cloned(),
        effort: settings
            .and_then(|s| s.get("effort"))
            .and_then(Value::as_str)
            .map(str::to_string),
        headers,
    })
}

/// The models of the merged config, in config order.
pub fn catalog(config: &Config) -> Vec<Model> {
    config
        .models()
        .map(|models| {
            models
                .iter()
                .map(|(id, info)| Model::from_config(id, info))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("Invalid model reference: {0}")]
    Invalid(String),
    #[error("Model unavailable: {0}")]
    Unavailable(String),
    #[error("Variant unavailable for {0}: {1}")]
    Variant(String, String),
}

/// Resolves a reference against the catalog.
pub fn resolve(models: &[Model], reference: &ModelRef) -> Result<Model, ModelError> {
    let model = models
        .iter()
        .find(|m| reference.provider == PROVIDER && m.id == reference.id)
        .ok_or_else(|| ModelError::Unavailable(reference.label()))?;
    if let Some(variant) = &reference.variant
        && model.variant(variant).is_none()
    {
        return Err(ModelError::Variant(reference.label(), variant.clone()));
    }
    Ok(model.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_versions() {
        assert_eq!(
            claude_version("claude-sonnet-5-5"),
            Some(("sonnet".into(), 5, 5))
        );
        assert_eq!(
            claude_version("anthropic/claude-opus-4-8"),
            Some(("opus".into(), 4, 8))
        );
        assert_eq!(
            claude_version("claude-haiku-4-5-20251001"),
            Some(("haiku".into(), 4, 5))
        );
        assert_eq!(claude_version("auto"), None);
    }

    #[test]
    fn refs() {
        let r = ModelRef::parse("anysola/auto#economy").unwrap();
        assert_eq!(
            (r.provider.as_str(), r.id.as_str(), r.variant.as_deref()),
            ("anysola", "auto", Some("economy"))
        );
        assert!(ModelRef::parse("nope").is_none());
        assert_eq!(
            r.to_json(),
            json!({"id": "auto", "providerID": "anysola", "variant": "economy"})
        );
    }
}
