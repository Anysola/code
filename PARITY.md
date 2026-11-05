# PARITY.md: what "Anysola Code 1.0.0 done" means

Every unchecked box below is a user-visible behaviour of Anysola Code 0.5.1 (or a deliberate replacement of one)
that the Rust binary must show before 1.0.0 ships. A box is ticked only when the acceptance test for its milestone
covers it. Specs live in `spec/`; exact texts, schemas and captures live in `spec/verbatim/` (`V/` below). This file
points at them and never copies long text.

Conventions
- One line per behaviour: `- [ ] <behaviour> — <where the exact text/rule is>`. Section references use the spec's own
  headings (`§2.1`, or the heading name for specs whose headings are unnumbered, e.g. `spec/tools.md §read`).
- `[cond: <gate>]` = present in 0.5.1 but off until the user turns it on; ported with the same gate.
- `[fix]` = 0.5.1 bug, 1.0 does what the line says (see "Deliberate differences" §B for the old behaviour).
- `[win]` = Windows-specific; `[mac]` = macOS-specific.
- "home" = the user's home directory (`$HOME`, Windows `%USERPROFILE%`). "data dir" = `~/.anysola/` (layout in
  "Deliberate differences" §A2).
- Version strings: `<ver>` = the Rust release version (`1.0.0`).

---

## Deliberate differences from 0.5.1

### A. Decisions (made; recorded here, not reopened)

1. **One process.** No background `serve --service`, no service registration file, no port 49374/28894, no
   `service`/`serve` commands, no `service set remote` / OpenTunnel. The TUI, `run`, account commands and ACP each run
   the agent in-process; several processes coordinate through SQLite (WAL) and a per-session busy marker
   (ARCHITECTURE.md "Process model"). Consequences, all accepted:
   - `--standalone` is accepted everywhere 0.5.1 accepted it and does nothing (the code-action passes it).
     `--server <url>` and `OPENCODE_PASSWORD`/`OPENCODE_SERVER_PASSWORD` are accepted and ignored with one stderr
     warning line `--server is not supported since Anysola Code 1.0; running locally.`
   - `anysola reload` stays as a no-op that prints `Configuration reloaded` (scripts keep working); the TUI `/reload`
     reloads config in-process.
   - Texts that told users to `run \`anysola service restart\`` lose that sentence (account/budget errors, run errors).
   - The "Starting background server..." line, the update-preflight splash, the "Restarting service…"/"Connection
     lost…" reconnect overlay and `/restart` are gone.
   - Persistent terminals (`<leader>t` pane) are in-process PTYs: they live as long as the TUI process (0.5.1's PTY
     daemon outlived it).
   - Background shells and background subagents live as long as the process that started them (as `run` already
     behaved in 0.5.1).
   - A session currently driven by another process cannot be prompted: `run -s <id>` exits 1 with
     `Session <id> is busy in another Anysola Code process.`; the TUI shows the session read-only with that text.
   - `[cond: experimental.schedules]` schedules fire only while some `anysola` process is running.
2. **Data in `~/.anysola/`** (Windows `%USERPROFILE%\.anysola`), not the shared `opencode` XDG dirs; layout in §A2.
   One-time automatic import from 0.5.x on first start (milestone M3). Old files are never modified or deleted.
   Project `opencode.json[c]` / `.opencode/` and global `~/.config/opencode/` keep being read; the new global
   `~/.anysola/config.json` has precedence over the old global config.
3. **Upward discovery stops at home.** One rule for every upward walk (project `opencode.json[c]`, `.opencode/`
   directories, `AGENTS.md`, `.claude/skills`, `.agents/skills`, `.opencode/themes`): start at the session directory,
   walk to the parent repeatedly, include each directory, and stop after home (inclusive) when the start is inside home;
   when the start is outside home, stop after the project root (the VCS root found by project identification, or the
   start directory itself when there is no VCS). Never read `/` or other ancestors above that bound. Home-level
   compatibility dirs are still read explicitly regardless of the walk: `~/.claude/skills`, `~/.agents/skills`
   (unless `OPENCODE_DISABLE_CLAUDE_CODE` / `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS`), `~/.config/opencode/` (all its
   sources), `~/.anysola/` (all its sources). Rationale: 0.5.1's AGENTS.md walk already uses exactly this bound
   (`spec/session.md §4.2`); config and skill walks went to `/` (`spec/storage.md §3.1` item 6, `spec/session.md §7.1`).
   With a real home the inclusive bound reads the same `~/.claude/skills` 0.5.1 reads; the observed "leak" was an
   isolated-HOME test artifact (`spec/storage.md §1.2`).
4. **No third-party network calls.** No models.dev / models.opencode.ai catalog fetch (Anysola models come only from
   `/v1/code/config`), no tree-sitter grammar downloads (all highlighting built in), no npm plugin installs, no
   OpenTunnel, no ripgrep download (ripgrep crates linked; `rg` on PATH is not used), no OTLP export. Remaining outbound
   traffic: the Anysola gateway, `www.anysola.com` install scripts (update), npm (only for `anysola update` of npm
   installs), user-configured MCP servers and `instructions`/`skills` URLs, `webfetch` targets, git remotes for
   `references`.
5. **No JS plugin host.** `plugin`/`plugins` config keys are accepted and ignored with one warning per process
   (`Plugins are not supported since Anysola Code 1.0; ignoring "plugins".`). The four Anysola built-in plugins
   (`anysola.provider`, `anysola.provider-lock`, `anysola.account`, `anysola.websearch`) and the upstream built-ins
   0.5.1 exercises become native code (items in M1/M4). Code Mode (`execute`) stays, on embedded QuickJS (rquickjs).
   Native replacements (from `spec/integrations.md §6.2`), with the section that carries each: `anysola.provider`
   (remote config, provider pin, device + env credentials: M1.4, M2), `anysola.provider-lock` (M1.4, M1.13),
   `anysola.account` (credits, budgets, 402 rewrite, account calls: M1.3, M1.4, M4.6), `anysola.websearch` (M1.8);
   prompt identity block and anthropic prompt append (M1.5); tool context hooks: shell OS clause, subagent list,
   websearch year, `opencode` namespace and worktree hint (M1.8); `tool.input.repair` `bash`→`shell` (M1.6); plan
   reminders and plan edit guard (M1.7); goal (M1.10); warming (M1.10); title (M1.11); config.policy (M1.9);
   config agents/commands/skills/mcp/reference/websearch transforms (M1.7, M1.10, M1.13, M5); compatibility skill dirs
   (M1.10); mcp.codemode.defaults (M5); vcs git/hg (M1.12); TUI notifications (M4.7); mermaid and LaTeX renderers (M4.2).
6. **Bugs fixed, not replicated** — §B.
7. **Identity on the wire** (gateway unchanged):
   - `POST /v1/code/messages`: `User-Agent: opencode/latest/2.2.5+anysola.<ver>/cli` (`/acp` instead of `/cli` under
     `anysola acp`, as 0.5.1 with `OPENCODE_CLIENT=acp`), `x-anysola-client: opencode`, `x-anysola-version: <ver>`,
     `x-anysola-credits-event: 1`, `x-opencode-client: cli` (`acp` under ACP; the TUI also sends `cli`).
   - Every other HTTP request the binary makes (account calls, `/config`, `/version`, `/device/*`, `/search`,
     `webfetch` except its own UA, MCP HTTP, `instructions`/`skills` URLs): `User-Agent: opencode/latest/2.2.5/cli`.
   - `/search` additionally `x-anysola-client: opencode`; account calls and `/search` carry `x-anysola-version: <ver>`;
     `/version` carries neither (as 0.5.1, `spec/protocol.md §1.0`).
   - `POST /device/code` body `client: "anysola-code/<ver>"`; MCP `initialize` clientInfo
     `{"name":"opencode","version":"2.2.5+anysola.<ver>"}`; stored session `version` = `2.2.5+anysola.<ver>`.
8. **Release assets.** Same 12 archive names + `install.sh`, `install.ps1`, `SHA256SUMS` (`spec/cli.md §4.1`). Six
   real builds: `x86_64-unknown-linux-musl` (static), `aarch64-unknown-linux-musl` (static), `aarch64-apple-darwin`,
   `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`. x64 builds target baseline x86-64 (no
   AVX2 requirement). Byte-identical duplicates: `linux-x64` = `linux-x64-baseline` = `linux-x64-musl` =
   `linux-x64-baseline-musl`; `linux-arm64` = `linux-arm64-musl`; `darwin-x64` = `darwin-x64-baseline`;
   `windows-x64` = `windows-x64-baseline`. Static musl is required so the glibc-named and musl-named archives can be the
   same bytes (the install script picks `-musl` on Alpine).
9. **ACP is in 1.0, implemented last (M7).**

### A2. Data layout and environment

- `~/.anysola/bin/anysola[.exe]` (unchanged install location; `ANYSOLA_INSTALL_DIR` still overrides).
- `~/.anysola/anysola.db` (+`-wal`, `-shm`), mode 0600 on POSIX: sessions, messages, KV, credentials, saved
  permissions, projects, instruction state.
- `~/.anysola/config.json` (JSONC syntax accepted): global core config, same schema as `opencode.json`
  (`V/storage/config.schema.json`). `~/.anysola/` is also a global directory source (`agent(s)/`, `command(s)/`,
  `mode(s)/`, `skill(s)/`, `themes/`, `AGENTS.md`), loaded after `~/.config/opencode/`.
- `~/.anysola/tui.json`: TUI settings, schema of 0.5.1 `cli.json` (`V/storage/cli.schema.json`); written temp+rename,
  0600.
- `~/.anysola/state/` (prompt history, stash, model recents/variants, TUI storage such as tabs/layout, locks),
  `~/.anysola/cache/` (skill index cache), `~/.anysola/log/anysola.log`, `~/.anysola/snapshot/`,
  `~/.anysola/tool-output/`, `~/.anysola/shell/`, `~/.anysola/repos/`, `~/.anysola/worktree/`.
- Unchanged on purpose (model-visible or user files): plan dir `<home>/.opencode/plan`, temp dir `<os tmp>/opencode`
  named in the system prompt, project-id cache file `<git common dir>/opencode`.
- `XDG_*`, `OPENCODE_CONFIG_DIR`, `OPENCODE_DB` and `OPENCODE_DISABLE_CHANNEL_DB` are used only to locate the 0.5.x
  files to import and the old global config dir to read; they never move `~/.anysola/`.
- Env vars still honoured: `ANYSOLA_TOKEN`, `ANYSOLA_API_URL`, `ANYSOLA_INSTALL_DIR`, `ANYSOLA_VERSION` and
  `ANYSOLA_DOWNLOAD_URL` (scripts), `OPENCODE_CONFIG`, `OPENCODE_CONFIG_CONTENT`, `OPENCODE_CONFIG_DIR`,
  `OPENCODE_DISABLE_PROJECT_CONFIG` / `OPENCODE_CONFIG_PROJECT_DISABLE`, `OPENCODE_DISABLE_CLAUDE_CODE`,
  `OPENCODE_DISABLE_CLAUDE_CODE_SKILLS`, `OPENCODE_CLI_CONFIG_CONTENT` (overlays `tui.json`),
  `OPENCODE_DISABLE_FILEWATCHER` / `OPENCODE_FILEWATCHER_DISABLE`, `OPENCODE_GIT_BASH_PATH`, `OPENCODE_LOG_LEVEL`,
  `OPENCODE_PRINT_LOGS`, `OPENCODE_CLIENT`, `OPENCODE_EDITOR_SSE_PORT`, `CLAUDE_CODE_SSE_PORT`, `OPENCODE_ZED_DB`,
  `NO_PROXY`/`no_proxy`, `VISUAL`/`EDITOR`, `SHELL` — `spec/cli.md §1.6`, `spec/storage.md §1.3`.

### B. 0.5.1 bugs and what 1.0 does instead

Code-action check (`spec/cli.md §3`): the action passes the prompt on stdin with no message argument, reads only
`--format json` stdout, the last `error` event, and greps the strings in §3.6. None of the fixes below changes any
of those; where a fix touches them it says so.

1. `run` wraps every argv element containing a space in literal double quotes (`spec/cli.md §2.1`,
   `spec/storage.md §2.5`). 1.0: message = argv elements joined by single spaces, unquoted. Action unaffected (stdin).
2. Arguments after `--` are appended twice. 1.0: they are appended once, after the arguments before `--`.
3. `run` hangs forever when stdin is a pipe that never closes. 1.0 rule: stdin is read only when it is not a TTY.
   With no message argument, stdin is read to EOF with no time limit (the action's path; `You must provide a message`
   if it is empty). With a message argument: a regular file or `/dev/null` is read immediately; a pipe/FIFO/socket is
   read to EOF if its first byte (or EOF) arrives within 3 s, otherwise it is ignored and stderr gets
   `No input on stdin after 3s; continuing without it.`; the result is merged as in 0.5.1
   (`message + "\n" + piped`).
4. Free tier: client reads `freeQuota.usedTurns` but `/me` sends `turns`, so "used" is always 0
   (`spec/protocol.md §1.4`). 1.0 reads `turns`, falling back to `usedTurns` (status line, `/credits`, `anysola status`).
5. 402 rewrite appends `\nUpgrade: https://account.anysola.com/billing` even when the gateway message already contains
   the billing URL (printed twice). 1.0 appends it only when the message does not already contain
   `account.anysola.com/billing`. The action's grep strings (`used this cycle`, `Daily free limit`, `may cost up to`,
   `Anysola credits`) are in the gateway text and stay.
