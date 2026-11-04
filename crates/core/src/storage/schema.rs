//! Database schema, versioned with `PRAGMA user_version`. Each entry migrates from the previous
//! version; never edit a released entry, append a new one.
//!
//! The shape keeps everything 0.5.1's `opencode.db` stores so its history imports losslessly:
//! message rows hold the same JSON `data` per `type`, with assistant parts inline.

pub const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE project (
    id TEXT PRIMARY KEY,
    worktree TEXT NOT NULL,
    vcs TEXT,
    name TEXT,
    icon_url TEXT,
    icon_color TEXT,
    sandboxes TEXT NOT NULL DEFAULT '[]',
    commands TEXT,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    time_initialized INTEGER,
    time_archived INTEGER
);

CREATE TABLE worktree (
    project_id TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    directory TEXT NOT NULL,
    strategy TEXT,
    time_created INTEGER NOT NULL,
    PRIMARY KEY (project_id, directory)
);

CREATE TABLE session (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    parent_id TEXT,
    fork_session_id TEXT,
    fork_boundary TEXT,
    slug TEXT NOT NULL,
    directory TEXT NOT NULL,
    path TEXT,
    title TEXT NOT NULL,
    version TEXT NOT NULL,
    metadata TEXT,
    cost REAL NOT NULL DEFAULT 0,
    tokens_input INTEGER NOT NULL DEFAULT 0,
    tokens_output INTEGER NOT NULL DEFAULT 0,
    tokens_reasoning INTEGER NOT NULL DEFAULT 0,
    tokens_cache_read INTEGER NOT NULL DEFAULT 0,
    tokens_cache_write INTEGER NOT NULL DEFAULT 0,
    revert TEXT,
    permission TEXT,
    agent TEXT,
    model TEXT,
    summary TEXT,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    time_busy INTEGER,
    time_idle INTEGER,
    idle_outcome TEXT,
    time_viewed INTEGER,
    time_compacting INTEGER,
    time_archived INTEGER,
    busy_owner TEXT,
    busy_heartbeat INTEGER
);
CREATE INDEX session_project ON session(project_id, time_updated);
CREATE INDEX session_parent ON session(parent_id);
CREATE INDEX session_directory ON session(directory, time_updated);

CREATE TABLE message (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    type TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    data TEXT NOT NULL
);
CREATE UNIQUE INDEX message_session_seq ON message(session_id, seq);

CREATE TABLE inbox (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    payload TEXT NOT NULL,
    delivery TEXT NOT NULL,
    enqueued_seq INTEGER NOT NULL,
    time_created INTEGER NOT NULL
);
CREATE INDEX inbox_session ON inbox(session_id, enqueued_seq);

CREATE TABLE instruction_state (
    session_id TEXT PRIMARY KEY REFERENCES session(id) ON DELETE CASCADE,
    epoch_start INTEGER NOT NULL,
    through_seq INTEGER NOT NULL,
    initial_values TEXT NOT NULL,
    current_values TEXT NOT NULL
);

CREATE TABLE instruction_blob (
    hash TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE instruction_entry (
    session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    removed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, key)
);

CREATE TABLE kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL
);

CREATE TABLE credential (
    id TEXT PRIMARY KEY,
    integration_id TEXT NOT NULL,
    label TEXT,
    connector_id TEXT,
    method_id TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    value TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL
);
CREATE INDEX credential_integration ON credential(integration_id);

CREATE TABLE permission (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    action TEXT NOT NULL,
    resource TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL
);
CREATE UNIQUE INDEX permission_rule ON permission(project_id, action, resource);

CREATE TABLE job (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    data TEXT NOT NULL,
    time_created INTEGER NOT NULL
);
"#];
