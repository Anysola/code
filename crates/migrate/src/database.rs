//! The rows of `opencode.db` that 1.0 keeps, copied into `anysola.db` inside the caller's
//! transaction. Ids, sequence numbers and message `data` are kept; columns are read by name so a
//! database touched by a newer upstream OpenCode (extra columns) imports too.

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value as Sql;
use rusqlite::{Connection, Transaction, params_from_iter};
use serde_json::Value;

use crate::{Error, rows};

const MIGRATIONS: &str = include_str!("migrations.txt");

type Row = HashMap<String, Sql>;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    pub sessions: usize,
    pub messages: usize,
    pub projects: usize,
}

fn tables(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut statement = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
    let names = statement.query_map([], |r| r.get(0))?;
    names.collect()
}

fn select(conn: &Connection, sql: &str) -> rusqlite::Result<Vec<Row>> {
    let mut statement = conn.prepare(sql)?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let rows = statement.query_map([], |r| {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| Ok((name.clone(), r.get::<_, Sql>(i)?)))
            .collect()
    })?;
    rows.collect()
}

fn get(row: &Row, column: &str) -> Sql {
    row.get(column).cloned().unwrap_or(Sql::Null)
}

fn text(row: &Row, column: &str) -> Option<String> {
    match get(row, column) {
        Sql::Text(text) => Some(text),
        _ => None,
    }
}

/// Stored paths use `/`; 1.0 stores the platform's separator.
fn native(path: Sql) -> Sql {
    match path {
        Sql::Text(path) if cfg!(windows) => Sql::Text(path.replace('/', "\\")),
        other => other,
    }
}

fn insert(tx: &Transaction, table: &str, values: Vec<(&str, Sql)>) -> rusqlite::Result<()> {
    let columns: Vec<&str> = values.iter().map(|(c, _)| *c).collect();
    let marks = vec!["?"; columns.len()].join(", ");
    let sql = format!(
        "INSERT OR IGNORE INTO {table} ({}) VALUES ({marks})",
        columns.join(", ")
    );
    tx.execute(&sql, params_from_iter(values.into_iter().map(|(_, v)| v)))
        .map(drop)
}

/// Copies `columns` of every row of `from` into `to` unchanged.
fn copy(
    conn: &Connection,
    tx: &Transaction,
    from: &str,
    to: &str,
    columns: &[&str],
) -> Result<usize, Error> {
    let rows = select(conn, &format!("SELECT * FROM {from}"))?;
    for row in &rows {
        insert(tx, to, columns.iter().map(|c| (*c, get(row, c))).collect())?;
    }
    Ok(rows.len())
}

/// Every one of 0.5.1's 59 migrations must have run; later ones (a newer upstream) are fine.
pub fn check_schema(conn: &Connection) -> Result<(), Error> {
    let tables = tables(conn)?;
    if !tables.contains("migration") || !tables.contains("session_v2") {
        return Err(Error::Outdated);
    }
    let done: HashSet<String> = select(conn, "SELECT id FROM migration")?
        .iter()
        .filter_map(|r| text(r, "id"))
        .collect();
    if MIGRATIONS.lines().any(|id| !done.contains(id)) {
        return Err(Error::Outdated);
    }
    if ["session", "message", "part"]
        .iter()
        .any(|t| tables.contains(*t))
    {
        anysola_core::log::info("skipping OpenCode v1 tables session, message and part", &[]);
    }
    Ok(())
}

pub fn import(conn: &Connection, tx: &Transaction) -> Result<Counts, Error> {
    let tables = tables(conn)?;
    let mut counts = Counts {
        projects: copy_projects(conn, tx)?,
        ..Counts::default()
    };
    let sessions = copy_sessions(conn, tx)?;
    counts.sessions = sessions.len();
    counts.messages = copy_messages(conn, tx, &tables)?;
    copy(
        conn,
        tx,
        "instruction_state",
        "instruction_state",
        &[
            "session_id",
            "epoch_start",
            "through_seq",
            "initial_values",
            "current_values",
        ],
    )?;
    copy(
        conn,
        tx,
        "instruction_entry",
        "instruction_entry",
        &["session_id", "key", "value", "removed"],
    )?;
    for row in select(
        conn,
        "SELECT hash, value FROM instruction_blob WHERE value IS NOT NULL",
    )? {
        insert(
            tx,
            "instruction_blob",
            vec![("hash", get(&row, "hash")), ("value", get(&row, "value"))],
        )?;
    }
    copy(
        conn,
        tx,
        "credential",
        "credential",
        &[
            "id",
            "integration_id",
            "label",
            "connector_id",
            "method_id",
            "active",
            "value",
            "time_created",
            "time_updated",
        ],
    )?;
    copy(
        conn,
        tx,
        "permission",
        "permission",
        &[
            "id",
            "project_id",
            "action",
            "resource",
            "time_created",
            "time_updated",
        ],
    )?;
    copy_kv(conn, tx, &sessions)?;
    Ok(counts)
}

