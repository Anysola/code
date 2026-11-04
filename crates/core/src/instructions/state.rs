//! The instruction baseline frozen per session and the updates after it. Each source's value is
//! hashed (sha256 of canonical JSON, as 0.5.1 hashed it, so imported state stays valid); the first
//! step stores the baseline, later changes become one `system` message with every source's update
//! text. The system prompt keeps rendering the baseline, so its bytes stay cacheable.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::gateway::error::SessionError;
use crate::storage::{Db, Values};

/// What a source reads right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Observed {
    Value(Value),
    /// The read failed for now; the stored value stands.
    Unavailable,
    /// The source exists but has nothing to say.
    Removed,
}

pub type Render = Box<dyn Fn(&Value) -> Option<String>>;
pub type RenderChange = Box<dyn Fn(&Value, &Value) -> Option<String>>;

/// One instruction source in block 2. Renderers return `None` for a value they cannot read.
pub struct Source {
    pub key: String,
    pub observed: Observed,
    pub initial: Render,
    pub changed: RenderChange,
    pub removed: Render,
}

pub fn never(_: &Value) -> Option<String> {
    None
}

/// The baseline text for block 2 and, when something changed, the update recorded for the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub baseline: String,
    pub update: Option<String>,
}

/// `JSON.stringify` with object keys sorted.
pub fn canonical(value: &Value) -> String {
    match value {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let entries: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical(&map[k])))
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        other => other.to_string(),
    }
}

