# Storage, config, file locations, migration (Anysola Code 0.5.1 parity spec)

Scope: everything Anysola Code 0.5.1 persists or reads from disk, the SQLite database it shares with upstream
OpenCode, the config system, and what a Rust rewrite must import. Fork: `/home/ubuntu/anysola-code/build/fork`
(branch `anysola` on tag `v2.2.5`). All `file:line` references are relative to the fork root. Status tags:
`[0.5.1]` = live in the shipped 0.5.1 binary; `[dead]` = present in source but not reachable / not used by 0.5.1
(reason given); `[observed]` = confirmed by running `/home/ubuntu/anysola-code/rust/testing/bin/anysola-0.5.1`.

Verbatim artifacts (in `spec/verbatim/storage/`):

| File | What |
|---|---|
| `schema.sql` | `sqlite3 .schema` of a database created by the 0.5.1 binary (all 24 tables, all indexes) |
| `schema.gen.ts.txt` | `packages/core/src/database/schema.gen.ts`: the DDL a fresh database is bootstrapped with |
| `migrations-src.ts.txt` | all 59 migration files of `packages/core/src/database/migration/` concatenated in order |
| `migration-runner.ts.txt` | `packages/core/src/database/migration.ts` (bootstrap + journal logic) |
| `migration-table.txt` | the `migration` journal rows of a 0.5.1 database (59 ids) |
| `example-rows.observed.json` | sanitized rows of every table from two databases written by the 0.5.1 binary (token redacted, catalog blobs clipped) |
| `example-rows.schema-derived.json` | example rows for tables/variants that could not be produced by running the binary (derived from the schemas cited) |
| `session-export.example.json` | output of `anysola session export <id>` for the rich test session (the user-facing transfer format) |
| config schema files | see section 3 |

How the observed rows were produced: an isolated HOME, a local fake gateway
(`rust/testing/runs/storage/rich-gateway.mjs`, port 18831) that streams Anthropic SSE with `thinking`+`signature_delta`,
`redacted_thinking` and scripted `tool_use` blocks (shell, write, edit, read, read-missing-file, todowrite, subagent),
then `anysola run --standalone --auto`, a second `run -s <id> --agent plan --model anysola/claude-haiku-5-5#high
--budget 40`, a third `run -s <id> --fork --model anysola/claude-sonnet-5-5#high -f pixel.png -f note.txt`, and
`anysola login --standalone` against fake `/v1/code/device/{code,token}` routes.

---

## 1. Files and directories

### 1.1 Roots (identical logic on Linux, macOS and Windows)

- [ ] Roots come from XDG variables with home fallbacks; there is **no** platform branch (no `%APPDATA%`,
  `%LOCALAPPDATA%`, `~/Library`). `home` = `os.homedir()` (`OPENCODE_TEST_HOME` overrides it). App name `opencode`.
  — `packages/util/src/global-roots.ts:4-18`, `packages/util/src/global.ts:12-27` [0.5.1]
- [ ] `config` can be overridden by `OPENCODE_CONFIG_DIR`. All of data, config, state, log, bin, repos, tmp are
  `mkdir -p`'d at start. — `global.ts:63-79`, `packages/cli/src/index.ts:139` [0.5.1]

| Root | Variable | Linux / macOS default | Windows default |
|---|---|---|---|
| data | `$XDG_DATA_HOME/opencode` | `~/.local/share/opencode` | `%USERPROFILE%\.local\share\opencode` |
| config | `$OPENCODE_CONFIG_DIR` or `$XDG_CONFIG_HOME/opencode` | `~/.config/opencode` | `%USERPROFILE%\.config\opencode` |
| state | `$XDG_STATE_HOME/opencode` | `~/.local/state/opencode` | `%USERPROFILE%\.local\state\opencode` |
| cache | `$XDG_CACHE_HOME/opencode` | `~/.cache/opencode` | `%USERPROFILE%\.cache\opencode` |
| bin | `<cache>/bin` | `~/.cache/opencode/bin` | `%USERPROFILE%\.cache\opencode\bin` |
| log | `<data>/log` | `~/.local/share/opencode/log` | `…\.local\share\opencode\log` |
| repos | `<data>/repos` | | |
| tmp | `os.tmpdir()/opencode` | `/tmp/opencode` (macOS `$TMPDIR/opencode`) | `%TEMP%\opencode` |
| install | `$ANYSOLA_INSTALL_DIR` or `~/.anysola/bin` | `~/.anysola/bin/anysola` | `%USERPROFILE%\.anysola\bin\anysola.exe` |
| system config | — | `/etc/opencode` (Linux), `/Library/Application Support/opencode` (macOS) | `%ProgramData%\opencode` |

### 1.2 Inventory

Release channel of the 0.5.1 binary is `latest` (log line `channel=latest local=false`) [observed], so the
channel-specific names below resolve to the plain names.

| Artifact | Path | Writer / format | Status |
|---|---|---|---|
| Database | `<data>/opencode.db` (+`-wal`, `-shm`), mode 0600 | §2 | [0.5.1] |
| Log | `<data>/log/opencode.log` (channel `local` builds: `opencode-local.log`) | §5 | [0.5.1] |
| Log trim lock | `<data>/log/opencode.log.trim/` (directory used as a mutex) | `packages/util/src/observability/logging.ts:98-108` | [0.5.1] |
| Snapshots | `<data>/snapshot/<projectID>/<sha1(worktree path)>/` = a bare git dir (`HEAD`, `config`, `opencode.gitconfig`, `index`, `objects/` with `info/alternates` to the project repo) holding the tree ids stored in `assistant.snapshot` | `packages/core/src/snapshot.ts:96` | [0.5.1] [observed] |
| Shell output | `<data>/shell/<projectID>/<shellID>.out` (combined stdout/stderr of `shell` tool and user shells), deleted after 7 days by an hourly sweep | `packages/core/src/shell.ts:33-34,91-118,144` | [0.5.1] [observed] |
| Tool output overflow | `<data>/tool-output/tool_<ascending id>[.ext]`: full output when a result exceeds `tool_output.max_lines` (2000) / `max_bytes` (51200); 7-day retention, hourly sweep | `packages/core/src/tool-output.ts:14-18,40-50,140-150` | [0.5.1] |
| Repository cache | `<data>/repos/…` clones for `references` / `skills` from Git | `packages/core/src/repository-cache.ts:129`, `reference.ts:72` | [0.5.1] |
| Worktrees | `<data>/worktree/<first 6 chars of projectID>/…` for worktrees the app creates | `packages/core/src/worktree/strategies.ts:51` | [0.5.1] |
| Artifacts | `<data>/artifacts/` | `packages/core/src/artifact.ts:213` | [dead] unless `experimental.artifacts` |
| Legacy v1 credentials | `<data>/auth.json` (read once by a migration, never deleted) | §2.2 | [0.5.1] read-only |
| Legacy pre-split db | `<data>/opencode-next.db` (read by V1 import) | §2.2 | [0.5.1] read-only |
| Service config | `<config>/service.json` (mode 0600): `{disabled?, remote?, remoteAccepted?, hostname?, port?, password?, cors?, env?}`; created with a random `password` | `packages/cli/src/services/service-config.ts:20-31,41-44,116-131,183-191` | [0.5.1] [observed] |
| Service registration | `<state>/service.json` (mode 0600, written via temp+rename): `{"id":uuid,"version":"2.2.5+anysola.0.5.1","url":"http://127.0.0.1:49374","pid":…,"password":…}`; removed when the service stops | `packages/cli/src/services/service-registration.ts:16-53`, `packages/client/src/effect/service.ts:142-148` | [0.5.1] [observed] |
| Service port | default `0xC0DE` = 49374 on Linux/macOS, `0x70DE` = 28894 on Windows; `anysola service set port` | `service-config.ts:46-60` | [0.5.1] [observed 49374] |
| Lock files | `<state>/locks/<sha1(key)>.lock/` directories with `meta.json` and `heartbeat` (cross-process flock emulation; e.g. around `cli.json` edits) | `packages/util/src/flock.ts:14-20,149-150,320` | [0.5.1] |
| TUI storage | `<state>/<channel>/tui/<key>.json`, e.g. `latest/tui/tabs.json` `{"global":{"tabs":[],"unread":{}},"cwd":{"<dir>":{"tabs":[{"sessionID","title"}],"unread":{}}}}`, `highlights.json` `{"seen":"2.2.5"}`; other keys: `layout`, `session-list`, `directory-recents`, `subagents-filter`, `system-theme`, `update-notifications`; locks in `<state>/<channel>/locks/` | `packages/tui/src/context/storage.tsx:48-58,136-137` | [0.5.1] [observed] |
| Prompt history | `<state>/prompt-history.jsonl`, one JSON per line `{"text","files":[],"agents":[],"skills":[],"pasted":[],"mode":"normal"}`, last 50 kept, rewritten on load | `packages/tui/src/prompt/history.tsx:64,78,97-105` | [0.5.1] [observed] |
| Persistent PTYs | `<state>/pty/` | `packages/core/src/persistent-pty/index.ts:152` | [0.5.1] |
| CLI/TUI config | `<config>/cli.json` (theme, keybinds, TUI settings; §3.6), written as temp+rename mode 0600 | `packages/cli/src/config/config.ts:31,42-47` | [0.5.1] |
| Legacy TUI config | `<config>/tui.json`, `<state>/kv.json` (v1) — read once and migrated into `cli.json` | `packages/cli/src/config/migrate.ts:100-112,160-235` | [0.5.1] read-only |
| ripgrep | `<cache>/bin/rg[.exe]` downloaded when not on PATH | `packages/core/src/ripgrep/binary.ts:69-133` | [0.5.1] |
| PTY helper | resolved from `<cache>/bin` | `persistent-pty/index.ts:155,197` | [0.5.1] |
| OpenTUI | `<XDG_DATA_HOME or ~/.local/share>/opentui/tree-sitter/{languages,queries}` (tree-sitter grammars cached by the OpenTUI library, not by Anysola code) | library default | [0.5.1] [observed] |
| Plan files | `<home>/.opencode/plan/` named in the plan-mode reminder (observed with the project under `$HOME`) | session spec | [0.5.1] [observed] |
| Install | `~/.anysola/bin/anysola` (0755, via `.anysola.new` + `mv`), SHA-256 checked against `SHA256SUMS`; PATH line appended to the shell rc (unless `--no-modify-path`); Windows: user `Path` env var | `install:5-12,130-170`, `install.ps1:4-14,67-110` | [0.5.1] |
| Uninstall | removes `<cache>` only, the `.anysola/bin` PATH lines from fish/zsh/bash/ash/sh rc files, the Windows user PATH entry, and the package; **keeps** data, config, state | `packages/cli/src/commands/handlers/uninstall.ts:24-181` | [0.5.1] |

