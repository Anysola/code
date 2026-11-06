//! Editor context: the file and selection open in an IDE (Claude Code IDE protocol over a
//! local WebSocket) or in Zed (its state database), sent before a prompt as a hidden note.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    pub text: String,
    /// 1-based `(line, character)` of both ends.
    pub start: (u64, u64),
    pub end: (u64, u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub path: String,
    pub ranges: Vec<Range>,
}

/// What an editor reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Selection(Selection),
    /// `@path#a-b` for the prompt (`at_mentioned`).
    Mention(String),
}

fn label(range: &Range) -> Option<String> {
    if range.start == range.end {
        return None;
    }
    Some(if range.start.0 == range.end.0 {
        format!("#{}", range.start.0)
    } else {
        format!("#{}-{}", range.start.0, range.end.0)
    })
}

/// The hidden note sent before the prompt.
pub fn reminder(selection: &Selection) -> String {
    let selected: Vec<(&Range, String)> = selection
        .ranges
        .iter()
        .filter_map(|r| label(r).map(|l| (r, l)))
        .collect();
    if selected.is_empty() {
        return format!(
            "<system-reminder>Note: The user opened the file \"{}\". This may or may not be relevant to the current task.</system-reminder>\n",
            selection.path
        );
    }
    let many = selected.len() > 1;
    let notes: Vec<String> = selected
        .iter()
        .enumerate()
        .map(|(i, (range, label))| {
            let prefix = if many {
                format!("Selection {}: ", i + 1)
            } else {
                String::new()
            };
            format!(
                "Note: The user selected {prefix}{label} from \"{}\". ```{}```\n\n",
                selection.path, range.text
            )
        })
        .collect();
    format!(
        "<system-reminder>{} This may or may not be relevant to the current task.</system-reminder>\n",
        notes.join("\n")
    )
}

fn position(value: &Value) -> Option<(u64, u64)> {
    Some((
        value.get("line")?.as_u64()?,
        value.get("character")?.as_u64()?,
    ))
}

/// `selection_changed` params: one range (`text`, `selection`) or several (`ranges`).
fn decode_selection(params: &Value) -> Option<Selection> {
    let path = params.get("filePath")?.as_str()?.to_string();
    let range = |v: &Value| -> Option<Range> {
        let selection = v.get("selection")?;
        Some(Range {
            text: v.get("text")?.as_str()?.to_string(),
            start: position(selection.get("start")?)?,
            end: position(selection.get("end")?)?,
        })
    };
    let ranges = match params.get("ranges").and_then(Value::as_array) {
        Some(ranges) => ranges.iter().filter_map(range).collect(),
        None => vec![range(params)?],
    };
    Some(Selection { path, ranges })
}

fn decode_mention(params: &Value) -> Option<String> {
    let path = params.get("filePath")?.as_str()?;
    let start = params.get("lineStart").and_then(Value::as_u64);
    let end = params.get("lineEnd").and_then(Value::as_u64);
    Some(match (start, end) {
        (Some(a), Some(b)) if a != b => format!("@{path}#{a}-{b}"),
        (Some(a), _) => format!("@{path}#{a}"),
        _ => format!("@{path}"),
    })
}

/// Where to connect: the port from the environment, else the best IDE lock file for `cwd`.
pub fn connection(home: &Path, cwd: &Path) -> Option<(u16, Option<String>)> {
    let env = std::env::var("CLAUDE_CODE_SSE_PORT")
        .ok()
        .or_else(|| anysola_core::env::var("EDITOR_SSE_PORT"))
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|p| *p > 0);
    if let Some(port) = env {
        return Some((port, None));
    }
    let entries = std::fs::read_dir(home.join(".claude").join("ide")).ok()?;
    let mut best: Option<(usize, std::time::SystemTime, u16, Option<String>)> = None;
    for entry in entries.flatten() {
        let file = entry.path();
        let Some(port) = file
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".lock"))
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|p| *p > 0)
        else {
            continue;
        };
        let Some(value) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        if value.get("transport").is_some_and(|t| t != "ws") {
            continue;
        }
        let score = value
            .get("workspaceFolders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|folder| cwd.starts_with(folder))
            .map(str::len)
            .max()
            .unwrap_or(0);
        if score == 0 {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let token = value
            .get("authToken")
            .and_then(Value::as_str)
            .map(str::to_string);
        if best.as_ref().is_none_or(|b| (score, mtime) > (b.0, b.1)) {
            best = Some((score, mtime, port, token));
        }
    }
    best.map(|(_, _, port, token)| (port, token))
}

/// Keeps a connection to the IDE, reconnecting with a 1 s backoff doubling to 10 s.
pub async fn watch_ide(port: u16, token: Option<String>, updates: impl Fn(Update)) {
    let mut backoff = Duration::from_secs(1);
    loop {
        if session(port, token.as_deref(), &updates).await {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(10));
    }
}

