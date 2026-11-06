# Anysola Code 0.5.1: CLI surface, headless `run`, packaging/install/update

Paths are relative to the fork `/home/ubuntu/anysola-code/build/fork` unless noted. `cli/` = `packages/cli/src/`.
Verbatim strings live in `rust/spec/verbatim/cli/` (referred to as `V/`). Captures were made with the shipped
linux-arm64 0.5.1 binary against `rust/testing/fake-gateway.mjs` (8797) and a variant
`rust/testing/runs/cli-agent/fake2/fake2.mjs` (usage, tokens, device flow, shell tool, thinking, 402, tunable /version).

Tags: `[0.5.1]` = exercised by the shipped binary; `[dead]` = present in source but off/removed in 0.5.1;
`[bug]` = shipped behaviour that is a defect (recommendation given).

---

## 1. Command tree, flags, global behaviour

Parser: effect/cli (`cli/commands/commands.ts`, `cli/framework/{spec,runtime}.ts`). Binary name `anysola`
(`OPENCODE_CLI_NAME`, build.ts:199). All `--help` outputs: `V/help/root_<cmd>.txt` (43 files, exit 0 each).

### 1.1 Root (`anysola [flags] [<directory>]`) — default command = TUI
- `[0.5.1]` Description `Anysola Code command line interface (based on OpenCode, MIT)` (commands.ts:48).
- `[0.5.1]` Positional `directory` (optional): `process.chdir(dir)` before anything else (default.ts:28). An unknown
  word such as `anysola foo` is this argument → uncaught `Error: ENOENT: ... chdir '<cwd>' -> 'foo'` + stack, exit 1
  (`V/framework-errors.txt`). Note `AnysolaVersion.enforceMinimum()` (network call) runs before the chdir.