- [ ] The models.dev catalog **is fetched** in 0.5.1 (from `https://models.opencode.ai`) and cached in KV, despite the
  Anysola comment, because the CLI passes `fetch: !OPENCODE_DISABLE_MODELS_FETCH` (true by default) to
  `ModelsDev.configured`. — `packages/core/src/models-dev.ts:493,547-549`, `packages/cli/src/server-process.ts:118-122`,
  `packages/server/src/routes.ts:129` [0.5.1] [observed: KV `models-dev:v2:catalog:meta` with a live `etag`]. Parity
  question for the gateway/packaging owners; Rust does not need the key.
- [ ] Project config walk goes from the session directory up to the filesystem root and also reads `~/.claude`
  (skills) and `~/.agents`; in the test run a skill from the real `/home/ubuntu/.claude/skills` leaked into an isolated
  HOME because the project lived under `/home/ubuntu`. — `packages/core/src/config/discovery.ts:69-80,129-143` [observed]

### 1.3 Environment variables (paths, storage, config)

`OPENCODE_TEST_HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `OPENCODE_CONFIG_DIR`,
`OPENCODE_DB`, `OPENCODE_DISABLE_CHANNEL_DB`, `OPENCODE_CONFIG`, `OPENCODE_CONFIG_CONTENT`,
`OPENCODE_DISABLE_PROJECT_CONFIG` / `OPENCODE_CONFIG_PROJECT_DISABLE`, `OPENCODE_DISABLE_CLAUDE_CODE`,
`OPENCODE_DISABLE_CLAUDE_CODE_SKILLS`, `OPENCODE_CLI_CONFIG_CONTENT`, `OPENCODE_MODELS_URL`, `OPENCODE_MODELS_PATH`,
`OPENCODE_DISABLE_MODELS_FETCH`, `OPENCODE_DISABLE_FILEWATCHER` / `OPENCODE_FILEWATCHER_DISABLE`, `OPENCODE_DISABLE_FFF`,
`OPENCODE_GIT_BASH_PATH`, `OPENCODE_LOG_LEVEL`, `OPENCODE_PRINT_LOGS`, `OPENCODE_SERVER_PASSWORD`, `OPENCODE_PASSWORD`,
`ANYSOLA_API_URL`, `ANYSOLA_TOKEN`, `ANYSOLA_INSTALL_DIR`, `ANYSOLA_ENABLE_WEB_UI`, `ANYSOLA_ENABLE_REMOTE_TUNNEL`.
Full list of names read anywhere: the grep in this section's source (`process.env.(OPENCODE|ANYSOLA)_*` over
`packages/{cli,core,util,server,tui}/src`) also yields `OPENCODE_CLIENT`, `OPENCODE_DRIVE`, `OPENCODE_EDITOR_SSE_PORT`,
`OPENCODE_FAST_BOOT`, `OPENCODE_NODE_ASSETS_DIR`, `OPENCODE_NODE_PTY_PATH`, `OPENCODE_PARCEL_WATCHER_PATH`,
`OPENCODE_PHOTON_WASM_PATH`, `OPENCODE_PTY_BIN`, `OPENCODE_PTY_HANDOFF`, `OPENCODE_REPO_CLONE_GITHUB_BASE_URL`,
`OPENCODE_ROUTE`, `OPENCODE_SESSION_ID`, `OPENCODE_SIMULATE`, `OPENCODE_SSH_ASKPASS_PORT`, `OPENCODE_SSH_ASKPASS_TOKEN`,
`OPENCODE_STORY`, `OPENCODE_TUI_CHANNEL`, `OPENCODE_ZED_DB`, `OPENCODE_DISABLE_FREE_MODELS`. `OPENCODE_DISABLE_AUTOUPDATE`
(set by `testing/env.sh`) is not read by 0.5.1.

---

## 2. SQLite database

### 2.1 Engine, file, pragmas

- [ ] Engine is Bun's built-in `bun:sqlite` (SQLite 3.53.4 embedded in the 0.5.1 binary per `strings`), driven through
  Drizzle ORM (`drizzle-orm/sqlite-core`) on Effect SQL. — `packages/core/package.json:35` (`#sqlite` → `bun`:
  `src/database/sqlite.bun.ts`), `packages/core/src/database/sqlite.bun.ts:1` [0.5.1] [observed]
- [ ] File path: `<data>/opencode.db` where `<data>` = `$XDG_DATA_HOME/opencode` or `~/.local/share/opencode`
  (same on Windows/macOS: `%USERPROFILE%\.local\share\opencode\opencode.db`; there is no APPDATA/Library branch).
  `OPENCODE_DB` overrides the file name (relative names resolve against `<data>`, `:memory:` = in-memory). Release
  channels `latest|dev|beta|next|prod` (and `OPENCODE_DISABLE_CHANNEL_DB=1|true`) use `opencode.db`; any other channel
  uses `opencode-<channel>.db`. — `packages/cli/src/database-path.ts:4-13`,
  `packages/core/src/database/database.ts:76`, `packages/util/src/global-roots.ts:5,11-18` [0.5.1]
- [ ] The app name is literally `opencode` (`packages/util/src/global.ts:12`): Anysola Code shares the data dir, the
  database and the config dir with any upstream OpenCode install of the same user. [0.5.1]
- [ ] On open: `PRAGMA journal_mode = WAL`, `synchronous = NORMAL`, `busy_timeout = 5000`, `cache_size = -64000`,
  `wal_checkpoint(PASSIVE)`, then `foreign_keys = ON`. — `packages/core/src/database/database.ts:40-47` [0.5.1]
  Observed `PRAGMA journal_mode` = `wal`, `user_version` = 0, `auto_vacuum` = 0, `encoding` = UTF-8. [observed]
- [ ] POSIX: the db file is created with mode `0600` before SQLite opens it and `chmod 0600` is applied to the db,
  `-wal` and `-shm` every open; Windows relies on profile ACLs (no chmod). — `database.ts:85-97` [0.5.1] [observed]
- [ ] In-process: one semaphore per db path serialises bootstrap/migrations between two layers in one process
  (`lockFor`). Cross-process concurrency is only SQLite WAL + busy_timeout; the TUI, `run` and the background service
  share the file. — `database.ts:55-66` [0.5.1]
