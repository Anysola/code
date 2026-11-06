# Integrations and remaining subsystems: parity inventory (Anysola Code 0.5.1)

Source: fork `/home/ubuntu/anysola-code/build/fork` (branch `anysola`, OpenCode v2.2.5 + Anysola commits). Paths below are
relative to the fork. Tags: `[0.5.1]` = exercised by the shipped binary; `[0.5.1 opt-in]` = reachable only by an
explicit config/env switch that defaults off; `[dead]` = not reachable by a user of the shipped binary (reason given).
Runtime checks used the linux-arm64 0.5.1 binary in `testing/runs/integrations` with a fake MCP stdio server, a fake
OAuth MCP server (DCR and CIMD variants) and a capturing proxy in front of the fake gateway.

Verbatim material: `spec/verbatim/integrations/`
- `strings.txt` (every string with file:line), `mcp-observed-output.txt` (exact CLI output, wire bodies, DB row),
  `cli-help.txt` (`mcp`, `acp`, `plugin` help), `codemode-catalog-with-mcp.txt` (system prompt section with an MCP server),
  `oauth-callback-page.ts` + two rendered HTML pages, `cimd-client.json`, `acp-transcript-*.log`,
  `plugin-list-builtin.txt`.

Cross-cutting quirk seen twice: `anysola mcp list` and `anysola plugin list` answer before the location's plugins have
activated, so the first call on a cold service (and every `--standalone` call) prints `No MCP servers configured` /
`No plugins found`. Rust must decide: replicate, or wait for initial activation (recommended; nobody depends on the race).

Error output of plain CLI handlers (e.g. `mcp add`, `mcp logout`) is `Error: <message>` followed by Bun stack lines on
stderr, exit 1. Only the first line is meaningful; Rust should print `Error: <message>`.

---

## 1. MCP

SDK: `@modelcontextprotocol/client|core|server` 2.3.1 (`packages/core/package.json:98-100`). Client identity sent in
`initialize`: `{"name":"opencode","version":"2.2.5+anysola.0.5.1"}` (`packages/server/src/routes.ts:144-150`); HTTP
User-Agent `opencode/latest/2.2.5/cli`. Default protocol revision offered: `2025-11-25`.
`[1.0.1]` `clientInfo {name: "anysola-code", version: <ver>}`, User-Agent `anysola-code/<ver>`.

### 1.1 Config format [0.5.1]
- [ ] Native (v2) shape: `"mcp": { "timeout": {startup,catalog,execution}, "servers": { "<name>": Server } }`
  (`packages/schema/src/config/mcp.ts:19-22`).
- [ ] Local server: `{type:"local", command:string[], cwd?, environment?:{k:v}, disabled?, codemode?, timeout?, protocol?}`
  (`packages/schema/src/mcp.ts:26-39`). `cwd` relative to the location directory; default = location directory
  (`mcp/client.ts:305`). Env = host env + `environment` (+ `BUN_BE_BUN=1` when command is `opencode`, `client.ts:307`).
- [ ] Remote server: `{type:"remote", url, headers?, oauth?: OAuthConfig | false, disabled?, codemode?, timeout?, protocol?}`
  (`mcp.ts:53-64`). OAuth config: `client_id, client_secret, scope, callback_port (1..65535), redirect_uri,
  auth_server_metadata_url` (`mcp.ts:41-51`).
- [ ] `protocol`: `"legacy"` (default: plain initialize, revisions up to 2025-11-25) | `"auto"` (probe 2026-07-28, fall
  back) | `"2026-07-28"` (required) (`mcp.ts:19-24`, `client.ts:479-483`).
- [ ] Legacy v1 shape still accepted and migrated (`config/normalize.ts:250-319`, `v1/config/mcp.ts`,
  `v1/config/migrate.ts:77-108`): `"mcp": {"<name>": {type, command, environment, enabled, timeout:number, oauth:{clientId,
  clientSecret, scope, callbackPort, redirectUri}}}`; `enabled:false` -> `disabled:true`; numeric `timeout` -> all three
  timeouts; `experimental.mcp_timeout` -> catalog+execution only; an entry with only `enabled` is dropped with a diagnostic;
  native wins over legacy on conflict. Names `servers`/`timeout` are treated as native keys unless the value has a `type`.
- [ ] Merge across config documents (`config/plugin/mcp.ts:38-56`): global timeouts `Object.assign` in config order, per
  server `{...global, ...server.timeout}`; later document's server of the same name replaces earlier.
- [ ] Config file changes reload servers live (observed: `mcp add` while the service runs picks the server up).
  Reconcile compares by deep equality, restarts only changed servers (`mcp/index.ts:665-709`).
- [ ] Defaults plugin (`plugin/mcp-codemode-defaults.ts`): `executor mcp` local servers and `https://executor.sh/<x>/mcp`
  get `codemode:false`; `mcp(.us|.eu|-eu)?.posthog.com` gets header `x-posthog-mcp-mode: tools` unless set.
- [ ] Policy: `policies` statements with `mcp.use` can deny a server (`config/plugin/policy.ts`, `ctx.mcp.transform`).

### 1.2 Transports
- [ ] stdio [0.5.1] (`mcp/stdio.ts`): newline-delimited JSON; strips trailing `\r`; blank lines ignored; frame limit 16 MiB
  (error + close); stderr drained to debug log, last 1000 chars kept for the exit message; on close: wait 2 s for exit after
  stdin end, SIGTERM, SIGKILL after 2 s more; whole process group killed. Unexpected exit -> `MCP server process exited with
  code N: <stderr tail>`.
