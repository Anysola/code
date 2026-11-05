//! `anysola debug config` (config sources with secrets redacted) and `anysola debug paths`.

use std::path::PathBuf;
use std::sync::LazyLock;

use anysola_core::config::{self, LoadOptions, Source};
use anysola_core::paths::Paths;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::cli::DebugCommand;

pub async fn debug(command: DebugCommand) -> i32 {
    match command {
        DebugCommand::Config => sources(),
        DebugCommand::Paths { name } => paths(name.as_deref()),
    }
}

/// The global locations, keyed as 0.5.1 keyed them, pointing into `~/.anysola`.
fn locations(paths: &Paths) -> [(&'static str, PathBuf); 10] {
    let data = paths.data.clone();
    [
        ("home", paths.home.clone()),
        ("data", data.clone()),
        ("cache", data.join("cache")),
        ("config", data.clone()),
        ("state", paths.state_dir()),
        ("tmp", Paths::temp_dir()),
        ("bin", data.join("bin")),
        ("log", data.join("log")),
        ("repos", data.join("repos")),
        ("db", paths.database()),
    ]
}

fn paths(name: Option<&str>) -> i32 {
    let locations = locations(&Paths::from_env());
    for (key, path) in &locations {
        match name {
            Some(name) if name == *key => println!("{}", path.display()),
            Some(_) => {}
            None => println!("{key:<10} {}", path.display()),
        }
    }
    0
}

fn sources() -> i32 {
    let directory = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => return crate::fail(&error.to_string()),
    };
    let config = config::load(&Paths::from_env(), &directory, &LoadOptions::default());
    let entries: Vec<Value> = config
        .sources
        .iter()
        .map(|source| match source {
            Source::Directory(path) => {
                json!({"type": "directory", "path": path.display().to_string()})
            }
            Source::Document { path, data } => json!({
                "type": "document",
                "path": path,
                "info": redact(&Value::Object(data.clone()), false),
            }),
        })
        .collect();
    println!("{}", anysola_core::json::pretty(&Value::Array(entries)));
    0
}

static SECRET_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)api.?key|secret|password|token$|authorization$|cookie$|credential|private.?key",
    )
    .expect("valid pattern")
});

static WEB_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^https?://").expect("valid pattern"));

fn secret_name(key: &str) -> bool {
    SECRET_NAME.is_match(key)
}

/// URLs carrying credentials (user info, or a secret-looking query parameter) are hidden whole.
fn secret_url(text: &str) -> bool {
    match url::Url::parse(text) {
        Ok(url) => {
            !url.username().is_empty()
                || url.password().is_some()
                || url.query_pairs().any(|(name, _)| secret_name(&name))
        }
        Err(_) => true,
    }
}

/// Hides secrets in debug output only; every string under a `headers` key is one.
fn redact(value: &Value, headers: bool) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(|v| redact(v, headers)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let hidden = match item {
                        Value::String(text) => {
                            headers
                                || secret_name(key)
                                || (WEB_URL.is_match(text) && secret_url(text))
                        }
                        _ => false,
                    };
                    let value = if hidden {
                        json!("***")
                    } else {
                        redact(item, headers || key.eq_ignore_ascii_case("headers"))
                    };
                    (key.clone(), value)
                })
                .collect::<Map<_, _>>(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_headers_and_credential_urls() {
        let config = json!({
            "apiKey": "k", "api_key": "k", "myToken": "t", "tokens": "kept", "nested": {"privateKey": "p"},
            "mcp": {"servers": {"x": {"type": "remote", "url": "https://u:p@h/x", "headers": {"A": "b"}}}},
            "plain": "https://example.com/?q=1", "keyed": "https://example.com/?access_token=1",
            "bad": "http://", "count": 3
        });
        assert_eq!(
            redact(&config, false),
            json!({
                "apiKey": "***", "api_key": "***", "myToken": "***", "tokens": "kept", "nested": {"privateKey": "***"},
                "mcp": {"servers": {"x": {"type": "remote", "url": "***", "headers": {"A": "***"}}}},
                "plain": "https://example.com/?q=1", "keyed": "***",
                "bad": "***", "count": 3
            })
        );
    }
}
