//! Opening a folder, or a file of it at a line, in the user's editor: the editors found on this
//! computer (PATH first, then where their Windows installers put them), the one picked in
//! Settings. With none found a folder opens in the file manager, and a file is shown in its
//! folder there: never opened with its default app, which for a script or a program (a link the
//! model wrote can name one) would run it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, text};

/// How an editor is told to go to a line.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Style {
    /// VS Code and its forks: `<folder> -g <file>:<line>`, in the folder's window.
    VsCode,
    /// `<file>:<line>` (Zed, Sublime Text).
    Suffix,
    /// JetBrains IDEs: `--line <n> <file>`.
    JetBrains,
    /// Notepad++: `-n<line> <file>`.
    NotepadPlusPlus,
}

/// Where a Windows installer puts an editor: a base folder from the environment and the rest.
type Installed = (&'static str, &'static str);

struct Candidate {
    id: &'static str,
    name: &'static str,
    /// Names tried on PATH, in order.
    commands: &'static [&'static str],
    /// Tried before PATH on Windows: an `.exe` needs no `cmd.exe` in between, which a `.cmd`
    /// shim does (and std refuses arguments it can't quote for it).
    windows: &'static [Installed],
    style: Style,
}

const LOCAL: &str = "LOCALAPPDATA";
const PROGRAMS: &str = "ProgramFiles";

const CANDIDATES: &[Candidate] = &[
    Candidate {
        id: "vscode",
        name: "VS Code",
        commands: &["code"],
        windows: &[
            (LOCAL, r"Programs\Microsoft VS Code\Code.exe"),
            (PROGRAMS, r"Microsoft VS Code\Code.exe"),
        ],
        style: Style::VsCode,
    },
    Candidate {
        id: "cursor",
        name: "Cursor",
        commands: &["cursor"],
        windows: &[(LOCAL, r"Programs\cursor\Cursor.exe")],
        style: Style::VsCode,
    },
    Candidate {
        id: "windsurf",
        name: "Windsurf",
        commands: &["windsurf"],
        windows: &[(LOCAL, r"Programs\Windsurf\Windsurf.exe")],
        style: Style::VsCode,
    },
    Candidate {
        id: "vscode-insiders",
        name: "VS Code Insiders",
        commands: &["code-insiders"],
        windows: &[(
            LOCAL,
            r"Programs\Microsoft VS Code Insiders\Code - Insiders.exe",
        )],
        style: Style::VsCode,
    },
    Candidate {
        id: "vscodium",
        name: "VSCodium",
        commands: &["codium"],
        windows: &[
            (LOCAL, r"Programs\VSCodium\VSCodium.exe"),
            (PROGRAMS, r"VSCodium\VSCodium.exe"),
        ],
        style: Style::VsCode,
    },
    Candidate {
        id: "zed",
        name: "Zed",
        commands: &["zed", "zeditor"],
        windows: &[(LOCAL, r"Programs\Zed\zed.exe")],
        style: Style::Suffix,
    },
    Candidate {
        id: "idea",
        name: "IntelliJ IDEA",
        commands: &["idea", "idea64"],
        windows: &[(LOCAL, r"JetBrains\Toolbox\scripts\idea.cmd")],
        style: Style::JetBrains,
    },
    Candidate {
        id: "pycharm",
        name: "PyCharm",
        commands: &["pycharm", "pycharm64"],
        windows: &[(LOCAL, r"JetBrains\Toolbox\scripts\pycharm.cmd")],
        style: Style::JetBrains,
    },
    Candidate {
        id: "webstorm",
        name: "WebStorm",
        commands: &["webstorm", "webstorm64"],
        windows: &[(LOCAL, r"JetBrains\Toolbox\scripts\webstorm.cmd")],
        style: Style::JetBrains,
    },
    Candidate {
        id: "rustrover",
        name: "RustRover",
        commands: &["rustrover", "rustrover64"],
        windows: &[(LOCAL, r"JetBrains\Toolbox\scripts\rustrover.cmd")],
        style: Style::JetBrains,
    },
    Candidate {
        id: "goland",
        name: "GoLand",
        commands: &["goland", "goland64"],
        windows: &[(LOCAL, r"JetBrains\Toolbox\scripts\goland.cmd")],
        style: Style::JetBrains,
    },
    Candidate {
        id: "sublime",
        name: "Sublime Text",
        commands: &["subl"],
        windows: &[(PROGRAMS, r"Sublime Text\subl.exe")],
        style: Style::Suffix,
    },
    Candidate {
        id: "notepad-plus-plus",
        name: "Notepad++",
        commands: &[],
        windows: &[(PROGRAMS, r"Notepad++\notepad++.exe")],
        style: Style::NotepadPlusPlus,
    },
];