- [ ] Streamable HTTP [0.5.1] (`client.ts:313-345`): adds `?codemode=false` to the URL unless the server's `codemode` is
  false, URL already has `codemode`, or host is `*.googleapis.com`, `zohomcp.*`, localhost/127.x/[::1] (`client.ts:467-476`);
  on HTTP 400/404 with the added param, one retry at the configured URL. Connect retries: 2 more attempts (250 ms, 1 s)
  for transient errors (408, 429, 5xx except 501, network TypeError) until the startup deadline. Concurrent startups to
  the same URL are serialized (`index.ts:109,482`). Legacy session expiry (404, or 400 "Server not initialized" while a
  session id is held) reconnects and retries the request once (`client.ts:279-295`, `index.ts:367-394`). On shutdown
  sends DELETE (terminateSession) with 1 s timeout.
- [ ] SSE transport: **not supported** (no SSEClientTransport anywhere; ACP advertises `sse:false`). Rust: skip.
- [ ] WebSocket "attached" client MCP [0.5.1 opt-in]: server route `mcp.client` (`packages/server/src/handlers/mcp.ts:91-157`,
  `mcp/socket-client.ts`) lets a client host an MCP server; only the TUI experiment `client-mcp` uses it (server name
  `tui`, tools `open_terminal`, `write_terminal`; `packages/tui/src/context/client-mcp.tsx`). Experiment off by default
  (`tui config experimental["client-mcp"]`), reachable only through the devtools bar (`debug.devtools`). Defer.
- [ ] Client capabilities sent: `roots:{}` always (answers `roots/list` with `[{uri: file://<directory>}]`), `elicitation
  {form:{}, url:{}}` (`client.ts:212-240`).
- [ ] Malformed tools in `tools/list` are dropped one by one with a warning, the rest are kept (observed: tool with
  `inputSchema.type:"string"` skipped) (`client.ts:242-266`).
- [ ] Capability gating: tools/prompts/resources are only listed if the server advertises them (`client.ts:384-415`).

### 1.3 Timeouts [0.5.1]
- [ ] startup 30 000 ms, catalog 30 000 ms, execution 12 h; attached clients 10 min (`client.ts:35-39`). Tool calls request
  progress so long calls are kept alive; execution is the hard wall.

### 1.4 Lifecycle and status [0.5.1]
- [ ] Status union: `connected | pending | disabled | failed{error} | needs_auth{error}` (`schema/src/mcp.ts:69-88`).
- [ ] Servers start asynchronously on location boot; disabled ones go straight to `disabled` (`index.ts:665-695`).
- [ ] Tools are listed as part of connect; a listing failure marks the server failed (`index.ts:466-506`).
- [ ] Connection closed -> `failed` with `Connection closed[: cause]`. **No automatic retry** of failed servers (observed:
  a server that came up later stayed failed); recovery is `/mcps` retry, credential change, or config change.
- [ ] `notifications/*/list_changed` refresh tools/prompts/resources (tools debounced 100 ms, `tool/mcp.ts:132-142`).
- [ ] Credential switch for a server's integration restarts it (OAuth login takes effect without restart, `index.ts:713-732`).

### 1.5 How MCP tools reach the model [0.5.1] (verified by request capture)
- [ ] Default `codemode` true: MCP tools are **not** direct tools. They appear only in the system-prompt "Code Mode"
  catalog as `tools.<namespace>.<tool>(...)` and are invoked through the single direct tool `execute` (JS code). See
  `verbatim/integrations/codemode-catalog-with-mcp.txt`. Namespace = server name with `[^a-zA-Z0-9_-]` -> `_`
  (`tool/mcp.ts:22`); tool path segment = tool name sanitized the same way (`get.weather` -> `get_weather`;
  `tool/runtime.ts:287`). Catalog line: `tools.<ns>.<tool>(<TS-like input>): Promise<unknown> // <description>`.
- [ ] `codemode:false` (or `execute` wholly denied by permissions) -> direct tool named `<ns>_<tool>`
  (`tool/runtime.ts:289-292`, `tool.ts:274-279`); description passthrough (`""` if absent); input schema passthrough
  (default `{type:"object",properties:{}}`).
- [ ] Permission action per MCP tool: `<ns>_<tool>` with resource `*` (`tool/mcp.ts:58-70`). Default rules allow it
  (observed: no prompt in `run`).
- [ ] Server `instructions` (from initialize) rendered in system prompt inside `<mcp_instructions>` only if the session can
  reach at least one of the server's tools; delta texts when servers come/go (`mcp/instructions.ts`).
- [ ] A server connecting after the session started yields an appended `role:"system"` message "The Code Mode tool catalog
  has changed. ..." (observed; exact text in `mcp-observed-output.txt`).
- [ ] Call: `_meta: {"ai.opencode/sessionID": <session>}` (+ SDK `progressToken`) on `tools/call` (`client.ts:432`).
  `[1.0.1]` `ai.anysola/sessionID`: neither the gateway nor any spec relies on the old key.
- [ ] Result mapping (`client.ts:485-504`, `tool/mcp.ts:85-163`): `isError` -> tool failure with joined text; text parts
  joined by `\n`; `structuredContent` wins as output; text starting `{`/`[` parsed as JSON when no outputSchema; images /
  audio / blobs with a read-tool media mime sent inline as files; other textual mimes inlined as
  `[Resource …]`; other binaries saved to tool-output dir with `[Binary resource …]`; `resource_link` -> its URI text.
- [ ] Resources: two Code-Mode tools in namespace `opencode`: `list_mcp_resources({server?})`, `read_mcp_resource({server,
  uri})` (`tool/plugin/mcp-resource.ts`), always registered even with no MCP server (observed in catalog). Permission
  actions `opencode_list_mcp_resources` (resources = server names) and `opencode_read_mcp_resource` (resource
  `<server>:<uri>`, save `<server>:*`).
- [ ] Prompts: every MCP prompt becomes a slash command `<server>:<prompt>` (sanitized), args positional (quoted strings
  supported, `[Image N]` tokens kept), result messages' text joined by `\n` and sent as the user prompt
  (`plugin/command.ts:55-113`). Visible in TUI `/` list and in ACP `available_commands_update` (observed `testsrv:greet`).
  `anysola run "/x:y"` does **not** expand commands (sends text).