fn copy_projects(conn: &Connection, tx: &Transaction) -> Result<usize, Error> {
    let projects = select(conn, "SELECT * FROM project")?;
    for row in &projects {
        let sandboxes = match get(row, "sandboxes") {
            Sql::Null => Sql::Text("[]".into()),
            other => other,
        };
        insert(
            tx,
            "project",
            vec![
                ("id", get(row, "id")),
                ("worktree", native(get(row, "worktree"))),
                ("vcs", get(row, "vcs")),
                ("name", get(row, "name")),
                ("icon_url", get(row, "icon_url")),
                ("icon_color", get(row, "icon_color")),
                ("sandboxes", sandboxes),
                ("commands", get(row, "commands")),
                ("time_created", get(row, "time_created")),
                ("time_updated", get(row, "time_updated")),
                ("time_initialized", get(row, "time_initialized")),
                ("time_archived", get(row, "time_archived")),
            ],
        )?;
    }
    for row in select(conn, "SELECT * FROM worktree")? {
        insert(
            tx,
            "worktree",
            vec![
                ("project_id", get(&row, "project_id")),
                ("directory", native(get(&row, "directory"))),
                ("strategy", get(&row, "strategy")),
                ("time_created", get(&row, "time_created")),
            ],
        )?;
    }
    Ok(projects.len())
}