- [ ] Integers are read with `safeIntegers` controlled by Effect SQL context; all timestamps are integer epoch
  milliseconds (UTC). — `sqlite.bun.ts:30-31`, `packages/core/src/database/schema.sql.ts:3-10` [0.5.1]
- [ ] `time_created` defaults to `Date.now()` on insert; `time_updated` is set by Drizzle `$onUpdate(Date.now())` on
  every ORM update (not a SQL trigger: raw SQL updates do not bump it). — `schema.sql.ts:3-10` [0.5.1]

### 2.2 Bootstrap and migrations

- [ ] On every open `DatabaseMigration.apply`: list non-`sqlite_%`, non-underscore tables. If `session` or
  `session_v2` exists → incremental path (`applyOnly`). If the db is empty → run `schema.gen.ts` (creates the final
  schema directly), create `migration(id TEXT PRIMARY KEY, time_completed INTEGER NOT NULL)` and insert all 59 ids as
  completed (that is why every id in a fresh db has nearly the same `time_completed`). If the db has other tables but
  no session table → fatal `Database is not empty and has no session table`. Tables whose name starts with `_` are
  reserved for embedders and ignored. — `packages/core/src/database/migration.ts:22-52` [0.5.1] [observed]
- [ ] Incremental path: `CREATE TABLE IF NOT EXISTS migration`; if the journal is empty and a legacy Drizzle
  `__drizzle_migrations` table exists, seed the journal from it (by `name` column, or by matching `created_at`
  timestamps to migration id prefixes; unknown timestamp → fatal). Then each missing migration runs in its own
  transaction and inserts its journal row; migrations flagged `foreignKeys: false` run with `PRAGMA foreign_keys=OFF`
  around them. Logged as `database migration started/completed/failed`. — `migration.ts:54-149` [0.5.1]
- [ ] Migration ids (ordered, 59): `20260127222353_familiar_lady_ursula` … `20261030054400_job`; full list in
  `verbatim/storage/migration-table.txt`, sources in `migrations-src.ts.txt`, registry in
  `packages/core/src/database/migration.gen.ts`. No Anysola commit touches the schema or migrations
  (`git diff --stat v2.2.5..anysola -- packages/core/src/database` is empty): every Anysola release 0.2.0–0.5.1 has
  the identical schema. [0.5.1]
- [ ] Upstream lineage notes the importer may meet in a shared db: a db first created by upstream OpenCode v1 still
  contains the v1 tables `session`, `message`, `part`, `todo`, `session_share` (created by
  `20260127222353_familiar_lady_ursula`; never dropped); `20260804233008_loose_psylocke` renames an older v2 `session`
  table to `session_v2` and drops `data_migration`, `session_context_epoch`, `session_input`;
  `20261025154125_drop_session_pending` drops `session_pending`. — migration sources above [0.5.1]
- [ ] V1 → V2 data import (upstream feature, runs inside the 0.5.1 server): when a table named `session` exists
  (v1 history), the server layer forks `V1Migration.run()` at start: it deletes all `event` rows, converts every v1
  session/message/part into `session_v2`/`session_message` (tool renames `bash→shell`, `task→subagent`,
  `apply_patch→patch`; `todowrite` calls flagged as removed), also imports `<data>/opencode-next.db` if present (a
  read-only pre-split v2 db), and records progress in KV key `migration.v1-v2` (`{"phase":"sessions","cursor"?}` →
  `{"phase":"completed"}`). The TUI shows a migration overlay while it runs. Source tables are left in place.
  — `packages/core/src/database/v1-migration.bun.ts:220,512-528,541-760,798-803,996-998,1229-1234`,
  `packages/server/src/routes.ts:199`, `packages/tui/src/app.tsx:59` [0.5.1] (only acts on dbs that have v1 tables)
- [ ] `20260805200742_import_legacy_credentials` reads `<data>/auth.json` (upstream v1 credential file) into the
  `credential` table and `wellknown:sources` KV; it never deletes `auth.json`. Runs only on the incremental path, i.e.
  not for a db bootstrapped fresh by 0.5.1. —
  `packages/core/src/database/migration/20260805200742_import_legacy_credentials.ts:38-39,92-99` [0.5.1]

### 2.3 Tables

Column types/nullability/defaults/FKs/indexes are exactly as in `verbatim/storage/schema.sql`. Drizzle definitions:
`packages/core/src/session/sql.ts`, `project/sql.ts`, `kv/sql.ts`, `credential/sql.ts`, `permission/sql.ts`,
`event/sql.ts`, plus the remaining `*/sql.ts` matched by `packages/core/drizzle.config.ts`. All FKs that point at
`session_v2` / `project` / `session_message` are `ON DELETE CASCADE` (deleting a session deletes its messages, parts,
inbox, instruction state/entries, blob refs, schedules).

| Table | Holds | Used by 0.5.1 |
|---|---|---|
| `project` | one row per project id (see §4) | [0.5.1] |
| `worktree` | directories known for a project (`strategy` = `git` for git worktrees) | [0.5.1] |
| `project_directory` | deprecated predecessor of `worktree` | [dead] `@deprecated` in `project/sql.ts`, no writer |
| `session_v2` | one row per session (root, child/subagent, fork) | [0.5.1] |
| `session_message` | the projected transcript, one row per message, parts inline in `data.content` | [0.5.1] |
| `session_part` | per-part rows for assistant messages | [0.5.1] read-preferred, but only written when rows already exist (see 2.6) |
| `session_inbox` | queued/steered prompts waiting for a busy session | [0.5.1] transient |
| `instruction_state`, `instruction_blob`, `instruction_entry` | per-session system-instruction epoch (key → sha256) and content-addressed instruction values | [0.5.1] |
| `kv` | key/value store (JSON values) | [0.5.1] |
| `credential` | integration credentials, incl. the Anysola device token | [0.5.1] |
| `permission` | "always allow" decisions per project | [0.5.1] |
| `event_sequence` | last durable event seq per aggregate (session id or project id) | [0.5.1] |
| `event` | durable event log | [dead] locally: `Bus` persists only with `events.persist` (default `false`, `packages/core/src/bus.ts:184,208`; only `packages/server/src/workerd.ts:71` sets it). Always 0 rows. |
| `blob`, `blob_ref` | media bytes moved out of messages | [dead] unless `experimental.media_blob: true` (`packages/core/src/session/blob.ts:255-259`); default is inline base64 |
| `schedule` | scheduled prompts | [dead] unless `experimental.schedules: true` (`packages/core/src/tool/plugin/schedule.ts:55`) |
| `job` | background subagent/shell job recovery records | [0.5.1] transient (rows deleted with the session, `packages/core/src/job.ts:497`) |
| `workspace` | remote workspaces | [dead] for Anysola (no workspace provider ships) |
| `account`, `account_state`, `control_account` | OpenCode Console accounts | [dead] Console login removed (Anysola commits 67cbb0e156, 12bab1edde) |
| `migration` | migration journal | [0.5.1] |

#### `session_v2` columns (decoder: `packages/core/src/session/info.ts:12-61`)