- [ ] Elicitation (form + URL) -> `Form` asked under the sentinel session `global`, title `<server> is requesting input`;
  JSON-schema properties mapped to form fields (`index.ts:260-331,955-1015`). Shown by TUI form UI / ACP elicitation.

### 1.6 OAuth [0.5.1] (verified end to end)
- [ ] Every remote server without `oauth:false` is registered as an integration `mcp_<sha1(name+"\0"+url)[0..16]>`, method
  type `oauth`, label = server name (`index.ts:206-245`). Anonymous servers just never use it.
- [ ] Connect uses stored tokens; with no tokens a 401 ends as `needs_auth` (connect never opens a browser,
  `oauth.ts:141,158,198,205`).
- [ ] `authorize` (`oauth.ts:335-546`), runs **inside the server process**:
  1. Loopback callback server on `127.0.0.1`, port = `oauth.callback_port` ?? port of `redirect_uri` ?? ephemeral (0);
     path = `redirect_uri` path ?? `/callback`. (The v1 doc string "default 19876, /mcp/oauth/callback" is stale; observed
     ephemeral port + `/callback`.)
  2. POST initialize probe (5 s) to read `WWW-Authenticate` (`resource_metadata`, `scope`).
  3. Discovery: `auth_server_metadata_url` if set, else RFC 9728 / RFC 8414 discovery.
  4. Client registration: static `client_id` if configured; else **CIMD** when server metadata has
     `client_id_metadata_document_supported:true` and `token_endpoint_auth_methods_supported` includes `none` and no
     `redirect_uri` configured -> `client_id=https://www.anysola.com/oauth/anysola-code/client.json` (no /register);
     else **DCR** POST /register with body in `mcp-observed-output.txt` (`client_name "Anysola Code"`, `client_uri
     https://www.anysola.com/code`, `application_type native`, auth method `none` or `client_secret_post` with secret).
  5. PKCE S256, `state` = 32 random bytes base64url, `scope` = configured ?? challenge scope, `resource` = configured URL
     (query/fragment stripped as identity).
  6. Returns `{url, instructions:"Authorize <name> in your browser. This window will close automatically.", mode:"auto"}`.
     Callback: wrong path 404; no params -> 400 error page and keep waiting; state mismatch / missing code -> 400 page and
     fail; success -> token exchange, then 200 success page (page waits for the exchange).
- [ ] Token storage: SQLite `credential` table in `~/.local/share/opencode/opencode.db` (Windows: the data dir from
  storage spec), `value` JSON `{type:"oauth", methodID, access, refresh, expires(ms|0), metadata:{serverUrl, tokenType,
  scope?, issuer?, client:{registered client}}}` (`oauth.ts:243-275`). Refresh rotates in place; concurrent refreshes of
  the same token are coalesced (`oauth.ts:34-58`); invalidation removes the row only if it still holds the presented
  refresh token. Credentials survive server removal (keyed by name+URL).
- [ ] CIMD document hosted by Anysola (`cimd-client.json`) lists redirect URIs `http://127.0.0.1/callback`,
  `http://localhost/callback`, `http://127.0.0.1/mcp/oauth/callback` (port-agnostic loopback).
- [ ] Callback HTML: `oauth/page.ts` (Anysola wordmark, light/dark CSS, auto-close 2.5 s). Port verbatim.
- [ ] Windows: nothing platform-specific; browser opened by `@opencode/util/open` (best effort, only when stdin and stdout
  are TTYs).

### 1.7 `anysola mcp` subcommands [0.5.1] (no `debug`, `connect`, `remove` subcommands exist)
- [ ] `mcp list [--standalone|--server URL]`: via server API; sorted by name; format in strings.txt. Race noted above.
- [ ] `mcp add <name> [--url U] [--header k=v]... [--env k=v]... [--global] [-- command args...]`: does not need the
  server; reads `--` operands from raw argv; validates (strings.txt); writes `mcp.servers.<name>` (native shape) with
  `jsonc-parser` `modify` (2-space indent, keeps comments) into the first existing of `opencode.json`, `opencode.jsonc`,
  `.opencode/opencode.json`, `.opencode/opencode.jsonc` in cwd (or global config dir with `--global`), creating
  `opencode.json` if none (`handlers/mcp/add.ts:40-68`). Prints `MCP server "<name>" added to <path>`.
  `[1.0.1]` Candidates `anysola.json`, `anysola.jsonc`, `.anysola/anysola.json[c]`, then the `opencode` ones;
  creates `anysola.json` if none.
- [ ] `mcp auth [name]`: clack UI; without a name needs a TTY and shows a picker of OAuth-capable servers; existing
  credentials prompt "Re-authenticate?" (TTY only); runs the integration OAuth attempt, prints instructions + URL, opens
  browser on TTY, polls status every 500 ms; on success removes the previous credentials (`handlers/mcp/auth.ts`,
  `auth/login.ts:155-268`). Ctrl+C cancels the attempt (finalizer).
- [ ] `mcp logout <name>`: removes all credential connections of the server's integration.
- [ ] Exit codes: 0 success, 1 failure, 130 cancelled prompt.

### 1.8 TUI MCP surface [0.5.1] (strings in strings.txt)
- [ ] `/mcps` dialog: list with status, search, `space` toggles (connect/disconnect/retry/sign in), `enter` shows error
  details or starts sign-in (integration dialog with autoConnect) for `needs_auth`.
- [ ] Toast once per state change for `needs_auth` / `failed`.
- [ ] Home footer `⊙ N MCP` / `⊙ N MCP failed` + `/mcps`; sidebar MCP section per session.
- [ ] Disconnect from the dialog sets status `disabled` for the life of the location (not persisted to config).

---

## 2. ACP (`anysola acp`) [0.5.1]

