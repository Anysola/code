# Anysola Code client ↔ gateway contract (parity inventory, 0.5.1; usage limits `[1.0]`)

Scope: everything the client sends to or reads from `https://api.anysola.com/v1/code/*` (and the few other Anysola
URLs it opens). Rust rewrite must match the client side; the gateway stays.

Paths: **C** = `/home/ubuntu/anysola-code/build/fork/` (branch `anysola`, 0.5.1). **G** = `/home/ubuntu/web/anysola/services/cloud/src/` (gateway, PM2 `code-index.ts`, :3520). **K** = `/home/ubuntu/web/anysola/packages/contracts/src/code/` (shared contracts).
Verbatim copies: `spec/verbatim/protocol/` (`client/`, `contracts/`, `gateway/`, `captures/requests-0.5.1.jsonl` (wire captures from the real 0.5.1 binary), `captures/sse-gateway-examples.txt`, `user-strings.txt` (every user-facing string with file:line)).

Status tags: `[1.0]` = the 1.0 protocol, which replaces the 0.5.1 text it sits next to (usage limits instead of credits; source of truth: K `client-api.ts` and `packages/contracts/src/account/usage-limits-core.ts`, minimum client version 1.0.0, no credit compatibility). `[0.5.1]` = live behaviour to reproduce. `[dead]` = code path that never fires against the real gateway. `[bug]` = 0.5.1 behaviour that looks wrong. Reproduce it or fix it deliberately, and say which.

Method: source reading on both sides plus runs of the shipped binary `rust/testing/bin/anysola-0.5.1` against a logging copy of the fake gateway (headers below are **observed**, not inferred).

---

## 0. Client-side Anysola files (all of them)

| File | Role |
|---|---|
| C `packages/core/src/anysola.ts` | constants (`providerID/integrationID="anysola"`, `tokenEnv="ANYSOLA_TOKEN"`, `version`), `apiURL()` (`ANYSOLA_API_URL` override, trailing `/` stripped), `sanitize()` (drop foreign providers/models from local config) |
| C `packages/schema/src/anysola.ts` | `AnysolaRpc` definition (`anysola.account`: session, budget, budgetDefault, me, usage, tokens, revoke, estimates, notes; event `credits`) |
| C `packages/core/src/plugin/provider/anysola.ts` | device login, `/config` load+poll+cache, provider pin (baseURL, headers), provider lock |
| C `packages/core/src/plugin/provider/anysola-account.ts` | per-turn/session credits, budgets (KV), 402 rewrite, RPC proxies for /me /usage /tokens /revoke |
| C `packages/core/src/plugin/provider/anysola-credits-stream.ts` | SSE filter that strips `event: anysola_credits` |
| C `packages/core/src/plugin/websearch/anysola.ts`, `anysola-credits.ts`, `index.ts` | `/search` provider, search credits hand-off |
| C `packages/core/src/plugin/internal.ts` | plugin order: `AnysolaPlugin`, `AnysolaAccountPlugin`, then `AnysolaProviderLockPlugin` **last**; all three guarded from removal |
| C `packages/core/src/mcp/oauth.ts:32,188-190` | MCP OAuth CIMD URL + client metadata |
| C `packages/core/src/wellknown.ts:175-176` | well-known login sources disabled |
| C `packages/cli/src/commands/handlers/anysola/{account,shared,login,logout,status,usage,devices,devices-revoke}.ts` | CLI account commands |
| C `packages/cli/src/services/anysola-version.ts`, `services/updater.ts:235-271`, `commands/handlers/upgrade.ts` | version check, minimum gate, update |
| C `packages/cli/src/commands/handlers/{default,run}.ts`, `run/run.ts:140-236` | startup gate, `run --budget`, run summary |
| C `packages/cli/script/build.ts:190` | Bun `--user-agent=opencode/${channel}/${OPENCODE_VERSION}/cli` (default UA of every plain `fetch`) |
| C `packages/tui/src/anysola/{store,format,wordmark,home}.ts(x)`, `feature-plugins/system/anysola.tsx`, `component/prompt/index.tsx:1183-1191`, `component/dialog-model.tsx:86-93`, `util/presentation.ts` | TUI status line, banners, dialogs, budget gate, picker notes, exit summary |
| G `services/web/src/routes/oauth/anysola-code/client.json/route.ts` (monorepo) | the CIMD document the client points MCP servers at |

---

## 1. Endpoints

All under `${ANYSOLA_API_URL or https://api.anysola.com}/v1/code`. Gateway routes: G `code/app.ts:90-292`. Unknown path → 404 `not_found_error` "Not found: METHOD path", `x-should-retry: false` (app.ts:288-291). There is **no** `/messages/count_tokens` route (a 404). The client never calls it. `[0.5.1]`

### 1.0 Common header sets (observed)

**A. "account" calls** (core `call()` C anysola.ts:36-60, CLI `call()` C cli/.../anysola/account.ts:47-64):
```
accept: application/json
authorization: Bearer asc_…          (omitted for device/* calls)
x-anysola-version: 0.5.1
content-type: application/json       (only when a body is sent)
user-agent: opencode/latest/2.2.5/cli   (Bun runtime default, from build.ts:190)
```
Timeout 15 s (`AbortSignal.timeout(15_000)`). No `x-anysola-client`. Body is parsed as JSON if possible, else `{}`, so a proxy HTML page still gives a status.

**B. model calls** (`POST /messages`): see §1.7.

**C. version check** (`GET /version`): only `accept: application/json` + runtime UA. No token, no version header.

**D. search** (`POST /search`): `accept: application/json`, `authorization: Bearer <token>` (always Bearer, even for an env token), `content-type: application/json`, `x-anysola-client: opencode`, `x-anysola-version: 0.5.1`, runtime UA.

### 1.1 `POST /device/code`: start login `[0.5.1]`
- Client: C anysola.ts:123-146. Body `{"client":"anysola-code/<version>","deviceName":<os.hostname()>}`. Header set A without auth.
- Server: G app.ts:205-220. Rate limit 10 starts/hour per IP (`cf-connecting-ip` → `x-forwarded-for` → "local") → 429 `{"error":"slow_down","error_description":"Too many sign-in attempts from this address. Try again later."}`. Body schema K device-api.ts:99-105: `client` `^[A-Za-z0-9._/-]{1,60}$` (trimmed), `deviceName` 1..80 chars trimmed. Else 400 `{"error":"invalid_request"}`. Body read limit 4096 bytes (G body.ts:31). `cf-ipcountry` stored as country.
- 200 (K device-api.ts:107-113, G device.ts:46-52, `cache-control: no-store`):
  `{"device_code":"<32 random bytes base64url, 43 chars>","user_code":"XXXX-XXXX","verification_uri":"https://account.anysola.com/device","interval":5,"expires_in":600}`. user_code alphabet `BCDFGHJKLMNPQRSTVWXZ` (K token-core.ts:49-56).
