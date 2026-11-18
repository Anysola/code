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

/// The most a `{file:…}` placeholder reads.
const MAX_FILE: u64 = 1 << 20;

/// A placeholder's file: missing reads as empty (as always); too large, or not a regular file in
/// a project document (a device or a pipe could hang the load), is refused with the reason.
fn read(path: &Path, project: bool) -> Result<String, &'static str> {
    use std::io::Read;
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(String::new());
    };
    if project && !meta.is_file() {
        return Err("it is not a regular file");
    }
    if meta.is_file() && meta.len() > MAX_FILE {
        return Err("it is larger than 1 MiB");
    }
    let Ok(file) = std::fs::File::open(path) else {
        return Ok(String::new());
    };
    let mut bytes = Vec::new();
    if file.take(MAX_FILE + 1).read_to_end(&mut bytes).is_err() {
        return Ok(String::new());
    }
    if bytes.len() as u64 > MAX_FILE {
        return Err("it is larger than 1 MiB");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn substitute_files(
    text: &str,
    dir: &Path,
    home: &Path,
    project: bool,
    problems: &mut Vec<String>,
) -> String {
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
        match read(&path, project) {
            Ok(content) => out.push_str(&json_escape(content.trim())),
            Err(reason) => {
                let problem = format!("left {token} as written: {reason}");
                if !problems.contains(&problem) {
                    problems.push(problem);
                }
                out.push_str(token);
            }
        }
    }
    out.push_str(&text[cursor..]);
    out
}

/// Substitutes variables; env values are JSON-escaped only when the raw result does not parse.
#[cfg(test)]
fn substitute(text: &str, dir: &Path, home: &Path) -> String {
    checked(text, dir, home, false, &mut Vec::new())
}

/// [`substitute`] with the `{file:…}` limits of a project (`project`) or user document; what it
/// left as written is added to `problems`.
pub(crate) fn checked(
    text: &str,
    dir: &Path,
    home: &Path,
    project: bool,
    problems: &mut Vec<String>,
) -> String {
    let raw = substitute_files(&substitute_env(text, false), dir, home, project, problems);
    if !text.contains("{env:") || super::jsonc::parse(&raw).is_ok() {
        return raw;
    }
    substitute_files(&substitute_env(text, true), dir, home, project, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_placeholders_are_capped_and_projects_read_regular_files_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("small.txt"), "ok").unwrap();
        std::fs::write(
            dir.path().join("big.txt"),
            "x".repeat(MAX_FILE as usize + 1),
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        let text = r#"{"a": "{file:small.txt}", "b": "{file:big.txt}", "c": "{file:folder}"}"#;
        let mut problems = Vec::new();
        let out = checked(text, dir.path(), dir.path(), true, &mut problems);
        assert_eq!(
            out,
            r#"{"a": "ok", "b": "{file:big.txt}", "c": "{file:folder}"}"#
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        let mut problems = Vec::new();
        let out = checked(text, dir.path(), dir.path(), false, &mut problems);
        assert!(
            out.contains(r#""b": "{file:big.txt}""#),
            "the cap holds for users too"
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_device_is_never_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let text = r#"{"a": "{file:/dev/zero}"}"#;
        let started = std::time::Instant::now();
        for project in [true, false] {
            let mut problems = Vec::new();
            let out = checked(text, dir.path(), dir.path(), project, &mut problems);
            assert_eq!(out, text);
            assert_eq!(problems.len(), 1);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

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