SDK `@agentclientprotocol/sdk` 1.6.0 (`packages/cli/package.json:27`). Verified with a stdio client
(`acp-transcript-*.log`).
- [ ] Transport: ndjson JSON-RPC 2.0 on stdin/stdout; starts a private standalone server (`Standalone.start()`), sets
  `OPENCODE_CLIENT=acp` (`handlers/acp.ts:21-32`). Exits 0 when the client closes; message size limit error and server-exit
  error go to stderr as `anysola acp: …` with exit 1.
- [ ] `--login`: runs the Anysola device sign-in (`anysola login`) instead of the server (`handlers/acp.ts:16-20`).
- [ ] `initialize` response (exact in transcript): `protocolVersion:1`, `agentCapabilities{loadSession:true,
  mcpCapabilities{http:true,sse:false}, promptCapabilities{embeddedContext:true,image:true}, sessionCapabilities
  {additionalDirectories,close,delete,fork,list,resume}, _meta{"opencode/child-session-updates":true}}`, one auth method
  `opencode-login` (see strings), `agentInfo{name:"Anysola Code",version:"0.5.1"}` (`acp/service.ts:106-131`).
  Client capabilities read: `elicitation.form`, `session.compaction`, `_meta["opencode/child-session-updates"]`.
- [ ] Requests: `initialize, authenticate (only methodId opencode-login, returns {}), session/new, session/load,
  session/list (cwd filter, desc, limit 100, cursor), session/delete, session/resume, session/close, session/fork,
  session/set_config_option, session/set_mode, session/prompt`; notification `session/cancel` (`acp/agent.ts:68-119`).
- [ ] Session mapping: ACP sessionId = Anysola session id (`ses_…`); `cwd` must be absolute and must equal the session's
  directory for load/resume/fork; `additionalDirectories` become directory grants; `mcpServers` (stdio and `http` only,
  others rejected) are added to the location's MCP set (`client.experimental.mcp.add`, stdio -> local, http -> remote with
  `oauth:false`) and persist for the life of the process (`acp/sessions.ts:93-209`). Note: they are location-scoped, so
  every session in that directory sees them.
- [ ] Config options: `model` (values `anysola/<id>`, names `anysola/<Name>`), `effort` (model variants + `default`;
  observed `low medium high xhigh max economy default`), `mode` (primary agents; observed build, plan)
  (`acp/config-option.ts`). Updates pushed as `config_option_update`; commands as `available_commands_update` (config
  commands + MCP prompts + `goal` + builtin `compact`).
- [ ] Prompt turn updates: `user_message_chunk` (replay), `agent_message_chunk`, `agent_thought_chunk`, `tool_call`
  (`kind`: shell->execute, webfetch->fetch, edit/patch/write->edit, grep/glob->search, read->read, subagent->think, else
  other; `acp/tool.ts:16-39`), `tool_call_update` (in_progress/completed/failed, diffs for edits), `usage_update
  {used,size,cost{amount:0,currency:"USD"}}`. Response `{stopReason, usage{inputTokens,outputTokens,totalTokens
  [,thoughtTokens,cachedReadTokens,cachedWriteTokens]}, _meta}`. stopReason: cancelled (interrupt/abort/rejected
  permission), max_tokens (finish length), refusal (content filter), else end_turn (`acp/translate.ts:470-479`).
- [ ] Permissions: core `permission.asked` -> `session/request_permission` with options once/always/reject (strings);
  title = path / url / query / pattern / "N files"; diff previews for edits; reply mapped back; reject -> tool fails "The
  user declined this tool call", turn ends `cancelled` (observed) (`acp/permission.ts`).
- [ ] Elicitation: core forms -> ACP `elicitation/create` when the client supports form elicitation (`acp/elicitation.ts`).
- [ ] Errors: strings.txt (JSON-RPC -32602 with `Invalid params: …`).
- [ ] Anysola specifics: none beyond branding and `--login`; credits/budget are not surfaced over ACP (no Anysola code in
  `acp/`). Evidence of use: only the 0.5.0 changelog line about `--login`; README does not mention ACP or Zed. Ship it in
  v1 only if cheap; otherwise defer (see table).

---

## 3. LSP [dead]

- [ ] v2.2.5 has **no LSP runtime**: only the config schema (`packages/schema/src/config/lsp.ts`: `lsp: boolean |
  {<name>: {disabled:true} | {command[], extensions?, disabled?, env?, initialization?}}`), the normalizer
  (`config/normalize.ts:576-582`), an `lsp.updated` event type, and TODOs (`file-mutation.ts:131`,
  `tool/plugin/edit.ts:258`, `tool/plugin/write.ts:44`). No language servers are detected or downloaded, no diagnostics are
  fed to the model, no `lsp` tool is registered (observed tool list). TUI only has a permission renderer for an `lsp`
  action (`tui/src/util/permission.ts:104`). Rust: accept and ignore the `lsp` key; nothing else.

## 4. Formatters [0.5.1 opt-in]

- [ ] **Off unless config mentions `formatter`** (`config/plugin/formatter.ts:29-31`): no config -> no formatting
  (verified). `formatter: true` enables built-ins; `formatter: false` disables; an object enables built-ins and merges
  per-name entries `{disabled?, command?[], environment?, extensions?}`; boolean in a later document resets earlier
  objects (`formatter.ts:62-76`). Custom formatter verified (`$FILE` placeholder replaced in each argv element).
- [ ] Runs after `edit`, `write`, `patch` succeed, on the absolute path (`tool/plugin/edit.ts:370`, `write.ts:89`,
  `patch.ts:288`); first formatter whose extension list contains `path.extname(file)` and exits 0 wins (in list order);
  failures are logged only; stdin/stdout/stderr ignored; cwd = location directory; BOM restored afterwards; edit's
  returned diff reflects the formatted text. Commands are cached after first detection.