- Client handling: 429 → "Too many login attempts. Try again later." Other non-2xx → "Could not start login: <errorOf(reply).message>". Missing device_code/user_code/verification_uri → "Could not start login: unexpected response from the server." `interval = max(1, interval ?? 5)`, `lifetime = min(expires_in ?? 600, 900)` s, deadline = now + lifetime.
- Shown to user: `Open ${verification_uri} and enter ${user_code}`, then the URL alone. Browser opened only if stdin and stdout are TTYs (C auth/login.ts:182). The code is **never** put in the URL (anysola.ts:133).
- **Edge:** a hostname > 80 chars gives a 400 → "Could not start login: invalid_request". `[0.5.1]`

### 1.2 `POST /device/token`: poll `[0.5.1]`
- Client: C anysola.ts:71-118. Loop: sleep `wait` s **first**, then check the deadline (→ "The login code expired. Run `anysola login` again."), then POST `{"device_code":"…"}`.
  - 2xx: `access_token` must match `^asc_[0-9A-Za-z]+$` (server issues exactly `^asc_[0-9A-Za-z]{40}$`, K token-core.ts:15), else "Login failed: the server returned no usable token." `expires_at` ISO → ms, fallback now+365 d. Stored as credential (§5).
  - status < 500: reset failure count. `error` (string or `.type`): `authorization_pending` → continue. `slow_down` → `wait += 5` (the server's returned `interval` is **ignored**). `expired_token` → "The code expired before it was approved. Run `anysola login` again." `access_denied` → "Login was denied." Anything else → `Login failed: ${message}` (e.g. `invalid_grant`).
  - network error or ≥500: `failures += 1`. At 3 → "Login failed: <message>" / "Login failed: network error".
- Server: G app.ts:222-232, device.ts:78-131. Per-IP 30 polls/min → 429 `{"error":"slow_down","interval":10}` (the client treats it as slow_down: +5). Schema `device_code` `^[A-Za-z0-9_-]{43}$`, else 400 invalid_request. A poll sooner than `interval_s` after the previous one → 400 `{"error":"slow_down","interval":interval_s+5}` and the server interval grows by 5. Unknown/consumed → `invalid_grant`. Expired → `expired_token`. Pending → `authorization_pending`. Denied → `access_denied`. Approved → 200 `{"access_token":"asc_…","token_type":"bearer","expires_at":"<ISO>"}` (K device-api.ts:134-138), code consumed.
- Issuing (G tokens.ts:140-183): a device token gets a 90 d sliding / 365 d absolute expiry. Live device tokens per user are capped at 10: a new one revokes the least recently used (`revoked_by='limit'`).
- Approval page: monorepo `apps/account/src/routes/_shell/device.tsx` (user types the code. It shows `client`, `deviceName`, `country`, created/expires from `DeviceLookup` K device-api.ts:196-203).

### 1.3 `GET /config`: remote config `[0.5.1]`
- Client: C anysola.ts:282-338. Called (a) at plugin start, (b) on every `Credential.Event.Switched/Updated`, (c) every **10 min** while a connection exists (FiberHandle loop). Header set A with Bearer (token from OAuth credential `access` or key credential `key`, which includes the env token).
  - 2xx → clear connection status, save the raw body to KV `anysola:config`, use it.
  - 401 → connection status `needs_auth`, message "Run `anysola login` to sign in to Anysola". Then (and on any other failure or network error) **fall back to the KV cached body** so models survive offline restarts.
  - Body → config document (anysola.ts:151-178): normalized and decoded as `Config.Info` (excess ignored). Only `providers.anysola` kept. `model` kept only if on anysola. `agents` kept as is.
  - Then the provider is **pinned** (anysola.ts:192-206): name "Anysola", package `@opencode/ai/providers/anthropic-compatible`, `settings.baseURL = ${apiURL}/v1/code` (the server's `baseURL` is overridden), headers merged `{"x-anysola-client":"opencode","x-anysola-version":<ver>,"x-anysola-credits-event":"1"}`.
  - Provider lock (anysola.ts:226-246, runs last): removes every non-anysola provider, restores endpoint keys (`baseURL, apiKey, authToken, accessToken`, package, canonical) for the provider and each model from what the plugin and remote config set.
- Server: G app.ts:95-118. `authenticate()` (app.ts:58-72) refuses only Claude Code marks and bad tokens. Access check (`codeAccess`): not ok → 403 `permission_error`, final, "Anysola Code isn't open to this account yet." (user) or "Anysola Code needs a Pro or Team plan: https://account.anysola.com/billing". Account unavailable → 503. `cache-control: no-store`.
- Response (K config-core.ts:110-158). Example (Auto + economy on):
```json
{"$schema":"https://opencode.ai/config.json",
 "model":"anysola/claude-sonnet-5-5","small_model":"anysola/claude-haiku-5-5",
 "agents":{"title":{"model":"anysola/claude-haiku-5-5"}},
 "providers":{"anysola":{"name":"Anysola","package":"@opencode/ai/providers/anthropic-compatible",
  "settings":{"baseURL":"https://api.anysola.com/v1/code","apiKey":"{env:ANYSOLA_TOKEN}"},
  "headers":{"x-anysola-client":"opencode"},
  "models":{
   "auto":{"name":"Auto","description":"Picks Haiku or Sonnet per turn","limit":{"context":1000000,"output":32000},
     "capabilities":{"reasoning":{"supported":true,"effort":["low","medium","high","xhigh","max"]}},
     "compatibility":{"supportsThinkingBlockBinding":true,"supportsEffortUpdates":true},
     "variants":[{"id":"economy","settings":{"thinking":{"type":"adaptive","display":"summarized"},"effort":"low"},"headers":{"x-anysola-variant":"economy"}}]},
   "claude-sonnet-5-5":{"name":"Claude Sonnet 5.5","description":"Default","limit":{"context":1000000,"output":32000}, "capabilities":{…}, "variants":[economy]},
   "claude-haiku-5-5":{"name":"Claude Haiku 5.5","description":"Fastest","limit":{"context":200000,"output":32000}},
   "claude-opus-5-5":{"name":"Claude Opus 5.5","description":"Most capable","limit":{"context":1000000,"output":32000}, …}}}}}
```
  - `auto` is first and present only while Auto is on and both of its models are open to the plan. Economy variants appear only on `economy.models` targets. `limit.output = caps.maxOutputTokens` (32000 default). Contexts: haiku 200K, others 1M. `model`/`small_model`/`agents` are omitted if the plan has no models.
  - `[1.0]` `credits_per_turn_estimate` is gone; the picker shows only the note. 0.5.1: **`credits_per_turn_estimate`**: the client reads it (anysola-account.ts:251-261, picker `~N credit(s)/turn`), but the real gateway **never sends it** (only `rust/testing/fake-*.mjs` do). `[dead on gateway]`
  - `description` → picker note (trimmed, ≤160 chars, anysola-account.ts:239-249). `[0.5.1]`
  - `small_model` is the legacy key. V2 reads `agents.title.model`. `[0.5.1]`
  - `settings.apiKey:"{env:ANYSOLA_TOKEN}"` resolves to the env token. When set, model calls carry `x-api-key` (§1.7).

### 1.4 `GET /me` `[1.0]`
- Client: the TUI on start, on reconnect, every 5 min and **1.5 s after each usage event** (debounced); `anysola status`.
- Shape (`CodeMeResponse`): `access` is null when the plan can't use Code; `event` is an admin promotion (`unlimited` meters nothing, `multiplier` counts each request at a multiple).
```json
{"plan":"free|pro|team","access":"paid|free|null",
 "limits":<UsageLimitsWire, §1.5a>,
 "usageToday":{"requests":3},
 "event":{"mode":"unlimited|multiplier","label":"…","until":"<ISO>|null"} | null}
```
- 0.5.1 sent `credits`, `lowBalance` and `freeQuota` instead of `limits`; 1.0 reads none of them.

### 1.5 `GET /usage?days=7|30` `[1.0]`
- Client: TUI `/usage` (key `t` toggles 7/30), CLI `anysola usage --days 7|30` (default 7), `--format default|json` (json prints the body pretty-printed with 2 spaces).
- Server: `days` 1..90, default 30. Days are UTC. Usage is shown as a share of the limits, never as money.
```json
{"days":7,"totals":{"requests":42,"cacheHitRate":0.734},
 "byDay":[{"date":"2026-11-04","requests":3}],          // every UTC day of the window, oldest first
 "byModel":[{"model":"claude-sonnet-5-5","requests":3}],
 "today":{"requests":3},"plan":"team",
 "limits":<UsageLimitsWire>}
```
- `cacheHitRate` = prompt tokens read from the cache over all prompt tokens, 0 to 1, null with none. The client shows `Cache hit rate ${round(rate*100)}%`.
- Display (`anysola usage`, `/usage`): `Limits`, one line per window `  5-hour  ████░░░░░░   42%  resets in 2 h` (10-cell bar, percent right-aligned, `resets in` only while the window runs), `  Banked resets  2 · next expires Dec 1, 12:00 AM` (or `none`), then `Totals` (requests, cache hit rate), `By day`, `By model` with a `requests` column. The TUI adds `  This session   N turns · X% of the 5-hour window` under the bank line.

### 1.5a `GET /limits`, `POST /limits/redeem` `[1.0]`
- `UsageLimitsWire` (`usageLimitsWireSchema`): one meter per account shared by Chat and Code, two windows. A window starts with the first request after the previous one ended and lasts 5 hours (`five_hour`) or 7 days (`weekly`). Percent is 0 to 100 rounded down; a request already running is never cut.
```json
{"plan":"free|pro|team","allowed":true,"blockedBy":"five_hour|weekly|null",
 "fiveHour":{"percent":42,"resetsAt":"<ISO>|null"},      // null while the window hasn't started
 "weekly":{"percent":18,"resetsAt":"<ISO>|null"},
 "bank":[{"id":"…","expiresAt":"<ISO>","scope":"five_hour|both"}]}   // banked resets, at most 3 by default
```
- `POST /limits/redeem` spends the banked reset that expires first and answers the new `UsageLimitsWire`; 409 `invalid_request_error` when the bank is empty. `scope: both` also clears the weekly window.
- Client: `anysola limits [--format json]` prints the windows (as §1.5), `  5-hour limit reached. Resets in 2 h.` when blocked, and `Banked resets: N` with one `  5-hour[ and weekly] · expires <date>` line each (`Banked resets: none` when empty). `anysola limits redeem` prints `Banked reset used.`, a blank line and the new limits; an empty bank prints `No banked resets to spend.` on stderr, exit 1. The TUI polls `/limits` after a refused turn (§2.4) and `run --use-banked-reset` before the turn (§4.3).

### 1.6 `GET /tokens`, `POST /tokens/:id/revoke`, `POST /tokens/self/revoke` `[0.5.1]`
- `GET /tokens` (G app.ts:243-261) → `{"tokens":[{"id":"<uuid>","name":"<deviceName>","client":"anysola-code/0.5.1","kind":"device|ci","createdAt":"…","lastUsedAt":"…|null","expiresAt":"…|null","current":true}]}`. `client` is rewritten to `anysola-code/<x-anysola-version>` whenever a request carries a valid version header (G tokens.ts:35-43,103-113).
- `POST /tokens/:id/revoke` (app.ts:264-280): non-UUID or not owned → 404 `not_found_error` "No such token." → CLI shows "This Anysola server does not offer this yet." (404 mapping, shared.ts:14) `[bug: misleading text]`. 200 `{"revoked":true}`.
- `POST /tokens/self/revoke` (app.ts:234-241): revokes the calling token. Used by `logout` and `login --force` (C cli/.../account.ts:74-83). **A 401 counts as success** (already dead).
- Token cache on the gateway: 30 s TTL. A revoke from Account takes up to 30 s to bite, a revoke through the gateway is immediate (G tokens.ts:52-96).

### 1.7 `POST /messages`: model call (observed headers) `[0.5.1]`
```
content-type: application/json
user-agent: opencode/latest/2.2.5+anysola.0.5.1/cli     (App.useragent: opencode/${channel}/${OPENCODE_VERSION}+anysola.${ANYSOLA_VERSION}/${OPENCODE_CLIENT ?? "cli"})
anthropic-version: 2023-06-01
anthropic-beta: interleaved-thinking-2025-05-14,thinking-binding-controls-2026-08-01
x-anysola-client: opencode
x-anysola-usage-event: 1                                 ([1.0]; 0.5.1 sent x-anysola-credits-event: 1)
x-anysola-version: 0.5.1
x-anysola-variant: economy                               (only with the economy variant: from config variants[].headers)
authorization: Bearer asc_…    ← token from `anysola login` (OAuth credential)
  or x-api-key: asc_…          ← token from ANYSOLA_TOKEN env (via settings.apiKey "{env:ANYSOLA_TOKEN}")
x-opencode-client: cli
x-opencode-project: <projectID, "global" outside a git project>
x-opencode-session: <ses_…>   x-opencode-session-id: <ses_…>   x-session-affinity: <ses_…>   x-session-id: <ses_…>
x-opencode-parent-session-id / x-parent-session-id      (subagent sessions)
b3: …  traceparent: …                                    (tracing; gateway ignores)
accept: */*
```
- Header sources: C core/src/session/model-request.ts:293-303 (session/UA headers), C ai/src/protocols/anthropic-messages.ts:1666-1701 (version and betas), provider pin (x-anysola-*).
- Betas added by the client (anthropic-messages.ts:1673-1689): always `interleaved-thinking-2025-05-14`. `thinking-binding-controls-2026-08-01` whenever thinking has `block_binding` (all observed requests). `mid-conversation-output-config-2026-07-01` when any `role:"system"` message carries `output_config` (effort changed mid-session on a model with `supportsEffortUpdates`). `compact-2026-01-12` when `context_management.edits` or replayed `compaction` blocks exist. `fast-mode-2026-02-01` when `speed` is set.
- Observed body keys: `model, system, messages, tools, stream:true, max_tokens:32000, thinking:{type:"adaptive",block_binding:{prefix_mismatch_behavior:"drop_block"}}` plus `output_config:{effort}` and `thinking.display:"summarized"` when a variant/effort is chosen. The title request uses the small model, no tools, and also `max_tokens:32000`, `stream:true`. Full bodies belong to the session-loop spec.
- Gateway request id for logging: `x-opencode-session ?? x-session-id` (G proxy.ts:169).
- Response handling: §2.

### 1.8 `POST /search`: web search `[0.5.1]`
- Client: C websearch/anysola.ts:76-129. Default and only websearch provider (`"websearch": false` in config disables the tool). Body `{"query":<string>}`. Response limit 1 MiB ("…returned too much data"), timeout 30 s ("Anysola web search timed out"), other transport failures → "Cannot reach <api>". Non-2xx → `failure()` (anysola.ts:42-53): 401/402/429/other texts in user-strings.txt.
- 200 `{"results":[{"url","title?","content?","published?":ms}]}` (K search-core.ts:48-59). The client maps results to `{url,title?,content?,time:{published?}}`. A decode failure → "Anysola web search returned an unexpected response".
- `[1.0]` No `credits` field and no search bookkeeping: what a search takes of the windows shows in the next usage event or `/limits` poll. A 429 reads "Anysola web search is limited: <message>"; there is no 402 text. (0.5.1 added `credits` to the session's turn/session totals.)
- Server: G search.ts. Same `gate()` as /messages with +2 extra concurrency. 403 when `settings.search.enabled` is false ("Web search is not available on Anysola Code right now."). Body ≤4 KB, query trimmed 1..500. Paid reserve `searchWorstCaseReserve`. Haiku 5.5 runs `web_search_20250305` with `max_uses` 1..2, non-stream. Response headers `x-credits-charged`, `x-anysola-model: claude-haiku-5-5`, `cache-control: no-store`. Free tier: credits 0, counts toward daily USD but not turns.

### 1.9 `GET /version` `[0.5.1]`
- Server G app.ts:189-203: public, 30/min per IP → 429 `rate_limit_error` "Too many version checks from this address." with `retry-after`. 200 `{"latest":"0.5.1","minimum":"0.2.0","npmPackage":"@anysola/code","notes":null|"…"}` (K client-api.ts:209-214). Values come from admin `release` settings (K settings-core.ts:184-191).
- Client `fetchInfo(timeout=5000)` (C anysola-version.ts:54-71): non-ok or no string `latest` → undefined. Leading `v` stripped from latest/minimum.
- **Minimum gate** `enforceMinimum()` (anysola-version.ts:92-100): run before the TUI starts (default.ts:25) and before `run` (handlers/run.ts:9). Skipped for dev builds (`/^\d+\.\d+\.\d+/` fails or starts `0.0.0`). 3 s timeout. Offline/failure → allowed. If `compare(version, minimum) < 0` → stderr `Anysola Code X is no longer supported (minimum Y).\nUpdate Anysola Code with: anysola update\n`, exit 1. **The gateway never enforces a minimum version**: it only records `x-anysola-version`.
- Background check (TUI only): `Updater.run` at start and then every **10 min** (default.ts:88-105). Result `{type:"available",version:latest}` when `compare(latest, current) > 0` and released. It only notifies, never auto-installs (updater.ts:238-262). So a TUI start makes 2 `/version` calls.
- `compare` (anysola-version.ts:74-89): numeric major.minor.patch, then a prerelease sorts before its release, prerelease strings compared lexically.

### 1.10 `anysola update` (alias `upgrade`) `[0.5.1]`
C upgrade.ts. `fetchInfo()`. Prints "Installed X, latest Y" and notes. If released and latest ≤ current and not `--force` → "Already up to date". Command (anysola-version.ts:36-43): if `realpath(execPath) == realpath(${ANYSOLA_INSTALL_DIR or ~/.anysola/bin}/anysola[.exe])` (case-insensitive) → `sh -c "curl -fsSL https://www.anysola.com/install | sh -s -- --no-modify-path[ --version V]"` (Windows: `powershell -NoProfile -ExecutionPolicy Bypass -Command "[$env:ANYSOLA_VERSION='V'; ]irm https://www.anysola.com/install.ps1 | iex"`). Else `npm i -g @anysola/code@<V|latest>`. `--print` only prints it. Packaging agent owns the details.

### 1.11 Non-API URLs `[0.5.1]`
| Use | URL | Source |
|---|---|---|
| verification (from server) | `https://account.anysola.com/device` | K device-api.ts:96 |
| `/account` | `https://account.anysola.com` | C anysola.tsx:16 |
| `/feedback` | `https://www.anysola.com/contact` | C anysola.tsx:18 |
| billing (402, credits dialog) | `https://account.anysola.com/billing` | C anysola-account.ts:16, store.ts:23 |
| revoke manually | `https://account.anysola.com/apps` | C cli/.../account.ts:12 |
| product (TUI) | `https://www.anysola.com/code` | C tui/src/app.tsx:1150 |
| installer | `https://www.anysola.com/install`, `.ps1` | C anysola-version.ts:10 |
| MCP CIMD client_id | `https://www.anysola.com/oauth/anysola-code/client.json` | C mcp/oauth.ts:32 |

`/account` and `/feedback` call `openUrl(url)` and then toast the URL (success) or `Open <url>` (failure).

MCP OAuth (C mcp/oauth.ts:156-190, 358-470): CIMD is used only when the server metadata has `client_id_metadata_document_supported: true` and `token_endpoint_auth_methods_supported` includes `none`, and the config has no `client_id`/`redirect_uri`. Otherwise DCR with `client_name:"Anysola Code"`, `client_uri:"https://www.anysola.com/code"`, `grant_types:["authorization_code","refresh_token"]`, `response_types:["code"]`, `token_endpoint_auth_method: client_secret ? "client_secret_post" : "none"`. Local callback: `http://127.0.0.1:<callback_port | redirect port | ephemeral><path default /callback>`. The served document (monorepo services/web/.../client.json/route.ts) lists redirect_uris `http://127.0.0.1/callback`, `http://localhost/callback`, `http://127.0.0.1/mcp/oauth/callback`, `application_type:"native"`, `logo_uri:"https://www.anysola.com/icon.png"`, `cache-control: public, max-age=3600`.

No traffic to opencode.ai or models.dev (commits `7f00c5395b`, `8ee8951f4d`). The `$schema:"https://opencode.ai/config.json"` string in /config is never fetched. `[0.5.1]`

---

## 2. Responses, SSE, usage, errors, retries

### 2.1 Success headers (G proxy.ts:384-388, 421-424, 459-482)
| | stream | non-stream |
|---|---|---|
| `x-request-id`, `x-anysola-model` (model actually run), `x-upstream-request-id?` | yes | yes |
| `x-credits-*` | `[1.0]` gone (0.5.1: `x-credits-available-before`, `x-credits-charged` non-stream) | gone |
| `cache-control: no-cache, no-transform`, `x-accel-buffering: no` | yes | no |

### 2.2 SSE (exact bytes in `verbatim/protocol/captures/sse-gateway-examples.txt`)
- A pure pass-through of Anthropic's SSE, byte for byte (proxy.ts:748-751). The gateway adds:
  1. `: keepalive\n\n` after 15 s without sending, only at an event boundary (runtime.ts:28, proxy.ts:716-723).
  2. `[1.0]` `event: anysola_usage\ndata: {"type":"anysola_usage","fiveHourPercentUsed":<float>,"fiveHour":{"percent":<int>,"resetsAt":<ISO|null>},"weekly":{…}}\n\n` after upstream EOF, only if the request had `x-anysola-usage-event: 1` **exactly**, upstream sent `message_stop`, and the stream ended at an event boundary (same rules as 0.5.1's `anysola_credits` event, which it replaces). `fiveHourPercentUsed` is what this request took of the 5-hour window, a float so a session budget can add small turns up; the windows are the account's after it.
  3. `event: error\ndata: {"type":"error","error":{"type":"api_error","message":"The model's stream was interrupted."}}` on upstream idle (300 s) or cut, or "Anysola Code is restarting. Try again in a moment." on shutdown, at an event boundary (proxy.ts:641-651, 700-708, 752-760). No usage event then.
- `[1.0]` Client strip: buffer text, split on `\n\n`, remove `\r` before testing. A block starting `event: anysola_usage` is dropped, its first `data:` line is JSON-parsed, and a usage event with a finite `fiveHourPercentUsed ≥ 0` is reported (a malformed one is dropped silently). Everything else is passed to the SSE parser as received, including the trailing partial block at end of stream. (0.5.1 stripped `event: anysola_credits` the same way.)

### 2.3 Usage accounting in the client `[1.0]`
- State per session: `{latest, turn, total, turns, open, budget}`; `turn`/`total` are percent of the 5-hour window (floats). Persisted in KV `anysola:usage:<sessionID>` = `{used, turns, budget}` (on each usage event with `fiveHourPercentUsed > 0`, on idle with turn > 0, on budget change). A fresh session's budget = KV `anysola:budget:five-hour` (whole percent 1..100) or none.
- `record(sessionID, event)`: if `!open`: `turn = 0`. `open = true`. `turn += fiveHourPercentUsed`, `total += fiveHourPercentUsed`, `latest = event`. Emit the snapshot to the front ends, which also take the event's windows as the account's (the bank stays) and refresh `/me` 1.5 s later.
- Turn end (session idle): if open: `open = false`; if `turn > 0` then `turns += 1` and save. Emit.
- Every model call of the session counts: the main loop, titles, compaction, subagents (their own sessions).
- 0.5.1 credit state (`anysola:session:<id>` = `{total, turns, budget}` in credits, `anysola:budget:default`) is neither read nor imported: a credit count is no percentage, so credit budgets are dropped, not converted.

### 2.4 Error envelope and client display
Gateway errors: `{"type":"error","error":{"type":T,"message":M},"request_id":"…"}` + `x-request-id`, plus `x-should-retry: false` when final (G admission.ts:46-61, K wire-core.ts:18-51). Types → status: invalid_request_error 400, authentication_error 401, billing_error 402, permission_error 403, not_found_error 404, request_too_large 413, rate_limit_error 429, api_error 500 (also 499/503/504), overloaded_error 529.

| Status | When (gateway) | final? | Client |
|---|---|---|---|
| 400 | bad anthropic-version, disallowed beta, bad JSON, field not allowed, server tool, URL document, non-stream max_tokens > 4096 | yes | InvalidRequest, shown, not retried |
| 401 | no/malformed/unknown/expired token | yes | Authentication. `anysola run` prints `Error: <message>` (observed) |
| 402 | `[1.0]` not sent for usage (0.5.1: out of credits, body rewritten with the billing URL) | yes | Quota by classification, shown as sent; no rewrite |
| 403 | client not allowed, plan/user, region, paused, model not in plan | yes | Authentication, not retried |
| 404 | unknown model `model: X` | yes | |
| 413 | body > maxBodyBytes (10 MiB) | yes | payload-too-large |
| 429 | per-user concurrency (`retry-after: 2`), upstream 429 ("busy upstream", upstream retry-after) | no | RateLimit → retried |
| 429 + `x-anysola-limit: five_hour\|weekly` | `[1.0]` a full usage window (`limitMessage`: "5-hour limit reached. Resets in 2 h. You have 1 banked reset."), type `rate_limit_error`, `x-should-retry: false` | yes | UsageLimit, session error kind `provider.usage-limit`, **never retried** (whatever the classification). TUI: the message as a toast, or, when `/limits` shows a non-empty bank, a dialog **Use a banked reset?** (§4.6). `run`: `Error: <message>`, exit 1, or one redeem with `--use-banked-reset` (§4.3) |
| 499 | client closed | no | |
| 503 | restarting/draining, account unavailable, upstream unreachable, upstream 401/402/403 (final), missing key (final) | mixed | ProviderInternal → retried unless final |
| 504 | upstream header timeout (stream 60 s / non-stream 95 s) | no | retried |
| other upstream non-2xx | passed through with upstream body/content-type | – | by classification |

Gateway wording avoids "usage limit"/"quota" on 429 so OpenCode keeps retrying (admission.ts:31). The client's `QUOTA_TEXT` would make a 429 permanent (C ai/src/provider-error.ts:183).

### 2.5 Retry policy (client) `[0.5.1]`
C core/src/model-retry.ts, C ai/src/provider-error.ts:87-125.
- `x-should-retry: "true"`/`"false"` overrides every other rule.
- Retryable: RateLimit, ProviderInternal (≥500, 408, 409), Transport (unless delivery rejected / accepted-but-not-read), incomplete stream, UnknownProvider.
- Not retryable: Authentication (401/403), QuotaExceeded (402, `billing_error`), InvalidRequest (other 4xx), ContentPolicy, and `[1.0]` UsageLimit (429 with `x-anysola-limit`).
- Schedule: exponential from 2 s, each gap capped at 10 s, at most 10 retries, jittered. Delay = max(schedule, retry-after) with retry-after capped at 15 min (RateLimit/ProviderInternal only). Timeouts: at most 3 retries.
- One-shot calls (title etc.) retry only "not run" cases (429, 503/529, not-sent transport), at most 3.
- Gateway side: retries upstream once, and only on ECONNREFUSED/ENOTFOUND/EAI_AGAIN (proxy.ts:48, 359-367).

---

## 3. What the gateway enforces (decides whether a new client passes)

Order for `/messages` and `/search` (G admission.ts:139-220 `gate`, then proxy.ts / search.ts):
1. Draining → 503 restarting.
2. `cf-ipcountry` in `blockedCountries` → 403 region, final. Seeded: AF BY CN CU HK IR KP MM MO RU SY VE YE (G schema seed).
3. **Client classification** (K wire-core.ts:66-82, admission.ts:160-172) must equal `"opencode"`, else 403 `permission_error` "Anysola Code tokens work only with the Anysola Code client.", final (logged as `rejected_client` when a token was present):
   - **Claude Code (refused)** if `user-agent` starts `claude-cli/`, OR header `x-app` trimmed/lowercased equals `cli`, OR any comma-separated `anthropic-beta` value starts `claude-code-`.
   - else **opencode** only if `x-anysola-client` is present (**any non-empty value**) AND `user-agent` starts with one of `allowedUserAgents` (plain prefix match, not a regex). Seeded `["opencode/"]` (G scripts/schema.sql:181). The live DB may differ, since Admin edits it.
   - else "unknown" → refused.
4. Token (G tokens.ts:26-33): `authorization` is checked **first**. If present it must be `Bearer <tok>`, otherwise the request is malformed even when `x-api-key` is valid. Without it, `x-api-key` is used. The token must match `^asc_[0-9A-Za-z]{40}$`, be in `code_tokens` unrevoked, and not expired → else 401, final. Touch: `last_used_at` and device sliding expiry, at most once a minute.
5. `x-anysola-version` recorded as the token's `client` if it matches `^\d{1,4}\.\d{1,4}\.\d{1,4}(?:-[0-9A-Za-z.-]{1,30})?$`. Otherwise it is ignored (never rejected).
6. `/messages` only: `anthropic-version` (default `2023-06-01` when absent) must be exactly `2023-06-01` → else 400, final (proxy.ts:46, 61-70).
7. Account (Account service balance). Access `codeAccess`: kill switch, `allowedUserIds`, free plan → free tier if enabled, paid plan in `plans` → else 403 final.
8. Global USD brakes (daily 30 / monthly 300 default) → 403 "Anysola Code is paused.", final.
9. Concurrency: in-flight requests per user ≥ cap → 429 `retry-after: 2`. Cap = `caps.concurrencyPerUser` (default **2**) for paid, `freeTier.concurrency` (1) for free. `/search` gets +2. A main stream plus a title request already use 2, so a third parallel `/messages` (e.g. a subagent) gets a 429 and is retried.
10. `/messages` only: betas (K wire-core.ts:95-105). Each distinct beta must be in `allowedBetas` and must not start with `compact-`, `fast-mode-`, `server-side-fallback-`, `oauth-`, `claude-code-`, `code-execution-`, `mcp-client-` → else 400 "beta X is not available on Anysola Code", final. Seeded allowlist: `interleaved-thinking-2025-05-14`, `thinking-binding-controls-2026-08-01`. `mid-conversation-output-config-2026-07-01` passes only if Admin added it (Admin warns when Auto/economy is on without it: monorepo apps/admin/.../code-feature-forms.tsx:153-161). Unverified on live. **A client that changes effort mid-session must expect a 400 if it is missing.**
11. Body (K request-core.ts:136-253): body ≤ `caps.maxBodyBytes` (10 MiB, checked by content-length and while streaming). Must be a JSON object with non-empty `model` string, `max_tokens` a positive int, and a `messages` array. Allowed top-level keys: `model messages system max_tokens stream tools tool_choice thinking output_config metadata stop_sequences cache_control context_management`. `temperature/top_p/top_k` are dropped silently. Anything else → 400 (`speed` → "Fast mode is not available on Anysola Code."). Tools: only custom (no `type` / `custom`), `bash_*`, `text_editor_*`. A document with `source.type:"url"` → 400.
12. Model: `auto` (Auto on and open to the plan), `default`, `small`, admin alias, or an allowlisted enabled id open to the plan → else 404 `model: X` / 403 "X is not included in your Anysola plan.". Auto picks per turn: light model if every real user message is short (≤500 chars, no code block, no attachment, no edit/plan words) and the prompt up to the turn anchor is ≤ 32K tokens estimated as bytes/4. Otherwise heavy. The move is one-way within a session (K auto-core.ts). **A Rust client's system prompt and tool sizes shift this threshold**: OpenCode's are about 44 KB.
13. `max_tokens` clamped to `caps.maxOutputTokens` (32000). Non-stream with clamped > 4096 → 400. `metadata.user_id` replaced by a salted hash. Economy (`x-anysola-variant: economy`, economy on, target listed): `output_config.effort` set or capped to `economy.effort` (low), and every `role:"system"` message's `output_config.effort` capped too.
14. `[1.0]` Usage admission: a request is let in while both windows of the account's meter are under their plan's limit; otherwise 429 `rate_limit_error` + `x-should-retry: false` + `x-anysola-limit` with `limitMessage` (§2.4). 0.5.1 billing admission (admission.ts:261-319): Paid: free = available − reserved in flight − owed. ≤0 → 402 "You've used this cycle's Anysola credits. They renew on YYYY-MM-DD. https://account.anysola.com/billing". free < worst-case reserve → 402 "This request may cost up to N credits and you have M. …". Free tier: daily USD / daily turns → 402 "Daily free limit reached. …".
15. `[1.0]` Each finished request's model cost is added to both windows (percentages only reach the client). 0.5.1 charge: `credits = max(1, ceil(round(costMicros × markup × eventMultiplier) / round(creditUsd × 1e6)))` when cost > 0, else 0 (K metering-core.ts:180-187). Settled after the answer. An aborted stream after `message_start` is charged on an estimate. An upstream failure is not charged.

The `/config, /me, /usage, /tokens*` gate (`authenticate`, G app.ts:58-72) refuses **only** Claude Code marks and bad tokens. UA and `x-anysola-client` are not checked there. `/version` and `/device/*` are unauthenticated.

### 3.1 What a Rust client must send to pass (no server change needed)
- `user-agent` starting `opencode/`. Recommended: identical to 0.5.1, `opencode/latest/2.2.5+anysola.<ver>/cli` for model calls and `opencode/latest/2.2.5/cli` elsewhere.
- `x-anysola-client: opencode` on `/messages` and `/search`.
- No `x-app`, no `claude-cli/` UA, no `claude-code-*` beta.
- `anthropic-version: 2023-06-01`. Only allowlisted betas, and never `compact-*`/`fast-mode-*`. So: no server-side compaction, no `speed`, no `context_management` edits through this gateway.
- Exactly one token header: `authorization: Bearer asc_…` (or `x-api-key` for the env token, as 0.5.1 does).
- `x-anysola-version: <semver>` so Admin's versions panel stays right (optional to pass).

**Recommendation only (server change):** if the Rust client wants its own identity, it can keep the `opencode/` UA prefix (no change needed), or an admin can add its prefix (e.g. `anysola-code/`) to `allowedUserAgents` in Admin > Operations > Code before release. Also confirm `mid-conversation-output-config-2026-07-01` is in `allowedBetas` on live. (The `x-credits-remaining`/`credits_per_turn_estimate` note is moot in 1.0: both are gone.)

---

## 4. Client-side Anysola features (exact logic)

### 4.1 Status line (TUI, `prompt.footer.status`) `[1.0]`
- The account's windows: `/me.limits`, with the windows of each later usage event (the bank stays until the next `/me`, `/limits` or `/usage`). A window whose `resetsAt` has passed reads as 0%.
- Text: `5h ${fiveHour}%`, then `· week ${weekly}%` when the weekly window is the fuller one or at 80% or more, then `· session ${used}/${budget}%` under a session budget (`used` with one decimal under 10%, e.g. `0.4%`). Hover appends `· resets in ${humanDuration}` (the 5-hour window). Nothing before `/me` or a usage event arrives. Click opens `/usage`.
- Examples: `5h 42%`, `5h 30% · week 85%`, `5h 42% · session 12%/20%`, `5h 42% · resets in 2 h`.
- Warning colour when a window is at 80% or more, or the session budget is spent.
- Home footer: `${Plan}[ · 5h resets in 2 h]`.
- 0.5.1 showed `${n} credits · this turn N · session N[/budget]`, `today used/daily` for free accounts, and `renews <date>` on hover.

### 4.2 Banners and warnings `[1.0]`
`session.composer.top` and home footer: event banner `${label}[ · until ${formatDate(until)}]` when `/me.event.label` is set (info colour). The fuller window at 80% or more: `5-hour limit 84% used · resets in 2 h · /usage`, or `5-hour limit reached · resets in 2 h · /usage` at 100% (warning colour; `Weekly limit …` for the weekly one). A toast says the same once per window (keyed on the window and its reset time): `84% of the 5-hour limit used. Resets in 2 h. /usage`. (0.5.1: `Low balance: N credits left · /credits`.)

### 4.3 Budgets `[1.0]`
- Unit: **percent of the 5-hour window**, a whole number 1..100 (`20` and `20%` both parse). The session's use is the sum of `fiveHourPercentUsed` of its usage events. KV `anysola:budget:five-hour` (default) and `budget` in `anysola:usage:<id>`; 0.5.x credit budgets are dropped (§2.3).
- TUI `/budget`: `default` → show/set/`off` the default (`New sessions start with a budget of 20% of the 5-hour window` / `No default budget` / `Default budget removed`). No session → `Open a session first, or set one for new sessions: /budget default <percent>`. No args → `Session budget 12%/20% of the 5-hour window` or `No budget · 4.5% of the 5-hour window used. Set one: /budget <percent>`. `<1..100>` or `off` → set (`Session budget …`) or clear (`Budget removed`). Anything else (0, over 100, text) → `Usage: /budget <percent of the 5-hour window> | off | default <percent>`.
- Gate: before submitting a non-shell prompt, if `total ≥ budget`, the select `Budget reached · 21%/20%`: `Continue once`, `Add 20%` (`Budget 40% of the 5-hour window`: `min(100, max(budget, ceil(total)) + budget)`; left out when the budget is already 100), `Remove the budget`, `Stop` (keeps the composer). A running turn is never cut.
- `anysola run --budget N` (1..100; 0 → "Expected a value greater than or equal to 1", over 100 → "Expected a value less than or equal to 100"): sets the session budget. If the session already used N% → `Budget reached: …` and stop. Otherwise before each step with total ≥ N the run stops with `Budget reached: ${used}/${N}% of the 5-hour window. Continue with \`anysola run -s ${id} --budget <percent>\`.` (exit 1).
- `anysola run --use-banked-reset`: before the turn `GET /limits`; when `allowed` is false and the bank holds one, `POST /limits/redeem` and print `Anysola · banked reset used · 5h 0% · week 18%` (dim; the windows after the redeem, status-line rule). If a full window refuses the turn later (§2.4) and no reset was spent yet, redeem once and send `Continue.` to the same session; an empty bank (409) leaves the refusal (exit 1). Without the flag the refusal prints `Error: <message>` and exits 1.

### 4.4 Run summary `[1.0]`
After a non-JSON run whose session used a share of the 5-hour window (`this run` > 0): `Anysola · 5h 42%[ · week 85%] · this run 2%` in dim gray (`this run` = session total − total at start; the windows are the last event's, with the status line's weekly rule). (0.5.1: `Anysola · N credit(s)[ · M left]`.)

### 4.5 Exit epilogue `[1.0]`
TUI exit: wordmark, then `Session <title>`, `Used ${turns} turn(s) · ${total} of the 5-hour window` (only if total > 0), `Limits 5h 42%[ · week …]` when known, `Continue anysola -s <id>`. Plus `Run anysola to use <ver>` after an update. (0.5.1: `Spent N turn(s) · N credits`.)

### 4.6 Dialogs `[1.0]`
- `/credits` is gone. `/usage` (title `Usage · last 7 days`, key `t` toggles 7/30) shows §1.5's lines plus `This session`. Clicking the status line or the home plan line opens it.
- **Banked reset**: when the open session's turn is refused for a full window (§2.4), the client polls `/limits`. With a non-empty bank, a confirm dialog titled `Use a banked reset?` with the message `<gateway message>[ You have N banked reset(s).] The next one expires <date>. Using one clears the window and sends your prompt again.` and the buttons `Wait` / `Use banked reset` (focused). `Use banked reset` → `POST /limits/redeem` → toast `Banked reset used` → the turn goes again: a turn refused before any tool ran is undone (revert to its user message) and resubmitted as it was, so the history keeps one copy; one that already ran tools continues with `Continue.` so their work stays. A failed redeem toasts the error under `Banked reset`. An empty bank (or a session not on screen): an error toast `Usage limit` with the gateway message (it names the reset time).
- `/devices` unchanged (select; the current device toasts "sign out with `anysola logout`"; others confirm, then revoke, then reopen). Error text: 404 → "This server does not offer it yet.", else body message, else `Request failed (HTTP n)` / "Cannot reach Anysola".

### 4.7 Model picker `[1.0]`
Note = `description` from /config (replaces the provider name). No trailing estimate (0.5.1's `~N credit(s)/turn` from `credits_per_turn_estimate` is gone).

### 4.8 Auto and economy `[0.5.1]`
The client has no special logic. `auto` is an ordinary model from /config. Variants come from `variants[]` (`--model anysola/auto#economy` sends `x-anysola-variant: economy`, `output_config.effort:"low"`, `thinking.display:"summarized"`. Observed). The model actually run is in response header `x-anysola-model` (the client does not read it).

---

## 5. Token / credential lifecycle `[0.5.1]`
- Device login result is stored in the credential store as `{type:"oauth", methodID:"device", access:<asc_ token>, refresh:"", expires:<ms from expires_at or now+365d>, metadata:{apiURL, device: hostname}}` under integration `anysola` (C anysola.ts:87-95). Location and encryption: see the storage spec. No refresh flow.
- `ANYSOLA_TOKEN`: integration method `{type:"env", names:["ANYSOLA_TOKEN"]}` (anysola.ts:268). Used for CI. CI tokens (`kind:"ci"`, 30/90/365 d, never sliding, max 5) are minted in Account > Apps > Code. The gateway applies the same gates to both kinds; it has no CI-specific restrictions besides the expiry. On model calls the env token goes in `x-api-key` and a login token in `Authorization: Bearer` (observed).
- Precedence with both a login and `ANYSOLA_TOKEN`: `/config` and `/me` used the stored login (Bearer, observed). `logout` acts on the stored credential. **Model-call behaviour with both present is UNVERIFIED** (the test run lost its fake gateway). Possible risk: `{env:ANYSOLA_TOKEN}` apiKey plus OAuth bearer may collide (C ai/src/providers/anthropic-compatible.ts:69-73 throws when both apiKey and authToken are set). Retest before relying on it.
- `anysola login`: if a token exists and there is no `--force` → "Already signed in (mask)…". `--force` with a stored credential → `POST /tokens/self/revoke` (401 = ok; failure → warn, continue) → remove credential → device flow. First TUI start with no connection, on a TTY → device flow ("Welcome to Anysola Code").
- `anysola logout`: env-only → "Signed in through ANYSOLA_TOKEN; unset it to sign out.". Else revoke on the server (401 = ok). On failure → error and **keep** the token, exit 1. On success → remove the credential.
- 401 handling: there is no automatic re-login. `/config` 401 marks the connection `needs_auth` ("Run `anysola login` to sign in to Anysola") and keeps the cached config. Model-call 401 shows the gateway text. CLI account commands say "The token was revoked or expired. Run `anysola login --force`.". Search says "Your Anysola sign-in has expired or was revoked. Run `anysola login`.".
- The token is never handed to the TUI: all TUI account calls go through the server RPC `anysola.account`.

---

## 6. Rust client contract (code against this)

1. Base = `env ANYSOLA_API_URL` (strip trailing `/`) or `https://api.anysola.com`. All routes are under `/v1/code`.
2. Plain HTTP UA `opencode/latest/2.2.5/cli`. Model UA `opencode/latest/2.2.5+anysola.<ver>/cli` (must start with `opencode/`).
3. Account calls (`GET /config`, `GET /me`, `GET /usage?days=7|30`, `[1.0]` `GET /limits`, `POST /limits/redeem`, `GET /tokens`, `POST /tokens/{urlencoded id}/revoke`, `POST /tokens/self/revoke`): headers `accept: application/json`, `authorization: Bearer <token>`, `x-anysola-version: <ver>`. 15 s timeout. Parse JSON leniently (non-JSON → `{}`).
4. Device login: `POST /device/code {client:"anysola-code/<ver>", deviceName:hostname}` (no auth, `content-type` + `x-anysola-version`). Show `Open <verification_uri> and enter <user_code>`. Poll `POST /device/token {device_code}`: sleep first, `interval = max(1, interval||5)`, +5 per `slow_down` (ignore the server interval), deadline `min(expires_in||600, 900)` s, 3 consecutive 5xx/network failures abort. Accept only `^asc_[0-9A-Za-z]+$`. Error strings per §1.2.
5. `GET /version` (no auth, 5 s; 3 s for the minimum gate). Gate before TUI start and `run`, for released builds only, offline passes, exit 1 with the exact message. TUI: check at start and every 10 min, notify only.
6. `/config`: fetch at start, on credential change, and every 10 min. Cache the raw body in KV `anysola:config`. On failure use the cache. On 401 mark needs_auth. Keep only `providers.anysola`. Override `baseURL` with `<base>/v1/code`. Add headers `x-anysola-client: opencode`, `x-anysola-version`, `[1.0]` `x-anysola-usage-event: 1`. Allow no other provider.
7. `/messages`: Anthropic Messages API, `stream:true`, `anthropic-version: 2023-06-01`, betas ⊆ {`interleaved-thinking-2025-05-14`, `thinking-binding-controls-2026-08-01`, and `mid-conversation-output-config-2026-07-01` only if the live allowlist has it}. Only the allowed top-level fields (§3 item 11). Token: Bearer for a login token, `x-api-key` for `ANYSOLA_TOKEN`. Send the session headers of §1.7 (at least `x-opencode-session`). Variant headers from `variants[].headers`.
8. `[1.0]` Stream filter: drop and parse the `event: anysola_usage` block (§2.2). Pass `: keepalive` and everything else through to the SSE parser. Treat `event: error` as a stream error.
9. `[1.0]` Usage: each usage event → record (§2.3). Turn closes on idle. Persist `{used,turns,budget}` in `anysola:usage:<id>`. Emit to the UI. No `x-credits-*` headers.
10. `[1.0]` A 429 with `x-anysola-limit` on a model call: session error kind `provider.usage-limit`, never retried; TUI offers a banked reset, `run` exits 1 or redeems with `--use-banked-reset` (§2.4, §4.3, §4.6). No 402 rewrite.
11. Retry: honour `x-should-retry` first. Retry 429/5xx/408/409/transport with exp backoff 2 s → cap 10 s, ≤10 retries, jitter, `max(delay, retry-after ≤ 15 min)`. Never retry 400/401/402/403/404/413 or a usage-limit 429.
12. `/search`: `POST {query}`, Bearer + `x-anysola-client: opencode` + version. 30 s timeout, 1 MiB cap. Map results. `[1.0]` No credits bookkeeping. Error texts per user-strings.txt.
13. `[1.0]` UI numbers: windows from `/me.limits` and usage events; warn at 80% (§4.2). /me refresh every 5 min and 1.5 s after usage events. Budget semantics and texts per §4.3. Run summary per §4.4. Exit summary per §4.5.
14. Logout/login --force: `POST /tokens/self/revoke` (401 = success), then delete the local credential (logout keeps it if the revoke failed).
15. Known 0.5.1 quirks to decide on explicitly (the credit ones are moot in 1.0): free-tier "used" always 0 (`turns` vs `usedTurns`). 402 billing URL printed twice. Revoking an unknown id says "This Anysola server does not offer this yet.". `credits_per_turn_estimate` and `x-credits-remaining` are never sent by the real gateway. `remaining` after a turn reflects whichever request finished last.