/// The v1 diff summary columns, kept together as one JSON object.
fn summary(row: &Row) -> Sql {
    let mut summary = serde_json::Map::new();
    for (key, column) in [
        ("additions", "summary_additions"),
        ("deletions", "summary_deletions"),
        ("files", "summary_files"),
        ("diffs", "summary_diffs"),
    ] {
        let value = match get(row, column) {
            Sql::Integer(n) => Value::from(n),
            Sql::Real(n) => Value::from(n),
            Sql::Text(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            _ => continue,
        };
        summary.insert(key.into(), value);
    }
    if summary.is_empty() {
        Sql::Null
    } else {
        Sql::Text(anysola_core::json::compact(&Value::Object(summary)))
    }
}

/// 0.5.x leaves a session's title NULL until one is generated; 1.0 needs one, and a row the
/// insert skipped would orphan its messages. It gets the title a new session starts with.
fn title(row: &Row) -> Sql {
    if let Some(title) = text(row, "title") {
        return Sql::Text(title);
    }
    let created = match get(row, "time_created") {
        Sql::Integer(ms) => ms.max(0) as u64,
        _ => 0,
    };
    let kind = match get(row, "parent_id") {
        Sql::Text(_) => "Child",
        _ => "New",
    };
    Sql::Text(format!(
        "{kind} session - {}",
        anysola_core::time::iso(created)
    ))
}

/// Imports every session and returns their ids. A session that was busy when 0.5.x stopped is
/// recorded as interrupted.
fn copy_sessions(conn: &Connection, tx: &Transaction) -> Result<HashSet<String>, Error> {
    let mut ids = HashSet::new();
    for row in select(conn, "SELECT * FROM session_v2")? {
        let busy = matches!(get(&row, "time_busy"), Sql::Integer(_));
        let interrupted = match (get(&row, "time_busy"), get(&row, "time_idle")) {
            (Sql::Integer(busy), Sql::Integer(idle)) => busy > idle,
            _ => busy,
        };
        let (time_idle, idle_outcome) = if interrupted {
            (get(&row, "time_busy"), Sql::Text("interrupted".into()))
        } else {
            (get(&row, "time_idle"), get(&row, "idle_outcome"))
        };
        let mut values = vec![
            ("directory", native(get(&row, "directory"))),
            ("title", title(&row)),
        ];
        for column in [
            "id",
            "project_id",
            "parent_id",
            "fork_session_id",
            "fork_boundary",
            "slug",
            "path",
            "version",
            "metadata",
            "cost",
            "tokens_input",
            "tokens_output",
            "tokens_reasoning",
            "tokens_cache_read",
            "tokens_cache_write",
            "revert",
            "permission",
            "agent",
            "model",
            "time_created",
            "time_updated",
            "time_busy",
            "time_viewed",
            "time_archived",
        ] {
            values.push((column, get(&row, column)));
        }
        values.push(("summary", summary(&row)));
        values.push(("time_idle", time_idle));
        values.push(("idle_outcome", idle_outcome));
        insert(tx, "session", values)?;
        if let Some(id) = text(&row, "id") {
            ids.insert(id);
        }
    }
    Ok(ids)
}

fn copy_messages(
    conn: &Connection,
    tx: &Transaction,
    tables: &HashSet<String>,
) -> Result<usize, Error> {
    let mut parts: HashMap<String, Vec<Value>> = HashMap::new();
    for row in select(
        conn,
        "SELECT message_id, type, data FROM session_part ORDER BY message_id, ordinal",
    )? {
        let (Some(message), Some(data)) = (text(&row, "message_id"), text(&row, "data")) else {
            continue;
        };
        let mut part: Value = serde_json::from_str(&data)?;
        if let (Some(map), Some(kind)) = (part.as_object_mut(), text(&row, "type"))
            && !map.contains_key("type")
        {
            map.insert("type".into(), Value::String(kind));
        }
        parts.entry(message).or_default().push(part);
    }
    let mut blob = |hash: &str| -> Option<Vec<u8>> {
        if !tables.contains("blob") {
            return None;
        }
        conn.query_row("SELECT data FROM blob WHERE hash = ?1", [hash], |r| {
            r.get(0)
        })
        .ok()
    };
    let mut count = 0;
    for row in select(
        conn,
        "SELECT id, session_id, type, seq, time_created, time_updated, data FROM session_message
         ORDER BY session_id, seq",
    )? {
        let (Some(id), Some(kind), Some(data)) =
            (text(&row, "id"), text(&row, "type"), text(&row, "data"))
        else {
            continue;
        };
        let mut data: Value = serde_json::from_str(&data)?;
        if kind == "compaction" && data["status"] == "running" {
            continue;
        }
        if kind == "assistant" {
            rows::merge_parts(&mut data, parts.remove(&id).unwrap_or_default());
        }
        rows::hydrate(&mut data, &mut blob);
        rows::upgrade(&kind, &mut data);
        insert(
            tx,
            "message",
            vec![
                ("id", Sql::Text(id)),
                ("session_id", get(&row, "session_id")),
                ("seq", get(&row, "seq")),
                ("type", Sql::Text(kind)),
                ("time_created", get(&row, "time_created")),
                ("time_updated", get(&row, "time_updated")),
                ("data", Sql::Text(anysola_core::json::compact(&data))),
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Budgets, per-session credit totals of imported sessions and the cached remote config, which
/// 1.0 keeps as the response text.
fn copy_kv(conn: &Connection, tx: &Transaction, sessions: &HashSet<String>) -> Result<(), Error> {
    for row in select(conn, "SELECT * FROM kv")? {
        let Some(key) = text(&row, "key") else {
            continue;
        };
        let keep = key == "anysola:budget:default"
            || key == "anysola:config"
            || key
                .strip_prefix("anysola:session:")
                .is_some_and(|id| sessions.contains(id));
        if !keep {
            continue;
        }
        let value = match (key.as_str(), get(&row, "value")) {
            ("anysola:config", Sql::Text(body)) if !body.starts_with('"') => {
                Sql::Text(anysola_core::json::compact(&Value::String(body)))
            }
            (_, value) => value,
        };
        insert(
            tx,
            "kv",
            vec![
                ("key", Sql::Text(key)),
                ("value", value),
                ("time_created", get(&row, "time_created")),
                ("time_updated", get(&row, "time_updated")),
            ],
        )?;
    }
    Ok(())
}