- [ ] Built-ins in order (`formatter/builtins.ts:241-268`) with detection: gofmt(.go, PATH) `-w`; mix(.ex .exs .eex .heex
  .leex .neex .sface) `format`; oxfmt(js/ts family, package.json dep `oxfmt` + npm bin); prettier(js/ts/html/css/vue/
  svelte/json/yaml/toml/xml/md/mdx/graphql, package.json dep `prettier` + npm bin) `--write`; biome(same set,
  biome.json[c] found + `@biomejs/biome` bin) `format --write`; zig(.zig .zon) `fmt`; clang-format(C/C++ ext, `.clang-format`
  found) `-i`; ktlint(.kt .kts) `-F`; ruff(.py .pyi, ruff on PATH and ruff config or `[tool.ruff]` or "ruff" in
  requirements/pyproject/Pipfile) `format`; air(.R, `air --help` first line mentions R formatter) `format`; uv(.py .pyi,
  `uv format --help` exits 0) `format --`; rubocop `--autocorrect`; standardrb `--fix`; htmlbeautifier(.erb; `.html.erb`
  never matches because extname is `.erb`); dart `format`; ocamlformat(`.ocamlformat` found) `-i`; terraform(.tf .tfvars)
  `fmt`; latexindent(.tex) `-w -s`; gleam `format`; shfmt(.sh .bash) `-w`; nixfmt; rustfmt(.rs); pint(.php,
  composer.json requires laravel/pint) `./vendor/bin/pint`; ormolu(.hs) `-i`; cljfmt `fix --quiet`; dfmt(.d) `-i`.
  `findUp` searches from the location directory up to the worktree root. npm-based ones get env `BUN_BE_BUN=1`.
- [ ] Executable lookup: `which` over PATH plus the global `bin` dir (`util/which.ts`). Windows: honours `PATHEXT`.

## 5. File watching (@parcel/watcher 2.5.1) [0.5.1]

- [ ] Backends: inotify (Linux), fs-events (macOS), windows (Windows) for recursive directories; `fs.watch` for single
  files/directory entries (`filesystem/watcher.ts:23-28,58`). Unsupported platform -> error log, streams end.
- [ ] What is watched (observed in logs): config files and config dirs, skill dirs (`~/.config/opencode/skill(s)`,
  `~/.claude/skills`, `~/.agents/skills`), `AGENTS.md` candidates, directory entries of every ancestor of the cwd (for
  instruction/config discovery), local plugin sources, and the VCS head (`.git/HEAD` or `.hg/branch`) for branch changes
  (`filesystem/location-watcher.ts:33-80`). The project tree itself is **not** watched recursively in 0.5.1.
- [ ] Ignore: config `watcher.ignore: string[]` (`schema/config/watcher.ts`), applied to the location watcher (an entry
  matching `.git`/the git dir turns off HEAD watching) and to recursive subscriptions.
- [ ] `FileWatch` API (per-file watches requested by clients, TTL 24 h, sweep 5 min, max 256 per location;
  `filesystem/file-watch.ts:15-17`).
- [ ] Disable: `OPENCODE_DISABLE_FILEWATCHER` / `OPENCODE_FILEWATCHER_DISABLE` (`cli/src/server-process.ts:140`,
  `tui/src/util/watch.ts:11`).
- [ ] Rust: `notify` crate is enough; needed for config/skill/AGENTS.md hot reload and branch display.

## 6. Plugin system

### 6.1 What a plugin is in 0.5.1 [0.5.1]
- [ ] Module with default export `{id, effect(ctx)}` (Effect API) or `{id, setup(ctx)}` (Promise API); anything else fails
  with "Plugin must export a default definition…" (`core/src/plugin/module.ts:75-187`). Classic OpenCode v1 hook objects
  are not supported.
- [ ] Sources (`config/plugin/source.ts:113-160`, `plugin/source-directory.ts:7`): every config directory's `plugin/` and
  `plugins/` (global `~/.config/opencode/`, project `.opencode/`): `.ts`/`.js` files and directories; config `plugins`
  (v2) or `plugin` (v1, `[name, options]` tuples migrated) entries: npm spec (installed with npm into the cache), `file://`
  or `./` paths (must be directories); `-<id>` / `-*` / `-prefix.*` removes plugins except guarded ones
  (`anysola.provider`, `anysola.account`, `anysola.provider-lock`, `opencode.config.policy`; `plugin/internal.ts:259-264`).
  `{package, options}` passes options. Policy `plugin.use` deny blocks a target. Sources are watched and hot-reloaded.
  Verified: a `.opencode/plugins/test-plugin.ts` loaded; its `session.hook("context")` edit reached the request;
  its provider edit was undone by the lock; a `tool.transform` add did not show up in the model's tools (silently).
- [ ] `anysola plugin list [--builtin] | add <pkg> | check [target] | update [target] | remove <target>`
  (`commands.ts`), npm registry from npm config (default `https://registry.npmjs.org`, `util/src/npm-config.ts:38`).
- [ ] Context domains (`packages/plugin/src/effect/plugin.ts:27-60`): app, location, options, agent, aisdk, command,
  credential.activate, event, experimental{terminal.read, generate, artifact}, integration, mcp, model, permission, plugin.list,
  provider, reference, rpc, session, shell, skill, storage, tool, vcs, websearch, worktree.
  Transform hooks: agent, command, integration, mcp, model, provider, reference, skill, tool, vcs, websearch.
  Runtime hooks: session `prompt, context, compaction, generate, title, model.request, http.request, http.response,
  experimental.ws.handshake|send|receive, retry, experimental.idle` (`effect/session.ts:185-199`); tool `execute.before,
  execute.after`; aisdk `sdk, language`; shell `create.before`; permission `evaluate`.
