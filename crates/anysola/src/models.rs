//! `anysola models`: one `provider/model` line per model the gateway offers, sorted as
//! `localeCompare` sorts them.

use std::cmp::Ordering;

use anysola_core::config::{self, LoadOptions};
use anysola_core::gateway::{Gateway, Token};
use anysola_core::model::{PROVIDER, catalog};
use anysola_core::paths::Paths;
use anysola_core::storage::Db;

pub async fn models() -> i32 {
    let paths = Paths::from_env();
    let directory = match std::env::current_dir() {
        Ok(directory) => directory,
        Err(error) => return crate::fail(&error.to_string()),
    };
    let loaded = async {
        let db = Db::open(&paths.database())?;
        let gateway = Gateway::new(Token::resolve(&db)?);
        let remote = anysola_core::gateway::config::fetch(&gateway, &db).await;
        Ok::<_, anysola_core::storage::StorageError>(LoadOptions {
            remote: remote.body,
            api_url: gateway.base().to_string(),
        })
    };
    let options = match loaded.await {
        Ok(options) => options,
        Err(error) => return crate::fail(&error.to_string()),
    };
    let config = config::load(&paths, &directory, &options);
    let mut lines: Vec<String> = catalog(&config)
        .iter()
        .map(|model| format!("{PROVIDER}/{}", model.id))
        .collect();
    lines.sort_by(|a, b| locale_compare(a, b));
    for line in lines {
        println!("{line}");
    }
    0
}

/// ICU root collation for ASCII-ish ids: whitespace, then punctuation in ICU's order, then
/// symbols, digits and letters (case-insensitive first, lower case before upper on ties).
fn locale_compare(a: &str, b: &str) -> Ordering {
    const PUNCTUATION: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    let key = |c: char| -> (u8, u32) {
        if c.is_whitespace() {
            (0, c as u32)
        } else if let Some(index) = PUNCTUATION.find(c) {
            (1, index as u32)
        } else if c.is_ascii_digit() {
            (2, c as u32)
        } else if c.is_alphabetic() {
            (3, c.to_lowercase().next().unwrap_or(c) as u32)
        } else {
            (4, c as u32)
        }
    };
    let primary = a.chars().map(key).cmp(b.chars().map(key));
    primary.then_with(|| {
        let case = |c: char| c.is_uppercase();
        a.chars().map(case).cmp(b.chars().map(case))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_locale_compare() {
        let mut ids = vec![
            "anysola/claude-sonnet-5-5",
            "anysola/Zeta",
            "anysola/auto",
            "anysola/claude-haiku-5-5",
            "anysola/claude_x",
        ];
        ids.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(
            ids,
            [
                "anysola/auto",
                "anysola/claude_x",
                "anysola/claude-haiku-5-5",
                "anysola/claude-sonnet-5-5",
                "anysola/Zeta"
            ]
        );
    }
}