/// One connection; true when it got as far as connecting.
async fn session(port: u16, token: Option<&str>, updates: &impl Fn(Update)) -> bool {
    let Ok(mut request) = format!("ws://127.0.0.1:{port}").into_client_request() else {
        return false;
    };
    if let Some(token) = token
        && let Ok(value) = token.parse()
    {
        request
            .headers_mut()
            .insert("x-claude-code-ide-authorization", value);
    }
    let Ok((mut socket, _)) = tokio_tungstenite::connect_async(request).await else {
        return false;
    };
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25",
        "capabilities": {},
        "clientInfo": {"name": "opencode", "version": "0.0.0"},
    }});
    if socket
        .send(Message::text(initialize.to_string()))
        .await
        .is_err()
    {
        return true;
    }
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        match value.get("method").and_then(Value::as_str) {
            Some("selection_changed") => {
                if let Some(selection) = decode_selection(&params) {
                    updates(Update::Selection(selection));
                }
            }
            Some("at_mentioned") => {
                if let Some(mention) = decode_mention(&params) {
                    updates(Update::Mention(mention));
                }
            }
            _ if value.get("id") == Some(&json!(1)) && value.get("result").is_some() => {
                let ready = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
                let _ = socket.send(Message::text(ready.to_string())).await;
            }
            _ => {}
        }
    }
    true
}

/// Running inside Zed's terminal.
pub fn in_zed() -> bool {
    std::env::var("ZED_TERM").as_deref() == Ok("true")
        || std::env::var("TERM_PROGRAM").is_ok_and(|t| t.eq_ignore_ascii_case("zed"))
}

/// Zed's state database (macOS and Linux paths; no Windows path, as in 0.5.1).
pub fn zed_db(home: &Path) -> Option<PathBuf> {
    let candidates = [
        anysola_core::env::var_os("ZED_DB").map(PathBuf::from),
        Some(home.join("Library/Application Support/Zed/db/0-stable/db.sqlite")),
        Some(home.join(".local/share/zed/db/0-stable/db.sqlite")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_file())
}

/// `(line, character)`, 1-based, of a byte offset.
fn place(text: &str, offset: usize) -> (u64, u64) {
    let before = &text[..offset];
    let line = before.matches('\n').count() as u64 + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[start..].chars().count() as u64 + 1)
}

fn boundary(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

/// The active editor of the Zed workspace containing `cwd` and its selections.
pub fn zed_selection(db: &Path, cwd: &Path) -> Option<Selection> {
    let conn =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    let mut statement = conn
        .prepare(
            "select i.kind, e.item_id, i.workspace_id, w.paths, w.timestamp, e.buffer_path
             from items i
             join panes p on p.pane_id = i.pane_id and p.workspace_id = i.workspace_id
             join workspaces w on w.workspace_id = i.workspace_id
             left join editors e on e.item_id = i.item_id and e.workspace_id = i.workspace_id
             where i.active = 1 and p.active = 1
             order by w.timestamp desc",
        )
        .ok()?;
    type Row = (
        String,
        Option<i64>,
        i64,
        Option<String>,
        String,
        Option<String>,
    );
    let rows: Vec<Row> = statement
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .ok()?
        .flatten()
        .collect();
    let score = |paths: &Option<String>| {
        let paths = paths.as_deref().unwrap_or_default();
        let list: Vec<String> = serde_json::from_str(paths)
            .unwrap_or_else(|_| paths.lines().map(str::to_string).collect());
        list.iter()
            .filter(|p| cwd.starts_with(p.as_str()))
            .map(String::len)
            .max()
            .unwrap_or(0)
    };
    let row = rows
        .iter()
        .filter(|r| score(&r.3) > 0)
        .max_by(|a, b| score(&a.3).cmp(&score(&b.3)).then(a.4.cmp(&b.4)))?;
    let (kind, Some(editor), workspace, _, _, Some(path)) = row else {
        return None;
    };
    if kind != "Editor" {
        return None;
    }
    let mut ranges: Vec<(usize, usize)> = conn
        .prepare(
            "select start, end from editor_selections where editor_id = ?1 and workspace_id = ?2",
        )
        .ok()?
        .query_map([editor, workspace], |r| {
            Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize))
        })
        .ok()?
        .flatten()
        .map(|(a, b)| (a.min(b), a.max(b)))
        .collect();
    ranges.sort();
    if ranges.is_empty() {
        return None;
    }
    let stored: Option<String> = conn
        .query_row(
            "select contents from editors where item_id = ?1 and workspace_id = ?2",
            [editor, workspace],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let text = stored.or_else(|| std::fs::read_to_string(path).ok())?;
    let ranges = ranges
        .into_iter()
        .map(|(a, b)| {
            let (a, b) = (boundary(&text, a), boundary(&text, b));
            Range {
                text: text[a..b].to_string(),
                start: place(&text, a),
                end: place(&text, b),
            }
        })
        .collect();
    Some(Selection {
        path: path.clone(),
        ranges,
    })
}