/// An editor found on this computer.
struct Found {
    candidate: &'static Candidate,
    program: PathBuf,
}

fn locate(candidate: &'static Candidate) -> Option<Found> {
    let installed = if cfg!(windows) {
        candidate
            .windows
            .iter()
            .filter_map(|(var, rest)| {
                let base = std::env::var_os(var).filter(|v| !v.is_empty())?;
                Some(PathBuf::from(base).join(rest))
            })
            .find(|path| path.is_file())
    } else {
        None
    };
    let program = installed.or_else(|| {
        candidate
            .commands
            .iter()
            .find_map(|c| anysola_core::which::which(c))
    })?;
    Some(Found { candidate, program })
}

/// The editors found, `preferred` first when it is one of them.
fn found(preferred: Option<&str>) -> Vec<Found> {
    let mut list: Vec<Found> = CANDIDATES.iter().filter_map(locate).collect();
    if let Some(at) = preferred.and_then(|id| list.iter().position(|f| f.candidate.id == id)) {
        let chosen = list.remove(at);
        list.insert(0, chosen);
    }
    list
}

/// A `file://` link as a path: `file:///C:/a%20b` → `C:/a b`.
fn from_file_url(path: &str) -> String {
    let Some(rest) = path.strip_prefix("file://") else {
        return path.to_string();
    };
    // `file://localhost/x` and `file:///x` name the same file.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8_lossy(&out).into_owned();
    // `/C:/x` is a Windows drive path.
    match decoded.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => decoded[1..].to_string(),
        _ => decoded,
    }
}

fn is_absolute(path: &str, windows: bool) -> bool {
    if !windows {
        return path.starts_with('/');
    }
    let bytes = path.as_bytes();
    // `C:\x`, `C:/x`, `\\server\share`, and `\x` (the current drive's root).
    matches!(bytes, [drive, b':', b'\\' | b'/', ..] if drive.is_ascii_alphabetic())
        || path.starts_with(['\\', '/'])
}

/// The file `path` names: as given when absolute, else under `directory`; with Windows'
/// separators on Windows (editors there take either, not every one takes a mix).
fn resolve(directory: &str, path: &str, windows: bool) -> String {
    let path = from_file_url(path.trim());
    let joined = if is_absolute(&path, windows) {
        path
    } else {
        let base = directory.trim_end_matches(['/', '\\']);
        let rest = path.trim_start_matches("./");
        let rest = if windows {
            rest.trim_start_matches(".\\")
        } else {
            rest
        };
        let sep = if windows { '\\' } else { '/' };
        format!("{base}{sep}{rest}")
    };
    if windows {
        joined.replace('/', "\\")
    } else {
        joined
    }
}

/// The arguments that open `folder`, or `file` of it at `line`.
fn arguments(style: Style, folder: &str, file: Option<&str>, line: Option<u32>) -> Vec<String> {
    let line = line.filter(|n| *n > 0);
    let Some(file) = file else {
        return match style {
            Style::NotepadPlusPlus => vec!["-openFoldersAsWorkspace".into(), folder.into()],
            _ => vec![folder.into()],
        };
    };
    match (style, line) {
        (Style::VsCode, Some(n)) => vec![folder.into(), "-g".into(), format!("{file}:{n}")],
        (Style::VsCode, None) => vec![folder.into(), file.into()],
        (Style::Suffix, Some(n)) => vec![format!("{file}:{n}")],
        (Style::JetBrains, Some(n)) => vec!["--line".into(), n.to_string(), file.into()],
        (Style::NotepadPlusPlus, Some(n)) => vec![format!("-n{n}"), file.into()],
        (_, None) => vec![file.into()],
    }
}

