//! `{env:VAR}` and `{file:path}` substitution in config text.

use std::path::Path;

fn json_escape(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_default();
    quoted[1..quoted.len() - 1].to_string()
}

fn substitute_env(text: &str, escape: bool) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{env:") {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let name = &rest[start + 5..start + end];
        out.push_str(&rest[..start]);
        if name.is_empty() {
            out.push_str(&rest[start..start + end + 1]);
        } else {
            let value = std::env::var(name).unwrap_or_default();
            out.push_str(&if escape { json_escape(&value) } else { value });
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

fn substitute_files(text: &str, dir: &Path, home: &Path) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    while let Some(found) = text[cursor..].find("{file:") {
        let index = cursor + found;
        let Some(end) = text[index..].find('}') else {
            break;
        };
        let token = &text[index..index + end + 1];
        out.push_str(&text[cursor..index]);
        cursor = index + token.len();
        let line_start = text[..index].rfind('\n').map_or(0, |p| p + 1);
        if text[line_start..index].trim_start().starts_with("//") {
            out.push_str(token);
            continue;
        }
        let reference = &token[6..token.len() - 1];
        let path = match reference.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => dir.join(reference),
        };
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        out.push_str(&json_escape(content.trim()));
    }
    out.push_str(&text[cursor..]);
    out
}

/// Substitutes variables; env values are JSON-escaped only when the raw result does not parse.
pub fn substitute(text: &str, dir: &Path, home: &Path) -> String {
    let raw = substitute_files(&substitute_env(text, false), dir, home);
    if !text.contains("{env:") || super::jsonc::parse(&raw).is_ok() {
        return raw;
    }
    substitute_files(&substitute_env(text, true), dir, home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_and_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret.txt"), "line \"one\"\n").unwrap();
        let text = "{\"a\": \"{file:secret.txt}\",\n // \"{file:missing}\"\n \"b\": \"{env:ANYSOLA_TEST_UNSET_VAR}\"}";
        let out = substitute(text, dir.path(), dir.path());
        assert_eq!(
            out,
            "{\"a\": \"line \\\"one\\\"\",\n // \"{file:missing}\"\n \"b\": \"\"}"
        );
    }
}