- [ ] TUI plugins are a separate system (TUI agent's area): `<configdir>/plugins/<dir>` and tui config; built-ins include
  `anysola.account` (status line/credits/usage/budget/devices/account/feedback), mermaid and LaTeX renderers.
- [ ] Provider lock (`plugin/provider/anysola.ts:220-245`): last `post` plugin; removes every provider except Anysola and
  restores Anysola's endpoint/package/settings for provider and each model, so no plugin/config can add a provider or move
  the endpoint (verified).

### 6.2 Built-in plugins shipped (58, `plugin-list-builtin.txt`) and the hooks they use
Anysola-specific (must be native in Rust):
- [ ] `anysola.provider` (`plugin/provider/anysola.ts:247-340`): `integration.transform` (integration "Anysola", device
  method + env method `ANYSOLA_TOKEN`), `provider.transform` (pin provider, integrate remote config document),
  `provider.reload`, `integration.connection.{active,status,resolve}`; fetches `GET /v1/code/config`, caches in KV,
  refreshes every 10 min and on credential events. Adds headers `x-anysola-client`, `x-anysola-version`, credits-event
  header.
- [ ] `anysola.provider-lock`: `provider.transform` only.
- [ ] `anysola.account` (`plugin/provider/anysola-account.ts`): `rpc.register` (methods session, budget, budgetDefault, me,
  usage, tokens, revoke, estimates, notes; event `credits`), `session.hook("http.response", {providerID:"anysola"})`
  (credits headers, 402 rewrite), `tool.hook("execute.after")` (websearch credits), `session.hook("experimental.idle")`
  (turn close). Gateway agent owns the protocol details.
- [ ] `anysola.websearch` (`plugin/websearch/anysola.ts`): `websearch.transform`.
Upstream built-ins whose hook usage Rust must reproduce natively (behaviour belongs to other areas):
- [ ] `session.hook("context"|"compaction"|"generate")`: prompt.identity, prompt.verbosity (+`title`), warming, the
  `opencode.prompt.*` optimize plugins, tools opencode/patch/shell/subagent/websearch, goal (+`experimental.idle`), plan.
- [ ] `tool.hook("execute.before"|"execute.after")`: tool.input.repair, subagent (before), plan (after).
- [ ] `aisdk.hook("sdk")`: provider.dynamic (loads the provider SDK package; Rust replaces with a fixed Anthropic-compatible
  client).
- [ ] `permission.hook("evaluate")` + `mcp/skill/provider.transform`: config.policy.
- [ ] Transforms: agent (agent, plan, config.agent, browser), command (command, config.command, goal, schedule), mcp
  (config.mcp, mcp.codemode.defaults, policy), model/provider/integration (config.provider), reference (config.reference),
  skill (config.skill, compatibility, policy, browser), vcs (vcs.git, vcs.hg), websearch (config.websearch).
- [ ] `opencode.browser` registers but exposes nothing without a desktop browser attachment [dead in effect].

### 6.3 Rust parity assessment
Third-party JS plugins cannot run without embedding a JS engine. No Anysola feature depends on third-party plugins;
README/CHANGELOG mention plugins only as restricted ("cannot add a model provider"). Recommendation: v1 implements all
built-ins natively as ordinary code (no plugin host), accepts and ignores `plugin`/`plugins` config with a one-time
warning, and keeps `anysola plugin` commands out or stubbed (decision for the CLI area). The `rpc` channel used by
`anysola.account` becomes an internal API between core and TUI.

---

## 7. Everything else

| Item | Status | What it is / what the user sees | Evidence |
|---|---|---|---|
| `core/src/voice.ts` | [dead] | Realtime voice sessions via OpenAI / OpenCode-hosted GPT-Live; server routes `voice.models`/session only; no TUI/CLI consumer; needs OpenAI integration which Anysola-only build lacks | `voice.ts:61-75`, `server/src/handlers/voice.ts`, consumer was deleted app/desktop |
| `schedule` (+ `tool/plugin/schedule.ts`, `/schedule` command text) | [0.5.1 opt-in] | Scheduled prompts (cron/every/at) waking a session; tools `opencode.schedule_create/list/pause/resume/delete` (Code Mode). Gated by `experimental.schedules: true` (default false); runs only while a server is alive (leader lease) | `tool/plugin/schedule.ts:55,75`, `schema/config/experimental.ts:37-38`, `schedule.ts:129-160,593,668` |
| `worktree` | [0.5.1] | Git worktree create/list/remove as Code Mode tools `opencode.worktree_*` (seen in catalog), `worktree` config | `tool/plugin/opencode.ts:163-215`, `config/plugin/worktree.ts` |
| `workspace` | [dead] | Remote sandbox locations via workspace drivers; no driver is registered in the CLI build | `workspace.ts`, `workspace/driver.ts:64-80` (no `registry(` caller) |
| `pty` / `persistent-pty` / `process-daemon` | [0.5.1] | TUI terminal pane: `<leader>t` toggle, `<leader>down` select, `<leader>up` close, `ctrl+z` suspend (keybind.ts:99-101,288). Backed by the embedded `opencode-pty` Rust daemon (portable-pty + libghostty-vt) from npm `@opencode/pty-<platform>`; plain `pty` uses `bun-pty`. Windows: named pipe; build supports win32. Agent can also read terminals (`experimental.terminal.read`). TUI agent owns UX | `cli/script/build.ts:120-136`, `cli/script/opencode-pty.ts`, `process-daemon/README.md`, `tui/src/context/session-terminals.tsx:69-82`, `core/src/pty/pty.bun.ts:2` |
| `ssh-askpass` (cli) | [dead] | Entry used when `OPENCODE_SSH_ASKPASS_PORT` is set: relays an SSH password prompt to a local socket. Nothing in the shipped code sets that env (issuer was the deleted desktop) | `cli/src/index.ts:43-44`, `cli/src/ssh-askpass.ts` |
| editor integration incl. `editor-zed` | [0.5.1] | TUI picks up the current editor selection: WebSocket JSON-RPC (MCP 2025-11-25 framing) to IDE extensions found via `~/.claude/ide/<port>.lock` (Claude Code IDE extensions, `x-claude-code-ide-authorization`), or `CLAUDE_CODE_SSE_PORT`/`OPENCODE_EDITOR_SSE_PORT`; inside a Zed terminal (`ZED_TERM`/`TERM_PROGRAM=zed`) reads Zed's sqlite db (`OPENCODE_ZED_DB`, `~/Library/Application Support/Zed/db/0-stable/db.sqlite`, `~/.local/share/zed/db/0-stable/db.sqlite`). Windows: no Zed path candidate, so Zed selection unavailable. `$VISUAL`/`$EDITOR` external editor (Windows uses `shell:true`) | `tui/src/context/editor.ts:112-374`, `tui/src/editor.ts`, `tui/src/editor-zed.ts:187-195` |
| attention (sounds/notifications) | [0.5.1 opt-in] | Desktop notifications + sounds on done/subagent done/question/permission/error; TUI config `attention {notifications=false, sound=false, volume=0.4, sound_pack "opencode.default", sounds{}}`; mp3s from `packages/ui/audio`; title "Anysola Code" | `tui/src/attention.ts:35-45`, `tui/src/config/index.tsx:99-111,306-311`, `tui/src/feature-plugins/system/notifications.ts:54-88` |
| `git.ts` / `vcs` | [0.5.1] | Git plumbing for project discovery, snapshots/undo, worktrees, diffs, branch in UI; `vcs.git`/`vcs.hg` plugins (Mercurial supported) | `core/src/git.ts`, `plugin/vcs/{git,hg}.ts`, `vcs/patch.ts` |
| share | [dead] | Session sharing removed upstream in v2 (only a `share_url` column remains) | `session/sql.ts:45`, commit 7f00c5395b message |
| `artifact.ts` | [0.5.1 opt-in → dead] | Tools `artifact_create/read/update/delete` only with `experimental.artifacts: true` and a provider; none ships | `tool/plugin/artifact.ts:64-72` |
| `form.ts` | [0.5.1] | Question/elicitation/auth forms rendered by TUI, ACP, CLI | `core/src/form.ts`, `mcp/index.ts:274-323` |
| `job` | [0.5.1] | Background jobs table (subagents, background shells, image jobs) | `core/src/job.ts`, `session/subagent-job.ts` |
| `integration.ts` | [0.5.1] | Integration/credential registry: Anysola device + env methods, MCP OAuth methods | `core/src/integration.ts`, `mcp/index.ts:206-245` |
| `reference` + `repository-cache` | [0.5.1 opt-in] | Config `references {name: "path"|{repository,branch?}|{path}}`; git repos cloned into `~/.local/share/opencode/repos`; model sees `<available_references>` block | `schema/config/reference.ts`, `reference/instructions.ts:16-26`, `repository-cache.ts:138,211` |
| `codemode` package | [0.5.1] | Pure JS-subset interpreter (no imports, fs, timers; `fetch` allowed) behind the `execute` tool; every non-direct tool (MCP, `opencode.*`, plugin tools) lives in its catalog. Required in Rust for MCP parity | `packages/codemode/README.md`, `core/src/tool.ts:265-338`, `core/src/codemode/*` |
| `merman` | [0.5.1] | TUI renders ```mermaid blocks as terminal diagrams (flowchart, sequence, state, gantt, gitgraph, timeline) | `packages/merman/src/plugin.ts`, `tui/src/plugin/builtins.ts:14` |
| `latex` | [0.5.1] | TUI renders ```latex / ```math blocks | `packages/latex/src/plugin.ts`, `builtins.ts:13` |
| `simulation` | [dead] | Deterministic simulated backend/frontend, only with `OPENCODE_SIMULATE` (dev) | `cli/src/server-process.ts:114`, `server/src/routes.ts:162-167` |
| `ui` package | [0.5.1] (assets only) | Supplies attention mp3s and the OAuth page tokens; no UI components used by the CLI | `tui/src/attention-sounds.bun.ts` |
| `http-recorder` | [dead] | Test-only HTTP cassette recorder | `packages/http-recorder/README.md`; no src consumer |
| `plugin-browser` | [dead in effect] | Registered (`opencode.browser`) but tools/skill hidden until a desktop browser attaches; desktop deleted. Observed: no browser tools | `packages/plugin-browser/README.md`, `plugin/internal.ts:103` |
| photon image processing | [0.5.1] | WASM photon normalizes images for prompts and tool results: limits 2000x2000, 5 MiB base64 (config `media.image {auto_resize,max_width,max_height,max_base64_bytes}`), JPEG qualities 80/70/55/40, PNG passthrough ≤384 KiB, Lanczos3 shrink by 0.75 steps, transparency keeps PNG. Tools agent should own details | `core/src/image.ts:60-90`, `image/photon-normalize.ts:29-141`, `schema/config/media.ts` |
| image generation tool | [dead] | `experimental_image_generate` offered only when a connected provider has image models; Anysola has none | `tool/plugin/image-generate.ts:26,94` |
| `@opentunnel/client` | [dead] | Remote access tunnel (opentunnel.xyz); bundled but gated by `ANYSOLA_ENABLE_REMOTE_TUNNEL` (unset → log only) | `cli/src/services/remote-tunnel.ts:21-24` |
| web UI | [dead] | `ANYSOLA_ENABLE_WEB_UI` unset → 404 text "The web UI is not available in Anysola Code…"; assets not built (`--skip-web-ui`) | `cli/src/services/web-ui.ts:30-33` |
| `@ff-labs/fff` | [0.5.1] (not Windows) | "fff" = native fuzzy file finder (index + scanner + git threads) for file search (@-mentions, find APIs). Used when the location is a VCS repo, not the home dir, not a workspace, not Windows by default, native lib present; else ripgrep. `OPENCODE_DISABLE_FFF` toggles (also forces on Windows when set falsy) | `core/src/filesystem/search.ts:221-238`, `filesystem/fff.bun.ts`, `cli/src/server-process.ts:142-144` |
| `ffi-rs` | [dead] | Only used by the Node build path of fff (`fff-node`); the Bun binary uses Bun FFI. `strings` finds no ffi-rs | `cli/package.json:69-73`, binary strings |
| `@parcel/watcher` | [0.5.1] | See §5 | |
| `web-tree-sitter` 0.25.10 | [0.5.1] | TUI syntax highlighting only (code blocks, diffs). Bundled grammars: javascript, typescript, tsx, markdown, markdown_inline, zig. 34 more (python rust go cpp c_sharp bash c java kotlin ruby php scala html vue hcl json yaml haskell css julia lua ocaml clojure swift toml nix diff elixir fsharp r make vim xml agda) are **downloaded on first use from GitHub releases** with queries from `raw.githubusercontent.com/nvim-treesitter`, cached in `~/.local/share/opentui/tree-sitter/{languages,queries}` | `tui/src/parsers-config.ts`, binary strings |
| node-pty / bun-pty | [0.5.1] | Runtime uses `bun-pty` (pty.bun.ts) and the opencode-pty daemon; `@lydell/node-pty` only for the Node build | `core/src/pty/pty.bun.ts:2`, `cli/package.json:58-63` |
| telemetry / OpenTelemetry | [0.5.1 opt-in] | Nothing is sent by default. OTLP export only when `OTEL_EXPORTER_OTLP_ENDPOINT` (and optional `OTEL_EXPORTER_OTLP_HEADERS`) is set | `cli/src/index.ts:146-147`, `util/src/observability.ts:22-23`, commit 7f00c5395b |
| upstream traffic | mixed | models.dev / `models.opencode.ai` never fetched (`models-dev.ts:493`, commit 8ee8951f4d); `opencode.ai/console` URLs only in deleted/unused usage-limit paths; `$schema: https://opencode.ai/v2/cli.json` written into migrated TUI config (no request); **live**: tree-sitter grammar downloads from GitHub, npm registry for plugin installs and npm updates, MCP servers the user configures, Anysola gateway | `cli/src/config/schema.ts:4`, `tui/src/util/usage-limits.ts:116`, `util/src/npm-config.ts:38` |
| managed policy / managed config | [0.5.1] | System config dir `/etc/opencode` (Linux), `/Library/Application Support/opencode` (macOS), `%ProgramData%\opencode` (Windows), plus macOS MDM plist `ai.opencode.managed(.plist)` (`/Library/Managed Preferences/[<user>/]`). `policies: [{action, resource(wildcard), effect allow|deny}]`, actions `provider.use, tool.use (<tool>:<input>), integration.connect, integration.use, mcp.use, skill.use, plugin.use`; earliest-loaded (managed) wins; denial messages in strings.txt | `core/src/config/discovery.ts:17-42`, `schema/config/policy.ts`, `core/src/managed-policy.ts` |

---

## 8. Recommendation table for Rust v1

| Subsystem | Decision | Evidence / reason |
|---|---|---|
| MCP stdio + Streamable HTTP, config (v2 + v1 shapes), statuses, timeouts, retries, session-expiry recovery | keep v1 | §1, verified at runtime; user-facing via README/CHANGELOG ("MCP sign-ins name Anysola Code") |
| MCP OAuth (DCR + CIMD + static client, loopback callback page, credential row) | keep v1 | §1.6, verified end to end; Anysola-branded in 0.5.0 |
| MCP via Code Mode (`execute` + catalog + instructions + catalog deltas) | keep v1 | MCP tools are unreachable to the model without it (default codemode true) |
| MCP resources tools, prompts as slash commands, elicitation forms | keep v1 | §1.5 |
| `anysola mcp list/add/auth/logout` | keep v1 | §1.7 (fix the cold-start empty-list race) |
| TUI `/mcps`, toasts, footer, sidebar | keep v1 | §1.8 (TUI agent implements, strings here) |
| MCP SSE transport | drop | not in 0.5.1 |
| Client-attached WebSocket MCP (`tui` tools experiment) | defer | opt-in experiment behind devtools |
| ACP server (`anysola acp`, `--login`) | defer (keep CLI entry printing a clear message, or port later) | complete but no Anysola usage evidence beyond one changelog line; large surface (session replay, diffs, elicitation) |
| LSP | drop (accept `lsp` key) | no runtime in v2.2.5 |
| Formatters (opt-in) | keep v1 | small, user-visible when `formatter` set; list in §4 |
| File watching (config/skills/AGENTS.md/VCS head) | keep v1 | hot reload behaviour; `notify` crate |
| JS plugin host (third-party plugins) | drop | needs a JS engine; Anysola restricts plugins; ignore config with a warning |
| Built-in plugins (Anysola provider/lock/account/websearch, prompt/identity/goal/plan/etc.) | keep v1 as native code | §6.2 hook list |
| Code Mode interpreter (`codemode` package) | keep v1 | required for MCP and `opencode.*` tools |
| Worktree tools, git/vcs (incl. hg), forms, jobs, integrations | keep v1 | exercised (other areas own details) |
| PTY terminal pane (opencode-pty daemon) | keep v1 if TUI keeps the pane (TUI decision); else defer | exercised via `<leader>t` |
| Editor selection (IDE lock files, Zed db) | defer | niche; TUI decision |
| Attention sounds/notifications | defer | off by default |
| Mermaid / LaTeX rendering | defer | TUI nicety |
| Tree-sitter highlighting | keep (TUI) with bundled grammars; reconsider GitHub downloads | only remaining third-party network fetch |
| fff fuzzy finder | replace with a Rust fuzzy matcher over `ignore`-walked files | same UX, no native lib |
| photon image normalization | keep v1 (via `image` crate) | prompts with images |
| schedules, artifacts, references | defer | experimental/opt-in, off by default |
| voice, workspace, share, simulation, http-recorder, plugin-browser, opentunnel, web UI, ssh-askpass, ffi-rs, image-generate | drop | dead in 0.5.1 (reasons in §7) |
| Telemetry | drop by default; optional OTLP later | nothing sent unless env set |
| Managed policy + system config dirs (incl. Windows `%ProgramData%\opencode`, macOS plist) | keep v1 | config agent overlaps; enforcement for mcp/tool/plugin/skill |