6. `anysola devices revoke <unknown id>` prints `This Anysola server does not offer this yet.` (gateway 404 "No such
   token."). 1.0 prints `No signed-in device with ID <id>. Run \`anysola devices\` to list them.` (exit 1); the TUI
   `/devices` revoke uses the same text. The generic 404 text stays for the other account routes.
7. `anysola status` reads fields `/me` never sends (`email`, `user.email`, `plan.name`, numeric `credits`, `freeTier`,
   `turnsUsed`, `turnsLimit`, `defaultModel`, `model`) — `spec/protocol.md §1.4`. 1.0 prints from the real `/me`
   shape, keys padded to the longest key + 2: `Plan <Plan>` (first letter upper), `Credits <n> available, renews
   <YYYY-MM-DD>` (paid), `Free quota <turns>/<dailyTurns> turns today` (free), `Today <requests> requests · <credits>
   credits`, `Event <label>[ until <formatDate>]` (when set), `Token <mask>`. The captured `Account <email>` line
   (`V/cli/account-commands.txt`) disappears; nothing automated parses it.
8. `--version` in any other argv position (`anysola run --version`, `-v run`) prints `anysola v2.2.5`. 1.0 prints the
   exact version line (`Anysola Code <ver> (OpenCode 2.2.5)`) for every `--version`/`-v`, exit 0.
9. TUI `/debug` reports `Version 2.2.5 (latest)`. 1.0 reports `Version <ver> (latest)`.
10. Terminal title `OC | <title>`. 1.0: `Anysola | <title>` (same 40/37-char cut); plugin-route titles are gone.
11. Settings shows Animations `off` while animations run (declared default false, runtime default true). 1.0: the
    Settings row default is `on`, matching runtime.
12. `<leader>q` is bound to both `app.exit` and `session.queued_prompts`. 1.0: `<leader>q` = exit only (the shown hint
    stays `ctrl+c, ctrl+d, ctrl+x q`); `session.queued_prompts` has no default key (the palette entry and a click on the
    queued dock still open it; users can bind it).
13. Stored login + `ANYSOLA_TOKEN` both present: 0.5.1 used the stored login for `/config`/`/me` and may throw on model
    calls (apiKey + authToken collision, `spec/protocol.md §5`). 1.0: `ANYSOLA_TOKEN` wins everywhere. Model calls send
    it as `x-api-key` only (no `Authorization`); account calls, `/config`, `/search` send it as
    `Authorization: Bearer` (as 0.5.1 does for the env token on those routes, `spec/protocol.md §1.0`). `anysola logout`
    with both: revokes and removes the stored login, then prints
    `ANYSOLA_TOKEN is still set and stays in use; unset it to sign out.` `anysola login` with `ANYSOLA_TOKEN` set prints
    `Signed in through ANYSOLA_TOKEN; unset it to use a device login.` and exits 0 unless `--force`.
14. Text-mode `run` prints `\x1b[0m\n` only once per process (`UI.empty` flag never reset). 1.0 prints one blank line
    before and after each block (block tools, framed text on a TTY), as the code intends. stderr only.
15. Text-mode `run` dumps an unhandled ECONNRESET stack when the session is already at its budget. 1.0 prints only the
    budget error (exit 1).
16. `anysola <unknown word>` dies with an uncaught `chdir` stack. 1.0: `Error: No such directory: <word>` on stderr,
    exit 1.
17. Plain CLI handler errors print `Error: <msg>` followed by Bun stack lines (e.g. `mcp add`). 1.0 prints only
    `Error: <msg>`, exit 1.
18. `anysola mcp list` races server activation and prints `No MCP servers configured` on a cold start. 1.0 lists from
    config and waits up to the startup timeout for each server's status.
19. `remaining` credits after a turn come from whichever concurrent request finished last (title vs main,
    `spec/protocol.md §2.3`). 1.0: within one turn, the lowest `max(0, before - charged)` reported wins; a new turn
    starts fresh.
20. Without a TTY the TUI writes escape sequences to stdout and never exits (`spec/cli.md §1.1`). 1.0: `anysola` with
    stdin or stdout not a TTY prints
    `Error: The interactive UI needs a terminal. Use \`anysola run\` for non-interactive use.` and exits 1.
21. Account/budget error texts suggest `anysola service restart` (`spec/tui.md §5`, `V/cli/run-errors.txt`). 1.0 drops
    that sentence (no service).

Kept on purpose although odd (model-visible; changing them changes model behaviour):
- shell truncation notice line numbers computed from total newlines (wrong when the byte cut is mid-line) —
  `spec/tools.md §shell: truncation`;
- grep keeps rg's trailing newline in `Line N:` text (extra blank lines) — `spec/tools.md §grep`;
- edit asks the `edit` permission before match failures are known — `spec/tools.md §edit`;
- `run` sends `/cmd` and `@path` as plain text — `spec/session.md §7.2`, `§8`;
- formatter `htmlbeautifier` never matches `.html.erb` — `spec/integrations.md §4`;
- `--version` keeps the `(OpenCode 2.2.5)` suffix (see Open questions).

---

## M1 — Core and headless `run`

Note: the `execute` tool (Code Mode runtime on QuickJS, the `opencode` namespace and the system-block-2 catalog) is in
M1, not M5, because every captured default request advertises `execute` and the catalog
(`V/tools/_captured-request-build-sonnet.json`); M1's request replay cannot pass without it. M5 adds MCP to Code Mode.

### M1.1 `run` command line
- [x] `anysola run [message...]` with flags `-c/--continue`, `-s/--session`, `--fork`, `-m/--model`, `--agent`, `--title`, `--thinking`, `--format default|json`, `-f/--file` (≤100), `--budget <int ≥1>`, `--auto` (+ hidden `--yolo`, `--dangerously-skip-permissions`), `--standalone` (no-op), `--print-logs`, `--log-level`; help text per `V/cli/help/root_run.txt` — spec/cli.md §2.1 (help text verbatim without the `--wizard` line, which is not carried over)
- [x] [fix] message = argv joined by single spaces, no quoting; `--` operands appended once — §B1, §B2; spec/cli.md §2.1
- [x] [fix] stdin rule of §B3 (no message → read to EOF; message + idle pipe → 3 s first-byte wait); empty input → `You must provide a message` — spec/cli.md §2.1
- [x] `-c` = latest top-level session whose directory equals cwd, else new; `-s <id>` resumes, unknown id → new session created with exactly that id — spec/cli.md §2.1
- [x] `--fork` requires `-c`/`-s` (`--fork requires --continue or --session`); `-s unknown --fork` → `Session not found`; forked title `<title> (fork #N)` — spec/cli.md §2.1, spec/storage.md §2.3
- [x] Model refs `provider/model#variant`; errors `Invalid model reference: …`, `Model unavailable: anysola/<id>`, `Variant unavailable for anysola/<m>: <v>`, `Cannot select a variant before selecting a model`, `No model is available for session <id>` (type `provider.no-route`) — V/cli/run-errors.txt
- [x] `--agent` accepts any existing agent incl. subagent-mode ones; unknown → `Agent not found: "<x>"` — spec/session.md §3
- [x] `--title <t>` for new sessions only; `--title ""` → first 50 chars of the message + `...` — spec/cli.md §2.1
- [x] `-f`: path resolved against cwd; missing → `File not found: <p>`; directory/special/>10 MiB → `Cannot attach a directory, special file, or file larger than 10 MiB: <p>`; UTF-8 text inlined as `\n\n<file name="<basename>">\n<content>\n</file>`; images/PDF/binary as inline attachments (no `Attached file:` line) — spec/cli.md §2.1, spec/session.md §8, V/session/requests/run-file-attachments.json
- [x] `run` does not expand `/commands` or `@path` (sent as text) — spec/session.md §7.2, §8
- [x] Minimum-version gate runs before anything else (3 s timeout, offline = allowed, dev builds skip); message `Anysola Code X is no longer supported (minimum Y).\nUpdate Anysola Code with: anysola update`, exit 1 — spec/protocol.md §1.9, V/cli/run-errors.txt (builds whose version ends in `-dev` count as dev builds; the gate itself is unit-tested)
- [x] Shell tool env = the process environment (minus `OPENCODE_PASSWORD`/`OPENCODE_SERVER_PASSWORD`) — spec/cli.md §2.1, spec/tools.md §shell: execution

### M1.2 `run` output
- [x] Text mode: answer text on stdout (trimmed + EOL) when stdout is not a TTY; on a TTY to stderr framed by blank lines — spec/cli.md §2.2
- [x] `--thinking` reasoning `Thinking: <text>` (stdout non-TTY; stderr dim italic on TTY) — spec/cli.md §2.2
- [x] stderr step header `> <agent> · <model id>` (repeated on each retry), tool lines with icons/titles from the shared tool formatter (`$`, `→ Read`, `← Write/Edit` + diff, `✱ Glob/Grep`, `% WebFetch`, `◈ Web Search via Anysola`, `→ Skill`, `# Todos`, `→ Asked N question(s)`, subagent `✓/✗/•`, `⚙` fallback), failed tool `✗ <title> failed` + `Error: <msg>`, errors `\x1b[91m\x1b[1mError: \x1b[0m<msg>` (leading `Error: ` stripped first) — spec/cli.md §2.2, V/cli/run-errors.txt, V/cli/run/text-*.stderr
- [x] [fix] blank-line framing per §B14 — spec/cli.md §2.2
- [x] Spend line `\x1b[90mAnysola · <spent> credit[s] · <remaining> left\x1b[0m` on stderr after the run (also after errors), only text mode and spent > 0; `· <remaining> left` omitted when unknown; no thousands separator — spec/cli.md §2.2, spec/protocol.md §4.4
- [x] JSON mode: stdout strictly one JSON object per line `{type,timestamp,sessionID,…}` with types `step_start`, `text`, `reasoning` (only with `--thinking`), `tool_use`, `step_finish`, `error`; field shapes per spec; nothing else on stdout — spec/cli.md §2.3, V/cli/run/json-*.stdout (`snapshot` omitted until snapshots exist, M1.11)
- [x] JSON part ids: live `prt_<event id tail>`, reconciled `prt_<msg tail>_<step-finish|text-N|reasoning-N|tool-<id>>`; text emitted once per finished text part — spec/cli.md §2.3 (the last step's `step_finish` takes the reconciled form; other parts keep their live ids, no reconcile pass)
- [x] JSON errors: server errors keep their shape (`provider.quota` with `status`, `response.body`; `aborted`); CLI errors `{type:"unknown",message}` with `sessionID:""` before a session exists — spec/cli.md §2.3, V/cli/run-errors.txt
- [x] Exit codes: 0 finished (incl. auto-rejected permissions), 1 any error event/bad flags/budget/minimum version/no message, 130 SIGINT (first SIGINT interrupts and sets 130, second exits 130 at once) — spec/cli.md §1.5, §2.6
- [x] SIGTERM/SIGINT teardown deadline 10 s then `shutdown did not finish within 10s; exiting. Still pending:` + list, exit 1; SIGHUP = SIGTERM only with a TTY — spec/cli.md §1.5 (pending entries are running turns `session <id> running` and shell commands `child <pid> running: <argv>`; an empty list reads `  nothing tracked`; a second SIGTERM reports `received SIGTERM again during shutdown` and exits 1, a second SIGINT exits 130; a teardown that finishes exits 1 after SIGTERM, where 0.5.1 crashed with an ECONNRESET dump and 130)

### M1.3 Non-interactive permissions, questions, budget
- [x] Without `--auto`: each ask of the run's session or its children → stderr `\x1b[93m\x1b[1m! \x1b[0mpermission requested: <action> (<resources>); auto-rejecting` (also in JSON mode), reply reject with the verbatim feedback; model continues; exit 0 — spec/tools.md §anysola run (non-interactive), V/cli/run-errors.txt
- [x] `--auto`: every ask answered `once` silently; deny rules still deny and denied tools are not advertised — spec/cli.md §2.4
- [x] Question tool cancelled with the "cannot ask the user questions" feedback — spec/cli.md §2.4
- [x] Other forms (MCP elicitation) cancelled and exit code set to 1 — spec/cli.md §2.4
- [x] `--budget N`: sets the session budget (persisted); if total ≥ N already → budget error before any prompt; else interrupts at the next step start once total ≥ N (may cut a running tool: `Tool execution interrupted`, `Step interrupted`); final error always `Budget reached: <total>/<N> credits. Continue with \`anysola run -s <id> --budget <credits>\`.` as the last `error` event, exit 1 — spec/cli.md §2.5, spec/protocol.md §4.3 (checked at each step start before the request, so a running tool is never cut; the 0.5.1 capture's interrupted web search was a race)
- [x] [fix] budget already reached in text mode prints only the error — §B15
- [x] `Cannot set the budget (<error>).` (without the service hint) — V/cli/run-errors.txt, §B21
- [x] `run` does not wait for background shells/subagents; they are killed when the run ends — spec/tools.md §shell: background

### M1.4 Gateway client
- [x] Base URL = `ANYSOLA_API_URL` (trailing `/` stripped) or `https://api.anysola.com`; all routes under `/v1/code` — spec/protocol.md §6
- [x] Header sets A (account), B (messages), C (version), D (search) exactly as specified, with the identity strings of decision 7; 15 s account timeout; lenient JSON (non-JSON → `{}`) — spec/protocol.md §1.0, §1.7, §6
- [x] `GET /config` at start, on credential change and every 10 min while connected; raw body cached in KV `anysola:config`; offline/failure → cache; 401 → needs-auth state (`Run \`anysola login\` to sign in to Anysola`) and cache — spec/protocol.md §1.3 (fetched only with a credential, as in 0.5.1; the KV value is the parsed JSON object; 1.0 has no in-process login yet, so a credential change is picked up by the next start or the 10-minute refresh)
- [x] Remote config handling: keep only `providers.anysola`, `model` only if on anysola, `agents` as is, `small_model` → `agents.title.model`; provider pinned (name `Anysola`, baseURL `<base>/v1/code` overriding the server's); headers merged; provider lock removes every other provider and restores endpoint keys — spec/protocol.md §1.3, spec/storage.md §3.3, spec/integrations.md §6.1
- [x] Model `description` → picker note (trimmed, ≤160 chars); `credits_per_turn_estimate` read when present (`~N credit(s)/turn`) — spec/protocol.md §1.3, §4.7 (on `Model`; the picker itself is M4)
- [x] `GET /version` (no auth, 5 s; 3 s for the gate), leading `v` stripped, semver compare with prerelease before release — spec/protocol.md §1.9
- [x] Credentials: device-login credential sent as `Authorization: Bearer`; `ANYSOLA_TOKEN` per §B13 (x-api-key on `/messages` only); `ANYSOLA_TOKEN` works with no stored row — spec/protocol.md §5, spec/storage.md §2.9
- [x] SSE parser: Anthropic events, `: keepalive` comments ignored, `event: error` = stream error, `event: anysola_credits` block stripped and its `charged` parsed (filter installed only for ok `text/event-stream` responses without `x-credits-*` headers) — spec/protocol.md §2.2, V/protocol/captures/sse-gateway-examples.txt
- [x] Credits accounting: per-session `{remaining, turn, total, turns, open, budget}`; header path (`x-credits-charged`/`x-credits-remaining`), stream path (`max(0, x-credits-available-before − charged)`); turn closes on idle (`turns += 1` when turn > 0); persisted to KV `anysola:session:<id>` = `{total,turns,budget}`; fresh session budget = KV `anysola:budget:default` — spec/protocol.md §2.3, spec/storage.md §2.8
- [x] [fix] `remaining` per §B19 — spec/protocol.md §2.3
- [x] Search credits added to the searching session's turn/total (FIFO per query, map capped at 64), never update `remaining` — spec/protocol.md §1.8 (recorded when the search returns, no per-query FIFO needed in-process)
- [x] Error envelope parsing and classification (overflow phrases, 413/payload, content policy, 402/quota text → QuotaExceeded, 401/403 → Authentication, 429 → RateLimit, 408/409/≥500 → ProviderInternal, other 4xx → InvalidRequest), `x-should-retry` overrides — spec/session.md §2.5, spec/protocol.md §2.4, §2.5
- [x] [fix] 402 rewrite `<message or 'Out of Anysola credits.'>\nUpgrade: https://account.anysola.com/billing` with the §B5 dedupe — spec/protocol.md §6 item 10
- [x] Retry schedule: exponential from 2 s, gap cap 10 s, ≤10 retries, jitter, `max(delay, retry-after ≤ 15 min)`, ≤3 transport-timeout retries; never 400/401/402/403/404/413; one immediate full resend for `retry-full` transport errors; each retry publishes a retry event — spec/session.md §2.5, spec/protocol.md §2.5 (a transport failure before any output is retried on the same schedule; there is no separate immediate full resend)
- [x] One-shot generations (title, goal judge, `/btw`, keep-alive) retry only "not run" failures (429, 503/529, unsent transport), ≤3 — spec/session.md §2.5
- [x] Only allowlisted betas; never `compact-*`/`fast-mode-*`; only allowed top-level body keys; no `x-app`, no `claude-cli/` UA — spec/protocol.md §3, §3.1
- [x] `POST /search` `{query}`: Bearer, 30 s timeout (`Anysola web search timed out`), 1 MiB cap, `Cannot reach <api>`, 401/402/429/other texts, decode failure text — spec/protocol.md §1.8, V/protocol/user-strings.txt "Web search"

### M1.5 Request construction
- [x] `POST /v1/code/messages`, `stream:true`, headers `anthropic-version: 2023-06-01`, betas in the order `interleaved-thinking-2025-05-14`, `thinking-binding-controls-2026-08-01` (when `block_binding`), `mid-conversation-output-config-2026-07-01` (when an effort marker exists); session headers `x-opencode-session-id`, `x-session-affinity`, `x-session-id`, `x-opencode-session`, `x-opencode-project` (`global` outside a project), `x-opencode-client`, parent headers for child sessions; `x-anysola-variant` from variant headers — spec/session.md §1.1, spec/protocol.md §1.7
- [x] Body keys in order `model, system, messages, tools, [tool_choice], stream, max_tokens, thinking, [output_config]`; never temperature/top_p/top_k/stop_sequences/metadata/service_tier/speed/container/context_management — spec/session.md §1.2
- [x] `max_tokens`: primary = `min(limit.output ?? 32000, 256000)` reduced near a full window (`max(1024, floor(context*0.99) − measured − ceil(estimated*1.15))`); title 32000; compaction per §5 — spec/session.md §1.2
- [x] System blocks: [agent prompt (`system.txt` with tool guidance for advertised tools, + `\n\n` + `anthropic.txt` unless the agent has its own system), identity `# Your Model…`, joined instructions, worktree hint]; empty blocks dropped; block 2 frozen per session — spec/session.md §1.3, V/session/system-block*.wire.txt, V/session/system.txt.template, V/session/anthropic.txt (block 2 is a list of keyed sources; the Code Mode catalog, MCP, reference and skill sources plug in there)
- [x] Thinking: `{type:"adaptive", block_binding:{prefix_mismatch_behavior:"drop_block"}}` for Claude ≥5.1 ids and for models with `compatibility.supportsThinkingBlockBinding`; `display:"summarized"` added with an effort variant or `#economy` — spec/session.md §1.4
- [x] Effort: no `output_config` by default; `#low…#max` → `output_config.effort`; `#economy` → `effort:"low"` + `x-anysola-variant: economy`; effort variants generated from `capabilities.reasoning.effort` — spec/session.md §1.5, §9
- [x] Mid-session effort switch on the same model → `{role:"system",content:[],output_config:{effort}}` marker + beta, top-level effort stays the session's first; only for Claude Opus ≥5 / Claude ≥5.1 ids or `supportsEffortUpdates`; economy switches emit no marker — spec/session.md §1.5
- [x] Cache control: 4 ephemeral breakpoints (last tool, first system block, last system block, tail message's last cacheable part; effort markers never the tail); title request 2 marks; cap 4 — spec/session.md §1.6
- [x] History → messages: user (skill texts, text, attachments), assistant (text, thinking with signature / redacted_thinking, tool_use with id sanitized), tool results in the next user turn (merged), single text → string, multi-part → array, errors → JSON string with `is_error:true` — spec/session.md §1.7, spec/storage.md §2.7 (skill texts arrive with skills, M1.10)
- [x] Reasoning dropped when the assistant message errored or reasoning lacks `time.completed`; on a different model sent without signature (then dropped by Anthropic lowering) — spec/session.md §1.4, spec/storage.md §2.7
- [x] Synthetic messages → user text; `location-switched` → `The working directory has been changed to <dir>.`; user shell → `The following shell command was executed by the user:…`; `agent-switched`/`idle` → nothing — spec/session.md §1.7
- [x] Instruction updates: native `{role:"system"}` after the latest user turn for Claude 5.x ids, `<system-update>…</system-update>` user text (escaped `& < >`) for `auto`; an update splitting a tool call from its result is a request error — spec/session.md §1.7, V/session/requests/instruction-update-*.json (the split case cannot arise: tool results are stored with their assistant message, so no update can fall between them)
- [x] Media: images as base64 `image` blocks, PDFs as `document`; unsupported modality → `ERROR: Cannot read "<name>" (this model does not support <modality> input). Inform the user.`; image budget 25 MiB → drop oldest to ≤15 MiB with the removed-image text — spec/session.md §1.7, spec/tools.md §Images and PDFs as tool results
- [x] Image normalization (prompts and tool results): max 2000×2000, 5 MiB base64, PNG passthrough rules, JPEG q80/70/55/40, 0.75 shrink steps, Lanczos3, transparency keeps PNG, decode/resize failure notes; config `media.image.*` — spec/tools.md §Images and PDFs as tool results (exact encoder bytes differ from photon; dimensions, formats and limits follow the rules)
- [x] `auto` sent as `model:"auto"`, identity `Name: Auto`, replay keeps ref `anysola/auto` — spec/session.md §9

### M1.6 Agent loop
- [x] Turn = steps; continue while tools ran or a recoverable stream issue occurred; inbox promotion on the first step; step resets to 1 on newly promoted input — spec/session.md §2.1, §2.4
- [x] Before each step: config/agent/tools/instructions re-selected, instruction deltas committed, auto-compaction check, then the request — spec/session.md §2.1 (config sources reload at the next step after `Core::reload`, which `/reload` and the file watcher call)
- [x] Drain start recovery: stale running compactions → `compaction.interrupted`, stale tool calls → `Tool execution interrupted: <tool>[ (sessionID: <child>)]` — spec/session.md §2.1, V/session/source-strings/stale-tool-texts.ts.txt
- [x] Tools start when each `tool_use` block completes, run concurrently while the stream continues; results kept in call order — spec/session.md §2.2
- [x] Unknown tool → `No tool named "<name>" is currently available. Please use a tool from the available tool list.`; hidden by context → `Tool is not available for this request: <name>`; `bash` repaired to `shell` with the extra note — spec/tools.md §Tool sets per agent, spec/session.md §2.2 (`Tool is not available for this request` needs context hooks that remove tools, M1b)
- [x] Invalid arguments text `Invalid arguments for tool "<name>":\n- <path>: <issue>` (≤5 issues) + arguments JSON + retry line, with Effect-Schema issue wording — spec/tools.md §Shared, §Rust implementation notes
- [x] Interrupt: running tools interrupted, unsettled tools `{type:"aborted", message:"Tool execution interrupted"}`, assistant `Step interrupted`; nothing recorded if cancelled before the start snapshot — spec/session.md §2.3
- [x] Decline without message ends the step (`The user declined this tool call`), rejects other pending asks; reject with message continues — spec/tools.md §Asking, replies and persistence
- [x] `steps: N` agents: N-th step adds the `MAX_STEPS_PROMPT` assistant prefill and `tool_choice:{type:"none"}`; tool calls then fail `Tools are disabled after the maximum agent steps` — spec/session.md §2.4, V/session/requests/max-steps-final-step.json
- [x] Incomplete stream after output → keep partial, add `CONTINUE_AFTER_INCOMPLETE_STREAM`; `max_tokens` stop → `CONTINUE_AFTER_OUTPUT_LIMIT`, error `Response still truncated after two output token limit continuations` after three — spec/session.md §2.5, V/session/source-strings/continuation-texts.ts.txt
- [x] Context overflow before output → one overflow compaction per step then re-run; second overflow surfaces — spec/session.md §2.7
- [x] Provider tool call without result → `tool.result-missing` / `Provider did not return a tool result` — spec/session.md §2.2 (with the `tool.input-json` / `tool.input-incomplete` texts for client calls whose arguments were invalid or cut off)
- [x] Step error texts verbatim — V/session/source-strings/step-error-texts.ts.txt
- [x] Inbox delivery `steer` (default, promoted at the next step boundary) and `queue` (promoted at idle, one at a time); pending items switchable/cancellable — spec/session.md §6.6

### M1.7 Agents
- [x] Built-ins build, plan, general, explore, compaction (hidden), title (hidden) with names, modes, prompts, descriptions and permission sets as specified — spec/session.md §3, V/session/source-strings/agent-build-general-explore-plan-defs.ts.txt, agents-explore-title-summary.ts.txt
- [x] Default agent: `default_agent` if selectable, else `build`, else first selectable — spec/session.md §3
- [x] `agent.model` overrides the session model; tools filtered by merged permissions — spec/session.md §3
- [x] Config agents `agents.<id>.{model,request,system,description,mode,hidden,color,steps,disabled,permissions}`; `disabled` removes built-ins too — spec/session.md §3.1
- [x] Markdown agents from `{agent,agents}/**/*.md` and `{mode,modes}/*.md` in every config directory source; nested names `a/b`; V2 vs V1 frontmatter detection and migration (colour names → hex, `model`+`variant` joined, unquoted-colon YAML retry) — spec/session.md §3.1 (a V1 agent's `options`/`temperature`/`top_p` and colours are parsed but not applied: agent `request` options and colours have no consumer yet)
- [x] Plan agent: enter/leave reminders as persisted synthetic messages, reminder re-added before the last user message per request, plan dir `<home>/.opencode/plan`, edit outside it → `Cannot use <tool> to modify files outside the Plan directory: <dir>` — spec/session.md §3.2, V/session/source-strings/plan-mode-reminders.ts.txt, V/session/requests/primary-plan-agent.json

### M1.8 Tools
Descriptions, schemas and every output/error text: `V/tools/<tool>.md`. Tool JSON emitted exactly as captured.
- [x] Default build tool list and order: edit, glob, grep, question, read, shell, skill, subagent, webfetch, websearch, write, then `execute` with `cache_control`; per-agent sets for plan/general/explore/title — spec/tools.md §Tools sent to the model, §Tool sets per agent (`skill` stays advertised even when an agent's rules end in `* deny`, as observed in 0.5.1 for explore and for config agents. Question, skill, subagent, webfetch, execute are registered with minimal bodies until M1b)
- [x] Error envelope `{"error":{"type":"<type>","message":"<msg>"},"content":[]}` with types `tool.execution`, `permission.rejected`, `unknown` — spec/tools.md §Shared
- [x] Path resolution: `~` expansion, relative to the session directory, internal vs external resources, `external_directory` ask with `<dir>/*` and save `<project root>/*`; [win] `/c/…`, `/cygdrive/c/…`, `/mnt/c/…` → `C:/…`, `\` → `/` in resources — spec/tools.md §Shared
- [x] Per-path mutation lock for edit/write — spec/tools.md §Shared
- [x] read: media magic bytes (PNG/JPEG/GIF/WebP/PDF), 20 MiB media limit, unpaged ≤51200 bytes whole file, paged mode (offset/limit, 2000-line cap, 51200-byte page), line format/header/continuation texts, 2000-char line clamp, NUL = binary, U+FFFD decoding — spec/tools.md §read
- [x] read directories: offset/limit on entries, dirs first with separator, `localeCompare`-compatible ordering, dotfiles shown, no ignore rules; [win] `\` suffix — spec/tools.md §read (ordering approximates `localeCompare`: case-insensitive, then lower case first)
- [x] read filename recovery (NFC, NBSP/U+202F, curly quotes), not-found suggestions and missing-parent text, out-of-range offset texts — spec/tools.md §read (no NFC step; NBSP/U+202F and curly quotes are folded)
- [x] read loads nested `AGENTS.md` files between the read path and the session dir as session instructions (internal reads only) — spec/tools.md §read (paths reported in `metadata.instructions`; the session picks them up with session instructions, M1.10)
- [x] edit: check order, four matching tiers (endings-only, LF view, typographic normalization, line-wise trimEnd), offset mapping back to raw bytes, per-line ending preservation, replaceAll, right-to-left application, miss hints, BOM handling, success text, diff metadata, parallel edits all apply — spec/tools.md §edit
- [x] write: parents created, BOM rule, no line-ending conversion, `Created file successfully:` / `Wrote file successfully:` / `Unable to write <path>` — spec/tools.md §write
- [x] [cond: `formatter` config key] formatters after edit/write with built-in list, detection rules, `$FILE`, first exit-0 wins, cwd = location dir, BOM re-sync, model text unchanged — spec/tools.md §Formatter after edits, spec/integrations.md §4 (oxfmt/prettier/biome take their bin from the project's `node_modules/.bin` up to the worktree; 0.5.1 looked only in its own npm cache, which nothing fills since 1.0 installs no npm packages; the extra bin dir is `~/.anysola/bin`)
- [x] shell: name/schema/description with per-OS clause `Commands run on <OS> using <shell>.`; shell selection (config → `$SHELL` → OS fallback, fish/nu skipped; [win] pwsh → powershell → Git Bash (`OPENCODE_GIT_BASH_PATH`) → cmd); args per shell kind — spec/tools.md §shell: registration and description, §shell: shell selection, V/tools/shell.md
- [x] shell execution: workdir resolution and errors, env additions (`TERM`, `OPENCODE_TERMINAL`, `AGENT`, `OPENCODE`, `AI_AGENT`, `OPENCODE_SESSION_ID`), stdin null, own process group, SIGTERM→3 s→SIGKILL ([win] `taskkill /T /F`), merged stdout+stderr in arrival order, 1 s descendant pipe grace, timeout default 120000 (0 disables), interrupt kills and deletes output — spec/tools.md §shell: execution
- [x] shell output: `(no output)`, notices `Timed out before completion` / `Killed by <SIGNAL>` / `Exited with code <n>`, timeout text, tail-kept truncation (51200 bytes then 2000 lines) with notice, output file `~/.anysola/shell/<projectID>/sh_<id>.out` kept 7 days — spec/tools.md §shell: output, §shell: truncation
- [x] shell background: `background:true` reply text, completion synthetic `<shell …>` message that wakes the session, move-to-background of running foreground shells (core side of ctrl+b) — spec/tools.md §shell: background (a completion enters the inbox as a steer and starts a turn when the session is idle; `Core::background` moves shell commands only; 0.5.1 also moved blocking subagents, which lands with ctrl+b in M4)
- [x] Shell permission scanner: hand-written bash/zsh/posix + PowerShell scanner ported from `shell/scan.ts`/`parse.ts` (not tree-sitter), opaque commands fail `Portable shell scanner cannot analyze command: <reason>` before running, resources = simple commands, save = ARITY prefix + ` *`, static `cd`/workdir checked as external_directory, no ask for empty command lists; [win] cmd scanned as posix, PowerShell path flags and env expansion — spec/tools.md §shell: permissions parsing, §shell: Windows, V/tools/shell.md
- [x] grep: permission order, file-or-dir path, `Search path does not exist`, rg semantics via the ripgrep crates (hidden on, ignore files honoured, `.git` excluded, include glob, fixed strings, ignore case), limit 100 (+1 for truncation), 30 s, 2000-char line cap, output format and quirks, `Invalid regex pattern: <regex error>` — spec/tools.md §grep, V/tools/grep.md (regex error wording follows the regex crate after `Invalid regex pattern: `)
- [x] glob: directory check, rg `--files` semantics incl. re-inclusion of ignored dirs by a positive glob, hidden only on request, limit 100, `No files found` — spec/tools.md §glob, V/tools/glob.md
- [x] Search result order: sorted by path (deterministic; 0.5.1 order was rg's parallel walk order) — spec/tools.md §ripgrep binary
- [x] webfetch: http(s) only, permission + deny re-check on each manual redirect (max 10), headers verbatim, Cloudflare challenge retry with UA `opencode`, timeout 30 s (max 120), 5 MiB body cap, image/non-text rejection texts, HTML → markdown converter matching `html-markdown.ts` output, text/html modes, non-2xx text, generic truncation — spec/tools.md §webfetch, V/tools/webfetch.md (connection failures read `Transport error (GET <url>)` and unparsable URLs `Invalid URL`, as observed from the 0.5.1 binary)
- [x] websearch: only the Anysola provider, permission `websearch` resource = query, `"websearch": false` removes the tool, token required (`Web search needs an Anysola sign-in. Run \`anysola login\`.`), output/no-results/error texts, description year = local process-start year — spec/tools.md §websearch (tool side), V/tools/websearch.md (the no-token text is not covered by a test)
- [x] question: permission, one `Questions` form, answer text template, `Unanswered` — spec/tools.md §question, V/tools/question.md
- [x] skill: lookup by id, `Unable to load skill <id>`, `<skill_content>` output with ≤10 sampled files — spec/tools.md §skill, V/tools/skill.md
- [x] subagent: description with `Available subagents:` list from caller permissions, depth limit `experimental.subagent_depth` (default 1), child session creation, foreground `<subagent …>` result, background mode with later synthetic completion, continuing a direct child via `sessionID`, interrupt cascades — spec/tools.md §subagent, spec/session.md §6.5, V/tools/subagent.md (a background completion is stored for the parent's next turn; waking an idle parent arrives with the TUI. A `sessionID` follow-up to a child still running in this process waits for that run, then runs the prompt)
- [x] [cond: `experimental.todo`] todowrite direct tool, state = latest todowrite part — spec/tools.md §todowrite, spec/storage.md §2.11, V/tools/todowrite.md
- [x] execute (Code Mode) on embedded QuickJS: last tool with `cache_control`, static description, no imports/fs/timers, `fetch`, `search(...)`, `console.log` → `Logs:` section, result `JSON.stringify(value, null, 2)`, runtime errors as plain results with verbatim texts (`Unknown tool '<x>'.\nUse search to find available tools.`), no default timeout but stopped by turn interrupt/budget/SIGINT — spec/tools.md §execute (Code Mode) and the opencode namespace, V/tools/execute.md (engine differences, kept in 0.5.1's message shape: QuickJS accepts classes/BigInt/getters; `x.f()` on a non-function says `The called value is not a function.` without the callee and columns may differ; cycles fail at copy-out as `Converting circular structure to JSON.`; a caught-and-rethrown tool error keeps its uncaught text; `undefined` tool arguments cross as `null`; `tools()` is `not a function`; no `browser` key in `Object.keys(tools)` (dead plugin); no 100 ms progress stream of call rows, the rows arrive with the result; `fetch` sends `User-Agent: opencode/latest/2.2.5/cli` instead of Bun's)
- [x] Code Mode catalog in system block 2 (2000-token budget, 120-char description cut, `(N tools, k shown)`), `opencode` namespace (`list_mcp_resources`, `models`, `read_mcp_resource`, `session_move`, `session_rename`, `worktree_create`, `worktree_list`, `worktree_remove`), catalog update texts; denied `execute` turns namespace tools into direct `opencode_<tool>` — spec/tools.md §execute, V/session/source-strings/codemode-header.ts.txt
- [x] Worktree tools: git worktrees under `~/.anysola/worktree/<projectID[0..6]>/…`, `worktree` config — spec/integrations.md §7 (`worktree`), spec/storage.md §1.2
- [x] Generic output bounding for tools without `truncated` metadata: 2000 lines / 51200 bytes (`tool_output` config), prefix kept, full text in `~/.anysola/tool-output/tool_<id>`, marker block, 7-day sweep — spec/tools.md §Shared output bounding
- [ ] [cond: `experimental.schedules`] `schedule_*` Code Mode tools and scheduled prompts (fire only while an anysola process runs) — spec/tools.md §Gated tools, spec/integrations.md §7, V/session/schedule.txt (not started: off by default and the largest open M1 item — `schedule` table, five tools in the `opencode` namespace, `/schedule` command, `in/at/every/cron` with IANA zones (jiff is in the local cargo registry cache), `ends`, shell and confined Code Mode checks with their caps, and a firing loop under the `schedule:leader` lease)

### M1.9 Permissions
- [x] Rules `{action, resource, effect}`, last match wins, no match = ask; wildcard semantics (`*` incl. `/` and newline, `?`, trailing ` *` optional, `\`→`/`, case-insensitive on Windows only) — spec/tools.md §Rule model and evaluation
- [x] Evaluation: agent rules ++ session rules; deny check first on configured rules; saved project allows appended; per-resource ask/allow; missing agent → deny all; whole-tool hiding when the last matching rule is `* deny` (edit/write/patch share `edit`) — spec/tools.md §Rule model and evaluation
- [x] Defaults per agent (`* allow`, `.env` reads ask, `*.env.example` allow, plus per-agent extras) — spec/tools.md §Defaults
- [x] Config: native `permissions` (top-level appended to every agent, then agent rules), legacy `permission`/`tools` migration with action renames (`write/patch/apply_patch→edit`, `task→subagent`, `bash→shell`), `~`/`$HOME` expansion for read/edit/external_directory — spec/tools.md §Config schema
- [x] Resource strings per tool and save patterns per tool — spec/tools.md §Config schema, §Asking, replies and persistence
- [x] Managed `policies` (`tool.use` deny → `Blocked by configuration policy` / org texts; `provider.use`, `mcp.use`, `skill.use`, `plugin.use`; earlier documents win) — spec/tools.md §Config schema, spec/integrations.md §7 (managed policy) (`integration.use` on `anysola` also withholds the provider and web search; `mcp.use` drops the server from the MCP config; `plugin.use` has nothing to block without a plugin host (§A5))
- [x] Replies once/always/reject(+message); `always` stores save patterns per project in the DB and auto-approves now-allowed pending asks; deny reaches the model as `Permission denied: <action>`; pending asks rejected on shutdown — spec/tools.md §Asking, replies and persistence, spec/storage.md §2.10

### M1.10 Instructions, skills, commands
- [x] Block 2 order: Code Mode catalog, MCP instructions, references, skill guidance, AGENTS.md files, `Today's date: <toDateString>`, `<env>` block (working dir, workspace root, git yes/no, platform, temp dir line), session entries; joined `\n\n` — spec/session.md §4.1, V/session/source-strings/env-and-date-builtins.ts.txt (session entries via `Core::set_instruction_entry`; MCP instructions as source `core/mcp-guidance`)
- [x] Only `AGENTS.md` (no CLAUDE.md): global `~/.config/opencode/AGENTS.md`, then `~/.anysola/AGENTS.md`, then project files nearest first within the decision-3 bound, then configured `instructions` (URLs fetched with 5 s timeout, `~/`, absolute/globs, relative globs searched upward to project root); render `Instructions from: <path>\n<content>` — spec/session.md §4.2, V/session/source-strings/instruction-file-render*.ts.txt
- [x] Access-denied project instruction file blocks the first turn with `Instruction initialization blocked by unavailable sources: …` — spec/session.md §4.2
- [x] Instruction baseline frozen per session (hashes + values stored); later changes emitted as one system update with the date/env/file/skill/catalog delta texts — spec/session.md §4.3–4.5, spec/storage.md §2.3 (`instruction_state`) (`epoch_start`/`through_seq` are message sequences, not event sequences; a changed Code Mode catalog is shown whole again instead of 0.5.1's itemized delta)
- [x] Skill discovery: config directory `skill/`+`skills/`, config `skills` (paths, `~/`, `http(s)` index URLs → `<url>/index.json`, cached), compatibility dirs per decision 3; later sources override ids; scan rules (SKILL.md ends descent, depth ≤6, top-level `*.md`), frontmatter `name`, `description`, `metadata["opencode/autoinvoke"]`, `disable-model-invocation` — spec/session.md §7.1 (the `.claude` disable env vars apply to `.claude` only, as in 0.5.1; skills are discovered once per process start)
- [x] Skill guidance listing rule and format; `<skill_content>` load format; user-invoked skills prepended to the user message — spec/session.md §7.1, V/session/source-strings/skill-guidance.ts.txt, skill-content.ts.txt (user-invoked skills are stored on the user message with their `<skill_content>` and lowered before its text)
- [x] Commands: built-ins `init` (`initialize.txt`), `review` (`review.txt`), `goal`; MCP prompts `<server>:<prompt>`; markdown commands `{command,commands}/**/*.md` + config `commands`; templating `$ARGUMENTS`, `$1..$N`, append rule, ``!`cmd` `` interpolation (concurrency 2, failure text); `@file` not expanded; agent/model switch; `subagent`/`subtask` → child session — spec/session.md §7.2, V/session/source-strings/command-*.ts.txt, V/session/initialize.txt, V/session/review.txt (markdown and config commands now apply per source in load order; `goal` done (next line); MCP prompts appended after the other commands, names already taken stay)
- [x] `/goal <objective> | pause | resume | clear`: kickoff, judge at idle, continuation, 50 rounds, 3 stalls, 30 min wait, ≤4000 chars, reminder after compaction — spec/session.md §6.8, V/session/goal/*.txt (the reviewer is the session generate request; continuations stay in one execution (one `idle` row); state in KV `goal:<sessionID>`, dropped with the session, not imported from 0.5.x; 0.5.1's startup pass that paused every goal of another process assumed one process per database: 1.0 pauses an active goal when it is read and its owning process is gone; the 0.5.1 judge request was not captured, the texts come from V/session/goal; `Event::GoalChanged` is the TUI's feed)
- [x] [cond: `references` config] references cloned into `~/.anysola/repos`, `<available_references>` block, `@` reference mentions — spec/integrations.md §7 (`reference`) (block-2 source `core/reference-guidance` between the Code Mode catalog and the skill guidance, with 0.5.1's delta/removal texts; legacy `reference` merged; checkouts `--depth 100`, one per remote and branch, refreshed at most daily (KV `repository-cache:<path>`), checked hourly and on config change, off the request path; core lists references and resolves `alias[/path]` to a path; inserting the mention and rendering it as an `Attached directory:`/`Attached file:` part belongs to the `@path` attachments of spec/session.md §8, which core does not render yet; two quick config reloads can start two clones of one checkout (0.5.1 held a per-path lock))
- [x] [cond: `warming`] keep-alive generate every 4 min for 30 min after activity, root sessions only — spec/session.md §6.7 (a pending instruction update is committed by the keep-alive, where 0.5.1 only previewed it; it lands where the next turn would put it)

### M1.11 Compaction, titles, sessions
- [x] Compaction settings (`compaction.auto`, `keep.tokens` 15000, `buffer`), ceiling math (window − max(15%, 16000)), measured + estimated (chars/4) trigger — spec/session.md §5 (window = `limit.context`; the gateway sends no input limit)
- [x] Triggers auto/overflow (85/70/50/35% budgets)/manual; summary request (`buildPrompt`, NUDGE, template `##` detection, failure texts) — spec/session.md §5, V/session/compaction-prompt-initial.wire.txt, V/session/source-strings/compaction-prompts.ts.txt (manual = `Core::compact`, an inbox control item)
- [x] Shrinking transcript levels and texts; checkpoint wrapper with `<summary>` and `<recent-context>` (tool outputs cut to 1250 chars + `\n[truncated]`) — spec/session.md §5, V/session/source-strings/compaction-transcript-texts.ts.txt, recent-text.ts.txt, V/session/requests/after-compaction-checkpoint.json
- [x] Compaction model = session model unless the `compaction` agent has one; usage recorded; failures `compaction.failed` / `Nothing to compact yet` / cannot-reduce text — spec/session.md §5
- [x] [cond: `experimental.background_compaction`] precompute — spec/session.md §5
- [x] Title: background, root sessions with no/fallback title, title agent model (gateway `agents.title.model`) with minimal variant else `small_model` else primary; fallback to primary once; prompt texts; first non-empty line, 100 chars; regeneration prompt — spec/session.md §6.1, V/session/agent-title.system.wire.txt, V/session/source-strings/title-prompts.ts.txt (`Core::regenerate_title` renames from the conversation)
- [x] Fallback titles `New session - <ISO>` / `Child session - <ISO>` — spec/session.md §6.1
- [x] Fork: copy semantics with `fork_session_id`/`fork_boundary`, own id — spec/session.md §6.3, spec/storage.md §2.3
- [x] Snapshots: per-location shadow git repo `~/.anysola/snapshot/<projectID>/<sha1(worktree)>` using system `git --git-dir/--work-tree`, tree at step start and end, untracked >2 MiB skipped, real repo ignore rules, pack every 10 min, only for git projects and `snapshots ≠ false`; changed files on the assistant message — spec/session.md §6.4 (packing is `git gc --auto`; the store is opened with `Store::at` so M3 can point it at the 0.5.x directory)
- [x] Revert stage / clear / commit semantics (file restore to the first changing step's start tree; commit truncates history, rewinds instruction state) — spec/session.md §6.4, spec/storage.md §2.12 (`Core::revert` / `Core::unrevert`; the next prompt commits)
- [x] Child sessions: synthetic completion `<subagent sessionID=… state=… description=…>` with the texts for empty/failed/cancelled — spec/session.md §6.5, V/session/source-strings/subagent-completion.ts.txt

### M1.12 Storage
- [x] `~/.anysola/anysola.db` via bundled SQLite: WAL, `synchronous=NORMAL`, `busy_timeout=5000`, foreign keys on, 0600 on db/wal/shm (POSIX), integer epoch-ms timestamps — spec/storage.md §2.1
- [x] Schema versioned by our own migrations; sessions, messages (ordered by a per-session strictly increasing seq), assistant parts, inbox, instruction state/blobs/entries, kv, credential, permission, project, worktree, job records; cascade deletes from session — spec/storage.md §2.3 (assistant parts stay inline in the message row, as 0.5.1 stored them)
- [x] Message `data` shapes for every type (`user`, `assistant` incl. snapshot/finish/tokens/error/retry, `synthetic`, `system`, `skill`, `shell`, `agent-switched`, `model-switched`, `location-switched`, `compaction`, `idle`) and assistant part shapes (text, reasoning with `native.signature`/`redactedData` and `time.completed`, tool states) kept JSON-compatible with 0.5.1 so imported rows replay unchanged — spec/storage.md §2.5, §2.6
- [x] Identifiers: 12 hex time+counter (descending for `ses_`) + 14 random base62; prefixes `ses_`, `msg_`, `prt_`, `cred_`, `psv_`, `sh_`; never validate id length — spec/storage.md §2.4
- [x] Session row fields (title, directory, parent, fork, agent, model `{id,providerID,variant?}`, token/cost sums, revert, permission ruleset, busy/idle/outcome/archived timestamps, `version` = `2.2.5+anysola.<ver>`) — spec/storage.md §2.3 (`session_v2` columns)
- [x] Cross-process busy marker on sessions (decision 1) — ARCHITECTURE.md "Process model"
- [x] KV keys used by 1.0: `anysola:config`, `anysola:session:<id>`, `anysola:budget:default`, `websearch:provider`, `repository-cache:<path>` — spec/storage.md §2.8 (all in use and tested except `websearch:provider`: 0.5.1 wrote it only from `WebSearch.select`, and 1.0 has the one Anysola provider and no selection, so nothing writes or reads it; 1.0 adds `goal:<sessionID>` for `/goal`)
- [x] Credential row for the device login `{type:"oauth",methodID:"device",refresh:"",access,expires,metadata:{apiURL,device}}`, plaintext JSON in the 0600 db; MCP OAuth credentials in the same table — spec/storage.md §2.9 (read here; the row is written by `anysola login` (M2) and MCP OAuth rows by M5)
- [x] Project id: `sha1("git-remote:"+host/path)` of origin, cached `<git common dir>/opencode`, root commit, Mercurial equivalents, `sha1("directory:"+dir)`, `global`; worktree records — spec/storage.md §4
- [x] Log file `~/.anysola/log/anysola.log`, logfmt keys (`timestamp`, `level`, `run`, `message`, annotations, `role`), trim above 50 MiB to ~25 MiB, `--log-level`, `OPENCODE_LOG_LEVEL`, `--print-logs`/`OPENCODE_PRINT_LOGS` to stderr — spec/storage.md §5, spec/cli.md §1.2

### M1.13 Config
- [x] Sources lowest → highest: managed policies, Anysola remote config document, global `~/.config/opencode/opencode.json[c]` (+ dir source; `OPENCODE_CONFIG_DIR` relocates it), global `~/.anysola/config.json` (+ dir source), `OPENCODE_CONFIG`, project `opencode.json[c]` farthest first (decision-3 bound; walk skipped when it starts in a global root; `OPENCODE_DISABLE_PROJECT_CONFIG`), project `.opencode/` dirs farthest first, `OPENCODE_CONFIG_CONTENT`, managed settings (`/etc/opencode`, [mac] `/Library/Application Support/opencode` + MDM plist via `plutil`, [win] `%ProgramData%\opencode`) — spec/storage.md §3.1 (well-known remote documents are not carried over (§A4))
- [x] Merge: scalars latest-wins; maps per entry (`agents`, `mcp.servers`, `providers`, `commands`); `instructions` union; `permissions` concatenated — spec/storage.md §3.2
- [x] JSONC with trailing commas; malformed document skipped with a log warning; unknown keys ignored; V1 key normalization; `{env:VAR}` and `{file:path}` substitution rules — spec/storage.md §3.3
- [x] Anysola provider lock applied to every document (foreign providers/models dropped, endpoint keys only from the remote document) — spec/storage.md §3.3
- [x] Keys accepted per schema (`V/storage/config.schema.json`); `lsp`, `share`, `enterprise`, `plugin(s)`, `update` accepted and ignored (`plugins` warns once) — spec/storage.md §3.4, spec/integrations.md §3 (the plugins warning is a WARN log line)
- [x] File watching (config files/dirs, skill dirs, AGENTS.md candidates, ancestor entries within the decision-3 bound, VCS HEAD) with `watcher.ignore` and the disable env vars; changes apply on the next step (100 ms debounce) — spec/integrations.md §5 (notify; `Shared::refresh` reloads every source once the last change is 100 ms old; the `FileWatch` client API is M4)

### M1.14 Code-action contract (`Anysola/code-action`)
- [x] `anysola run --standalone --format json -m anysola/<model> --budget <N> --agent anysola-review` with `OPENCODE_CONFIG_CONTENT` (agent with `mode:"primary"`, `description`, `permissions` on `*`, `read`, `grep`, `glob` and `*.env*` globs), `OPENCODE_DISABLE_PROJECT_CONFIG=1`, `OPENCODE_DISABLE_CLAUDE_CODE=1`, prompt on stdin: review agent cannot edit — spec/cli.md §3.4, §3.7
- [x] Issue mode `--auto`: edits land in the working tree (cwd = `$GITHUB_WORKSPACE`) — spec/cli.md §3.7
- [x] `ANYSOLA_TOKEN` (`asc_` + 40) as the only credential, `OPENCODE_DISABLE_AUTOUPDATE=1` tolerated — spec/cli.md §3.4
- [x] Answer extraction (text events of the last text event's `messageID`) and error extraction (last `error` event's `.error.message`) work on our output; classification strings appear verbatim — spec/cli.md §3.5, §3.6
- [x] `"$ANYSOLA_BIN" --version` runs — spec/cli.md §3.2

---

## M2 — Account and utility CLI commands

- [x] Root `anysola [<directory>]` starts the TUI after `chdir`; flags `-c`, `-s`, `--prompt`, `--auto` (+ hidden aliases), `--standalone` (no-op), `--server` (warning, §A1); description `Anysola Code command line interface (based on OpenCode, MIT)`; minimum-version gate before the TUI starts; help texts match `V/cli/help/*.txt` minus removed commands — spec/cli.md §1.1, §1.4, spec/protocol.md §1.9 (`-s <id>` creates the session under that ID through `Core::open`; help texts compared in `tests/cli_surface.rs`)
- [x] [fix] unknown directory → `Error: No such directory: <word>` (§B16); [fix] non-TTY TUI start → error (§B20) — spec/cli.md §1.1
- [x] First start with no connection on a TTY runs the device sign-in before the TUI (`Welcome to Anysola Code`); failure exits 1 — spec/cli.md §1.1, spec/protocol.md §5
- [x] Global flags `--help/-h`, `--version/-v`, `--log-level`, `--print-logs`, `--completions <bash|zsh|fish|powershell|sh|pwsh>`; parse errors print help on stdout and `\nERROR\n  <msg>` on stderr, exit 1 — spec/cli.md §1.2, §1.5, V/cli/framework-errors.txt
- [x] `--completions` prints a working completion script for each listed shell (our own generator) — spec/cli.md §1.2, V/cli/completions/
- [x] `--version`/`-v` anywhere → `Anysola Code <ver> (OpenCode 2.2.5)` + LF, exit 0, no network ([fix] §B8) — spec/cli.md §1.3, V/cli/version.txt
- [x] `login [-f]`: device flow (`POST /device/code`, poll `/device/token` sleep-first, `interval=max(1,…)`, +5 per `slow_down`, lifetime `min(expires_in,900)`, 3 network/5xx failures abort, token regex), clack-style output, browser opened only when stdin+stdout are TTYs, all error texts; `Already signed in (<mask>)…` without `--force`; `--force` revokes (401 = ok, failure warns with the apps URL) and removes the old token first — spec/protocol.md §1.1, §1.2, §5, V/cli/account-commands.txt
- [x] [fix] login/logout with `ANYSOLA_TOKEN` set per §B13 — spec/storage.md §2.9
- [x] `logout`: env-only → `Signed in through ANYSOLA_TOKEN; unset it to sign out.`; else `POST /tokens/self/revoke` (401 = success), failure keeps the token and exits 1, success removes the credential — spec/protocol.md §5, V/cli/account-commands.txt
- [x] [fix] `status` output per §B7 (incl. `Not signed in…`, `The token was revoked or expired…`, `Could not load status: HTTP <n>` + Token line exit 0) — V/cli/account-commands.txt
- [x] `usage [--days 7|30] [--format default|json]`: `Usage, last N days`, Totals/Cache hit rate/By day/By model tables, Balance line; json = body pretty-printed with 2 spaces — spec/protocol.md §1.5, V/cli/account-commands.txt, V/protocol/user-strings.txt "usageLines"
- [x] `devices` table (`*` current, `Unnamed device`, `never`, footer, `No devices are signed in.`) and `devices revoke <id>` (`Device <id> is signed out.`; [fix] unknown id §B6) — V/cli/account-commands.txt
- [x] Shared account errors: `Not signed in. Run \`anysola login\`.`, `The token was revoked or expired. Run \`anysola login --force\`.`, 404 text, `Request failed (HTTP <n>)` or server message, `Cannot reach <ANYSOLA_API_URL>`; exit 1 — V/cli/account-commands.txt
- [x] `update` (alias `upgrade`) `[--print] [--force]`: `Installed X, latest Y` + notes, `Already up to date`, method = script when the binary's realpath equals `<install dir>/anysola[.exe]` (case-insensitive) else npm; exact commands incl. [win] PowerShell and `npm.cmd` via shell; failure/success texts; gateway unreachable → command runs anyway; no minimum gate — spec/cli.md §4.3, spec/protocol.md §1.10, V/cli/update-uninstall.txt
- [x] `uninstall [--dry-run] [-f]`: method detection (curl; npm/pnpm/bun/yarn global probes with 30 s timeout), removes `~/.anysola/cache`, PATH lines in fish/zsh/bash/ash/sh rc files (only if the bin dir holds nothing else), [win] user PATH entry, package removal (5 min timeout), manual-removal instructions; keeps data/config; no TTY and no `--force` → exit 1; confirm prompt `Are you sure you want to uninstall?`; dry-run lines incl. `Installation method: curl` — spec/cli.md §4.4, V/cli/update-uninstall.txt
- [x] `models`: sorted `provider/model` lines (Anysola models from `/config` or its cache) — spec/cli.md §1.4, V/cli/other-commands.txt
- [x] `session list [-n N] [--format table|json]`, `session delete <id>`, `session export [id] [--sanitize]` (picker when no id, 130 on cancel), `session import <file|url> [--directory d]`; export JSON format of `V/storage/session-export.example.json` — spec/cli.md §1.4, V/cli/other-commands.txt
- [x] `debug config` (redacted config sources JSON) and `debug paths [name]` (1.0 paths: `~/.anysola/…` plus the old dirs read for compatibility) — spec/cli.md §1.4, V/cli/other-commands.txt
- [x] `reload` → `Configuration reloaded`, exit 0 (no-op, §A1) — spec/cli.md §1.4
- [x] `mcp add <name> [--url] [--header k=v]… [--env k=v]… [--global] [-- cmd…]`: validation texts, writes `mcp.servers.<name>` preserving comments (2-space indent) into the first existing of `opencode.json`, `opencode.jsonc`, `.opencode/opencode.json(c)` in cwd (or `~/.anysola/config.json` with `--global`), creating `opencode.json` if none; `MCP server "<name>" added to <path>` — spec/integrations.md §1.7, V/integrations/strings.txt
- [x] `mcp logout <name>` removes the server's credentials — spec/integrations.md §1.7
- [x] `plugin …` prints `Error: Plugins are not supported since Anysola Code 1.0; built-in features are native.`, exit 1 (decision 5) — spec/integrations.md §6.1
- [x] Error output of every handler is a single `Error: <msg>` line ([fix] §B17); clack cancel exits 130 — spec/integrations.md (preamble), spec/cli.md §1.5 (`main::fail`; the account and `session import` texts that 0.5.1 printed bare stay bare, as captured in V/cli/account-commands.txt and other-commands.txt; 130 from the export picker and the uninstall prompt)

---

## M3 — Migration from 0.5.x

- [x] On first start of any command except `--version`, `--help`, `--completions`, `update`, `uninstall`: if no import marker exists and a 0.5.x database is found, import once; the marker (KV `migration:0.5.x` with counts and time) is written in the same transaction; on failure nothing is written, one stderr warning `Could not import data from Anysola Code 0.5.x (<reason>); it will be tried again next start.` is printed and the import is retried next start — decision 2, spec/storage.md §6.3 (the command list in `main.rs` is an exhaustive match: each new command picks a side)
- [x] Source location: `$XDG_DATA_HOME/opencode` or `~/.local/share/opencode` (Windows `%USERPROFILE%\.local\share\opencode`), `opencode.db` (channel rules / `OPENCODE_DB` honoured); config `$OPENCODE_CONFIG_DIR` or `$XDG_CONFIG_HOME/opencode` or `~/.config/opencode`; state `$XDG_STATE_HOME/opencode` or `~/.local/state/opencode` — spec/storage.md §1.1, §6.1
- [x] The old db is never opened in place: even a `mode=ro` open creates `-wal`/`-shm` next to a WAL database, so db+wal are copied to a temp dir and the copy is read (`PRAGMA quick_check` must pass); no file under the old dirs is ever written, renamed or deleted — spec/storage.md §6.2
- [x] Schema check: all 59 migration ids present (extra ids tolerated); fewer ids → import refused with `Run Anysola Code 0.5.x once to upgrade its database, then start 1.0 again.` — spec/storage.md §6.2, V/storage/migration-table.txt
- [x] All `session_v2` rows (root, children, forks, archived) with every column 1.0 keeps (v1 `summary_*` as one `summary` JSON; dead `workspace_id`, `share_url`, `time_suspended`, `resume_attempts` dropped; a session busy when 0.5.x stopped is recorded `interrupted`) — spec/storage.md §6.2
- [x] All messages ordered by `seq`, `session_part` overlay merged first, `SessionLegacy` upgrades applied, blob refs resolved from `blob`, ids/types/seq kept, full `data` JSON kept including unknown fields — spec/storage.md §2.3, §2.5, §6.2
- [x] Thinking replay preserved: reasoning `text`, `native.signature`, `native.redactedData`, `time.completed`, assistant `model` and `error`, part order — spec/storage.md §2.7, §6.2
- [x] Not imported: `session_inbox`, `job`, running compactions (dropped), assistant rows without `time.completed` imported as interrupted; v1 tables (`session`/`message`/`part`) skipped with one log line — spec/storage.md §6.2
- [x] Projects and worktrees imported; new sessions in the same directory join the imported project (same id algorithm) — spec/storage.md §4, §6.2
- [x] Anysola token row (`integration_id='anysola'`) imported as the device credential; MCP OAuth credentials imported (every `credential` row is copied as stored); note: 0.5.1 signs with a stored device login even when `ANYSOLA_TOKEN` is set (`crates/migrate/tests/fixtures/rich/capture-*.json`), 1.0's `Core::start` prefers the env token — an M1/M2 precedence item — spec/storage.md §2.9, spec/integrations.md §1.6
- [x] KV `anysola:budget:default`, `anysola:session:<id>` (only for imported sessions; orphans dropped), `anysola:config` imported; `models-dev:*`, `migration.v1-v2`, `schedule:leader`, `wellknown:*` ignored — spec/storage.md §2.8, §6.2
- [x] Saved "always" permission rows imported per project; session rulesets come with the sessions — spec/storage.md §2.10
- [x] `cli.json` (theme, keybinds, every TUI setting) copied into `~/.anysola/tui.json`; when `cli.json` is absent, the v1 `tui.json` + `kv.json` are migrated the way 0.5.1 did — spec/storage.md §3.6
- [x] TUI state imported: `prompt-history.jsonl`, `prompt-stash.jsonl`, `model.json` (recents, favorites, variants) into `~/.anysola/state/`, every `latest/tui/*.json` (tabs, layout, …) into `~/.anysola/state/tui/` — spec/storage.md §1.2, spec/tui.md §2
- [x] Snapshots: for each imported project, a new shadow repo under `~/.anysola/snapshot/…` with `objects/info/alternates` pointing at the old shadow repo's objects, so revert of imported messages restores files without writing to the old dir — spec/session.md §6.4, spec/storage.md §1.2
- [x] Progress: the TUI shows an import overlay (`Importing sessions from Anysola Code 0.5.x…`); CLI commands print one stderr line `Imported <n> sessions from Anysola Code 0.5.x.` when something was imported (stdout untouched, so `run --format json` stays clean) — spec/storage.md §6.3 (`<n>` counts every imported `session_v2` row, children and forks included; the CLI line is done, the overlay waits for the TUI)
- [x] After import, `anysola -c` / `run -c` / `-s <old id>` continue a 0.5.x session and the next request replays its history exactly — spec/storage.md §2.7 (`run -c` and `run -s` are covered by `crates/anysola/tests/migrate.rs` against 0.5.1 captures; `anysola -c` waits for the TUI) (the TUI opens `-c`/`-s` through the same `Core::open` as `run`)

---

## M4 — TUI

Exact texts: `V/tui/dialogs.md`, `V/tui/slash-commands.md`, `V/tui/keybinds-default.md`, `V/tui/composer-strings.md`;
screens: `V/tui/screens/*.txt`.

### M4.1 Home and layout
- [x] Home at 120×36: tab strip (when tabs exist), centered 3-row wordmark with `Code` right-aligned, prompt box (`┃` rail, `╹▀▀▀` bottom edge), placeholder `Ask anything… "<example>"`, metadata row, footer row, `Recent` list (≤3, `ctrl+x l all sessions`), bottom plan line + version — spec/tui.md §0, V/tui/screens/home-120x36-*.txt
- [x] Narrow home (width < 64: A symbol + `Anysola Code`; height < 24 hides recents; height < 12 hides logo); 170×40 home — spec/tui.md §0, §5 "Wordmark", V/tui/wordmark.md, V/tui/screens/home-60x20-narrow.txt, home-170x40.txt
- [x] Home footer: MCP count `⊙ N` (click → `/mcps`), Anysola plan/banners slot, version; hidden when height < 14 or width < 44 — spec/tui.md §4 "Home route, tabs, keymap" (click on the count opens `/mcps`)
- [x] `/update to install v<version>` home notice when `/version` reports a newer release (checked at start and every 10 min) — spec/tui.md §4, spec/protocol.md §1.9
- [x] Layout regions, session frame (session pane + right pane), padding rules, one blank line between transcript rows, 3-cell indent, `┃` split border — spec/tui.md §Layout regions
- [ ] Responsive rules: sidebar docked when width > 120 else overlay (`RGBA(0,0,0,70)` dim, esc/ctrl+c close), assistant footer width rules, diff split > 120, permission diff uses terminal width, prompts stack < 80, form tab fit rule, docked panel ≥ 108, inline image size, crash screen sizes — spec/tui.md §Responsive breakpoints (done: diff split > 120, permission diff on terminal width, prompts stacking < 80, docked `/diff` ≥ 108, crash screen thresholds; not yet: the sidebar overlay below 121 columns (1.0 hides the sidebar there), inline image sizing (no image previews yet))

### M4.2 Session transcript
- [ ] Sticky-bottom transcript, optional scrollbar, wheel speed/acceleration, tail-first mounting with older-history paging, `Jump to latest ↓`, `Loading session history…`, per-tab scroll anchor — spec/tui.md §Transcript mechanics (done: sticky bottom, scrollbar toggle, wheel speed setting, `Jump to latest ↓`, per-tab scroll; not yet: tail-first mounting with older-history paging and `Loading session history…` (1.0 lays out the whole transcript and caches settled items))
- [x] Scroll and message-navigation keys (page/half/line, first/last, last user, next/previous) — spec/tui.md §Transcript mechanics, V/tui/slash-commands.md (message jumps put the target message at the top, from the message starts the layout records)
- [x] Streaming markdown with an unstable trailing block; rows appear on text/reasoning/tool-input events — spec/tui.md §Transcript mechanics
- [x] Grouping (`session.grouping`), exploration rows `Exploring:`/`Explored:` counts, verbosity low/medium/high with activity rows, instruction groups `◈ Instructions: N file(s)`, permission-blocked tools visible, per-tab expansion state — spec/tui.md §Grouping and verbosity (`transcript/grouping.rs`; instruction groups have no source in 1.0: nested `AGENTS.md` loads are session instruction entries (M1.8 read line), not synthetic transcript messages, so no `◈ Instructions` row can occur)
- [ ] Thinking hide/show modes, `Thought` rows with title/steps/duration, expanded markdown in muted colours, `[REDACTED]` removal, palette toggle — spec/tui.md §Thinking / reasoning (rows, modes, `[REDACTED]` removal and the toggle are in `transcript/rows.rs`; not yet: expanded thoughts as markdown in muted colours (shown as plain muted text))
- [ ] Markdown: rendered/source modes, headings, fenced code, lists, blockquotes, hr, grid tables, inline styles, OSC 8 links with ` (url)` fallback, clickable links; user messages plain — spec/tui.md §Assistant text (markdown) (not yet: OSC 8 hyperlinks and clickable links (links print with the ` (url)` fallback only))
- [x] Syntax highlighting built into the binary for every language 0.5.1 bundled or downloaded (javascript, typescript, tsx, markdown, zig, python, rust, go, cpp, csharp, bash, c, java, kotlin, ruby, php, scala, html, vue, hcl, json, yaml, haskell, css, julia, lua, ocaml, clojure, swift, toml, nix, diff, elixir, fsharp, r, make, vim, xml, agda) with the theme's scope → token mapping; no downloads — spec/tui.md §Assistant text (markdown), V/tui/themes-bundled.md §Syntax highlighting style
- [x] File-path → filetype map for tool output — spec/tui.md §Assistant text (markdown)
- [ ] ```` ```mermaid ```` diagrams (flowchart, sequence, state, gantt, gitGraph, timeline) as box drawings with the fallback rules — spec/tui.md §Assistant text (markdown) (1.0 draws flowcharts (top-down layers whose edges go one layer down, left-to-right chains), sequence diagrams as merman's placement plan in its compact form (messages and heads, self-message loops, notes, `alt`/`else`/`loop` frames, `box` groups, activations, autonumber), timelines, gantt charts and git graphs, each checked against merman's own test expectations; not yet: state diagrams (merman's 1.5k-line orthogonal router, `state/routing.ts`), which stay source)
- [x] ```` ```latex ````/```` ```math ```` blocks as 2-D Unicode math with limits and fallback — spec/tui.md §Assistant text (markdown) (`transcript/latex/`: a port of `@opencode/latex`'s parser, symbol tables and layout in the transcript's mode (strict, display, compact scripts); every text expectation of its render/layout/root/parser-render tests passes; cells carry no colour, so `	extcolor` and bold/italic variants draw in the text colour, as the other diagram blocks do)
- [ ] User message box (agent-colour rail, pending colour), attachment chips, image previews when enabled, click → Message Actions (Jump to / Revert / Copy / Fork), pending steer actions — spec/tui.md §User message (box, rail and chips done; not yet: image previews, click → Message Actions (reachable from `/timeline`), pending steer actions)
- [ ] Assistant footer `<Agent> · <model> · <duration> · <tok/s>[ · interrupted]`, duration format, step error line, retry countdown line, turn-level error, agent/model/variant switch notices, background completion notices, skill rows, background hint after 3 s, [cond: `debug.turn_tokens`] token table — spec/tui.md §Assistant footer, errors, retries (footer, duration, errors, notices, background hint done; not yet: the retry countdown line, the `debug.turn_tokens` table)
- [ ] Common tool rendering: canonical names, inline vs block rows, spinner (braille 80 ms, `⋯` without animations), failed/denied/awaiting styles, ` Background ` badge, tool-result images when enabled — spec/tui.md §Tool rendering (common) (not yet: tool-result images)
- [ ] Per-tool rendering for shell (collapse rules, background output polling), read, glob, grep, webfetch, websearch, write, edit (diff), patch, todowrite, subagent, question, skill, execute (nested calls, Execute dialog), generic/MCP — spec/tui.md §Tool rendering (per tool) (not yet: the `execute` nested-call rows and Execute dialog, background shell output polling in the row)
- [ ] Diff rendering (hunks, separators, gutter, split rows, virtualization > 3000 lines, wrap toggle) — spec/tui.md §Diff rendering (hunks, gutter and split rows done; not yet: virtualization past 3000 lines, the wrap toggle)
- [x] `/diff` viewer: sources, base branch picker, file tree, single patch, reviewed counter, states, all keys, right-click menu, docked vs full screen — spec/tui.md §Diff viewer (/diff) (sources All/Committed/Uncommitted/Last turn (`Core::last_turn_patch` over the session snapshots), `d` source list with the Base branch picker (remembered until exit), all-files scroll and `s` single patch, tree, reviewed counter, `?` Diff shortcuts, right-click Mark complete/incomplete, docked as the session's right pane from 108 columns (`f`/`diffs.fullscreen` keep it full screen); the docked panel takes the keys while open)
- [ ] Compaction rows (running/failed/cancelled/token counts/summary), queued compaction, `/compact` (`<leader>c`), revert banner, `/undo` (`<leader>u`, restores prompt text and images with the missing-image warnings), `/redo` (`<leader>r`), queued-prompt dock and dialog — spec/tui.md §Compaction, revert, queue, §2 "Undo / redo" (rows, `/compact`, `/undo`/`/redo`, queue dock and dialog done; not yet: queued compaction, the revert banner row)
- [ ] Sidebar (width 42): title with shimmer, Context section (tokens, % used), MCP section with toggle and statuses, Todo section, `Getting started` card (`/connect`), location footer with working-directory actions — spec/tui.md §Sidebar (Context, MCP (dots, statuses, `▼`/`▶` past two servers, kept in memory) and Todo done; location footer opens the `Working directory` menu; not yet: title shimmer while an auto-title is pending, the `Getting started` card (shown only with no provider connected, which 1.0 reaches only after a token is revoked mid-run))
- [x] Overlays: crash screen (texts, `r` restart, `q` quit, copy report, issue URL, hard-coded palette, version line), session-location-missing question; startup loading overlay — spec/tui.md §Overlays, §6 "Themes" (crash palette) (`crash.rs`: a panic in the UI loop shows it with the fixed palette, `c` copies the pre-filled issue link (≤ 6000 chars), `r` starts the UI again (in the launch directory, with the core it started with), `q` quits; location question in `view/prompts.rs`; the startup overlay only says `Loading plugins…`/`Finishing startup…` while JS plugins load, which 1.0 does not have, decision 5)
- [x] Session-level commands `/rename` (ctrl+r, empty regenerates), `/timeline`, `/fork` (`Forked session`), `/copy` texts, `/export` (markdown/JSON, include thinking/tools, sanitize, copy or file), Copy session ID/transcript, `Session not found: <id>` toast — spec/tui.md §Copy, selection, mouse, §Session-level commands registered by the session route

### M4.3 Permission and question prompts
- [ ] Permission prompt replacing the input: `△ Permission required`, per-action titles/bodies, `Allow once` / `Always allow` (only with save patterns) / `Reject`, "always" explanation texts, keys, fullscreen `ctrl+f`, child-session reject reason dialog, warning tint on awaiting rows, tab `!` marker — spec/tui.md §Permission prompt, spec/tools.md §TUI prompt (texts), V/tui/dialogs.md §29 (not yet: the reject-with-reason dialog for child-session requests)
- [x] Auto-accept mode (`--auto` or Settings Permissions = auto accept), `auto` label in the metadata row — spec/tools.md §TUI prompt (texts), spec/tui.md §2 "Metadata row"
- [x] Question/form prompt: title, fields/tabs, options, multiselect, custom answer row, keys, action labels, placeholders, review tab, answers kept across tab switches — spec/tui.md §Question / form prompt, V/tui/dialogs.md §30 (answers are kept per field while switching the form's field tabs)

### M4.4 Composer
- [x] Prompt box layout, colours, textarea height `max(6, floor(h/3))`, focus rules, hidden while prompts/forms/dock are active — spec/tui.md §2 "Layout of the prompt box"
- [x] Placeholders (home examples, shell-mode examples, none in session, width cut) — spec/tui.md §2 "Placeholder", V/tui/composer-strings.md
- [ ] Metadata row (agent, `auto`, model name, blank provider for Anysola, variant), responsive fit, fade-in, shell mode `Shell` — spec/tui.md §2 "Metadata row" (not yet: the fade-in)
- [x] Footer row: directory label with branch and working-directory menu, running spinner + `esc interrupt` / `esc again to interrupt`, workspace-move status, `/btw` spinner, credit status line, editor-file label, right hints (`shift+tab agents`, `ctrl+p commands`), subagent/shell counts, goal status, context usage `<tokens> (<pct>%)`, shell-mode hint — spec/tui.md §2 "Footer row under the prompt" (all of it; the directory label opens `Working directory` (Copy path / Open folder / Workspaces); the IDE file label is accent while the selection waits to be sent)
- [ ] Submit/newline/editing keys, in-input undo/redo, double-submit guard, `exit`/`quit`/`:q`, empty Enter behaviour, steer vs `<leader>return` queue, queue refusals, synchronous clear and restore on failure, optimistic session creation on home, model/budget/revert checks, submit error toasts, `ctrl+c` clear with history save, `ctrl+z` suspend (not on Windows), per-tab drafts — spec/tui.md §2 "Multiline input, submit, newline, editing keys" (not yet: per-tab drafts, the double-submit guard)
- [x] Interrupt with double Esc (5 s), `ctrl+b` background blocking tools — spec/tui.md §2 "Interrupt / background"
- [x] Prompt history (50 entries, up/down rules, shell entries, fall-through to subagent picker) in `~/.anysola/state/prompt-history.jsonl` — spec/tui.md §2 "Prompt history" (`composer::HISTORY_LIMIT` = 50 with its test; empty `down` falls through to the dock)
- [x] Text paste: bracketed paste normalisation, empty-paste clipboard read, ANSI strip, `[Pasted ~N lines]` placeholders (expand on re-paste/click/copy/submit), `prompt.paste` setting, toggle command — spec/tui.md §2 "Paste handling (text)" (clipboard read through wl-paste/xclip/xsel/pbpaste/PowerShell)
- [ ] File/image paste: local paths to images/PDF (≤32, 20 MiB), `[Image N]`/`[PDF N]`/`[SVG: name]` chips, `ctrl+v` clipboard image, clipboard error toasts, dedupe, image previews and viewer (`<leader>i`) — spec/tui.md §2 "Paste handling (files and images)" (chips, `ctrl+v` images and the viewer done; not yet: inline image previews)
- [ ] `@` autocomplete: trigger rules, sources (skills, references, agents, files limit 20), ranking rules, `#L` ranges, reference shortcut, mention extmarks, directory Tab, relative paths; file search = fuzzy matcher over an `ignore` walk — spec/tui.md §2 "`@` mention autocomplete", spec/integrations.md §7 (fff) (not yet: `#L` line ranges on file mentions)
- [x] `/` autocomplete: opening rule, rows per name and alias plus server commands, sorting/padding, fuzzy filter, run-or-insert, `/cd` directory argument completion with recents — spec/tui.md §2 "`/` command autocomplete" (`/cd` restarts the core in-process for the new directory and moves the session; recents in `~/.anysola/state/tui/directory-recents.json`, `ctrl+d` twice removes one (`Press ctrl+d to confirm`))
- [ ] Autocomplete popup look, keys, empty-state texts — spec/tui.md §2 "Autocomplete popup (shared)" (look, keys and the two common empty texts done; not yet: mouse hover/click, `Searching…`, `No matching directories` and the search-error texts)
- [x] `!` shell mode: enter/exit rules, look, runs through the session shell, output rows, skips model/budget checks — spec/tui.md §2 "`!` shell mode"
- [ ] External editor `/editor` / `<leader>e` (`$VISUAL`/`$EDITOR`, temp `.md`, mention realignment, error toasts) — spec/tui.md §2 "External editor" (not yet: mention realignment after editing)
- [x] Editor context: IDE lock files `~/.claude/ide/<port>.lock` / `CLAUDE_CODE_SSE_PORT` / `OPENCODE_EDITOR_SSE_PORT` WebSocket JSON-RPC, `at_mentioned` insertion, Zed DB polling ([mac]/Linux paths, `OPENCODE_ZED_DB`), hidden `<system-reminder>` texts, remove command, `prompt.editor` setting — spec/tui.md §2 "Editor context (IDE / Zed selection)", §Zed (footer file label `name#a-b +n`, accent while unsent)
- [x] Agent switch (`shift+tab`, `/agents`, `Agent not found` toast, per-session draft committed on submit) — spec/tui.md §2 "Agent switch" (`Agent not found` is unreachable in 0.5.1: `context/local.tsx:100-105` fires only for an id outside the list, and every caller passes a listed agent or `args.agent`, which the root command never sets, `cli/src/commands/handlers/default.ts:133-139`)
- [x] Model/variant cycling: `f2`/`shift+f2` recents (max 10), favourites, `<leader>m`, `ctrl+t`, `/variants` (+`/thinking`, `/effort`) with no-variant toast, variant remembered per model — spec/tui.md §2 "Model and variant cycling" (recent and favourite cycles over `~/.anysola/state/model.json`, which also keeps the `variant` map)
- [x] Stash (`Stash prompt` / `Stash pop` / `Stash list`, 50 entries, dialog texts) — spec/tui.md §2 "Stash"
- [x] `/skills` dialog, `/worktrees` (workspaces dialog, deferred creation), composer dock tabs Subagents/Shell/Terminals with their keys and empty texts, child-session navigation keys — spec/tui.md §2 "Skills", "Workspaces", "Composer dock", §Composer tab panel (a worktree picked on home is created on the first submit, which then runs there; footer `↓ N subagents` opens the dock; `up` at the dock's top returns to the parent, `left`/`right` switch dock tabs; `session.child.next`/`previous` have keybind ids but no registered command in 0.5.1 (`config/keybind.ts` only), so they do nothing there either)
- [x] `/btw [question]` one-shot side question (prompt dialog, footer spinner, answer dialog with copy) — spec/tui.md §2 "/btw side question"
- [x] Goal footer status `◎ goal` / `paused` / `met` and Goal dialog (pause/resume/clear, error toast) — spec/tui.md §2 "Goal" (`app/goal.rs`; follows `Event::GoalChanged`; actions run the `/goal` server command)
- [x] Terminal pane (in-process PTY): `/terminal`, `<leader>t`, `<leader>down`, `<leader>up`, `<leader>left/right`, resizable right pane, `Unable to load terminal` — spec/tui.md §Terminal pane (right pane), spec/integrations.md §7 (pty) (drag the pane's left edge; at least 24 columns, the session keeps 40)

### M4.5 Dialogs, tabs, keymap
- [x] Dialog frame and `DialogSelect` behaviour (search, fuzzy, categories, `●` current, footer actions, empty texts), confirm/alert/prompt dialogs — spec/tui.md §4 "Shared frame", V/tui/dialogs.md §0
- [x] Command palette (`ctrl+p`, Suggested group, categories, shortcut footer, settings entries on query) — V/tui/dialogs.md §1, V/tui/screens/command-palette*.txt
- [ ] Model picker (`Anysola` group, notes, `~N credits/turn`, favourites/recent, variant follow-up), agent picker, session list (groups, gutter markers, delete twice, rename, all projects, search debounce 150 ms, limit 50), rename, theme list, settings, status, MCP servers, error details, help, debug ([fix] §B9), timeline, message actions, fork, export, stash, skills, open (`ctrl+o`), worktrees, image preview, shell output, execute details, update, connect — V/tui/dialogs.md §2–§27, V/tui/screens/*.txt (most dialogs snapshot-tested against V/tui/screens; not yet audited row by row against V/tui/dialogs.md §2–§27)
- [x] `/connect`: `Connect an integration` listing only Anysola (`✓ Anysola  $ANYSOLA_TOKEN` when env-signed-in); choosing it runs the device sign-in inside a dialog — V/tui/dialogs.md §27, spec/tui.md §5 "Wordmark, version, exit summary" (the new login is stored; the running process keeps its token, so it takes effect on the next start)
- [x] `/update` dialog: up-to-date text, `Update available`, `Updating Anysola Code`, install via the same command as `anysola update`, `Restart` re-executes the new binary — V/tui/dialogs.md §25
- [x] Settings dialog: every row of dialogs.md §8 cycled with ←/→/Enter and written to `tui.json`; [fix] Animations default `on` (§B11) — V/tui/dialogs.md §8
- [ ] Every slash and palette command of `V/tui/slash-commands.md` marked [0.5.1], except those listed under "Not carried over"; `/new`, `/clear` (closes tab), `/open`, `/mcps`, `/status`, `/reload` (in-process), `/help`, `/exit` (`/quit`, `/q`), `Open docs` (`https://www.anysola.com/code`), theme mode switch/lock, toggles for terminal title/animations/file context/diff wrap/paste summary — V/tui/slash-commands.md (not yet audited command by command against V/tui/slash-commands.md)
- [ ] Session tabs: strip, `+ New session`, `!`/`?`/spinner markers, unread tracking, close `<leader>w`, reopen `ctrl+shift+t`, next/previous (+unread), select 1–10, quick slots when tabs are off, persisted per cwd in `~/.anysola/state/tui/tabs.json`, vertical layout (width 42, clamp, drag resize) — spec/tui.md §4 "Home route, tabs, keymap", §Layout regions (strip, `+ New session`, `!`/`?`/spinner/`•` unread markers, close, reopen, next/previous (+unread), select 1–10, `~/.anysola/state/tui/tabs.json` per cwd (strip, `+ New session`, `!`/`?`/spinner/`•` markers, close, reopen, next/previous (+unread), select 1–10, `~/.anysola/state/tui/tabs.json` per cwd (`tabs.scope = global` shares one list) done; quick slots do not exist with tabs off in 0.5.1 (`app.tsx:890-896`, `enabled: sessionTabs.enabled`); not yet: the vertical tab layout)
- [x] Keymap: leader `ctrl+x` with 2000 ms timeout, comma alternatives, display format (`ctrl+x m`), layers/modes, every v2 default and v1 legacy name accepted from config, [fix] `<leader>q` per §B12, [win] `ctrl+z` → undo, Kitty keyboard protocol for shift/ctrl+return — V/tui/keybinds-default.md, spec/tui.md §Rust implementation notes

### M4.6 Anysola UI
- [x] Account state refresh (`/me` at start, every 5 min, 1.5 s after credits; estimates/notes at start); balance source order; low balance = not free and (`lowBalance` or remaining < 100) — spec/tui.md §5 "State and refresh", spec/protocol.md §4.1
- [x] RPC failures as inline text (`Account details are unavailable (<detail>).` without the service hint, `This server does not offer it yet.`, `Request failed (HTTP <n>)`, `Cannot reach Anysola`) — spec/tui.md §5 "State and refresh", §B21 (1.0 has no TUI↔service RPC, so only the HTTP cases apply: 404 text, server message, `Request failed (HTTP <n>)`, `Cannot reach Anysola`)
- [x] Status line `<n> credits · this turn <t> · session <total>[/<budget>]` (en-US grouping), [fix] free `today <turns>/<daily>` (§B4), hover renews date, warning colour, click → `/credits`, stored totals loaded on session open — spec/tui.md §5 "Status line", spec/protocol.md §4.1
- [x] Banners: event label with `until`, `Low balance: <n> credits left · /credits`; placement above the composer and on the home footer; home plan line `<Plan>[ · renews <D Mon>]`; date formats — spec/tui.md §5 "Banners", V/tui/screens/home-low-balance-event-banner.txt
- [x] `/credits`, `/usage` (`t` toggles 7/30), `/budget` (all argument forms and texts), budget gate dialog (`Continue once`, `Add <b> credits`, `Remove the budget`, `Stop`), `/devices` (current device toast, revoke confirm, `Device signed out`), `/account`, `/feedback` — spec/tui.md §5 "Commands", spec/protocol.md §4.3, §4.6, V/protocol/user-strings.txt "TUI"
- [x] Exit summary after leaving the alternate screen (wordmark, `Run anysola to use <version>` after an update, `Session`, `Spent N turn(s) · C credits` when > 0, `Continue  anysola -s <id>`) — spec/tui.md §5 "Wordmark, version, exit summary", V/tui/screens/exit-summary.txt
- [x] Home recent sessions (≤3, `formatAgo`, click opens) and home version `<ver>` — spec/tui.md §5 "Wordmark, version, exit summary" (`view/home.rs`, snapshot home-120x36-recent; a click opens the session)

### M4.7 Themes and other UX
- [x] Default theme `anysola` (v2 "Grafit", dark + light token tables), unknown names fall back to it, load failure toast + switch to `opencode` — spec/tui.md §6 "Themes", V/tui/theme-anysola.json, V/tui/theme-anysola.md
- [x] `/themes` dialog with 35 entries, live preview, Esc restores, every change written to `tui.json`; bundled 32 v1 themes + `opencode` + `anysola` + `system` — spec/tui.md §6 "Themes", V/tui/themes-bundled.md §What `/themes` lists
- [x] Theme formats v2 and v1 (v1 → v2 migration with OKLCH hue inference), stateful colours, custom themes from `themes/*.json` in global dirs and `.opencode/` ancestors (decision-3 bound), discovery error fallback, `SIGUSR2` reload — spec/tui.md §6 "Themes", V/tui/themes-bundled.md (`SIGUSR2` reload done)
- [ ] Colour mode: config lock, else OSC 10/11 query within 1000 ms (brightness rule), DEC 2031 theme-change following, switch/lock commands, Settings rows; `system` theme from the 16-colour palette with transparent background — spec/tui.md §6 "Themes", V/tui/themes-bundled.md §Mode, §system theme (not yet: DEC 2031 theme-change following)
- [ ] Prompt extmark styles and syntax token mapping per theme — spec/tui.md §6 "Themes" (not yet: syntax token mapping checked per bundled theme)
- [ ] Clipboard: native write + OSC 52, read (image/png then text), `terminal.copy` select (default) / manual ([win] default), selection key rules, dialog copy-on-release, link click opens browser — spec/tui.md §Clipboard, §Copy, selection, mouse (native write + OSC 52 and select-to-copy done; not yet: clipboard read for text, the `terminal.copy = manual` mode)
- [x] Mouse capture on by default (`mouse` config), clickable targets ignore clicks while text is selected — spec/tui.md §Copy, selection, mouse
- [x] Terminal title: `Anysola Code` at home/untitled, [fix] `Anysola | <title>` (§B10), toggle command and setting, cleared on exit, not changed while suspended — spec/tui.md §Terminal title
- [x] Toasts: single visible toast, position/size/borders, title and action affordances, 5000 ms default, hover pause, queue `+N more`, texts listed — spec/tui.md §Toasts (hover pauses and resumes with the time left; a toast arriving while one is hovered or others wait queues, shown as `+N more`; titled toasts keep the blank row under the title)
- [x] [cond: `attention.notifications` / `attention.sound`] notifications and sounds for done/subagent done/question/permission/error with title/message limits, focus-out rule, built-in sounds, volume, Settings rows; `Session failed` toast always — spec/tui.md §Attention & notifications (notifications as OSC 777 + OSC 9 on focus-out; sounds are 0.5.1's MP3s played by the system player: afplay, paplay/pw-play/ffplay/mpv, PowerShell)
- [x] Program status records (OSC 7501) per tab/session, cleared on exit — spec/tui.md §Attention & notifications (`status.rs`: probed with `ESC ] 7501 ; ?` next to the OSC 11 query (DA1 when the theme mode is locked), records only when answered; title/msg base64 as the protocol encodes them)
- [x] Animations (runtime default on), `app.toggle.animations`, spinner frames, static fallbacks — spec/tui.md §Animations
- [x] Locale helpers (`time`, `datetime`, `1.2K`/`3.4M`, grapheme-aware truncation, east-asian width) — spec/tui.md §Locale helpers, §Rust implementation notes (en-US formats in the viewer's timezone)
- [x] Toasts for MCP needs-auth/failed (once per state change, action `Open MCP servers`) and `Invalid model format: <arg>` — spec/tui.md §Toasts, spec/integrations.md §1.8 (`› Open MCP servers` runs `/mcps`; `Invalid model format` is unreachable in 0.5.1: `app.tsx:665` reads `args.model`, which the root command never sets (`cli/src/commands/handlers/default.ts:133-139`, no `--model` flag), so 1.0 has no such toast)

---

## M5 — MCP and MCP in Code Mode

- [x] Config: native `mcp.timeout` + `mcp.servers.<name>` local (`command[]`, `cwd`, `environment`, `disabled`, `codemode`, `timeout`, `protocol`) and remote (`url`, `headers`, `oauth` object or `false`); legacy v1 shape migration rules; merge across documents; defaults plugin (`executor` → `codemode:false`, PostHog header); `mcp.use` policy — spec/integrations.md §1.1
- [x] Live reconcile on config change (restart only changed servers) — spec/integrations.md §1.1
- [x] stdio transport: NDJSON, `\r` strip, blank lines ignored, 16 MiB frame limit, stderr tail 1000 chars, close sequence (2 s, SIGTERM, 2 s, SIGKILL, process group), exit texts — spec/integrations.md §1.2, V/integrations/strings.txt
- [x] Streamable HTTP: `?codemode=false` rule and exceptions with one retry without it on 400/404, connect retries (250 ms, 1 s) for transient errors, serialized concurrent startups per URL, session-expiry reconnect + retry once, DELETE on shutdown (1 s) — spec/integrations.md §1.2
- [ ] Client capabilities `roots` (answers `roots/list`) and `elicitation {form, url}`; malformed tools dropped individually; capability-gated listing; protocol `legacy`/`auto`/`2026-07-28` — spec/integrations.md §1.1, §1.2 (open: `auto`/`2026-07-28` only offer `2026-07-28` in a plain initialize and check the answer; the revision's own discovery handshake is not ported)
- [x] Timeouts startup 30 s, catalog 30 s, execution 12 h with progress keep-alive — spec/integrations.md §1.3
- [x] Status union `connected | pending | disabled | failed{error} | needs_auth{error}`, async start, no automatic retry of failed servers, `list_changed` refresh (tools debounced 100 ms), credential switch restarts the server — spec/integrations.md §1.4
- [x] MCP tools in the Code Mode catalog as `tools.<ns>.<tool>(<TS-like input>): Promise<unknown> // <description>` with name sanitizing; `codemode:false` → direct `<ns>_<tool>` with passthrough description/schema; permission action `<ns>_<tool>` resource `*` — spec/integrations.md §1.5, V/integrations/codemode-catalog-with-mcp.txt
- [x] Server `instructions` in `<mcp_instructions>` only when a tool is reachable; delta texts; catalog-changed system message when a server connects mid-session — spec/integrations.md §1.5, V/integrations/mcp-observed-output.txt
- [x] `tools/call` with `_meta {"ai.opencode/sessionID"}` + progress token; result mapping (isError, text join, structuredContent, JSON text parsing, inline media, `[Resource …]`, `[Binary resource …]` files, resource links) — spec/integrations.md §1.5
- [x] `opencode.list_mcp_resources` / `opencode.read_mcp_resource` with their permission actions — spec/integrations.md §1.5
- [x] MCP prompts as `<server>:<prompt>` slash commands with positional args — spec/integrations.md §1.5
- [x] Elicitation (form + URL) as forms titled `<server> is requesting input`, shown by the TUI; `run` cancels them with exit 1 — spec/integrations.md §1.5, spec/cli.md §2.4 (core: `Event::FormAsked` under session `global`, answered with `Core::reply_form`; the TUI's form UI is M4's)
- [x] MCP error texts (not found, not connected, needs authentication, disabled, tool failed) — V/integrations/strings.txt
- [x] OAuth: integration per remote server `mcp_<sha1(name+"\0"+url)[0..16]>`, connect never opens a browser (401 → `needs_auth`), authorize flow (loopback callback on `127.0.0.1`, port/path rules, initialize probe, RFC 9728/8414 discovery, static client / CIMD `https://www.anysola.com/oauth/anysola-code/client.json` / DCR with `Anysola Code` metadata, PKCE S256, state, scope, resource), callback page HTML verbatim, token storage/refresh/coalescing/invalidation — spec/integrations.md §1.6, spec/protocol.md §1.11, V/integrations/oauth-callback-page.ts, V/integrations/*.html
- [x] `anysola mcp list` with live statuses ([fix] §B18) and `anysola mcp auth [name]` (picker on TTY, `Re-authenticate?`, instructions + URL, browser on TTY, 500 ms polling, Ctrl+C cancels, exit 130 on cancel) — spec/integrations.md §1.7, V/integrations/cli-help.txt (in-process the attempt is awaited directly rather than polled)
- [x] TUI `/mcps` dialog (status, search, `space` connect/disconnect/retry/sign in, `enter` details or sign-in), disconnect not persisted, sidebar MCP section, home footer counts, `/status` lines — spec/integrations.md §1.8, V/tui/dialogs.md §9, §10 (sidebar section and home footer click added in M4)

---

## M6 — Packaging, CI, release, install

- [ ] Release workflow on tag `v<ver>` (and manual dry run): builds the six targets of decision 8, smoke test (exact `--version` line; `--help` contains `anysola <subcommand>` and not `opencode command line`) — spec/cli.md §4.1 (needs a real run)
- [x] Archives: `anysola-<target>.tar.gz` (Linux; mtime 0, owner/group 0, numeric owner, single entry `anysola`, mode 0755) and `.zip` (darwin/windows; `zip -X`, entry `anysola`/`anysola.exe`), the 12 names with the byte-identical duplicates of decision 8 — spec/cli.md §4.1, V/cli/release.txt (script/pack.sh; checked locally with real linux arm64/x64 musl builds)
- [ ] [mac] ad-hoc codesign; [win] unsigned — spec/cli.md §4.1 (needs a real run)
- [ ] GitHub release `v<ver>` titled `Anysola Code <ver>`, assets = 12 archives + `install.sh` + `install.ps1` + `SHA256SUMS` (GNU `<hex>  <name>`), prerelease when the version has `-` — spec/cli.md §4.1, V/cli/release.txt (needs a real run)
- [x] npm: `@anysola/code` with `bin.anysola = bin/anysola.exe`, `postinstall: node ./postinstall.mjs`, `optionalDependencies` on the 12 `@anysola/code-<target>` at the exact version, placeholder bin with the "postinstall script was not run" error; platform packages with `os`/`cpu`, `bin/anysola[.exe]`, LICENSE, NOTICE; publish platform packages first, dist-tag `latest` or `next` — spec/cli.md §4.2, V/cli/release.txt (script/pack.sh; tarball contents checked against the 0.5.x tarballs; publishing needs a real run)
- [x] postinstall candidate selection (glibc/musl, AVX2, [win] x64 Node on ARM64), version check, hard link else copy, chmod 755, `--version` check, fallback `npm install --ignore-scripts` retry — spec/cli.md §4.2 (0.5.1 postinstall.mjs shipped verbatim; glibc hard link checked locally)
- [x] `install` (POSIX sh) and `install.ps1` unchanged in behaviour and flags (target detection, `/v1/code/version` default, SHA256 verification, atomic replace, PATH edits, `$GITHUB_PATH` under Actions, [win] rename-aside of a running exe, user PATH) and served by the www `/install` routes — spec/cli.md §4.3, V/cli/install-sh.txt, V/cli/install-ps1.txt (byte-identical to 0.5.1; `install` run locally against served 1.0 archives on linux arm64 glibc/musl and x64 musl)
- [ ] `/v1/code/version` semantics for update notices and the minimum gate unchanged; Admin Release flow unchanged — spec/cli.md §4.3, ARCHITECTURE.md "Contracts that do not change" (needs a real run)
- [ ] install-test matrix after each release: `curl | sh` on ubuntu x64/arm, macOS arm/intel, `irm | iex` on windows-2025 and windows-11-arm (PowerShell 5.1); asserts `^Anysola Code [0-9]`, `update --print`, `uninstall --dry-run` containing `Installation method: curl`; reinstall while installed — spec/cli.md §4.1 (needs a real run)
- [ ] CI on every push: build + tests + smoke run on Linux x64/arm64, macOS arm64/x64, Windows x64/arm64; `script/budget.sh` (binary ≤ 25 MB, archive ≤ 10 MB, `--version` ≤ 15 ms / 10 MB RSS, idle TUI ≤ 40 MB RSS against the fake gateway) — ARCHITECTURE.md "Why", "Testing" (needs a real run)
- [ ] Updating from 0.5.1 to 1.0.0 works through both 0.5.1 update paths (`anysola update` script and npm) — spec/protocol.md §1.10 (needs a real run)

---

## M7 — ACP (`anysola acp`)

- [x] `anysola acp`: NDJSON JSON-RPC 2.0 on stdin/stdout, in-process agent, `OPENCODE_CLIENT=acp` identity (decision 7); exits 0 when the client closes; message-size and fatal errors on stderr as `anysola acp: …`, exit 1 — spec/integrations.md §2, V/integrations/acp-transcript-session.log
- [x] `acp --login` runs the device sign-in instead — spec/integrations.md §2
- [ ] `initialize` response exactly as captured (protocolVersion 1, capabilities, auth method `opencode-login`, `agentInfo {name:"Anysola Code", version:<ver>}`); client capabilities read (`elicitation.form`, `session.compaction`, child-session updates meta) — spec/integrations.md §2 (response and `elicitation.form` done; `session.compaction` and child-session updates are read by nobody yet: compaction and subagent progress are not forwarded)
- [x] Requests `authenticate`, `session/new`, `session/load`, `session/list` (cwd filter, desc, limit 100, cursor), `session/delete`, `session/resume`, `session/close`, `session/fork`, `session/set_config_option`, `session/set_mode`, `session/prompt`; notification `session/cancel` — spec/integrations.md §2
- [x] Session mapping (ids = session ids, absolute `cwd` equal to the session directory for load/resume/fork, `additionalDirectories` grants, `mcpServers` stdio/http added for the process lifetime, location-scoped) — spec/integrations.md §2
- [x] Config options `model`, `effort` (variants + `default`), `mode` (primary agents); `config_option_update` and `available_commands_update` (config commands, MCP prompts, `goal`, `compact`) — spec/integrations.md §2
- [x] Prompt updates (`user_message_chunk` replay, `agent_message_chunk`, `agent_thought_chunk`, `tool_call` with kind mapping, `tool_call_update` with diffs, `usage_update`), response `{stopReason, usage, _meta}` with stop-reason mapping — spec/integrations.md §2
- [x] Permissions via `session/request_permission` (once/always/reject, titles, diff previews; reject → `The user declined this tool call`, turn `cancelled`) and elicitation via `elicitation/create` — spec/integrations.md §2, V/integrations/acp-transcript-permission.log
- [x] Error texts (JSON-RPC -32602 `Invalid params: …`) — V/integrations/strings.txt

---

## Not carried over

- `serve`, `serve --service`, `--stdio`, service registration/ports, `service start|restart|status|stop|get|set|unset`, `/restart`, update-preflight splash, reconnect overlay, "Starting background server..." — decision 1.
- `service set remote` / OpenTunnel / `ANYSOLA_ENABLE_REMOTE_TUNNEL` — decision 1; dead and ungated in 0.5.1.
- `--server <url>` remote attach (accepted, ignored) — decision 1.
- PTY daemon persistence across TUI restarts — decision 1 (pane kept in-process).
- Windows `RetainedImage` hard-link handling and "Stop background services" uninstall step — no service.
- Shared `opencode` XDG data/config/state/cache dirs as Rust's own storage, `OPENCODE_DB`, channel db names, log trim lock dir, `<state>/locks` flock emulation format — decision 2 (old dirs read only for import/compat).
- models.dev / models.opencode.ai catalog fetch and its KV keys, `OPENCODE_MODELS_URL/PATH`, `OPENCODE_DISABLE_MODELS_FETCH` — decision 4.
- tree-sitter grammar downloads to `~/.local/share/opentui` — decision 4 (built in).
- ripgrep download and `rg` on PATH — decision 4 (crates linked).
- OTLP telemetry (`OTEL_EXPORTER_OTLP_*`) — decision 4; off by default in 0.5.1.
- JS plugin host, npm plugin installs, `plugin list|add|check|update|remove` functionality, `<configdir>/plugin(s)/` sources, TUI plugins, `/plugins` dialog, home footer `N plugin(s) failed`, `plugin.use` policy target — decision 5.
- Web UI (`ANYSOLA_ENABLE_WEB_UI`) — dead.
- Upstream commands `pair`, `auth`, `api`, `mini`, `stats`, `debug agents`, `debug heap-dump`; V1 run bridge — dead.
- `--wizard` interactive command builder — effect/cli artifact; no Anysola use.
- DevTools bar (`debug.devtools`, Settings "Developer tools") with its Server/Runtime/Theme/Tools/Experiments panels and `debug.timing` — a developer surface of the OpenTUI build, off in the `latest` channel: its panels report the background server (decision 1), the Bun heap/event loop and OpenTUI theme timing, and its Experiments dialog toggles `new-designs`/`TUI tools`, neither carried over (`component/devtools-bar.tsx:1-611`, `app.tsx:493`). The Settings row stays and still writes `debug.devtools` for 0.5.x.
- effect/cli completion scripts byte-for-byte — replaced by our generator (same shells).
- SIGUSR1 heap snapshot, SIGPROF CPU profile — debug only in a Bun runtime.
- `OPENCODE_SIMULATE`, `OPENCODE_DRIVE*`, `OPENCODE_STORY`, `OPENCODE_ROUTE`, `OPENCODE_FAST_BOOT`, `OPENCODE_PTY_*`, native-binding path overrides, `OPENCODE_TEST_HOME` — developer/runtime internals.
- SSH askpass mode (`OPENCODE_SSH_ASKPASS_*`) — dead (issuer deleted).
- `OPENCODE_DISABLE_AUTOUPDATE` — read nowhere in 0.5.1; stays a no-op.
- Homebrew/WinGet/vp detection branches — never true for Anysola.
- `upstream.yml` merge workflow, `check-process-daemon.yml`, `script/anysola-deleted.txt` — fork maintenance only.
- `patch` tool (apply_patch grammar) — only for `gpt-` model ids; never offered for Anysola models.
- `experimental_image_generate` — needs an image model; the gateway has none.
- `experimental.artifacts` tools and `<data>/artifacts` — need a provider that never ships.
- LSP runtime/tool/diagnostics (`lsp` key accepted and ignored) — no LSP runtime in 2.2.5.
- Must-read-before-edit (FileTime) — not in 2.2.5.
- `doom_loop` permission (accepted in v1 config, never raised) — dead.
- Exa/Firecrawl/Parallel/Tavily/TinyFish web search providers and the provider-choice form — removed in the fork.
- Built-in skills (OpenCode docs / issue report) — removed in the fork.
- `list`/`ls`, `multiedit`, `batch`, `todoread` tools — do not exist in 2.2.5.
- Non-Anthropic prompt variants (gpt, gpt-astra, kimi, trinity, meta), verbosity plugin, OpenAI/Anthropic tools-trim plugins — never match Anysola models.
- `summary` agent — defined, never called.
- Tool-output pruning — not in 2.2.5.
- Native provider compaction (`context_management`), `speed`, manual `budget_tokens` thinking, `temperature`/`top_p` on the wire, agent `request.headers/body` — inert or gateway-refused.
- CLAUDE.md / CONTEXT.md as instruction files — never read by 2.2.5.
- `.claude/agents`, `.agents/agents` agent/command dirs — not sources in 0.5.1.
- `@file` expansion inside command templates — not in 0.5.1.
- Session sharing (`share_url`, share/unshare keybinds) — removed upstream.
- `event` table persistence, `blob`/`blob_ref` storage (`experimental.media_blob`), `workspace`, `account`/`account_state`/`control_account`, `project_directory` tables — dead locally; import still resolves blob refs.
- V1 → V2 in-place data import of upstream OpenCode v1 tables and `auth.json` credential migration — Anysola dbs are v2 from the start (M3 skips v1 tables).
- Workspaces with remote drivers, voice sessions, `plugin-browser`, `http-recorder`, simulation backend, `ffi-rs` — dead.
- Client-attached WebSocket MCP (`client-mcp` / "TUI tools" experiment) and the `new-designs` experiment (`opencode2` theme, rail autocomplete, `Free models` dialog) — hidden upstream design previews behind the devtools bar.
- MCP SSE transport — not supported in 0.5.1.
- Mini frontend and `mini.*` config keys — command deleted; keys accepted and ignored.
- Release highlights dialog — highlights list always empty.
- OpenCode Go upsell, free-limit notices, Console usage dialog, `$x left`/`Limit reached` label, model picker `Usage ctrl+u` — OpenCode Console only.
- Sidebar `$ spent` line and footer `$cost` — Anysola models carry no price (cost always 0).
- Frecency file and ranking — written but never affects ordering in 0.5.1.
- `No goal is set` toast — unreachable.
- `sound_pack` setting (accepted, ignored), v1 `opencode.json` theme asset, old TUI theme resolver — dead.
- Upstream sidebar "Modified files" and LSP sections — not in 2.2.5.
- Dead keybind ids (`app.clear`, `app.scrap`, `server.pair`, `usage.show`, tab history, share, deprecated diff ids, `which-key.*`, `model.dialog.usage`) — accepted in config, bound to nothing.
- OpenTUI debug overlay and console (`app.debug`, `app.console`) — OpenTUI internals; the commands are accepted and do nothing.
- `remaining`/`credits_per_turn_estimate` gateway-side gaps — not a client feature; client keeps reading them.
- Identical baseline archives as a "bug" — kept deliberately (decision 8).

---

## Open questions

Resolved 2026-11-04:
1. `--version` keeps `Anysola Code <ver> (OpenCode 2.2.5)` for 1.0 (contract, and it credits the MIT origin).
2. `mid-conversation-output-config-2026-07-01` was added to the live `allowedBetas` on 2026-11-03; the M6 release
   checklist re-checks it.
3. Highlighting: syntect + two-face extra syntaxes; tree-sitter only if a 0.5.1 language is still missing.
4. webfetch ports 0.5.1's HTML-to-Markdown converter (no htmd). grep/glob use the ripgrep crates; accepted
   differences: results sorted by path, regex error wording.
5. Linux builds are static musl (needed for the duplicate asset names).

## Acceptance tests

Shared normalization for every byte comparison against 0.5.1 captures: session ids (`ses_…`), message/part/permission
ids (`msg_…`, `prt_…`, `per_…`, `psv_…`, `sh_…`, `tool_…`), `traceparent`/`b3`, `timestamp`/`time.*` values, the date in
`Today's date:`, absolute paths and home prefix, data-dir paths (`~/.local/share/opencode/…` → `~/.anysola/…`), base64
payloads, `x-anysola-version`/UA version (`0.5.1` → `<ver>`), and the fixed-bug deltas listed per test.

### M1
- Request replay: for each `V/session/requests/*.json` and `V/tools/_captured-request-*.json` scenario, drive the
  same prompt/config through `run --standalone` against testkit and diff headers and body byte-for-byte after
  normalization (key order included).
- Tool golden tests: every output/error text and schema in `V/tools/*.md` (read paging, edit tiers, shell notices,
  grep/glob formats with the documented deltas (path-sorted order; regex-crate wording after
  `Invalid regex pattern: `), webfetch markdown of the captured pages, websearch error
  matrix 401/402/429/500/bad JSON/empty/multi, execute results).
- Shell permission scanner: corpus generated by running the fork's `ShellParse.scan` over a command list; resources,
  save patterns and opaque reasons must match.
- `run` output: replay every `V/cli/run/*.cmd` scenario; stdout and stderr equal `.stdout`/`.stderr` after
  normalization, except the §B1/§B2/§B14/§B15 deltas; exit codes equal.
- stdin rule: no-arg + closed pipe, no-arg + slow pipe (5 s), arg + idle open pipe (returns after 3 s with the note),
  arg + `/dev/null`, arg + piped data.
- Credits/budget: SSE streams with `anysola_credits`, concurrent title/main requests, `--budget` interrupt, budget
  already reached, 402 with and without the URL in the message, retry schedule with `retry-after` (testkit clock).
- Compaction: overflow path equals `compaction-overflow-request.json` then `after-compaction-checkpoint.json`;
  auto path at the ceiling; manual path.
- Discovery bound: temp home with `~/AGENTS.md`, `~/.claude/skills`, `/tmp/x/opencode.json` above a project outside
  home, and a project under home with a parent above home; only the decision-3 set is read.
- Code-action: run the action's `scripts/lib.sh` invocation (review and issue modes) against testkit with
  `ANYSOLA_TOKEN`; `events.jsonl` parses with the action's jq filters, answer extraction works, `Budget reached` is the
  last error on a budget run, review mode cannot edit, issue mode edits.
- Multi-process: TUI-less second `run -s <id>` on a busy session gets the busy error; two processes writing different
  sessions concurrently.

### M2
- Each command against testkit: output equals `V/cli/account-commands.txt`, `V/cli/update-uninstall.txt`,
  `V/cli/other-commands.txt` except §B6/§B7/§B13 deltas; `--help` output contains every flag of `V/cli/help/*.txt` for
  kept commands; `--version` in every argv position; parse-error format of `V/cli/framework-errors.txt`.
- Device flow with `slow_down`, `expired_token`, `access_denied`, 3×5xx, malformed token.
- `mcp add` writes the same file bytes as 0.5.1 for JSON and JSONC inputs (comment preserved).
- `--completions` script loads without error in bash, zsh, fish, pwsh.

### M3
- Import the two databases behind `V/storage/example-rows.observed.json` plus a synthetic db with `session_part`
  overlay rows, legacy (`providerState`) rows and a blob ref; every session/message/part round-trips (JSON-equal
  modulo upgrades); continuing the rich session produces a request whose thinking blocks equal
  `thinking-replay-next-turn.json`'s shape with the original signatures.
- Old files' bytes and mtimes unchanged after import (hash before/after); import marker prevents a second import;
  a failed import (corrupted db) writes nothing and warns.
- Token, budgets, always-allow rows, `cli.json` → `tui.json`, prompt history, tabs, model recents present after import;
  revert of an imported message restores a file through the alternates repo.

### M4
- ratatui test-backend text snapshots at the captured sizes equal every `V/tui/screens/*.txt` (text only, colours out of
  scope) after normalization and the §B9/§B10 deltas; the PNGs in `screens/png/` are a visual reference only.
- pty smoke test (like `testing/tui-*.sh`): start, prompt, permission prompt, `/credits`, `/usage`, `/budget 5`,
  `/undo`, exit summary.
- Keymap: every default of `V/tui/keybinds-default.md` resolves to the listed command; `<leader>q` exits.
- Theme: resolved colours of `anysola` dark/light equal `V/tui/theme-anysola.md`.

### M5
- Fake stdio MCP server: system block 2 equals `V/integrations/codemode-catalog-with-mcp.txt`; tool call results,
  list_changed refresh, malformed-tool drop, process-exit status texts as in `V/integrations/mcp-observed-output.txt`.
- Fake OAuth MCP server in DCR and CIMD variants: `/register` body, authorize URL parameters, callback pages equal the
  captured HTML, credential row shape, refresh coalescing.
- Streamable HTTP: `?codemode=false` retry, session-expiry recovery, transient connect retries.

### M6
- Release dry run produces exactly the asset list of `V/cli/release.txt` (names, single-entry archives, modes, mtimes,
  duplicate pairs byte-identical, `SHA256SUMS` format).
- install-test matrix green; npm install on each OS/arch incl. Alpine (musl) and Windows ARM64 with x64 Node.
- `script/budget.sh` within budgets on linux-x64.
- 0.5.1 → 1.0.0 upgrade via `anysola update` (script) and npm; the first 1.0 start imports the data.

### M7
- Replay `V/integrations/acp-transcript-session.log` and `acp-transcript-permission.log` as a scripted client;
  responses and notifications equal the transcript after normalization (`agentInfo.version` → `<ver>`).

---

## Counts

| Milestone | Items |
|---|---|
| M1 core + headless `run` | 161 |
| M2 account/utility CLI | 23 |
| M3 migration | 17 |
| M4 TUI | 82 |
| M5 MCP + MCP in Code Mode | 17 |
| M6 packaging/CI/release/install | 11 |
| M7 ACP | 9 |
| Total | 320 |

Not carried over: 55 entries. Deliberate fixes (§B): 21.