| Column | Meaning | Observed / shape |
|---|---|---|
| `id` | `ses_` + descending id (see 2.4) | `ses_e59ed5370ffeZXdiWgNSi2YA7r` |
| `project_id` | FK `project.id` | 40-hex or `global` |
| `workspace_id` | remote workspace | NULL [dead] |
| `parent_id` | parent session for subagent children | child of `subagent` tool |
| `fork_session_id`, `fork_boundary` | source session and JSON `{"type":"before"\|"through","messageID":"msg_…"}` for forks | `packages/schema/src/session-fork.ts` |
| `slug` | random `adjective-noun` (`Slug.create()`), not shown in 0.5.1 UI | `neon-rocket` |
| `directory` | absolute directory the session runs in (POSIX slashes in storage, converted to `\` on read on Windows; `''` allowed for legacy rows) | `packages/core/src/database/path.ts:6-62` |
| `path` | optional sub-path relative to the project (`subpath`) | NULL |
| `title` | title (generated by the `title` agent; forks get `<title> (fork #N)`) | `Rich storage test (fork #1)` |
| `version` | `SERVICE_VERSION` of the writer = `<opencode version>+anysola.<anysola version>` | `2.2.5+anysola.0.5.1` (`packages/cli/src/version.ts`) |
| `share_url` | share link | NULL [dead] (sharing off in Anysola) |
| `summary_additions/deletions/files/diffs` | v1 diff summary (`FileDiff.LegacyInfo[]` JSON) | NULL for v2 sessions; only v1-imported rows |
| `metadata` | host annotations JSON object | NULL |
| `cost` | USD sum of assistant `cost` | `0.0` (Anysola models carry no price) |
| `tokens_input/output/reasoning/cache_read/cache_write` | sums over assistant messages | `1080/378/0/7200/360` |
| `revert` | staged revert JSON `{"messageID","partID"?,"snapshot"?,"files"?:[FileDiff.Info]}` (v1 shape `RevertV1` also accepted) | `packages/schema/src/session-revert.ts` |
| `permission` | session ruleset JSON `[{"action","resource","effect":"allow"\|"deny"\|"ask"}]`, evaluated after the agent's rules | NULL |
| `agent` | last selected agent id; NULL = default `build` | `plan`, `general` |
| `model` | last selected model JSON `{"id","providerID","variant"?}`; NULL = config default; missing `variant` reads as `default` | `{"id":"claude-sonnet-5-5","providerID":"anysola","variant":"high"}` |
| `time_created/updated` | ms | |
| `time_busy`, `time_idle`, `idle_outcome` | start of last execution, end of last completed execution, `succeeded\|failed\|interrupted` | |
| `time_viewed` | last time a client marked it viewed | |
| `time_compacting` | set while a compaction runs | |
| `time_archived` | archived sessions | |
| `time_suspended`, `resume_attempts` | execution claim timestamp and resume counter (crash recovery) | internal |

#### `session_message` (decoder: `packages/core/src/session/history.ts:125-135`)

- [ ] Columns: `id` (`msg_…`), `session_id`, `type` (= the message's tag), `seq` (the durable event sequence number
  of the event that created the row: unique per session, strictly increasing, with gaps), `time_created`,
  `time_updated`, `data` (JSON of the message without `id` and `type`). — `packages/core/src/session/sql.ts:86-105`
  [0.5.1] [observed]
- [ ] Transcript order is `ORDER BY seq ASC`; `time_created` is not a reliable order key. — `history.ts:193-203` [0.5.1]
- [ ] Every row is upgraded by `SessionLegacy.message(type, data)` before decoding (pre-v2.1 rows): assistant
  `providerState`→`native`; content part `state`→`native` (text/reasoning), `providerState`→`native` and
  `providerResultState` merged into `native` (tool); compaction `providerState`→`native`, `providerContext`→
  `native.messages` (media parts `{mediaType,data}` → `{media:{source:{type:"base64",data,mediaType}}}`), `recent` →
  `metadata.recent`. The current name wins if both exist. — `packages/schema/src/session-legacy.ts:35-113` [0.5.1]
- [ ] Message IDs are normally `msg_` + 26 chars, but rows copied into a fork get `msg_<eventIdTail>_<parentSeq>`
  (e.g. `msg_1a614de1f00171kMnpIiCAWrQM_64`); importers must not validate the length. —
  `packages/core/src/session/projector.ts:189` [observed]

#### `session_part` (overlay table)

- [ ] Columns `message_id`, `session_id`, `ordinal` (PK with message_id), `type` (`text|reasoning|tool`), `call_id`
  (= tool `id` for tool parts, else NULL), timestamps, `data` (the encoded part). — `session/sql.ts:107-128`
- [ ] 0.5.1 never creates part rows for new messages: `updateMessage` writes content inline unless part rows already
  exist for that message ("A newer release may have moved this message's parts to session_part"), in which case it
  rewrites the part rows (insert/update/delete by ordinal) and stores `content: []` in the message row. —
  `packages/core/src/session/projector.ts:261-338` [0.5.1] [observed: 0 rows in both test dbs]
- [ ] Reads always prefer part rows: for each assistant row, if any `session_part` rows exist they replace
  `data.content` in `ordinal` order; otherwise inline content is used. Forks copy parts inline. —
  `packages/core/src/session/history.ts:90-123`, `projector.ts:205-213` [0.5.1]
- [ ] Consequence for the importer: a db last written by a newer upstream OpenCode may hold assistant rows with
  `content: []` and the real parts in `session_part`. Always merge part rows first.

#### Other tables (shapes)

- [ ] `session_inbox`: `id` (`msg_…`, becomes the message id when delivered), `type` `user|synthetic|compaction|move`,
  `payload` JSON (user payload = prompt `{text, files?, agents?, skills?}`), `delivery` `steer|queue`,
  `enqueued_seq`. Rows are deleted when delivered or reverted. — `session/sql.ts:130-152`,
  `packages/schema/src/session-inbox.ts:11-51`, `packages/core/src/session/inbox.ts:357,383` [0.5.1]
- [ ] `instruction_state`: per session `epoch_start`, `through_seq`, `initial_values` / `current_values` =
  JSON map `instruction key → sha256 hex` (keys like `core/date`, `core/environment`, `core/codemode`,
  `core/skill-guidance`); `instruction_blob(hash, value JSON)` holds the values (e.g. `"Wed Nov 04 2026"`, the `<env>`
  block); `instruction_entry(session_id,key,value,removed)` holds per-session entries. These track which system
  instruction text the model last saw; a change is emitted into the transcript as a `system` message. —
  `session/sql.ts:154-183`, `packages/schema/src/instruction.ts`, `packages/core/src/session/instruction-state.ts`
  [0.5.1] [observed]
- [ ] `event_sequence(aggregate_id, seq, owner_id)`: last seq per session (and `0` per project id); new message rows
  take the next seq. — `packages/core/src/event/sql.ts:4-8`, `packages/core/src/bus.ts:406-416` [0.5.1] [observed]
- [ ] `project`: `id`, `worktree` (canonical directory), `vcs` (`git|hg|NULL`), `name`, `icon_url`,
  `icon_url_override`, `icon_color`, `time_initialized`, `time_active`, `time_archived`, `sandboxes` (JSON array of
  absolute paths), `commands` (JSON `{"start"?: string}`). — `packages/core/src/project/sql.ts` [0.5.1] [observed]
- [ ] `worktree(project_id, directory, strategy, time_created)`: e.g. `(87eb…, /…/proj, NULL)`. [observed]
- [ ] `credential`, `kv`, `permission`: see 2.8–2.10.

### 2.4 Identifiers

- [ ] Generator: 12 hex chars of `(timestamp_ms * 0x1000 + counter)` (48 bits, bitwise-NOT for descending ids) + 14
  random chars from `0-9A-Za-z` (26 chars total), prefixed. Sessions use descending (`ses_`), messages ascending
  (`msg_`), credentials `cred_`, saved permissions `psv_`, shells `sh_`. — `packages/schema/src/identifier.ts:3-30`,
  `session-id.ts`, `session-message.ts:21-28`, `credential.ts:10-14`, `permission-saved.ts:8-13`, `shell.ts` [0.5.1]
- [ ] `Session.ID` only requires the prefix `ses`; `Message.ID` the prefix `msg_`. Messages created from an event take
  the event id with `evt_` replaced by `msg_`. — `session-id.ts`, `session-message.ts:26` [0.5.1]

### 2.5 `session_message.data` shapes by `type`

Schema: `packages/schema/src/session-message.ts`. All timestamps are epoch ms. `metadata` (any JSON object) may appear
on every type.

| `type` | `data` fields | Example (observed unless noted) |
|---|---|---|
| `user` | `time.created`, `text`, `files?` (`FileAttachment[]`), `agents?` (`[{name, mention?}]`), `skills?` (`[{id,name,text?,mention?}]`) — lines 76-84 | `{"time":{"created":…},"text":"\"third turn with files\"\n\n<file name=\"note.txt\">\nattached note\n\n</file>","files":[{"data":"iVBORw0…","mime":"image/png","source":{"type":"inline"},"name":"pixel.png"}]}` |
| `assistant` | `time.{created,streamed?,completed?}`, `agent`, `model{id,providerID,variant?}`, `content[]` (2.6), `snapshot?{start?,end?,files?[]}` (git tree ids before/after the step + relative paths changed), `finish?` (`stop\|tool-calls\|length\|…`), `rawFinish?` (provider stop reason), `native?`, `cost?` (USD), `tokens?{input,output,reasoning,cache{read,write}}`, `error?`, `retry?{attempt,at,error}` — lines 220-246 | see `example-rows.observed.json` |
| `synthetic` | `time.created`, `text`, `description?` — lines 93-99 | plan-mode `<system-reminder>` injected when switching to/from `plan` |
| `system` | `time.created`, `text` (model-facing instruction update), `description?` — lines 101-109 | schema-derived |
| `skill` | `time.created`, `skill` (id), `name`, `text` — lines 111-118 | schema-derived |
| `shell` | `time.{created,completed?}`, `shellID` (`sh_…`), `command`, `status` (`running\|exited\|timeout\|killed`), `exit?`, `output?` (`Shell.Output`); `metadata.background === true` marks background shells — lines 120-133 | schema-derived (user `!cmd` in the TUI) |
| `agent-switched` | `time.created`, `agent`, `previous?` — lines 46-52 | `{"agent":"build","previous":"plan"}` |
| `model-switched` | `time.created`, `model`, `previous?` — lines 54-60 | `{"model":{"id":"claude-sonnet-5-5","providerID":"anysola","variant":"high"},"previous":{…haiku…}}` |
| `location-switched` | `time.created`, `location{directory,workspaceID?}`, `projectID?`, `subpath?`, `previous?` — lines 62-74 | schema-derived |
| `compaction` | `status` `running` (`reason` `auto\|manual`, `summary`), `completed` (`reason`, `model?`, `native?` (provider window in `native.messages`), `summary`, `cost?`, `tokens?`; `metadata.keepID` = first kept message id), `failed` (`reason`, `error`, `cost?`, `tokens?`) — lines 248-289 | schema-derived |
| `idle` | `time.created`, `outcome` `succeeded\|failed\|interrupted`, `error?` — lines 296-302 | `{"outcome":"failed","error":{"type":"provider.no-route","message":"Variant unavailable for anysola/claude-haiku-5-5: high"}}` |

Notes:
- [ ] `FileAttachment` = `{data: base64, mime, source: {"type":"inline"} | {"type":"uri","uri"}, name?, description?,
  mention?{start,end,text}}`. With `experimental.media_blob` the stored form replaces `data` by `uri:"media:<sha256>"`
  and the bytes live in `blob`. — `packages/schema/src/prompt.ts:30-58`, `packages/core/src/session/stored.ts:23-44`
- [ ] Text attachments (`-f note.txt`) are not stored as files: the CLI inlines them into `text` as
  `<file name="…">…</file>`; images/PDF stay in `files`. [observed]
- [ ] `anysola run` stores each message argument that contains a space wrapped in literal double quotes
  (`"\"RICHTEST please …\""`). — `packages/cli/src/run/run.ts:245` [0.5.1] [observed]
- [ ] There are no v1 part types in v2 rows. Mapping of the v1 names the Rust team may expect:
  `file` → `user.files[]`; `snapshot`/`patch` → `assistant.snapshot{start,end,files}` + tool `metadata.files[]`
  (`FileDiff.Info{file,patch,additions,deletions,status}`); `step-start`/`step-finish` → each `assistant` row *is* one
  step (`finish`, `tokens`, `cost`); `compaction` → `compaction` message; `subtask` → `subagent` tool call + child
  `session_v2` row with `parent_id`; `agent` → `user.agents[]` mentions and `agent-switched` rows. v1 `part`/`message`
  tables are [dead] except as input to the V1 import.

### 2.6 Assistant `content[]` parts

Schema: `packages/schema/src/session-message.ts:135-211`; encoded part = `SessionPartData`.

- [ ] `{"type":"text","text":…,"native"?:{…}}` — lines 185-190 [observed]
- [ ] `{"type":"reasoning","text":…,"native"?:{…},"time"?:{"created","completed"?}}` — lines 192-201 [observed]
  - Anthropic thinking: `native = {"signature":"<signature>"}` (the signature from `signature_delta`) [observed]
  - Anthropic redacted thinking: `text: ""`, `native = {"redactedData":"<data>"}` [observed]
  - `native` is the provider-metadata object for the route's `providerMetadataKey` (for Anysola models the
    provider id `anysola`), stored unwrapped: `packages/core/src/session/runner/publish-llm-event.ts:145`,
    `packages/ai/src/protocols/anthropic-messages.ts:1290-1330`. A reasoning part whose `time.completed` is missing
    means the stream was interrupted.
- [ ] `{"type":"tool","id":<provider call id>,"name":<tool>,"executed"?:bool,"native"?:{…},"state":…,
  "time":{"created","ran"?,"completed"?}}` — lines 170-183. `executed: true` = provider-executed (hosted) tool whose
  result is part of the assistant turn; `false`/absent = client tool. [observed `executed:false`]
  - `state.status = "streaming"`: `{input: <raw JSON string so far>}`
  - `"running"`: `{input: object, metadata: object}`
  - `"completed"`: `{input, content: [Tool.Content, ≥1], metadata?}` [observed]
  - `"error"`: `{input, error: SessionError, content?: [...], metadata?}` [observed:
    `{"type":"tool.execution","message":"File not found: does-not-exist.txt"}`]
  - `Tool.Content` = `{"type":"text","text"}` | `{"type":"file","uri","mime","name"?}` —
    `packages/schema/src/tool.ts:184-200`
  - Tool metadata examples [observed]: shell `{"status":"completed","truncated":false,"exit":0}`; edit
    `{"files":[{"file":"notes.md","patch":"Index: …","status":"modified","additions":1,"deletions":1}],"truncated":false}`;
    todowrite `{"todos":[…],"truncated":false}`; subagent `{"sessionID":"ses_…","status":"completed","truncated":false}`
    with content `<subagent sessionID="…" state="completed">\n…\n</subagent>`.
- [ ] Tool names in 0.5.1 rows: `shell`, `read`, `write`, `edit`, `patch`, `glob`, `grep`, `subagent`, `todowrite`,
  `websearch`, `webfetch`, `question`, `skill`, `schedule`, `artifact`, `image_generate`, MCP tools, code-mode
  `execute` (see the tools spec for the authoritative list). Rows imported from v1 keep v1 names (`bash`, `task`, …)
  — `packages/core/src/tool/plugin/*.ts`, `v1-migration.bun.ts:996-998`.

### 2.7 Reconstructing a conversation (replay to Anthropic)

The exact algorithm 0.5.1 uses to rebuild model input from rows (this is what the Rust importer/replayer must match):

1. Select rows: find the latest `compaction` row with `data.status = 'completed'` (for a target model, only those
   whose window is replayable: no `native.messages`/`providerContext`, or same `model.providerID`+`model.id`); read
   all rows with `seq >= that seq` (or all rows), `ORDER BY seq`. Merge `session_part` rows, restore blob refs, apply
   `SessionLegacy` upgrades, decode. If the first row is a non-replayable native window, recurse to the previous
   compaction. For a summary compaction with `metadata.keepID`, rebuild `metadata.recent` from the rows
   `seq ∈ [seq(keepID), seq(compaction))` as text. Drop `system` rows older than the last native compaction. —
   `packages/core/src/session/history.ts:47-88,183-261`
2. Map each message (`packages/core/src/session/runner/to-llm-message.ts:242-324`):
   - `agent-switched`, `idle` → nothing; `model-switched` → an effort-change marker only when the same model changes
     between effort variants (`low…max`/`default`), lines 224-240.
   - `user` → one user message: skill texts, then `text` (if non-empty), then attachments: `text/plain` →
     `"\n\nAttached file: <name|uri>\n[Description: …]\n\n<decoded text>"`, `application/x-directory` → directory
     listing text, images (`png/jpeg/gif/webp`) and `application/pdf` → `Attached file: <path>` (for `file:` URIs)
     plus a base64 media block; other mimes dropped; duplicate inline mentioned images de-duplicated. Lines 18-103,
     258-275.
   - `synthetic`, `skill` → user message with `text`; `system` → system message; `location-switched` → user
     message `The working directory has been changed to <dir>.`; `shell` (non-background) → user message
     `The following shell command was executed by the user:\n\nCommand:\n<cmd>\n\nOutput:\n<output>`.
   - `compaction` completed → `native.messages` window if present, else a user message
     `<conversation-checkpoint>…<summary>…</summary>[<recent-context>…</recent-context>]</conversation-checkpoint>`
     (exact text lines 300-313); running/failed → nothing.
   - `assistant` → lines 155-222: provider metadata (`native`) is re-attached only when the stored
     `model.providerID`+`model.id` equal the target model and the message has no `error`; reasoning parts are dropped
     when the message has an `error` or any reasoning part lacks `time.completed`; on another model, reasoning is
     sent as plain text without signature (or dropped if empty). Client tools (`executed !== true`) produce a
     tool-call in the assistant message and a separate tool-result message (`completed` → text or content list,
     `error` → `{error, content}` with `resultType:"error"`); `streaming` tool input is JSON-parsed if possible.
3. Anthropic lowering (`packages/ai/src/protocols/anthropic-messages.ts:950-1000`): reasoning with a
   signature → `{"type":"thinking","thinking":text,"signature":sig}`; no signature but `redactedData` →
   `{"type":"redacted_thinking","data":…}`; no signature and non-empty text → `thinking` with `signature:""` unless the
   request requires signatures, else dropped; empty text blocks are skipped.

Therefore, to keep thinking replayable after migration the importer must preserve, per assistant row: `model`
(providerID + id), `error` presence, every reasoning part's `text`, `native.signature` / `native.redactedData` and
`time.completed`, and part order.

### 2.8 KV store (`kv` table)

`kv(key TEXT PK, value JSON NOT NULL, time_created, time_updated)`; API `get/set/remove/scan(prefix)` —
`packages/core/src/kv.ts:27-92`, `packages/core/src/kv/sql.ts`.

| Key | Value | Writer | Status |
|---|---|---|---|
| `anysola:config` | last good body of `GET /v1/code/config` (model, small_model, agents, providers.anysola incl. models, `credits_per_turn_estimate`, notes; `apiKey` is the literal `{env:ANYSOLA_TOKEN}` template, not a token) | `packages/core/src/plugin/provider/anysola.ts:27` | [0.5.1] [observed] |
| `anysola:session:<sessionID>` | `{"total":<credits spent>,"turns":<turns>,"budget":<int ≥1>\|null}` | `packages/core/src/plugin/provider/anysola-account.ts:27,80-83` | [0.5.1] [observed]; never deleted when the session is deleted |
| `anysola:budget:default` | integer credits (`/budget default <n>`); removed by `/budget default off` | `anysola-account.ts:28,128-134` | [0.5.1] |
| `anysola:usage:<sessionID>` | `{"used":<percent of the 5-hour window>,"turns":<turns>,"budget":<1..100>\|null}` | 1.0 `crates/core/src/usage.rs` | [1.0]; replaces `anysola:session:<id>`, which 1.0 neither reads nor imports |
| `anysola:budget:five-hour` | whole percent 1..100 (`/budget default <n>`); `null` after `/budget default off` | 1.0 `crates/core/src/usage.rs` | [1.0]; replaces `anysola:budget:default` (credit budgets are dropped, not converted) |
| `models-dev:v2:catalog`, `models-dev:v2:catalog:meta`, `models-dev:v2:catalog:<sha1(source)>` | cached models.dev catalog body / `{updatedAt,digest,etag}`; old `models-dev:*` keys removed | `packages/core/src/models-dev.ts:521-557` | [0.5.1] [observed: written with a live etag in an isolated run; see §1] |
| `websearch:provider` | selected web-search provider | `packages/core/src/websearch.ts:31` | [0.5.1] |
| `wellknown:sources`, `wellknown:manifest:<origin>`, `wellknown:config:<origin>` | remote/well-known config sources | `packages/core/src/wellknown.ts:105-108` | see §3 |
| `repository-cache:<localPath>` | `{attemptedAt}` refresh marker | `packages/core/src/repository-cache.ts:130,165` | [0.5.1] |
| `artifact:item:<id>` | artifact registry | `packages/core/src/artifact.ts:33` | [0.5.1] |
| `schedule:leader` | `{owner,until}` scheduler lease | `packages/core/src/schedule.ts:96,565-584` | [dead] unless `experimental.schedules` |
| `migration.v1-v2` | V1 import state | `v1-migration.bun.ts:220` | [0.5.1] (only with v1 tables) |
| plugin keys | plugins get KV access | `packages/core/src/plugin.ts:23` | [0.5.1] |

### 2.9 Credentials (Anysola device token)

- [ ] The token lives in the SQLite `credential` table as plain JSON (no encryption, no OS keychain); protection is
  the db file mode `0600` (POSIX) / profile ACLs (Windows). — `packages/core/src/credential/sql.ts:5-14`,
  `packages/schema/src/credential.ts:31-59` [0.5.1] [observed]
- [ ] Row written by `anysola login` (device flow): `id` `cred_…`, `integration_id` `anysola`, `label`
  `Anysola account`, `connector_id` NULL, `method_id` NULL, `active` 1, `value`
  `{"type":"oauth","methodID":"device","refresh":"","access":"asc_<REDACTED>","expires":<ms>,"metadata":{"apiURL":"<api origin>","device":"<hostname>"}}`.
  `expires` = `Date.parse(expires_at)` or now + 365 days; token must match `^asc_[0-9A-Za-z]+$`. —
  `packages/core/src/plugin/provider/anysola.ts:71-117` [observed]
- [ ] `ANYSOLA_TOKEN` env is registered as an `env` method of the `anysola` integration and is used without any
  credential row (CI); `anysola logout` then prints `Signed in through ANYSOLA_TOKEN; unset it to sign out.` —
  `anysola.ts:268`, `packages/cli/src/commands/handlers/anysola/logout.ts:17` [0.5.1]
- [ ] Other credential value kinds: `{"type":"key","key","metadata"?}`, `{"type":"external","methodID","metadata"?}`
  (MCP OAuth tokens are also credentials, see the integrations spec). [0.5.1]

### 2.10 "Always allow" permissions

- [ ] Replying `always` to a permission request inserts one row per `request.save` resource:
  `permission(id "psv_…", project_id, action, resource, time_created, time_updated)`, unique on
  `(project_id, action, resource)` (`ON CONFLICT DO NOTHING`). Scope is the **project**, not the session. —
  `packages/core/src/permission.ts:332-338`, `packages/core/src/permission/saved.ts:69-84` [0.5.1]
- [ ] Saved rows are evaluated as `{action, resource, effect:"allow"}` rules for every session in the project
  (wildcard match on action/resource). — `permission.ts:151-159` [0.5.1]
- [ ] Session-scoped rules live in `session_v2.permission` (JSON ruleset); migration
  `20260910120000_clear_v1_session_permission` nulls v1-format values. [0.5.1]

### 2.11 Todos

- [ ] There is no todo table in v2 (the v1 `todo` table is only read by the V1 import). The `todowrite` tool exists only
  when `experimental.todo === true`; its state is the `input.todos` / `metadata.todos` of the latest `todowrite` tool
  part in the session (`[{content, status: pending|in_progress|completed|cancelled}]`). —
  `packages/core/src/tool/plugin/todo.ts:11-77` [0.5.1] [observed]

### 2.12 What deletes what

- [ ] Revert: `session_v2.revert` is staged first (messages kept); committing deletes `session_message` and
  `session_inbox` rows with `seq >= seq(revert.messageID)`, clears `revert`, rewinds instruction state and releases blob
  refs. — `packages/core/src/session/projector.ts:869-913` [0.5.1]
- [ ] Session delete cascades through FKs; `anysola:session:<id>` KV rows remain. [0.5.1]

---

## 3. Config

Two separate config files:
- **Core config** `opencode.json` / `opencode.jsonc` (agents, models, permissions, MCP, LSP, …). JSON schema:
  `verbatim/storage/config.schema.json`, generated from `packages/schema/src/config.ts` by
  `packages/schema/script/config-schema.ts` (`generate()`, run via a wrapper with output written outside the fork;
  `$id` `https://opencode.ai/config.json`, accepts both V2 keys and deprecated V1 keys, `allowComments`,
  `allowTrailingCommas`).
- **CLI/TUI config** `<config>/cli.json` (theme, keybinds, TUI behaviour). JSON schema:
  `verbatim/storage/cli.schema.json`, generated by `packages/cli/script/schema.ts <out>` (`$id`
  `https://opencode.ai/v2/cli.json`), source `packages/tui/src/config/index.tsx:70-200`,
  `packages/tui/src/config/keybind.ts` (all keybind names and defaults).

### 3.1 Core config sources and precedence (lowest → highest)

Discovery: `packages/core/src/config/discovery.ts:59-145`; load order: `packages/core/src/config.ts:275-373`.

1. Managed **policies** (only the `policies` key) split out of the system dir and MDM documents.
2. Integration documents — for Anysola the `anysola` integration's document built from `GET /v1/code/config`
   (`providers.anysola` filtered to allowed providers, `model`, `agents`), cached in KV `anysola:config`. —
   `packages/core/src/plugin/provider/anysola.ts:150-178`
3. Well-known remote documents (`wellknown:*` KV sources, `.well-known/opencode` of connected origins). —
   `config.ts:214-228`, `packages/core/src/wellknown.ts:105-215`
4. Global directory `<config>/opencode.json`, `<config>/opencode.jsonc` (both loaded if both exist), plus the global dir
   as a directory source (`agent/`, `command/`, `plugin/`, `skill/` … markdown definitions).
5. `OPENCODE_CONFIG` (explicit file).
6. Project files: every `opencode.json[c]` found walking up from the session directory to `/`, farthest first. A walk
   that starts in a global root is skipped; `OPENCODE_DISABLE_PROJECT_CONFIG` disables it.
7. Project `.opencode/` directories (farthest first): `opencode.json[c]` inside them plus their markdown dirs.
8. `OPENCODE_CONFIG_CONTENT` (inline JSON; `{file:}` relative to the session directory).
9. Managed **settings** (last, override everything): system dir `/etc/opencode`, `/Library/Application Support/opencode`,
   `%ProgramData%\opencode` (`opencode.json[c]` + dir), then macOS MDM plist
   `/Library/Managed Preferences/<user>/ai.opencode.managed.plist` or `/Library/Managed Preferences/ai.opencode.managed.plist`
   (converted with `plutil -convert json`, MDM payload keys stripped). — `discovery.ts:27-57`, `config.ts:244-263,300-325`

Also discovered (not config files): `~/.claude` and `.claude/` dirs (skills only, unless `OPENCODE_DISABLE_CLAUDE_CODE*`),
`~/.agents` and `.agents/`. Config files are watched and reloaded live (`config.ts:378-420`, `config/watch.ts`).

### 3.2 Merge semantics

- [ ] Scalar keys: the highest-priority document that sets the key wins as a whole (`Config.latest`). —
  `config.ts:29-32` [0.5.1]
- [ ] Maps merged per entry in load order (later id overrides): `agents`, `mcp.servers`, `providers`, `commands`. —
  `config/plugin/{agent,mcp,provider,command}.ts` [0.5.1]
- [ ] `instructions`: union of all documents (deduplicated); `permissions`: concatenated in load order, last matching rule
  wins; `plugins`: ordered enable/disable directives applied in order. — `config/plugin/instruction.ts`,
  `config/plugin/agent.ts`, `config/plugin/source.ts` [0.5.1]

### 3.3 Parsing, normalisation, substitution

- [ ] JSONC via `jsonc-parser` with `allowTrailingComma`; any parse error rejects the whole document with a warning log
  `configuration normalization diagnostic … action="rejected malformed JSON or JSONC document"` (not fatal; the document
  is skipped). Unknown keys are ignored (`onExcessProperty: "ignore"`). — `config.ts:136-165` [0.5.1]
- [ ] V1 keys are normalised into V2 (`provider→providers`, `agent`/`mode→agents`, `permission→permissions`,
  `command→commands`, `plugin→plugins`, `snapshot→snapshots`, `autoupdate→update`, `autoshare→share`, `small_model→agents.title.model`, `disabled_providers`/`enabled_providers`,
  tool name renames `bash→shell` …) with diagnostics. — `packages/core/src/config/normalize.ts:70-185,421`,
  `packages/core/src/v1/config/migrate.ts:30` [0.5.1]
- [ ] `{env:VAR}`: replaced by the env value or `""`; if the result is not valid JSON the substitution is redone with
  JSON-escaped values. `{file:path}`: `~/` expands to home, relative paths resolve against the config file's directory
  (or the session directory for `OPENCODE_CONFIG_CONTENT`), content is trimmed and JSON-escaped, a missing file becomes
  `""` with a warning; tokens on lines starting with `//` are left alone. — `packages/core/src/config/variable.ts:26-81`
  [0.5.1]
- [ ] Anysola provider lock applied to every parsed document (`Anysola.sanitize`): drops `model` on another provider,
  every `providers.<other>`, agent `model`s on other providers, and from `providers.anysola` drops `package`,
  `canonical`, `env` and the settings `baseURL`, `apiKey`, `authToken`, `accessToken` (only the remote document may set
  them). — `packages/core/src/anysola.ts:13-82`, `config.ts:160` [0.5.1]
- [ ] Anysola's provider endpoint is pinned after config: name `Anysola`, package
  `@opencode/ai/providers/anthropic-compatible`, `baseURL = <ANYSOLA_API_URL>/v1/code`, `x-anysola-version` header. —
  `anysola.ts` plugin `pin()` `packages/core/src/plugin/provider/anysola.ts:191-210` [0.5.1]

### 3.4 Keys

Exact types/enums are in `config.schema.json`. Top-level V2 keys: `$schema`, `shell`, `model`, `default_agent`, `update`
(`disable|notify|auto`), `share` (`manual|auto|disabled`), `enterprise`, `username`, `timezone`, `permissions`,
`policies`, `agents`, `snapshots`, `watcher.ignore`, `formatter`, `lsp`, `media`, `tool_output{max_lines=2000,
max_bytes=51200}`, `mcp`, `compaction{auto=true, keep.tokens, buffer, prune, tail_turns, preserve_recent_tokens…}`,
`skills`, `commands`, `instructions`, `references`, `websearch` (`false` disables Anysola web search, or a provider
selection object), `plugins`, `worktree`, `warming` (default false), `providers`, `experimental{portable_shell_scanner,
todo, background_compaction, media_blob, image_generate, schedules, artifacts, subagent_depth, location_idle_timeout,
…}`. Deprecated V1 keys still accepted and normalised: `logLevel`, `server`, `command`, `reference`, `snapshot`,
`plugin`, `autoshare`, `autoupdate`, `disabled_providers`, `enabled_providers`, `small_model`, `subagent_depth`, `mode`,
`agent`, `provider`, `layout`, `permission`, `tools`, `attachment`.

Status for Anysola: `providers`/`model` restricted to `anysola` (§3.3) [0.5.1]; `share`/`enterprise` [dead] (sharing and
OpenCode Console removed); `update` — updates come from `GET /v1/code/version` + npm / install script (Anysola commits
11d757e9f3, 48bad1b2c3), the upstream self-updater is disabled; feature flags under `experimental` gate the `[dead]`
tables in §2.3.

### 3.5 Remote config from the Anysola gateway

- [ ] `GET <ANYSOLA_API_URL>/v1/code/config` (Bearer token) supplies `model`, `small_model`, `agents.title.model`,
  `providers.anysola{name, package, settings{baseURL, apiKey:"{env:ANYSOLA_TOKEN}"}, headers, models{…}}`; the last good
  body is cached in KV `anysola:config` and used when offline. Details belong to the gateway spec. —
  `packages/core/src/plugin/provider/anysola.ts:27,150-178` [0.5.1] [observed]

### 3.6 CLI/TUI config (`cli.json`)

- [ ] Location `<config>/cli.json`; `OPENCODE_CLI_CONFIG_CONTENT` overlays it; edits go through a flock in
  `<state>/locks` and temp+rename (mode 0600). — `packages/cli/src/config/config.ts:26-80` [0.5.1]
- [ ] Keys: `$schema`, `theme{name, mode: dark|light|system}` (default name `anysola`,
  `packages/tui/src/context/theme.tsx:139`), `keybinds`, `plugins`, `leader{timeout}`, `scroll{speed, acceleration}`,
  `attention{notifications, sound, volume, sound_pack, sounds}`, `diffs{…}`, `terminal{title, copy}`,
  `prompt{editor, paste, images}`, `session{sidebar, scrollbar, thinking, grouping, …, location, permissions}`, `tabs`,
  `mini`, `debug`, `experimental`, `animations`, `highlights`, `mouse`, `cursor`. — `packages/tui/src/config/index.tsx`
  [0.5.1]
- [ ] First run migrates V1 `<config>/tui.json` + `<state>/kv.json` (`theme`, `theme_mode_lock`, `diff_*`,
  `thinking_*`, `terminal_title_enabled`, …) into `cli.json`. — `packages/cli/src/config/migrate.ts:100-235` [0.5.1]

---

## 4. Project identification and worktrees

- [ ] Resolve: walk up from the directory to the first `.git` or `.hg`. Git project id = first of: `sha1("git-remote:" +
  host/path)` of the `origin` remote (normalised: lower-case host, no leading `/`, no `.git`), the cached id in the file
  `<git common dir>/opencode`, the repository's root commit hash; else `global`. Mercurial: cached `.hg/opencode` or the
  first root changeset. No VCS: `sha1("directory:" + <absolute directory>)`. — `packages/core/src/project.ts:291-440`,
  `packages/util/src/hash.ts:4-6` (`Hash.fast` = SHA-1 hex) [0.5.1] [observed: root-commit id `87eb537f…`; non-git
  `cf0597ed…` = `sha1("directory:/…/my-app")` verified]
- [ ] `project.worktree` = canonical directory (for linked git worktrees the main worktree); every linked worktree /
  directory used is recorded in `worktree(project_id, directory, strategy)`. Sessions keep their own `directory`. —
  `project.ts:336-363`, `packages/core/src/project/sql.ts` [0.5.1]
- [ ] Consequence for migration: the id depends on remote/root commit, so a moved checkout keeps its id, while a non-git
  directory gets a new id when moved. Stored paths are absolute (POSIX separators on disk, `C:/…` on Windows).

---

## 5. Logs

- [ ] One file shared by all processes: `<data>/log/opencode.log` (append mode). No rotation: an hourly trim (guarded by
  the `opencode.log.trim` directory lock, stale after 5 min) compacts it in place to about the last 25 MiB once it exceeds
  50 MiB. — `packages/util/src/observability/logging.ts:6-14,59-120` [0.5.1]
- [ ] Line format: `key=value` pairs separated by spaces, values JSON-quoted when they contain spaces, `=`, quotes or
  backslashes; keys `timestamp` (ISO), `level` (`INFO|WARN|ERROR|DEBUG`), `run` (8-hex per-process id), `message`, then
  flattened annotations/spans (e.g. `role=cli`, `version=2.2.5`, `channel=latest`). Example [observed]:
  `timestamp=2026-11-04T10:20:03.620Z level=INFO run=9e96f926 message="cli starting" version=2.2.5 channel=latest local=false args="[…]" role=cli`
  — `logging.ts:16-57`
- [ ] Level: `--log-level <all|trace|debug|info|warn|warning|error|fatal|none>` flag; env `OPENCODE_LOG_LEVEL`
  (`DEBUG|INFO|WARN|ERROR`, default INFO); `--print-logs` / `OPENCODE_PRINT_LOGS=1` mirrors to stderr (server logs only
  with `--standalone`). — `logging.ts:159-177`, `packages/cli/src/commands/commands.ts:5-23` [0.5.1]
- [ ] The CLI, the `serve --stdio` child and the background service all write to the same file (distinguished by `run`
  and `role`). [observed]

---

## 6. Migration to the Rust version

### 6.1 What exists on a user's machine (Linux and Windows identical apart from the home prefix)

| Item | Linux | Windows |
|---|---|---|
| Database | `~/.local/share/opencode/opencode.db` (+wal/shm) | `%USERPROFILE%\.local\share\opencode\opencode.db` |
| Anysola token | `credential` row (`integration_id='anysola'`, `value.access`) in that db | same |
| Budgets / per-session credit totals | KV `anysola:budget:default`, `anysola:session:<id>` | `[1.0]` not imported: credits are no percentage (see §2.8) |
| Always-allow permissions | `permission` table | same |
| TUI prefs (theme, keybinds, TUI settings) | `~/.config/opencode/cli.json` | `%USERPROFILE%\.config\opencode\cli.json` |
| Core config | `~/.config/opencode/opencode.json[c]` + project `opencode.json[c]` / `.opencode/` | same under `%USERPROFILE%` |
| Prompt history | `~/.local/state/opencode/prompt-history.jsonl` | `%USERPROFILE%\.local\state\opencode\prompt-history.jsonl` |
| Open tabs etc. | `~/.local/state/opencode/latest/tui/*.json` | same under `%USERPROFILE%` |
| Snapshots (for revert/undo) | `~/.local/share/opencode/snapshot/…` | same under `%USERPROFILE%` |
| Binary | `~/.anysola/bin/anysola` or npm global `@anysola/code` | `%USERPROFILE%\.anysola\bin\anysola.exe` or npm |

### 6.2 Import checklist (what Rust must read)

- [ ] Open `opencode.db` read-only (`mode=ro`, WAL aware; or copy db+wal+shm first). Never write to it: 0.5.1 and any
  upstream OpenCode keep using it.
- [ ] Check the schema: `migration` table contains all 59 ids of `verbatim/storage/migration-table.txt`. Extra ids = a
  newer upstream OpenCode touched the db (tolerate unknown columns; still merge `session_part`). Missing ids or a `session`
  v1 table without KV `migration.v1-v2 = {"phase":"completed"}` = not yet migrated: ask the user to run 0.5.1 once, or skip
  those sessions (decision below).
- [ ] Sessions: all `session_v2` rows (root, `parent_id` children, forks) with title, directory, project, agent, model,
  timestamps, archived, token/cost totals, `revert`, `permission`, `idle_outcome`.
- [ ] Messages: all `session_message` rows ordered by `seq`, `session_part` overlay merged, `SessionLegacy` upgrades applied
  (§2.5), blob refs resolved from `blob` (only if `media:` refs exist). Preserve ids, types, seq and the full `data`
  JSON (keep unknown fields verbatim so replay stays exact).
- [ ] Thinking: keep `reasoning.native.signature` / `native.redactedData`, `time.completed`, assistant `model` and
  `error` exactly; replay follows §2.7.
- [ ] Todos: derive from the latest `todowrite` tool part per session (no table).
- [ ] KV: `anysola:budget:default`, `anysola:session:<id>` (`total`, `turns`, `budget`), optionally `anysola:config`
  (offline cache). Ignore `models-dev:*`, `migration.v1-v2`, `schedule:leader`.
- [ ] Token: the `anysola` credential (`value.access`, `expires`, `metadata.apiURL`, `metadata.device`). Users on
  `ANYSOLA_TOKEN` have no row.
- [ ] Always-allow: `permission` rows (project-scoped) plus `session_v2.permission` rulesets.
- [ ] Projects: `project` rows + `worktree`; Rust must compute project ids the same way (§4) so new sessions in an existing
  directory join the imported project.
- [ ] Prefs: `cli.json` (`theme`, `keybinds`, …) and the core config files (read in place, same discovery, see 6.4).
- [ ] Optional: `prompt-history.jsonl`, `latest/tui/tabs.json`, `snapshot/` dirs (needed only if Rust keeps
  revert-to-snapshot for imported sessions).
- [ ] Not importable: `session_inbox` (pending prompts), `job` rows (running background jobs), running shells, in-flight
  `compaction` with `status='running'`, assistant rows without `time.completed` (interrupted turns: import as
  interrupted).

### 6.3 Decisions for the Rust team (options, not decided here)

1. **Where Rust stores its own data**
   - A. Keep the 0.5.1 layout and the same `opencode.db` (zero-migration, but the schema then constrains Rust forever and
     the file stays shared with upstream OpenCode).
   - B. Own XDG dirs with a new app name (`~/.local/share/anysola`, `~/.config/anysola`, `~/.local/state/anysola`,
     `~/.cache/anysola`; on Windows mirrored under `%USERPROFILE%` exactly like 0.5.1, or the native
     `%APPDATA%\Anysola` / `%LOCALAPPDATA%\Anysola`).
   - C. Everything under `~/.anysola/` (`bin/` already exists there): e.g. `~/.anysola/data/anysola.db`,
     `~/.anysola/config.json`, `~/.anysola/logs/`, `~/.anysola/state/` — one tree on every OS.
2. **Which sessions to import from a shared db**: all; only sessions whose messages use `providerID = 'anysola'`; or all,
   flagged as foreign when another provider appears (upstream OpenCode users).
3. **When**: automatic on first start (with a progress screen like the 0.5.1 migration overlay) vs an explicit
   `anysola migrate` command; one-shot (record a marker) vs re-runnable incremental import keyed by message id.
4. **Config files**: keep reading `~/.config/opencode/opencode.json[c]` + project `opencode.json`/`.opencode/` (parity
   for users and repos that already have them) vs copy into a new location vs support both with the old one lower
   priority.
5. **Token**: copy the `asc_…` token into Rust's store (keeping the 0600 file mode, or the OS keychain) vs require
   `anysola login` again.
6. **v1 / un-migrated dbs**: require running 0.5.1 once vs port the V1 import (`v1-migration.bun.ts`).
7. **Orphans**: `anysola:session:<id>` keys without a session (0.5.1 never deletes them) — drop or keep.
