//! The files hook commands run (`~/.anysola/hooks/guard.sh`, `"$CLAUDE_PROJECT_DIR"/x.mjs`):
//! rewriting one would change what a guard does, so writing it always asks, like a config file.
//! Only the command line is read; what a script sources in turn is not followed.

use std::path::{Path, PathBuf};

/// Where a hook command's relative paths and variables point.
pub struct Places<'a> {
    pub home: &'a Path,
    pub project: &'a Path,
    pub session: &'a Path,
}

/// The command line split into words as a POSIX shell would (quotes and backslashes), also at
/// unquoted `;`, `|`, `&`, `<`, `>`, `(` and `)`.
fn words(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut chars = command.chars();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    started = true;
                }
            }
            (None, c) if c.is_whitespace() || ";|&<>()".contains(c) => {
                if started || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                started = false;
            }
            (None, c) => word.push(c),
        }
    }
    if started || !word.is_empty() {
        out.push(word);
    }
    out
}

/// A word with a leading `~` and the home and project variables expanded.
fn expand(word: &str, places: &Places<'_>) -> String {
    let home = places.home.display().to_string();
    let project = places.project.display().to_string();
    if word == "~" {
        return home;
    }
    if let Some(rest) = word.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    let mut out = word.to_string();
    for (name, value) in [
        ("CLAUDE_PROJECT_DIR", &project),
        ("ANYSOLA_PROJECT_DIR", &project),
        ("HOME", &home),
    ] {
        out = out
            .replace(&format!("${{{name}}}"), value)
            .replace(&format!("${name}"), value);
    }
    out
}

/// The existing files a hook command names: each word after expansion, as an absolute path or
/// relative to the project root or the session directory. Directories are not files.
pub fn files(command: &str, places: &Places<'_>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for word in words(command) {
        let word = expand(&word, places);
        if word.is_empty() {
            continue;
        }
        let path = Path::new(&word);
        let candidates: Vec<PathBuf> = if path.is_absolute() {
            vec![path.to_path_buf()]
        } else {
            vec![places.project.join(path), places.session.join(path)]
        };
        for candidate in candidates {
            if candidate.is_file() && !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_follow_shell_quoting() {
        assert_eq!(words("sh ./x.sh"), ["sh", "./x.sh"]);
        assert_eq!(
            words(r#"node "$CLAUDE_PROJECT_DIR/hooks/a.mjs" --flag"#),
            ["node", "$CLAUDE_PROJECT_DIR/hooks/a.mjs", "--flag"]
        );
        assert_eq!(words("'/a b/c.sh' x"), ["/a b/c.sh", "x"]);
        assert_eq!(words("a.sh&&b.sh;c"), ["a.sh", "b.sh", "c"]);
        assert_eq!(words(r"a\ b ''"), ["a b", ""]);
    }

    #[test]
    fn files_are_the_existing_files_a_command_names() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let project = home.join("proj");
        let session = project.join("sub");
        std::fs::create_dir_all(home.join("hooks")).unwrap();
        std::fs::create_dir_all(project.join("hooks dir")).unwrap();
        std::fs::create_dir_all(&session).unwrap();
        let write = |path: &Path| std::fs::write(path, "exit 0").unwrap();
        write(&home.join("hooks/guard.sh"));
        write(&project.join("x.sh"));
        write(&project.join("hooks dir/a.mjs"));
        write(&session.join("local.sh"));
        let places = Places {
            home: &home,
            project: &project,
            session: &session,
        };
        let files = |command: &str| files(command, &places);
        let guard = home.join("hooks/guard.sh");
        assert_eq!(
            files(&guard.display().to_string()),
            std::slice::from_ref(&guard)
        );
        assert_eq!(files("~/hooks/guard.sh"), std::slice::from_ref(&guard));
        assert_eq!(files("bash ${HOME}/hooks/guard.sh --strict"), [guard]);
        assert_eq!(files("sh ./x.sh"), [project.join("./x.sh")]);
        assert_eq!(
            files(r#"node "$CLAUDE_PROJECT_DIR/hooks dir/a.mjs" --flag"#),
            [project.join("hooks dir/a.mjs")]
        );
        assert_eq!(
            files("'$ANYSOLA_PROJECT_DIR'/x.sh"),
            [project.join("x.sh")],
            "quotes around a variable are only stripped"
        );
        assert_eq!(files("./local.sh"), [session.join("./local.sh")]);
        assert!(files("npm test --silent").is_empty());
        assert!(files("ls hooks").is_empty(), "a directory is not a file");
    }
}
