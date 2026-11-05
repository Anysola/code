//! `GET /config`, the Anysola remote config document: fetched at start and every 10 minutes while
//! a credential exists. The last good body is cached in KV `anysola:config` and used whenever the
//! fetch fails, so the model list survives offline restarts.

use std::time::Duration;

use serde_json::{Map, Value};

use super::{ConfigFetch, Gateway};
use crate::storage::Db;

pub const CACHE_KEY: &str = "anysola:config";
pub const REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);
/// The connection status after a `401`.
pub const SIGNED_OUT: &str = "Run `anysola login` to sign in to Anysola";

/// The remote document's body and the connection status it left.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Remote {
    /// `None` without a credential: no remote document at all, not even the cached one.
    pub body: Option<Value>,
    pub needs_auth: bool,
}

/// A body that is not a JSON object (a proxy error page) reads as `{}`.
fn lenient(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(value @ Value::Object(_)) => value,
        _ => Value::Object(Map::new()),
    }
}

pub async fn fetch(gateway: &Gateway, db: &Db) -> Remote {
    if gateway.token().is_none() {
        return Remote::default();
    }
    let status = match gateway.config().await {
        ConfigFetch::Ok(text) => {
            let body = lenient(&text);
            let _ = db.kv_set(CACHE_KEY, &body);
            return Remote {
                body: Some(body),
                needs_auth: false,
            };
        }
        ConfigFetch::Unauthorized => "401".to_string(),
        ConfigFetch::Failed(status) => status.map_or("network".into(), |s| s.to_string()),
    };
    crate::log::warn("failed to load Anysola config", &[("status", &status)]);
    Remote {
        body: db.kv_get(CACHE_KEY).ok().flatten(),
        needs_auth: status == "401",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::Token;
    use serde_json::json;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("a.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn non_object_bodies_read_as_empty() {
        assert_eq!(lenient("<html>"), json!({}));
        assert_eq!(lenient("[1]"), json!({}));
        assert_eq!(
            lenient(r#"{"model":"anysola/m"}"#),
            json!({"model": "anysola/m"})
        );
    }

    #[tokio::test]
    async fn no_credential_means_no_document_even_with_a_cache() {
        let (_dir, db) = db();
        db.kv_set(CACHE_KEY, &json!({"model": "anysola/m"}))
            .unwrap();
        let gateway = Gateway::with_base("http://127.0.0.1:9".into(), None);
        assert_eq!(fetch(&gateway, &db).await, Remote::default());
    }

    #[tokio::test]
    async fn offline_falls_back_to_the_cache() {
        let (_dir, db) = db();
        db.kv_set(CACHE_KEY, &json!({"model": "anysola/m"}))
            .unwrap();
        let gateway = Gateway::with_base(
            "http://127.0.0.1:9".into(),
            Some(Token::Env("asc_x".into())),
        );
        let remote = fetch(&gateway, &db).await;
        assert_eq!(remote.body, Some(json!({"model": "anysola/m"})));
        assert!(!remote.needs_auth);
    }
}