- Flags: `--standalone`, `--server <url>`, `--auto`, `-c/--continue`, `-s/--session <id>` (ID to continue, or to
  create if it does not exist: becomes the new session's ID, default.ts:73-80,135-136), `--prompt <text>`.
  Hidden: `--yolo`, `--dangerously-skip-permissions` = `--auto` (commands.ts:40-44, default.ts:138).
- `[0.5.1]` Flow (default.ts): minimum-version gate → resolve server with `mismatch: "replace"` → prints
  `Starting background server...` or `Restarting background server (version mismatch)...` on stderr (or the
  update-preflight splash on a TTY, `cli/services/update-preflight.tsx`: "Anysola Code is updating from <svc
  version> to 0.5.1", stages "Keeping your session safe" / "Starting the new background service" / "Loading Anysola
  Code", "updated to", "update paused") → first-run sign-in when there is no Anysola connection and stdin+stdout are
  TTYs (`anysola/account.ts:90-98`) → TUI (TUI agent). Sign-in failure: clack error, exit 1.
- `[0.5.1]` Update check every 10 minutes while the TUI runs (default.ts:88-105): `GET /v1/code/version`, result
  surfaced in the TUI; install only when the user accepts (`updater.apply` → `npm i -g @anysola/code@<v>` or the
  install script with the version pinned, stdio ignored; updater.ts:245-258).
- `[0.5.1]` Without a TTY the TUI still renders escape sequences to stdout and does not exit (observed; TUI agent).

### 1.2 Global flags (every command)
`--help/-h`, `--version/-v`, `--wizard` (interactive command builder, effect/cli), `--completions
<bash|zsh|fish|powershell|sh|pwsh>` (scripts in `V/completions/`), `--log-level <all|trace|debug|info|warn|warning|
error|fatal|none>`, `--print-logs` ("Print logs to stderr (server logs require --standalone)"; sets
`OPENCODE_PRINT_LOGS=1`, runtime.ts:98; logfmt lines `timestamp=… level=INFO run=… message="…" … role=cli`).
`--standalone` / `--server` are global on: root, login, logout, status, usage, devices, debug config, mcp list/auth/
logout, plugin list/check/update, models, run, session *. They are accepted before the subcommand
(`anysola --server URL status` works). Log file always: `$XDG_DATA_HOME/opencode/log/opencode.log`.

### 1.3 Version
- `[0.5.1]` Exactly `anysola --version` or `anysola -v` → `Anysola Code 0.5.1 (OpenCode 2.2.5)` exit 0
  (index.ts:38-41). Release CI asserts this exact line (release.yml:80-84); install-test greps `^Anysola Code [0-9]`;
  code-action logs it.
- `[0.5.1]` Any other argv with `--version` (`anysola run --version`, `-v run`, `--version x`) → effect/cli prints
  `anysola v2.2.5`. Quirk; recommend Rust print the Anysola line for every `--version`.

### 1.4 Subcommands (all `[0.5.1]` unless tagged)
| Command | Behaviour (stdout unless noted) | Source |
|---|---|---|
| `login [-f/--force]` | Device flow through the server's `anysola` integration; "Already signed in (mask)…" unless `--force`; force revokes+removes old token first | anysola/login.ts |
| `logout` | Revoke `POST /v1/code/tokens/self/revoke`, then remove credential; env token → info line | anysola/logout.ts |
| `status` | `GET /v1/code/me` key/value table + `Token <mask>` | anysola/status.ts |
| `usage [--days 7\|30] [--format default\|json]` | `GET /v1/code/usage?days=N` tables | anysola/usage.ts |
| `devices` / `devices revoke <id>` | `GET /v1/code/tokens` table / `POST /v1/code/tokens/<id>/revoke` | anysola/devices*.ts |
| `update` (alias `upgrade`) `[--print] [--force]` | §4.3 | upgrade.ts |
| `uninstall [--dry-run] [-f/--force]` | §4.4 | uninstall.ts |
| `acp [--login]` | ACP server on stdio over a `--standalone` server; `--login` = `anysola login` (integrations agent) | acp.ts |
| `debug config` / `debug paths [name]` | Redacted config sources JSON / global paths | debug/*.ts |
| `mcp list\|add\|auth\|logout` | MCP config + OAuth (integrations agent); `mcp add <name> [--url u] [--header k=v] [--env k=v] [--global] -- cmd…` writes `mcp.servers.<name>` into the first existing of `opencode.json`, `opencode.jsonc`, `.opencode/opencode.json(c)` (cwd or global config dir) | mcp/*.ts |
| `plugin list [--builtin]\|add <pkg>\|check [t]\|update [t]\|remove <pkg>` | Plugin management (provider-locked; integrations agent) | plugin/*.ts |
| `models` | Sorted `provider/model` lines | models.ts |
| `run` | §2 | run.ts |
| `session list [-n N] [--format table\|json]`, `delete <id>`, `export [id] [--sanitize]`, `import <file\|url> [--directory d]` | `V/other-commands.txt` | session/*.ts |
| `service start\|restart\|status\|stop\|get [key] [name]\|set <key> <value> [env-value]\|unset <key> [name]` | §5 | service/*.ts, services/service-config.ts |
| `reload` | `Configuration reloaded` | reload.ts |
| `serve [--hostname] [--port] [--cors…] [--service] [--stdio]` | "Run the server the TUI connects to (started for you; internal)" — never invoked by users in practice | serve.ts, server-process.ts |

Removed vs upstream `[dead]` (commit 67cbb0e156, README "Differences"): `pair`, `auth`, `api`, `mini`, `stats`,
`debug agents`, `debug heap-dump`, opencode2 node build. Hidden internal entry: `OPENCODE_SSH_ASKPASS_PORT` turns
the binary into an SSH askpass helper (index.ts:43-46) `[0.5.1]`, internal.

Captured outputs: `V/account-commands.txt`, `V/update-uninstall.txt`, `V/service.txt`, `V/other-commands.txt`.

### 1.5 Exit codes
- 0 success; 1 any reported error (`process.exitCode = 1`), parse errors (help → stdout, `\nERROR\n  <msg>` →
  stderr; `V/framework-errors.txt`), minimum-version gate, budget reached, cancelled MCP/global form in run,
  uninstall without TTY/`--force`; 130 = SIGINT in `run` (first SIGINT interrupts the session, exitCode 130; second
  exits 130 immediately, noninteractive.ts:709-716) and clack prompt cancel (ui/prompt.ts:28-33, export picker).
- Shutdown: SIGTERM/SIGINT teardown deadline 10 s, then stderr `shutdown did not finish within 10s; exiting. Still
  pending:` + list, exit 1; SIGHUP treated as SIGTERM only with a TTY (cli/shutdown-deadline.ts). SIGUSR1 heap
  snapshot / SIGPROF 10 s CPU profile into the log dir (non-Windows; heap.ts, cpu-profile.ts) — debug only.

### 1.6 Environment variables read
User-facing / contract:
- `ANYSOLA_TOKEN` — token as an env connection (core/src/anysola.ts:15; CI path). `logout` refuses to remove it.
- `ANYSOLA_API_URL` — API origin (default `https://api.anysola.com`) for CLI account calls, version check, provider,
  and the install scripts.
- `ANYSOLA_INSTALL_DIR` — script install dir (default `~/.anysola/bin`); used by install scripts and update/
  uninstall method detection (anysola-version.ts:14-20).
- `ANYSOLA_VERSION` — install scripts: version to install (also build-time version).
- `ANYSOLA_DOWNLOAD_URL` — install scripts: release download base (default `https://github.com/Anysola/code/releases/download`).
- `OPENCODE_PASSWORD` (legacy `OPENCODE_SERVER_PASSWORD`) — basic-auth password for `--server`, adopted by `serve`.
- `OPENCODE_CONFIG`, `OPENCODE_CONFIG_CONTENT`, `OPENCODE_CONFIG_DIR`, `OPENCODE_DISABLE_PROJECT_CONFIG`
  (`OPENCODE_CONFIG_PROJECT_DISABLE`), `OPENCODE_DISABLE_CLAUDE_CODE`, `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS`,
  `OPENCODE_DISABLE_FILEWATCHER` (`OPENCODE_FILEWATCHER_DISABLE`), `OPENCODE_DISABLE_FFF` (default off on Windows),
  `OPENCODE_GIT_BASH_PATH` (Windows), `OPENCODE_DB`, `OPENCODE_DISABLE_CHANNEL_DB`, `OPENCODE_DISABLE_MODELS_FETCH`,
  `OPENCODE_MODELS_URL/PATH`, `OPENCODE_CLIENT`, `OPENCODE_TUI_CHANNEL`, `OPENCODE_LOG_LEVEL`,
  `OPENCODE_PRINT_LOGS`, `NO_PROXY/no_proxy` (merged with `127.0.0.1,localhost,::1`, index.ts:24-33),
  `OTEL_EXPORTER_OTLP_ENDPOINT/HEADERS`, `XDG_*` (storage/config agent owns semantics).
- Gates: `ANYSOLA_ENABLE_WEB_UI` `[dead]` (web-ui.ts:108), `ANYSOLA_ENABLE_REMOTE_TUNNEL` `[dead]` (remote-tunnel.ts:21).
- Internal: `OPENCODE_PTY_HANDOFF`, `OPENCODE_SIMULATE`, `OPENCODE_DRIVE*`, `OPENCODE_STORY`, `OPENCODE_ROUTE`,
  `OPENCODE_FAST_BOOT`, `OPENCODE_SSH_ASKPASS_*`, `OPENCODE_TEST_HOME`, native-binding path overrides.
- `OPENCODE_DISABLE_AUTOUPDATE` is set by code-action and `testing/env.sh` but **read nowhere** in 0.5.1 (only a test).

### 1.7 Outbound identity (cross-ref gateway agent)
CLI-process requests carry `User-Agent: opencode/latest/2.2.5/cli` (bun `--user-agent`, build.ts:190) and, for
account calls, `x-anysola-version: 0.5.1`; model requests from the server carry `opencode/latest/2.2.5+anysola.0.5.1/cli`
and `x-anysola-client: opencode`. `/v1/code/version` carries no `x-anysola-version`. release.yml notes the gateway
allowlists the UA.

---

## 2. `anysola run` (headless)

Source: `cli/commands/handlers/run.ts`, `cli/run/run.ts`, `cli/run/noninteractive.ts`, `cli/run/ui.ts`.
Captures: `V/run/*.{cmd,stdout,stderr}` (exit code at end of each `.cmd`).

### 2.1 Arguments and flags
- `message...` variadic. Parts containing a space are wrapped in `"…"` with inner `"` escaped, joined by spaces
  (run.ts:244-247): `anysola run "explain this" x` sends `"explain this" x` to the model `[0.5.1]`.
- `[bug]` Arguments after `--` are appended a second time (effect/cli already put them in `message`; run.ts:190,199
  re-appends `argv.slice(separator+1)`): `run a -- "b c" d` → `a "b c" d "b c" d`. Recommend not replicating.
- stdin: when stdin is not a TTY it is read to EOF and appended as `message + "\n" + piped` (or alone)
  (run.ts:80, mergeInput). A non-TTY stdin that never closes makes `run` wait forever. Empty → `You must provide a
  message` (exit 1). code-action passes the whole prompt on stdin with no message argument.
- `-c/--continue`: latest top-level session whose directory equals cwd (session-target.ts:120-131); none → new.
- `-s/--session <id>`: existing → resume; unknown → new session created with exactly that ID.
- `--fork`: requires `-c` or `-s` (`--fork requires --continue or --session`); `-s unknown --fork` → `Session not found`.
- `-m/--model provider/model#variant` (e.g. `anysola/auto#economy`): invalid → `Invalid model reference: …`,
  `Model unavailable: anysola/x`, `Variant unavailable for anysola/m: v`.
- `--agent <name>` (`Agent not found: "x"`), `--title <t>` (new sessions only; `--title ""` → first 50 chars of the
  message + `...`), `--thinking`, `--format default|json`, `-f/--file <path>` (max 100; resolved against cwd; must
  be a regular file ≤ 10 MiB else `Cannot attach a directory, special file, or file larger than 10 MiB: <p>`;
  missing → `File not found: <p>`). Text files (valid UTF-8, not binary) are inlined into the prompt as
  `\n\n<file name="<basename>">\n<content>\n</file>`; images/PDF/other binary go as data-URI attachments
  (run.ts:267-306, noninteractive.ts:878-889).
- `--budget <int ≥1>` (0 → parse error). `--auto` (+ hidden `--yolo`, `--dangerously-skip-permissions`).
- `--standalone` / `--server` honoured via global flags (server-connection.ts:23-57).
- Minimum-version gate runs first (run.ts:188); the background service is used with `mismatch: "ignore"` — `run`
  happily talks to an older running service (hence the "If you just updated, run `anysola service restart`" hint).
- Session gets the CLI's whole environment as session env when using the managed service (`Env.session()` minus
  passwords; run.ts:105).

### 2.2 Default (text) output
- Text parts: **stdout** (trimmed + EOL) when stdout is not a TTY; when stdout is a TTY they go to **stderr** framed
  by blank lines (noninteractive.ts:109-120; verified with `script`).
- Reasoning (only with `--thinking`): `Thinking: <text>`; stdout when not a TTY, else stderr dim italic
  (`\x1b[90m\x1b[3m…\x1b[0m\x1b[0m`).
- Always stderr: step header `> <agent> · <model id>` (e.g. `> build · claude-sonnet-5-5`), tool lines
  `\x1b[0m<icon> \x1b[0m<title> <dim description>` (block tools print icon+title, then body, blank lines), failed
  tools `\x1b[0m✗ \x1b[0m<title> failed` + `Error: <msg>`, errors `\x1b[91m\x1b[1mError: \x1b[0m<msg>`,
  permission line, and the spend summary.
- Blank lines: `UI.empty()` prints `\x1b[0m\n` only once per process (the `blank` flag is never reset, ui.ts:10-17)
  — observed output starts with one `\x1b[0m` line and has no other blank lines.
- Tool titles come from `packages/tui/src/mini/tool.ts` `toolInlineInfo` (shared with the TUI): `$ <command>`
  (block, body = output), `→ Read <path>`, `← Write/Edit <path>` (block with diff), `✱ Glob/Grep "<pat>"` (+ ` in
  <dir> · N matches`), `% WebFetch <url>`, `◈ Web Search via Anysola "<query>"`, `→ Skill "<name>"`, `# Todos`,
  `→ Asked N question(s)`, `✓/✗/• <description>` (+ `<Kind> Agent`) for subagents, `⚙ <name> <json>` fallback.
- `[1.0]` Summary line instead: stderr `\x1b[90mAnysola · 5h <n>%[ · week <n>%] · this run <p>%\x1b[0m` when a usage
  event arrived (`spec/protocol.md §4.4`). 0.5.1 spend summary (run.ts:178-197, 173-174): after the run, stderr `\x1b[90mAnysola · <spent> credit[s] · <remaining>
  left\x1b[0m`, spent = session credit delta from the server's `anysola.credits` RPC events; omitted when spent ≤ 0,
  in JSON mode, or when the service has no account RPC. Printed even after a budget/other error.

### 2.3 JSON output (`--format json`)
stdout is strictly one JSON object per line (`JSON.stringify` + EOL), `{type, timestamp, sessionID, ...}`:
- `step_start` `{part:{id,sessionID,messageID,type:"step-start",snapshot}}`
- `text` `{part:{id,sessionID,messageID,type:"text",text,time:{start,end}}}`
- `reasoning` (only with `--thinking`) `{part:{…,type:"reasoning",text,metadata,time}}`
- `tool_use` `{part:{partID,sessionID,messageID,type:"tool",id,tool,state:{status:"completed"|"error",input,
  output|error,title,metadata:{metadata,content,providerCall,providerResult,rawInput},time:{start,end}}}}`
- `step_finish` `{part:{id,…,type:"step-finish",reason,snapshot,cost,tokens:{input,output,reasoning,cache:{read,write}}}}`
- `error` `{error:{type,message,…}}` — server errors keep their shape (e.g. `provider.quota` with `status`,
  `response.body`; `aborted` for interruptions); CLI errors use `{type:"unknown",message}` and `sessionID:""` before
  a session exists.
Part IDs: live events `prt_<event id without evt_>`; parts recovered after idle by `reconcile()` use
`prt_<message id without msg_>_<step-finish|text-N|reasoning-N|tool-<id>>` (the final `step_finish` usually has this
form). Text is emitted once per text part when it ends (not streamed deltas). No spend line in JSON mode. The
permission `!` line still goes to stderr. Captures: `V/run/json-*.stdout`, `V/run/capture-out2/4/6.stdout`.

### 2.4 Permissions and questions without a user
- Default: each `permission.asked` for this run's session (or its subagent children) → stderr
  `! permission requested: <action> (<resources>); auto-rejecting`, reply `reject` with the feedback message; the tool
  fails with that text as model-visible output and the run continues, exit 0 (`V/run/*-permission-ask*`).
- `--auto`: reply `once` silently; explicitly `deny` rules still deny (denied tools are removed from the model's tool
  list: the model's call fails with `No tool named "shell" is currently available…`).
- Question forms: cancelled with the "cannot ask the user questions" feedback; other forms (MCP elicitations) are
  cancelled and set exit 1.

### 2.5 Budget
`[1.0]`: `--budget N` is a percent of the 5-hour window (1–100) and the message reads `Budget reached: <used>/<N>% of
the 5-hour window. Continue with \`anysola run -s <id> --budget <percent>\`.`; `--use-banked-reset` spends one banked
reset when a usage window is full (`spec/protocol.md §4.3`). 0.5.1:
`--budget N` sets the session budget via `anysola.budget` RPC (persisted on the session), watches `credits` events and
interrupts the session at the next `session.step.started` once total ≥ N (run.ts:203-236). The interrupt can cut a
running tool (observed: websearch failed with `Tool execution interrupted`, `Step interrupted`). Then the final error
is always the budget message — last `error` event in JSON (code-action relies on "last error"):
`Budget reached: <total>/<N> credits. Continue with \`anysola run -s <id> --budget <credits>\`.` exit 1. If the session
is already at/over the budget, the message is printed before any prompt (text mode additionally dumps an unhandled
ECONNRESET stack `[bug]`).

### 2.6 Exit codes of `run`
0 = finished (including auto-rejected permissions); 1 = any error event (step/execution failure, gateway 402/401,
budget, minimum version, bad flags/model/agent/file, no message, prompt not promoted); 130 = SIGINT.

---

## 3. GitHub Action `Anysola/code-action` — acceptance contract for Rust `run`

Source `/home/ubuntu/anysola-code/code-action` (`action.yml`, `scripts/{main,lib,install,review,issue}.sh`,
`test/bin/anysola` stub).
1. Install: `curl -fsSL --retry 3 https://www.anysola.com/install | sh -s -- --no-modify-path [--version X]`
   (install.sh:7-11), stdout redirected to stderr; expects the binary at `${ANYSOLA_INSTALL_DIR:-$HOME/.anysola/bin}/anysola`,
   executable; appends that dir to `$GITHUB_PATH` (the install script also does when `GITHUB_ACTIONS=true`).
   `ANYSOLA_CODE_BIN` overrides (tests).
2. `"$ANYSOLA_BIN" --version` must run (logged as `Using <line>`; failure tolerated).
3. Before install, `check_access` calls `GET $ANYSOLA_API_URL/v1/code/config` with `Authorization: Bearer` and
   `x-anysola-client: opencode` and reads `.providers.anysola.models` keys (gateway contract, not CLI).
4. Invocation (lib.sh:231-247):
   `anysola run --standalone --format json -m anysola/<model> --budget <N>` plus either
   `--agent anysola-review` with env `OPENCODE_CONFIG_CONTENT=<REVIEW_CONFIG>` `OPENCODE_DISABLE_PROJECT_CONFIG=1`
   `OPENCODE_DISABLE_CLAUDE_CODE=1` (review mode), or `--auto` (issue mode). Env: `ANYSOLA_TOKEN=<secret>`,
   `OPENCODE_DISABLE_AUTOUPDATE=1` (no-op); GitHub tokens unset. **Prompt on stdin, no message argument**; stdout →
   `events.jsonl`, stderr → `stderr.log`; exit status propagated (non-zero = failure).
   REVIEW_CONFIG requires config support for `agents.<name>.{mode:"primary",description,permissions:[{action,
   resource,effect}]}` with last-match-wins rules on actions `*`, `read`, `grep`, `glob` and glob resources
   (`*.env`, `*.env.*`, `*.env.example`) — config/agent agent owns semantics.
5. Parsing: non-JSON lines skipped. Answer = concatenation (`\n\n`) of `.part.text` of all `type=="text"` events whose
   `.part.messageID` equals the last text event's messageID. Error = `.error.message // .error.data.message //
   (.error|tostring)` of the **last** `type=="error"` event.
6. Failure classification greps the error (or last 5 stderr lines) for: `Budget reached`, `used this cycle`,
   `Anysola credits`, `Daily free limit`, `may cost up to`, `expired or revoked`, `authentication_error`,
   `Model unavailable: anysola/`, `needs a Pro or Team`, `isn't open to this account` (also `’`), `is paused`.
   These strings originate in the CLI (`Budget reached…`, `Model unavailable…`) or the gateway/account plugin.
7. Review mode: the agent must not edit; issue mode: edits in the working tree (cwd = `$GITHUB_WORKSPACE`), the action
   then commits. Token format checked by the action: `^asc_[0-9A-Za-z]{40}$`.

---

## 4. Packaging, distribution, install, update

### 4.1 Build and release (`.github/workflows/release.yml`) `[0.5.1]`
- Trigger: tag `v*` (`v<anysola version>`), or manual dry run. Bun 1.4.4 cross-compiles all 12 targets on
  ubuntu-24.04 with `ANYSOLA_VERSION`, `OPENCODE_VERSION`=packages/cli version (2.2.5), `OPENCODE_CHANNEL=latest`,
  `--skip-web-ui`. Smoke test: exact `--version` line, `--help` contains `anysola <subcommand>` and not
  `opencode command line`.
- Targets (build.ts:42-60; name = `[os(win32→windows), arch, baseline?, musl?]`): linux-arm64, linux-x64,
  linux-x64-baseline, linux-arm64-musl, linux-x64-musl, linux-x64-baseline-musl, darwin-arm64, darwin-x64,
  darwin-x64-baseline, windows-arm64, windows-x64, windows-x64-baseline.
- macOS binaries ad-hoc signed (`codesign --force --sign -`) on macos-15. Windows unsigned.
- Pack (pack.ts): npm tarballs + archives `anysola-<target>.tar.gz` (linux; `tar --mtime=@0 --owner=0 --group=0
  --numeric-owner -czf`, single entry `anysola`) or `.zip` (darwin/windows; `zip -X`, entry `anysola` /
  `anysola.exe`); binary mode 0755, mtime 1980-01-01.
- Publish: platform packages first then `@anysola/code`, dist-tag `latest` (or `next` for versions with `-`).
- GitHub release `v<ver>` titled `Anysola Code <ver>`, assets = 12 archives + `install.sh` + `install.ps1` +
  `SHA256SUMS` (`sha256sum anysola-* install.sh install.ps1`; GNU format `<hex>  <name>`), prerelease when the version
  has `-`. Exact list, sums and body: `V/release.txt`. Observation: x64 vs x64-baseline archives are byte-identical in
  0.5.1 (same for musl).
- `install-test.yml`: after each release, runs `curl … | sh` on ubuntu x64/arm, macOS arm/intel and `irm | iex` on
  windows-2025 / windows-11-arm (PowerShell 5.1), then asserts `anysola --version` matches `^Anysola Code [0-9]`,
  `anysola update --print` contains `anysola.com/install` / `install\.ps1` or `Already up to date`, and
  `anysola uninstall --dry-run` contains `Installation method: curl`; reinstall while installed.
- `upstream.yml`: daily 04:17 UTC merge attempt of new OpenCode v2 tags, keeping `script/anysola-deleted.txt` paths
  deleted (fork maintenance only; not needed for Rust). `check-process-daemon.yml` = upstream Rust PTY daemon CI.

### 4.2 npm layout `[0.5.1]`
- `@anysola/code`: `bin: {"anysola": "bin/anysola.exe"}` on every OS (even Linux/macOS the file is named `.exe`),
  `scripts.postinstall: "node ./postinstall.mjs"`, `optionalDependencies` = the 12 `@anysola/code-<target>` at the
  exact version, `os [darwin,linux,win32]`, `cpu [arm64,x64]`. Ships a placeholder `bin/anysola.exe` shell script that
  prints the "postinstall script was not run" error (`V/release.txt`).
- Platform packages: `@anysola/code-<target>` with `os`, `cpu` (no `libc`, so npm downloads every variant of the
  os/cpu), files `bin/anysola[.exe]`, LICENSE, NOTICE.
- postinstall (cli/../script/postinstall.mjs): picks candidates in order (glibc/musl via `/etc/alpine-release` or
  `ldd --version`; AVX2 via `/proc/cpuinfo`, `sysctl hw.optional.avx2_0`, or PowerShell `IsProcessorFeaturePresent(40)`;
  **Windows**: x64 Node on ARM64 detected via `PROCESSOR_IDENTIFIER` → arm64, with x64 emulated fallbacks), checks the
  platform package version equals the expected one, hard-links (else copies) its binary over `bin/anysola.exe`,
  chmod 755, verifies `--version` exits 0; else `npm install --ignore-scripts --no-save --prefix <tmp> <pkg>@<v>`
  (`--force` when CPU differs) and retries. Requires Node ≥ 20 (README).

### 4.3 Install scripts and update
- `install` (POSIX sh; `V/install-sh.txt`): flags `-h/--help`, `-v/--version <v>` (or env `ANYSOLA_VERSION`),
  `--no-modify-path`; unknown options warned and ignored. Needs curl, tar (unzip on macOS). Target detection: uname,
  Rosetta → arm64, AVX2 → `-baseline`, musl → `-musl`. MINGW/MSYS/CYGWIN → error pointing to install.ps1. Version
  default = `latest` from `GET $ANYSOLA_API_URL/v1/code/version` (sed on `"latest"`). Downloads archive + SHA256SUMS
  from `${ANYSOLA_DOWNLOAD_URL:-https://github.com/Anysola/code/releases/download}/v<ver>`, verifies (accepts `*name`),
  extracts, copies to `$INSTALL_DIR/.anysola.new`, chmod 755, `mv -f` over `$INSTALL_DIR/anysola` (running copies keep
  the old inode), runs `--version`. PATH: appends `\n# anysola\n<line>` to the first existing of fish
  `config.fish` / zsh `.zshrc .zshenv` / bash `.bashrc .bash_profile .profile` / other `.profile` (creates the first
  if none); writes `$GITHUB_PATH` under GitHub Actions. Prints `Installed: …` (if already on PATH), `Installing
  Anysola Code <v> (<target>)`, final block.
- `install.ps1` (**Windows**, `V/install-ps1.txt`): PowerShell 5.1-compatible, forces TLS 1.2, arch from
  `PROCESSOR_ARCHITEW6432`/`PROCESSOR_ARCHITECTURE` (AMD64/ARM64), AVX2 via P/Invoke else `windows-x64-baseline`,
  version from env `ANYSOLA_VERSION` or `/v1/code/version` (`Invoke-RestMethod`), downloads zip + SHA256SUMS, verifies,
  `Expand-Archive`, renames a running `anysola.exe` to `anysola.exe.old` (or `.<guid>.old`) then copies, deletes
  `anysola.exe*.old`, checks `--version`, adds `%USERPROFILE%\.anysola\bin` to the **user** PATH and the current
  session; `Fail` writes red `Error: …` and `throw`s (does not close the `iex` caller's window). Does not write
  `$GITHUB_PATH`.
- www routes `/install` and `/install.ps1` (`web/anysola/services/web/src/lib/code-install.ts`): proxy
  `https://github.com/Anysola/code/releases/latest/download/<install.sh|install.ps1>` (GitHub "latest" release),
  `cache-control: public, max-age=300`; on failure HTTP 502 with a script that prints the unavailable message.
- `GET /v1/code/version` → `{latest, minimum, npmPackage:"@anysola/code", notes|null}`, `cache-control: no-store`,
  rate-limited per IP (cloud/src/code/app.ts:189-201). CLI uses it for: the minimum gate (3 s timeout, offline =
  allowed, dev builds `0.0.0*`/`local` skip), `anysola update`, the TUI's periodic check, and the install scripts.
  Compare = semver core, prerelease sorts before release (anysola-version.ts:74-89).
- `anysola update` (upgrade.ts): method = script when `realpath(execPath)` equals `realpath(<install dir>/anysola[.exe])`
  (case-insensitive), else npm. Command shown/ran per `V/update-uninstall.txt`; `--print` prints only; `--force`
  ignores "Already up to date". npm on **Windows** runs through a shell (`npm.cmd`). Does not run the minimum gate.

### 4.4 Uninstall
- Method detection (updater.ts:156-187): `curl` (script path), brew/winget path patterns (upstream names; never true
  for Anysola), else if the executable is the `bin` of an `@anysola/code` package two directories up, probe
  `npm/pnpm list -g`, `bun pm ls -g`, `yarn global list`, `vp list -g --json` concurrently (30 s timeout each).
- Removes only the cache dir (`$XDG_CACHE_HOME/opencode`), stops all registered services (`service*.json` in the
  state dir) after shutting down persistent PTYs, removes `.anysola/bin` PATH lines from shell rc files (only if the
  bin dir holds nothing else; lines `export PATH=…/.anysola/bin…` or `fish_add_path …` and a preceding `# anysola`),
  runs the package manager removal (5 min timeout), prints how to delete the binary. Data/config/state are kept.
- **Windows**: removes the user PATH entry via PowerShell; moves retained `opencode-(service|upgrade)-<pid>-<hex>.exe`
  hard links out of the cache; retries cache removal on `Busy` 40×250 ms; npm removal holds a second hard link to the
  running exe (RetainedImage). `RetainedImage.installed()` only recognises `~/.opencode/bin/opencode.exe`, so a
  script-installed `~/.anysola/bin/anysola.exe` service does not keep a retained link `[bug, harmless]` — the
  install script's rename-aside handles the running exe instead.

---

## 5. Background service, as the user experiences it

- **Who spawns it**: every command that calls `ServerConnection.resolve()` without `--standalone`/`--server` and with
  service config `disabled != true`: the TUI, `run`, `login`, `logout`, `status`, `usage`, `devices`, `models`,
  `reload`, `session *`, `mcp list/auth/logout`, `plugin list/check/update`, `debug config`, `service start/restart`.
  Not: `--version`, `--help`, `debug paths`, `mcp add`, `plugin add/remove`, `service get/set/unset/status/stop`,
  `update`, `uninstall`, `acp` (always standalone). Only the TUI prints `Starting background server...`.
- **How**: `<execPath> serve --service` spawned `detached: true`, `windowsHide: true`, stdio `[ignore, ignore, pipe]`,
  `unref()` (client/src/service-contender.ts:14-55); up to 2 contenders, 5 s spawn delay doubling to 30 s, 120 s
  overall timeout, 2 s probe timeout, 3 consecutive probe timeouts → kill and replace. The service `chdir`s to
  `$HOME`, applies `service set env` variables (except `XDG_STATE_HOME`), binds `127.0.0.1:<port>` and registers.
- **Ports**: channel `latest` → 49374 (0xC0DE); **Windows** → 28894 (0x7000|0x0DE, avoids Hyper-V/WinNAT/WSL
  reserved ranges); channel `local` 0xC0DF; other channels hashed (service-config.ts:46-60). Overridable with
  `anysola service set port/hostname`. Port taken by something else → startup error with `PortOwner` description.
  `serve` without `--port` scans upward from 4096; `--standalone` uses `serve --stdio --port 0` (ephemeral port).
- **Files**: registration `$XDG_STATE_HOME/opencode/service.json` (0600, `{id, version:"2.2.5+anysola.0.5.1", url,
  pid, password}`); settings `$XDG_CONFIG_HOME/opencode/service.json` (0600, `{disabled, remote, remoteAccepted,
  hostname, port, password, cors, env}`); other channels `service-<channel>.json`. Clients authenticate with HTTP
  Basic `opencode:<password>`; health probe `GET /api/info`.
- **Lifetime**: no idle shutdown exists. It outlives the TUI and every CLI command (observed: ppid 1, own session;
  this is the 16-hour orphan). It exits on SIGTERM (`service stop`/`restart`, `service set …` for hostname/port/
  password/env/cors/remote/disabled=true, version-mismatch replacement by a newer TUI, uninstall) with SIGKILL after
  ~5 s, or by itself within ≤5 s when its registration file is removed or replaced (service-registration.ts:47-74;
  observed ~2 s).
- **Version mismatch**: TUI replaces an older service (`version-mismatch`); every other command reuses whatever is
  running (`mismatch: "ignore"`).
- `--standalone`: private `serve --stdio --port 0` child (own process group), password in `OPENCODE_PASSWORD`, prints
  `{"url":"…"}` on stdout; it ends when its stdin (lease pipe) closes, i.e. when the CLI exits; SIGTERM + 3 s
  force kill. The code-action and `acp` always use this.
- `service set disabled true` makes all commands use standalone servers. Persistent terminals (TUI) spawn a separate
  detached PTY daemon (`packages/process-daemon`, `core/src/persistent-pty/daemon.ts`) — TUI agent.
- `[0.5.1]`/`[bug]` `service set remote true` is not gated: it runs `RemoteTunnel.ensure()` (service-config.ts:264),
  contacts the third-party OpenTunnel service, stores an identity under `$XDG_DATA_HOME/opentunnel/default/` (token,
  key, certificate), prints `Setting up remote access; this can take a minute...`, and stores a route; the service
  then only logs "remote access is not available in Anysola Code". README promises no remote tunnel.

---

## Contracts the Rust binary must keep byte-compatible
1. `anysola --version` → `Anysola Code <ver> (OpenCode 2.2.5)` + LF, exit 0 (release CI, install-test, action log).
2. `run --format json` event lines: `type` values `step_start|text|reasoning|tool_use|step_finish|error`, top-level
   `sessionID`, `part.messageID` and `part.text` on text events, `error.message` on error events, one object per
   line on stdout, nothing else on stdout; budget error is the last error event.
3. (`[1.0]` percent wording, §2.5) Budget message `Budget reached: <t>/<n> credits. Continue with \`anysola run -s <id> --budget <credits>\`.` and
   exit 1; `Model unavailable: anysola/<id>` text; exit status non-zero on any error, 0 on success.
4. `run` reading the prompt from stdin when no message is given; flags `--standalone --format json -m --budget
   --agent --auto -s -c -f --title --thinking --fork`; `ANYSOLA_TOKEN`, `ANYSOLA_API_URL`, `OPENCODE_CONFIG_CONTENT`,
   `OPENCODE_DISABLE_PROJECT_CONFIG`, `OPENCODE_DISABLE_CLAUDE_CODE` env.
5. Text-mode stream split: answer on stdout (non-TTY), everything else on stderr; summary line (`[1.0]`
   `Anysola · 5h N% · this run P%`, 0.5.1 `Anysola · N credit[s] · M left`); `Error: ` prefix.
6. Release asset names `anysola-<target>.{tar.gz|zip}` with a single `anysola`/`anysola.exe` entry, `SHA256SUMS` GNU
   format, `install.sh`/`install.ps1` assets, tag `v<ver>`; npm `@anysola/code` with `bin.anysola = bin/anysola.exe`,
   postinstall, `@anysola/code-<target>` packages; install dir `~/.anysola/bin` and `ANYSOLA_INSTALL_DIR`.
7. `anysola update --print` output containing `anysola.com/install` / `install.ps1` / `Already up to date`;
   `anysola uninstall --dry-run` containing `Installation method: curl` for script installs.
8. `GET /v1/code/version` semantics (latest/minimum/notes) and the minimum-gate message; `x-anysola-version`
   header and the gateway-allowlisted User-Agent `opencode/latest/2.2.5/cli` (gateway agent to confirm).
9. Service/state paths that other installs share: `~/.config/opencode`, `~/.local/share/opencode`,
   `~/.local/state/opencode`, `~/.cache/opencode` (storage agent); if Rust keeps a service, registration
   `service.json` format and port 49374 / Windows 28894 so 0.5.1 and Rust do not fight over the port.
10. Account command outputs in `V/account-commands.txt` (status table, usage tables, devices table, login/logout lines).

## Not needed in Rust (dead or internal in 0.5.1)
- Web UI (`ANYSOLA_ENABLE_WEB_UI`, app archive; build uses `--skip-web-ui`) and remote tunnel
  (`ANYSOLA_ENABLE_REMOTE_TUNNEL`, OpenTunnel; including the ungated `service set remote true` provisioning).
- V1 run bridge (`cli/run/v1.ts`, exported only), upstream commands pair/auth/api/mini/stats/debug agents/heap-dump.
- `OPENCODE_DISABLE_AUTOUPDATE` (read nowhere), upstream update server/Homebrew/WinGet paths (method detection kept
  only for uninstall display), `vp`/`yarn`/`pnpm`/`bun` detection branches (nice-to-have only).
- SIGUSR1 heap snapshot, SIGPROF CPU profile, `OPENCODE_SIMULATE`/`OPENCODE_DRIVE*`/`OPENCODE_STORY`, SSH askpass mode
  (only if Rust does not shell out to ssh with askpass), `--wizard`, effect/cli completion scripts (generate own).
- The `upstream.yml` merge workflow and `check-process-daemon.yml`; `script/anysola-deleted.txt`.
- Bugs not to replicate: `--` argument duplication, `UI.empty` printing only once, ECONNRESET stack dump when the
  budget is already reached, `anysola run --version` printing `anysola v2.2.5`, uncaught chdir stack trace,
  Effect stack traces after `service set/get` errors, identical baseline archives.
- Whether the background `serve --service` architecture itself is kept is a design decision; if dropped, `service
  *`, `serve`, `--standalone`, `--server` can become no-ops/aliases, but `--standalone` must still be accepted (action).