/// The footer's file label: `name.rs#12-20 +1` (`index.*` keeps its directory).
pub fn file_label(selection: &Selection) -> String {
    let path = std::path::Path::new(&selection.path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let indexed = name.starts_with("index.") && name.matches('.').count() == 1;
    let file = match path.parent().and_then(|p| p.file_name()) {
        Some(parent) if indexed => format!("{}/{name}", parent.to_string_lossy()),
        _ => name,
    };
    let first = selection
        .ranges
        .iter()
        .find(|r| r.start != r.end)
        .or(selection.ranges.first());
    let mut out = file;
    if let Some(range) = first.and_then(label) {
        out.push_str(&range);
    }
    if selection.ranges.len() > 1 {
        out.push_str(&format!(" +{}", selection.ranges.len() - 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_labels() {
        let range = |a: u64, b: u64| Range {
            text: String::new(),
            start: (a, 1),
            end: (b, 4),
        };
        let selection = |path: &str, ranges: Vec<Range>| Selection {
            path: path.into(),
            ranges,
        };
        assert_eq!(
            file_label(&selection("/w/src/main.rs", vec![range(3, 9)])),
            "main.rs#3-9"
        );
        assert_eq!(
            file_label(&selection(
                "/w/app/index.ts",
                vec![range(4, 4), range(7, 8)]
            )),
            "app/index.ts#4 +1"
        );
    }

    #[test]
    fn reminders() {
        let opened = Selection {
            path: "/w/a.rs".into(),
            ranges: vec![Range {
                text: String::new(),
                start: (3, 1),
                end: (3, 1),
            }],
        };
        assert_eq!(
            reminder(&opened),
            "<system-reminder>Note: The user opened the file \"/w/a.rs\". This may or may not be relevant to the current task.</system-reminder>\n"
        );
        let selected = Selection {
            path: "/w/a.rs".into(),
            ranges: vec![Range {
                text: "fn main".into(),
                start: (1, 1),
                end: (2, 3),
            }],
        };
        assert_eq!(
            reminder(&selected),
            "<system-reminder>Note: The user selected #1-2 from \"/w/a.rs\". ```fn main```\n\n This may or may not be relevant to the current task.</system-reminder>\n"
        );
    }

    #[test]
    fn decodes_ide_messages() {
        let params = json!({"filePath": "/w/a.rs", "text": "x", "selection": {"start": {"line": 4, "character": 1}, "end": {"line": 4, "character": 2}}});
        assert_eq!(decode_selection(&params).unwrap().ranges[0].start, (4, 1));
        assert_eq!(
            decode_mention(&json!({"filePath": "a.rs", "lineStart": 2, "lineEnd": 5})),
            Some("@a.rs#2-5".into())
        );
    }

    #[test]
    fn lock_file_for_the_workspace() {
        let home = tempfile::tempdir().unwrap();
        let ide = home.path().join(".claude/ide");
        std::fs::create_dir_all(&ide).unwrap();
        std::fs::write(
            ide.join("4242.lock"),
            r#"{"workspaceFolders": ["/w"], "authToken": "t", "transport": "ws"}"#,
        )
        .unwrap();
        std::fs::write(ide.join("4343.lock"), r#"{"workspaceFolders": ["/other"]}"#).unwrap();
        if std::env::var_os("CLAUDE_CODE_SSE_PORT").is_none()
            && anysola_core::env::var_os("EDITOR_SSE_PORT").is_none()
        {
            assert_eq!(
                connection(home.path(), Path::new("/w/src")),
                Some((4242, Some("t".into())))
            );
        }
    }

    #[test]
    fn zed_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "create table workspaces(workspace_id integer, paths text, timestamp text);
             create table panes(pane_id integer, workspace_id integer, active integer);
             create table items(item_id integer, workspace_id integer, pane_id integer, kind text, active integer);
             create table editors(item_id integer, workspace_id integer, buffer_path text, contents text);
             create table editor_selections(editor_id integer, workspace_id integer, start integer, end integer);
             insert into workspaces values (1, '[\"/w\"]', '2026');
             insert into panes values (1, 1, 1);
             insert into items values (7, 1, 1, 'Editor', 1);
             insert into editors values (7, 1, '/w/a.rs', 'one\ntwo\n');
             insert into editor_selections values (7, 1, 4, 7);",
        )
        .unwrap();
        let selection = zed_selection(&db, Path::new("/w")).unwrap();
        assert_eq!(selection.path, "/w/a.rs");
        assert_eq!(selection.ranges[0].text, "two");
        assert_eq!(selection.ranges[0].start, (2, 1));
    }
}