pub fn hash(value: &Value) -> String {
    hex(&Sha256::digest(canonical(value).as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn storage(error: crate::storage::StorageError) -> SessionError {
    SessionError::new("unknown", error.to_string())
}

fn load(db: &Db, values: &Values) -> Result<BTreeMap<String, Value>, SessionError> {
    let mut out = BTreeMap::new();
    for (key, hash) in values {
        let value = db.instruction_blob(hash).map_err(storage)?.ok_or_else(|| {
            SessionError::new("unknown", format!("Instruction blob not found: {hash}"))
        })?;
        out.insert(key.clone(), value);
    }
    Ok(out)
}

fn render_initial(sources: &[Source], values: &BTreeMap<String, Value>) -> String {
    sources
        .iter()
        .filter_map(|s| values.get(&s.key).and_then(|v| (s.initial)(v)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Records what the sources show now. The first call stores the baseline; later calls record a
/// `system` message when anything changed. Unavailable sources block only the first call.
pub fn prepare(db: &Db, session_id: &str, sources: &[Source]) -> Result<Prepared, SessionError> {
    let stored = db.instruction_state(session_id).map_err(storage)?;
    if stored.is_none() {
        let blocked: Vec<&str> = sources
            .iter()
            .filter(|s| s.observed == Observed::Unavailable)
            .map(|s| s.key.as_str())
            .collect();
        if !blocked.is_empty() {
            return Err(SessionError::new(
                "unknown",
                format!(
                    "Instruction initialization blocked by unavailable sources: {}",
                    blocked.join(", ")
                ),
            ));
        }
    }
    let previous = stored
        .as_ref()
        .map(|s| s.current.clone())
        .unwrap_or_default();
    let mut current = previous.clone();
    let mut blobs = BTreeMap::new();
    let mut changed: Vec<(&Source, Option<Value>)> = Vec::new();
    for source in sources {
        match &source.observed {
            Observed::Unavailable => {}
            Observed::Removed => {
                if current.remove(&source.key).is_some() {
                    changed.push((source, None));
                }
            }
            Observed::Value(value) => {
                let next = hash(value);
                if previous.get(&source.key) == Some(&next) {
                    continue;
                }
                current.insert(source.key.clone(), next.clone());
                blobs.insert(next, value.clone());
                changed.push((source, Some(value.clone())));
            }
        }
    }
    db.instruction_blobs_put(&blobs).map_err(storage)?;
    let Some(state) = stored else {
        let seq = db.last_seq(session_id).map_err(storage)?;
        db.instruction_initialize(session_id, seq, &current)
            .map_err(storage)?;
        return Ok(Prepared {
            baseline: render_initial(sources, &load(db, &current)?),
            update: None,
        });
    };
    let mut update = None;
    if !changed.is_empty() {
        let before = load(db, &previous)?;
        let text = changed
            .iter()
            .filter_map(|(source, value)| match (before.get(&source.key), value) {
                (Some(old), None) => (source.removed)(old),
                (Some(old), Some(new)) => (source.changed)(old, new),
                (None, Some(new)) => (source.initial)(new),
                (None, None) => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let seq = if text.is_empty() {
            db.last_seq(session_id).map_err(storage)?
        } else {
            let data = json!({"time": {"created": crate::time::now_ms()}, "text": text});
            db.append_message(session_id, &crate::ids::message(), "system", &data)
                .map_err(storage)?
        };
        db.instruction_advance(session_id, seq, &current)
            .map_err(storage)?;
        update = (!text.is_empty()).then_some(text);
    }
    Ok(Prepared {
        baseline: render_initial(sources, &load(db, &state.initial)?),
        update,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(v: &Value) -> Option<String> {
        v.as_str().map(|s| format!("is {s}"))
    }

    fn changed(_: &Value, v: &Value) -> Option<String> {
        v.as_str().map(|s| format!("now {s}"))
    }

    fn gone(_: &Value) -> Option<String> {
        Some("gone".into())
    }

    fn source(key: &str, observed: Observed) -> Source {
        Source {
            key: key.into(),
            observed,
            initial: Box::new(text),
            changed: Box::new(changed),
            removed: Box::new(gone),
        }
    }

    fn value(s: &str) -> Observed {
        Observed::Value(Value::String(s.into()))
    }

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        db.create_session(&crate::storage::SessionRow {
            id: "ses_1".into(),
            project_id: "global".into(),
            parent_id: None,
            fork_session_id: None,
            fork_boundary: None,
            slug: "s".into(),
            directory: "/".into(),
            title: "t".into(),
            version: "v".into(),
            agent: None,
            model: None,
            time_created: 1,
            time_updated: 1,
        })
        .unwrap();
        (dir, db)
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let v = json!({"path": "/a", "content": "x\n\"y\"", "n": [1, {"b": 1, "a": null}]});
        assert_eq!(
            canonical(&v),
            r#"{"content":"x\n\"y\"","n":[1,{"a":null,"b":1}],"path":"/a"}"#
        );
        assert_eq!(hash(&json!("a")).len(), 64);
    }

    #[test]
    fn baseline_stays_frozen_and_changes_become_updates() {
        let (_dir, db) = db();
        let first = prepare(
            &db,
            "ses_1",
            &[source("a", value("1")), source("b", value("x"))],
        )
        .unwrap();
        assert_eq!(
            first,
            Prepared {
                baseline: "is 1\n\nis x".into(),
                update: None
            }
        );
        let same = prepare(
            &db,
            "ses_1",
            &[source("a", value("1")), source("b", value("x"))],
        )
        .unwrap();
        assert_eq!(same.update, None);
        let next = prepare(
            &db,
            "ses_1",
            &[
                source("a", value("2")),
                source("b", Observed::Removed),
                source("c", value("new")),
            ],
        )
        .unwrap();
        assert_eq!(
            next.baseline, "is 1\n\nis x",
            "the baseline stays as first seen"
        );
        assert_eq!(next.update.as_deref(), Some("now 2\n\ngone\n\nis new"));
        let rows = db.messages("ses_1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "system");
        assert_eq!(
            db.instruction_state("ses_1").unwrap().unwrap().through_seq,
            1
        );
        let unavailable = prepare(&db, "ses_1", &[source("a", Observed::Unavailable)]).unwrap();
        assert_eq!(
            unavailable.update, None,
            "an unavailable source keeps its value"
        );
    }

    #[test]
    fn unavailable_blocks_the_first_baseline() {
        let (_dir, db) = db();
        let error = prepare(
            &db,
            "ses_1",
            &[source("core/instructions", Observed::Unavailable)],
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "Instruction initialization blocked by unavailable sources: core/instructions"
        );
        assert!(db.instruction_state("ses_1").unwrap().is_none());
    }
}
