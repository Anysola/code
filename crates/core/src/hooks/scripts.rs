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

/// The command line split into words as a shell would (quotes), also at unquoted `;`, `|`, `&`,
/// `<`, `>`, `(` and `)`. `escapes`: a backslash escapes the next character, as in a POSIX
/// shell; without it a backslash is itself, as in `cmd` and PowerShell, where it separates the
/// parts of a path.
fn words(command: &str, escapes: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut chars = command.chars();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') if escapes => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') if escapes => {
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
    if let Some(rest) = word.strip_prefix("~/").or_else(|| word.strip_prefix("~\\")) {
        return format!("{home}/{rest}");
    }
    let mut out = word.to_string();
    // The longer spellings first: `$env:HOME` holds `$HOME`'s `$` and nothing else of it.
    for (name, value) in [
        ("CLAUDE_PROJECT_DIR", &project),
        ("ANYSOLA_PROJECT_DIR", &project),
        ("USERPROFILE", &home),
        ("HOME", &home),
    ] {
        out = out
            .replace(&format!("${{env:{name}}}"), value)
            .replace(&format!("$env:{name}"), value)
            .replace(&format!("%{name}%"), value)
            .replace(&format!("${{{name}}}"), value)
            .replace(&format!("${name}"), value);
    }
    out
}

/// The files a hook command names: each word after expansion, as an absolute path or relative
/// to the project root or the session directory. Directories are not files; a word that looks
/// like a path (a separator or an extension) names its file even before it exists. The shell
/// that will run the command is not known here, so the line is read both ways, with a backslash
/// as an escape and as a path separator; a file either reading names is protected.
pub fn files(command: &str, places: &Places<'_>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // A word read POSIX-style from a line with backslashes may have lost them (`C:\Users\…`
    // read as `C:Users…`): from that reading only files that exist are taken.
    let backslashes = command.contains('\\');
    let mut read: Vec<(String, bool)> = words(command, true)
        .into_iter()
        .map(|w| (w, !backslashes))
        .collect();
    read.extend(words(command, false).into_iter().map(|w| (w, true)));
    for (word, may_be_new) in read {
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
        // A script not there yet is guarded too: creating it would decide what the hook runs.
        let pathlike = word.contains('/') || word.contains('\\') || path.extension().is_some();
        let existing: Vec<PathBuf> = candidates.iter().filter(|c| c.is_file()).cloned().collect();
        let named = if !existing.is_empty() {
            existing
        } else if pathlike && may_be_new {
            candidates.into_iter().filter(|c| !c.exists()).collect()
        } else {
            Vec::new()
        };
        for candidate in named {
            if !out.contains(&candidate) {
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
        assert_eq!(words("sh ./x.sh", true), ["sh", "./x.sh"]);
        assert_eq!(
            words(r#"node "$CLAUDE_PROJECT_DIR/hooks/a.mjs" --flag"#, true),
            ["node", "$CLAUDE_PROJECT_DIR/hooks/a.mjs", "--flag"]
        );
        assert_eq!(words("'/a b/c.sh' x", true), ["/a b/c.sh", "x"]);
        assert_eq!(words("a.sh&&b.sh;c", true), ["a.sh", "b.sh", "c"]);
        assert_eq!(words(r"a\ b ''", true), ["a b", ""]);
        // `cmd` and PowerShell: a backslash is part of the path.
        assert_eq!(
            words(
                r#"pwsh -File "C:\Users\me\hooks dir\guard.ps1" -Strict"#,
                false
            ),
            [
                "pwsh",
                "-File",
                r"C:\Users\me\hooks dir\guard.ps1",
                "-Strict"
            ]
        );
        assert_eq!(words(r"a\ b", false), [r"a\", "b"]);
    }

    #[test]
    fn windows_spellings_expand() {
        let places = Places {
            home: Path::new("/h"),
            project: Path::new("/p"),
            session: Path::new("/p"),
        };
        for (word, expanded) in [
            (r"%USERPROFILE%\hooks\g.ps1", r"/h\hooks\g.ps1"),
            (r"$env:USERPROFILE\g.ps1", r"/h\g.ps1"),
            ("${env:CLAUDE_PROJECT_DIR}/g.ps1", "/p/g.ps1"),
            (r"%CLAUDE_PROJECT_DIR%\g.cmd", r"/p\g.cmd"),
            (r"~\hooks\g.ps1", r"/h/hooks\g.ps1"),
            ("$HOME/g.sh", "/h/g.sh"),
        ] {
            assert_eq!(expand(word, &places), expanded, "{word}");
        }
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
        // A bare name that is a file of the project is that file.
        write(&project.join("guard"));
        assert_eq!(files("sh guard"), [project.join("guard")]);
        assert!(files("npm test --silent").is_empty());
        // A script the command names that isn't there yet: creating it is guarded too.
        assert_eq!(
            files(r#"node "$CLAUDE_PROJECT_DIR/hooks/new.mjs""#),
            [project.join("hooks/new.mjs")]
        );
        assert_eq!(
            files("sh scripts/later.sh"),
            [
                project.join("scripts/later.sh"),
                session.join("scripts/later.sh")
            ]
        );
        assert!(files("ls hooks").is_empty(), "a directory is not a file");
    }
}