/// Starts the editor and lets it run on; reaps it so a launcher that exits leaves no zombie.
fn launch(program: &Path, args: &[String], directory: &str) -> std::io::Result<()> {
    let mut command = anysola_core::console::command(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if Path::new(directory).is_dir() {
        command.current_dir(directory);
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The editors on this computer, the preferred one first.
#[tauri::command]
pub async fn editors(app: State<'_, App>) -> Answer {
    let preferred = app.prefs().general.editor;
    let list = tauri::async_runtime::spawn_blocking(move || {
        found(preferred.as_deref())
            .iter()
            .map(|f| json!({"id": f.candidate.id, "name": f.candidate.name}))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(text)?;
    Ok(Value::Array(list))
}

/// Opens the folder, or one of its files at a line, in the preferred editor; with no editor
/// found (or one that won't start), shows it in the file manager.
#[tauri::command]
pub async fn open_in_editor(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    path: Option<String>,
    line: Option<u32>,
) -> Answer {
    let windows = cfg!(windows);
    let file = path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(|p| resolve(&directory, p, windows));
    let target = file.clone().unwrap_or_else(|| directory.clone());
    if !Path::new(&target).exists() {
        return Err(format!("{target} is no longer on this computer."));
    }
    let preferred = app.prefs().general.editor;
    let launched = {
        let (directory, file) = (directory.clone(), file.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let editor = found(preferred.as_deref()).into_iter().next()?;
            let args = arguments(editor.candidate.style, &directory, file.as_deref(), line);
            match launch(&editor.program, &args, &directory) {
                Ok(()) => Some(()),
                Err(error) => {
                    anysola_core::log::warn(
                        "the editor didn't start",
                        &[
                            ("editor", editor.candidate.id),
                            ("error", &error.to_string()),
                        ],
                    );
                    None
                }
            }
        })
        .await
        .map_err(text)?
    };
    if launched.is_none() {
        show(&handle, &target)?;
    }
    Ok(Value::Null)
}

/// What the file manager is asked to do with a path.
#[derive(Debug, PartialEq)]
enum Shown<'a> {
    /// Open this folder.
    Folder(&'a Path),
    /// Select this file in its folder.
    Reveal(&'a Path),
}

fn how_shown(target: &Path) -> Shown<'_> {
    if target.is_dir() {
        Shown::Folder(target)
    } else {
        Shown::Reveal(target)
    }
}

/// Shows the path in the file manager without running anything: a folder is opened, a file
/// selected in its folder (or, where that can't be done, its folder opened).
fn show(handle: &AppHandle, target: &str) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let opener = handle.opener();
    let open = |folder: &Path| {
        opener
            .open_path(folder.to_string_lossy(), None::<&str>)
            .map_err(|e| format!("Couldn’t open {}: {e}", folder.display()))
    };
    match how_shown(Path::new(target)) {
        Shown::Folder(folder) => open(folder),
        Shown::Reveal(file) => match opener.reveal_item_in_dir(file) {
            Ok(()) => Ok(()),
            Err(error) => {
                anysola_core::log::warn(
                    "the file manager didn't show the file",
                    &[("error", &error.to_string())],
                );
                let folder = file
                    .parent()
                    .filter(|p| p.is_dir())
                    .ok_or_else(|| format!("Couldn’t show {target}: {error}"))?;
                open(folder)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_distinct() {
        let mut ids: Vec<_> = CANDIDATES.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CANDIDATES.len());
        for c in CANDIDATES {
            assert!(!c.commands.is_empty() || !c.windows.is_empty(), "{}", c.id);
            for (_, rest) in c.windows {
                assert!(!rest.contains('/'), "{} uses Windows separators", c.id);
            }
        }
        let vscode = &CANDIDATES[0];
        assert_eq!(vscode.windows[0].1, r"Programs\Microsoft VS Code\Code.exe");
    }

    #[test]
    fn arguments_per_style() {
        let a = |style, file, line| arguments(style, "/p", file, line);
        assert_eq!(a(Style::VsCode, None, None), ["/p"]);
        assert_eq!(
            a(Style::VsCode, Some("/p/a.rs"), Some(12)),
            ["/p", "-g", "/p/a.rs:12"]
        );
        assert_eq!(a(Style::VsCode, Some("/p/a.rs"), None), ["/p", "/p/a.rs"]);
        assert_eq!(
            a(Style::VsCode, Some("/p/a.rs"), Some(0)),
            ["/p", "/p/a.rs"]
        );
        assert_eq!(a(Style::Suffix, Some("/p/a.rs"), Some(3)), ["/p/a.rs:3"]);
        assert_eq!(a(Style::Suffix, Some("/p/a.rs"), None), ["/p/a.rs"]);
        assert_eq!(
            a(Style::JetBrains, Some("/p/a.rs"), Some(3)),
            ["--line", "3", "/p/a.rs"]
        );
        assert_eq!(a(Style::JetBrains, None, None), ["/p"]);
        assert_eq!(
            a(Style::NotepadPlusPlus, Some("/p/a.rs"), Some(7)),
            ["-n7", "/p/a.rs"]
        );
        assert_eq!(
            a(Style::NotepadPlusPlus, None, None),
            ["-openFoldersAsWorkspace", "/p"]
        );
        // A path with spaces stays one argument: nothing goes through a shell.
        assert_eq!(
            arguments(
                Style::VsCode,
                r"C:\My Work",
                Some(r"C:\My Work\a b.rs"),
                Some(2)
            ),
            [r"C:\My Work", "-g", r"C:\My Work\a b.rs:2"]
        );
    }

    #[test]
    fn paths_resolve_on_unix() {
        assert_eq!(
            resolve("/home/u/p", "src/a.rs", false),
            "/home/u/p/src/a.rs"
        );
        assert_eq!(
            resolve("/home/u/p/", "./src/a.rs", false),
            "/home/u/p/src/a.rs"
        );
        assert_eq!(resolve("/home/u/p", "/etc/hosts", false), "/etc/hosts");
        assert_eq!(
            resolve("/home/u/p", "file:///tmp/a%20b.rs", false),
            "/tmp/a b.rs"
        );
        assert_eq!(
            resolve("/home/u/p", "../other/x", false),
            "/home/u/p/../other/x"
        );
    }

    #[test]
    fn paths_resolve_on_windows() {
        let r = |d, p| resolve(d, p, true);
        assert_eq!(r(r"C:\work\shop", "src/a.rs"), r"C:\work\shop\src\a.rs");
        assert_eq!(r(r"C:\work\shop\", r"src\a.rs"), r"C:\work\shop\src\a.rs");
        assert_eq!(r(r"C:\work\shop", r".\src\a.rs"), r"C:\work\shop\src\a.rs");
        assert_eq!(r(r"C:\work\shop", "./src/a.rs"), r"C:\work\shop\src\a.rs");
        assert_eq!(r(r"C:\work\shop", r"D:\other\b.rs"), r"D:\other\b.rs");
        assert_eq!(r(r"C:\work\shop", "d:/other/b.rs"), r"d:\other\b.rs");
        assert_eq!(
            r(r"C:\work\shop", r"\\server\share\c.rs"),
            r"\\server\share\c.rs"
        );
        assert_eq!(
            r(r"C:\work\shop", "file:///C:/My%20Work/a.rs"),
            r"C:\My Work\a.rs"
        );
        assert_eq!(r("C:/work/shop", "src/a.rs"), r"C:\work\shop\src\a.rs");
        assert_eq!(
            r(r"\\server\share\proj", "a.rs"),
            r"\\server\share\proj\a.rs"
        );
        // A relative name that only looks like a drive is still relative.
        assert!(!is_absolute("c.rs", true));
        assert!(!is_absolute("src:x", true));
    }

    #[test]
    fn a_file_is_only_ever_revealed() {
        let dir = std::env::temp_dir().join(format!(
            "anysola-editor-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("run.bat");
        std::fs::write(&script, "echo hi").unwrap();
        assert_eq!(how_shown(&dir), Shown::Folder(&dir));
        assert_eq!(how_shown(&script), Shown::Reveal(&script));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_urls() {
        assert_eq!(from_file_url("file://localhost/etc/x"), "/etc/x");
        assert_eq!(from_file_url("file:///C:/a"), "C:/a");
        assert_eq!(from_file_url("plain/path%20x"), "plain/path%20x");
        assert_eq!(from_file_url("file:///bad%zz"), "/bad%zz");
    }
}
