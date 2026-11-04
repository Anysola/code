//! Wildcard patterns of permission rules: `*` matches anything (including `/` and newlines), `?` one
//! character, and a pattern ending in ` *` also matches without that suffix (`git *` matches `git`).
//! Backslashes count as `/`; matching is case-insensitive on Windows only.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use regex::Regex;

pub fn matches(input: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let normalized = input.replace('\\', "/");
    compiled(pattern).is_match(&normalized)
}

fn compiled(pattern: &str) -> Regex {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(regex) = cache.get(pattern) {
        return regex.clone();
    }
    let regex = build(pattern);
    cache.insert(pattern.to_string(), regex.clone());
    regex
}

fn build(pattern: &str) -> Regex {
    let mut body = String::new();
    for ch in pattern.replace('\\', "/").chars() {
        match ch {
            '*' => body.push_str(".*"),
            '?' => body.push('.'),
            _ => body.push_str(&regex::escape(&ch.to_string())),
        }
    }
    if let Some(stripped) = body.strip_suffix(" .*") {
        body = format!("{stripped}( .*)?");
    }
    let flags = if cfg!(windows) { "(?si)" } else { "(?s)" };
    Regex::new(&format!("{flags}^{body}$")).expect("escaped wildcard is a valid regex")
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn wildcard_semantics() {
        assert!(matches("src/a.ts", "*"));
        assert!(matches("a\nb", "a*"));
        assert!(matches("git", "git *"));
        assert!(matches("git status", "git *"));
        assert!(!matches("gitk", "git *"));
        assert!(matches("x.env", "*.env"));
        assert!(!matches("x.env.example", "*.env"));
        assert!(matches("x.env.example", "*.env.*"));
        assert!(matches("ab", "a?"));
        assert!(matches("dir\\file", "dir/*"));
        assert!(!matches("a.b", "a+b"));
    }
}
