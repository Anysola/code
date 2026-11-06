# Session loop, prompts, agents, context management — parity checklist (Anysola Code 0.5.1)

Scope: what the 0.5.1 binary sends to the Anysola gateway and how a turn runs. Source paths are relative to
`/home/ubuntu/anysola-code/build/fork/packages/`. Tags: `[0.5.1]` = exercised by the shipped binary (many confirmed
against captured wire requests), `[dead]` = present in code but unreachable/disabled in the fork.

Ground truth: `anysola run --standalone` was driven against a logging fake gateway
(`/home/ubuntu/anysola-code/rust/testing/runs/session/logger.mjs`, request dumps under `.../runs/session/logs/<scenario>/`).
The interesting captures are copied to `spec/verbatim/session/requests/*.json` (full headers + body).

Verbatim artifacts: `spec/verbatim/session/` — see the index at the end of this file.

---

## 1. Request construction (Anthropic Messages via the Anysola provider)

### 1.1 Endpoint, transport, headers

- [0.5.1] Provider `anysola` is pinned to package `@opencode/ai/providers/anthropic-compatible`, baseURL
  `${ANYSOLA_API_URL or https://api.anysola.com}/v1/code`; POST `/v1/code/messages` (non-`anthropic` provider id ⇒ no
  `?beta=true` suffix). `core/src/plugin/provider/anysola.ts:192-206`, `ai/src/protocols/anthropic-messages.ts:1691-1702`.
- [0.5.1] Always `stream: true`; SSE parse of standard Anthropic events plus the gateway's extra
  `anysola_credits` event (gateway area; `[1.0]` `anysola_usage`, `spec/protocol.md §2.2`). `anthropic-messages.ts:1060-1131`.
- [0.5.1] Headers on every model request (captured, `requests/primary-hello-sonnet.json`):
  - `anthropic-version: 2023-06-01` (route default).
  - `anthropic-beta`: always `interleaved-thinking-2025-05-14`; plus `thinking-binding-controls-2026-08-01` whenever the
    body's `thinking` has `block_binding` (always for Anysola models, §1.4); plus
    `mid-conversation-output-config-2026-07-01` when any message is a `system` effort marker (§1.7);
    `compact-2026-01-12` / `fast-mode-2026-02-01` only with native compaction / `speed` [dead for Anysola].
    Order: interleaved first, then the others in the order above, comma-joined, deduped with any configured value.
    `anthropic-messages.ts:1667-1689`.
  - `x-api-key: <token>` when the credential is a key (the `ANYSOLA_TOKEN` env method); a device-login OAuth credential
    is sent as `Authorization: Bearer` instead (gateway area; code, not captured). `ai/src/providers/anthropic-compatible.ts:34-37,65-82`.
  - `x-anysola-client: opencode`, `x-anysola-version: 0.5.1`, `x-anysola-credits-event: 1` (`[1.0]` `x-anysola-usage-event: 1`)
    (`plugin/provider/anysola.ts:199-204`); `x-anysola-variant: economy` only when the economy variant is selected
    (comes from the variant's `headers` in `/v1/code/config`).
  - Session headers from `core/src/session/model-request.ts:291-305`: `x-opencode-session-id: <ses_…>`,
    `x-session-affinity`, `X-Session-Id` (wire: `x-session-id`), `x-opencode-session` (all = affinity = session id),
    `x-opencode-project: <project id or "global">`, `x-opencode-client: cli` (app name; TUI sends its own name),
    `User-Agent: opencode/latest/2.2.5+anysola.0.5.1/cli`; for child sessions also `x-opencode-parent-session-id` and
    `x-parent-session-id`.
  - Tracing headers `traceparent` and `b3` are present (OTel propagation; values random). Not required for parity of
    behavior, but harmless to send.
- [0.5.1] Title requests send the same header set (they are ordinary model requests).
- [0.5.1] `promptCacheKey` = session id minus `ses_` (only used by routes that support it; Anthropic route ignores it).
  `model-request.ts:307`.

### 1.2 Body fields (observed)

Captured body keys, in order: `model, system, messages, tools, [tool_choice], stream, max_tokens, thinking,
[output_config]`. Never sent by 0.5.1 for Anysola: `temperature`, `top_p`, `top_k`, `stop_sequences`, `metadata`,
`service_tier`, `speed`, `container`, `context_management`, top-level `cache_control`.
`anthropic-messages.ts:1093-1130`.

- [0.5.1] `model`: the gateway model id (`claude-sonnet-5-5`, `claude-haiku-5-5`, `auto`).
- [0.5.1] `max_tokens` (primary): `outputLimit(limit, inputTokens, 0.01)` = `min(limit.output (fallback 32000), 256000)`
  then, when the context window is known, `min(that, max(1024, floor(context*0.99) − measured − ceil(estimated*1.15)))`.
  With the gateway's `limit.output: 32000` this is 32000 unless the window is nearly full. `model-request.ts:50-99,270-274`.
- [0.5.1] `max_tokens` (title / generate): provider default `32000` (`DEFAULT_MAX_TOKENS`), hooks may change it.
- [0.5.1] `max_tokens` (compaction): `outputLimit(limit, inputTokens)` without headroom, then `fitOutput` (§5).
- [0.5.1] `tool_choice`: omitted, except `{type:"none"}` on the step-capped final step (§2.4). `{type:"auto"/"any"/"tool"}`
  lowering exists but no Anysola code path sets it.
- [0.5.1] Thinking (§1.4) and effort (§1.5).
- [dead] `temperature`/`top_p` from a V1-style agent file (`temperature: 0.3` in frontmatter) are migrated into
  `agent.request.body` (`v1/config/migrate.ts:34-53`, merged at `config/plugin/agent.ts:110-113`) but no consumer of
  `agent.request` exists in the request path (`session/runner/model.ts:75-85` only reads `agent.model`); not observed on
  the wire (`logs/stepper`). Agent `request.headers/body` are therefore inert in 0.5.1.

### 1.3 System prompt assembly (exact order)

`SessionModelRequest.baseTranscript` (`core/src/session/model-request.ts:101-124`) creates two parts, then `context` hooks
edit them in plugin registration order (`core/src/plugin/internal.ts:225-265`). Final wire order (4 blocks, captured):

| # | Content | Source |
|---|---|---|
| 0 | Agent system prompt: `agent.system` if the agent has one, else `system.txt` with `${OPENCODE_TOOL_GUIDANCE}` rendered from the advertised tool names; then, only when the agent has NO custom `system`, `"\n\n" + anthropic.txt` appended | `session/system-prompt.ts:5-28`, `plugin/optimize.ts:23-32,80-105` |
| 1 | Identity block `# Your Model\n- Name: <model.name>\n- Provider ID: anysola\n- Model ID: <id>` (spliced at index 1) | `plugin/identity.ts:10-31` |
| 2 | Joined initial instructions (§4): Code Mode catalog, MCP guidance, references, skill guidance, AGENTS.md/instruction files, date, env — each source separated by `\n\n` | `session/context.ts:152-163`, `instructions/index.ts:176-178,238-240` |
| 3 | Worktree hint (pushed last) | `tool/plugin/opencode.ts:99-107` |

- [0.5.1] Tool guidance lines are inserted only for tools actually advertised: `shell` ⇒ 2 lines, `write` ⇒ 1, `edit` ⇒ 1,
  joined by `\n`. Wire block 0 therefore ends `...overwriting them.\n` + `\n\n` + `# Code comments...` (three newlines
  between). Use `verbatim/session/system-block0-default-plus-anthropic.wire.txt` byte-for-byte.
- [0.5.1] Selection rule for the appended model prompt: `AnthropicPlugin` matches when model id contains `claude` OR
  provider is `anysola` (Anysola change, so `auto` also gets it); mode `append`; skipped when the agent has its own
  `system`. `plugin/optimize.ts:23-32,94`.
- [dead] GPT/GPT-astra (override), Kimi, Trinity (Arcee), Meta (`{{MODEL_NAME}}`) prompt variants: only match model ids
  containing `gpt`/`kimi`/`trinity`/`muse`; Anysola serves only Claude ids and `auto`, and the provider lock removes all
  other providers. Copied to `verbatim/session/dead-variants/` for reference.
- [dead] `OpenAIToolsPlugin` / `AnthropicToolsPlugin` (delete grep/glob) are defined but not registered;
  grep and glob are advertised. `plugin/optimize.ts:34-51`.
- [dead] `VerbosityPlugin` (`textVerbosity: low`) only for OpenAI packages. `plugin/verbosity.ts`.
- [dead] `PatchTool` swap (patch instead of edit/write) only for `gpt-` model ids. `tool/plugin/patch.ts:318-327`.
- [0.5.1] Custom-system agents (explore, title, summary, any agent file with a body): block 0 is exactly `agent.system`
  (no tool guidance, no anthropic.txt); identity, instructions and worktree hint still follow (captured
  `requests/primary-explore-agent.json`, `system-block0-explore-agent.wire.txt`).
- [0.5.1] Empty parts are filtered before sending (`text.length > 0`). `anthropic-messages.ts:1080`.
- [0.5.1] The initial instruction text (block 2) is FROZEN per session: it is the baseline recorded at the session's first
  step; later changes are delivered as chronological system updates (§4.5), so the system prefix stays byte-stable for
  the prompt cache. `session/instruction-state.ts:30-60`.

### 1.4 Thinking

- [0.5.1] For every Anysola model (`claude-*-5-5` parse as Claude 5.5 ⇒ block binding supported; `auto` gets it from
  the gateway's `compatibility.supportsThinkingBlockBinding: true`) the body always carries
  `thinking: {type:"adaptive", block_binding:{prefix_mismatch_behavior:"drop_block"}}` when no variant is selected,
  including title requests. `anthropic-messages.ts:1039-1050`, `ai/src/protocols/utils/claude-model.ts:6-19`.
- [0.5.1] With an effort variant (`#low|#medium|#high|#xhigh|#max`) or `#economy`, the variant settings add
  `display:"summarized"`: `thinking: {type:"adaptive", display:"summarized", block_binding:{...}}`.
  Effort variants are generated from `capabilities.reasoning.effort` by `core/src/variant.ts:42,325-339`
  (`ADAPTIVE_THINKING = {type:"adaptive", display:"summarized"}`); `economy` comes verbatim from `/v1/code/config`.
- [0.5.1] Haiku has no `capabilities.reasoning` ⇒ no effort variants, but still the adaptive default.
- [0.5.1] `budget_tokens` (manual thinking) path: only for models flagged manual (Claude 4.x) [dead for Anysola].
- [0.5.1] Thinking blocks are replayed with their signatures on later requests (captured
  `requests/thinking-replay-next-turn.json`): `{type:"thinking", thinking, signature}` precede text/tool_use in the
  assistant turn. Redacted thinking replays as `{type:"redacted_thinking", data}`. `anthropic-messages.ts:962-982`.
- [0.5.1] Reasoning is dropped from replay when the assistant message errored or its reasoning was interrupted
  (`time.completed` missing); when the model changed (different provider or model id) reasoning text is replayed
  without signature metadata, and Anthropic lowering then drops signature-less thinking
  (`requireThinkingSignature` true for Claude). `core/src/session/runner/to-llm-message.ts:155-222`.
- [0.5.1] Interleaved thinking: always requested via beta header; parser handles thinking between tool calls.

### 1.5 Effort

- [0.5.1] Default (no variant): no `output_config` at all.
- [0.5.1] Variant `#high` ⇒ `output_config: {effort:"high"}`; `#economy` ⇒ `output_config:{effort:"low"}` +
  `x-anysola-variant: economy` (captured). `anthropic-messages.ts:1065,1089-1092`.
- [0.5.1] (code; not captured) Switching variant mid-session on the same model (both effort-like ids or default) inserts an effort marker
  instead of changing the top-level effort, keeping the cache: on the wire a message
  `{role:"system", content:[], output_config:{effort:<new or "high">}}` at the switch position, plus beta
  `mid-conversation-output-config-2026-07-01`; the top-level `output_config.effort` stays at the session's first effort
  (`updates[0].previous`). Only when the model supports effort updates: Claude Opus ≥5 or any Claude ≥5.1
  (`claude-sonnet-5-5`, `claude-haiku-5-5`) or `compatibility.supportsEffortUpdates` (`auto` from config). Otherwise
  markers are stripped. `to-llm-message.ts:224-240`, `ai/src/effort-updates.ts`, `anthropic-messages.ts:917-925,1026-1037`.
  (`economy` is not an effort id ⇒ switching to/from economy emits no marker; the request settings change instead.)

### 1.6 Cache control

- [0.5.1] Policy "auto" (no TTL: gateway config has no `cache.ttl`): 4 breakpoints, all `{"type":"ephemeral"}`:
  last tool definition (`execute` in the default tool list), first system block, last system block (worktree hint),
  and the tail message's last text part (or last cacheable part, e.g. the last `tool_result`). Effort markers are not
  counted as the tail. `ai/src/cache-policy.ts:17-24,89-201`, observed in every capture.
- [0.5.1] Title request: system block 0 (only block, first=last ⇒ one mark) + the user message ⇒ 2 marks; no tools.
- [0.5.1] Cap 4; extra hints are dropped with a warning log. `anthropic-messages.ts:535-545`.

### 1.7 Message transformation (history → Anthropic messages)

`to-llm-message.ts:242-324` then `anthropic-messages.ts:903-1024`.

- [0.5.1] User message content order: user-invoked skill texts (each a `<skill_content …>` block, §7.1) first, then the
  typed text, then attachments (§8). Empty/whitespace text parts are skipped.
- [0.5.1] Assistant: text (skip empty), thinking (§1.4), `tool_use {id (non `[A-Za-z0-9_-]` → `_`), name, input}`.
  Tool results go into the following `user` turn as `tool_result` blocks; consecutive tool-result-only user turns
  merge. Multiple parallel tool calls ⇒ one assistant turn with N tool_use + one user turn with N tool_result
  (captured `requests/parallel-tools-replay.json`).
- [0.5.1] Tool result content: single text output ⇒ string; multi-part ⇒ array of text/image/document blocks; error ⇒
  string `JSON.stringify({error:{type,message},content:[...]})` with `is_error: true` (captured
  `requests/tool-error-replay-with-thinking.json`: `{"error":{"type":"tool.execution","message":"Invalid arguments…"},"content":[]}`).
  `ai/src/protocols/shared.ts:286-296`.
- [0.5.1] Synthetic messages (plan reminders, subagent completions, continuation notes, goal texts) ⇒ plain `user` text.
- [0.5.1] `location-switched` ⇒ user text `The working directory has been changed to <dir>.`
- [0.5.1] User shell (`!cmd` in TUI) ⇒ user text `The following shell command was executed by the user:\n\nCommand:\n<cmd>\n\nOutput:\n<output>`
  (background shells only enter through their completion item).
- [0.5.1] `agent-switched`, `idle` ⇒ nothing (plan reminders are separate synthetic messages).
- [0.5.1] Instruction updates (`system` messages, §4.5): for Claude 5.x model ids a native
  `{role:"system", content:[{type:"text",text}]}` placed after the latest user turn (captured
  `requests/instruction-update-native-system.json`); for `auto` (id does not parse as Claude) wrapped as user text
  `<system-update>\n<escaped text>\n</system-update>` with `& < >` escaped (captured
  `requests/instruction-update-wrapped-auto.json`). `anthropic-messages.ts:851-901`, `shared.ts:186-224`.
  A system update that would split a tool call from its result is a request error.
- [0.5.1] Images: `{type:"image", source:{type:"base64", media_type, data}}`; PDFs as `document` blocks.
  Unsupported modality for the model ⇒ text `ERROR: Cannot read "<name>" (this model does not support image input). Inform the user.`
  Image budget: when inline image payload (base64 chars) passes 25 MiB, oldest images are replaced by the
  `IMAGE_REMOVED` text until ≤15 MiB. `model-request.ts:44-47,126-222`.

---

## 2. The agent loop

### 2.1 Turn / step structure — `core/src/session/runner/llm.ts`

- [0.5.1] A turn = repeated steps; each step = one model request + its tool calls. Continue while the response
  "needs continuation" (tool calls executed, or recoverable stream issues); stop on a finish without pending tools.
  `llm.ts:209-217,230-377`.
- [0.5.1] Input is admitted through an inbox (§6.6). On the first step of a turn the pending input is promoted; title
  generation is forked when ≥1 input was promoted, the session is a root (no parent) and still untitled (§6.1).
  `llm.ts:184-201`.
- [0.5.1] Before every step: plugins settled, context re-selected (agent, tools, instructions), instruction deltas
  committed, then auto-compaction check (§5), then request built. `llm.ts:220-256`.
- [0.5.1] On start of a drain: stale `running` compactions are failed with `{type:"compaction.interrupted", message:"Compaction was interrupted"}`,
  and stale streaming/running tool calls are failed with `Tool execution interrupted: <tool>[ (sessionID: <child>)]`
  (process death recovery). `llm.ts:379-432`.

### 2.2 Tool execution

- [0.5.1] Tools run as soon as each `tool_use` block completes in the stream, concurrently (one fiber per call), while
  the stream continues; the step waits for all of them. `runner/step.ts:103-153`.
- [0.5.1] Tool output passes `ToolOutput.truncate` before being recorded (tools area).
- [0.5.1] Tool failure (`Tool.Error`) ⇒ tool marked error, loop continues; the model sees the error JSON (§1.7).
- [0.5.1] Permission decline / question dismissal ⇒ that tool fails `aborted`/`The user declined this tool call`
  (or the question's message), remaining unsettled tools `Tool execution interrupted`, assistant `Step interrupted`,
  and the turn ends (interrupt). `step.ts:62-64,207-275`.
- [0.5.1] Provider tool call without result ⇒ `tool.result-missing` / `Provider did not return a tool result`.
- [0.5.1] Unknown tool name `bash` is repaired to `shell` with an extra result text
  `There is no "bash" tool, so this ran as "shell". Call "shell" directly.` (tools area; observed).

### 2.3 Abort / interrupt (Esc)

- [0.5.1] `POST /api/session/:id/interrupt` interrupts the drain: stream interrupted ⇒ running tools interrupted,
  unsettled tools failed `{type:"aborted", message:"Tool execution interrupted"}`, assistant `Step interrupted`.
  If cancelled before the start snapshot existed nothing is recorded. `step.ts:140-225`.
- [0.5.1] Manual compaction cancelled ⇒ `{type:"aborted", message:"Compaction cancelled"}`. `llm.ts:158-166`.

### 2.4 Max steps

- [0.5.1] No step cap for built-in agents (`steps` undefined). Agents with `steps: N`: the N-th step of a turn appends
  an assistant message containing `MAX_STEPS_PROMPT` as the last message (assistant prefill; it receives the tail
  cache mark) and sends `tool_choice:{type:"none"}` while keeping the tool list (cache). Any tool call returned anyway
  fails with `Tools are disabled after the maximum agent steps`; that step never continues.
  Captured `requests/max-steps-final-step.json` (steps 3 ⇒ requests 1,2 normal, 3 capped).
  `llm.ts:249-269`, `runner/max-steps.ts`, `step.ts:91-93,279`.
- [0.5.1] `step` resets to 1 when new user input is promoted. `llm.ts:199`.

### 2.5 Retries

- [0.5.1] Classification `ai/src/provider-error.ts:245-320`: overflow phrases on 4xx ⇒ context-overflow; 413 or
  payload text ⇒ payload-too-large; content policy; 402 or quota codes/text on 429 ⇒ QuotaExceeded; 401/403/auth codes ⇒
  Authentication; 429/rate-limit ⇒ RateLimit; 408/409/≥500/server codes ⇒ ProviderInternal; other 4xx ⇒
  InvalidRequest; else Unknown. `x-should-retry: true|false` header overrides.
- [0.5.1] Retryable: RateLimit, ProviderInternal (incl. 529 overloaded), transport failures not rejected,
  incomplete streams, Unknown. Not retryable: auth, quota (402 = out of credits), content policy, invalid request,
  timeout-class. `provider-error.ts:89-121`. `[1.0]` Also never retried: a 429 with `x-anysola-limit` (a full usage
  window, session error `provider.usage-limit`, `spec/protocol.md §2.4`).
- [0.5.1] Schedule: exponential from 2 s, each gap capped at 10 s, max 10 retries, jittered; `retry-after` honoured as a
  minimum (capped 15 min); at most 3 transport-timeout retries. `core/src/model-retry.ts:89-138`.
- [0.5.1] Each scheduled retry publishes `session.retry.scheduled {attempt, at, error}` (TUI shows a retry notice);
  `anysola run` printed only a repeated `> build · <model>` header (observed for 529, 429 with `retry-after: 2`, 500).
  `runner/retry.ts:57-77`.
- [0.5.1] Failure after output started: if retryable and an interrupted stream, the partial assistant is kept and a
  synthetic user message `CONTINUE_AFTER_INCOMPLETE_STREAM` is added, then a new step. `llm.ts:37-38,327-335`.
- [0.5.1] `stop_reason: max_tokens` ("length"): synthetic `CONTINUE_AFTER_OUTPUT_LIMIT` and continue; after three
  consecutive length-finished assistant messages ⇒ error `Response still truncated after two output token limit
  continuations` (type `output-limit`). `llm.ts:40-41,336-366`.
- [0.5.1] Transport errors flagged `retry-full` before any output ⇒ one immediate full resend (no backoff).
  `step.ts:182-188`.
- [0.5.1] Unknown finish reason ⇒ incomplete-stream error (retryable).
- [0.5.1] One-shot generations (title, goal judge, `generate`) retry only "not run" failures (429, 503/529, not-sent
  transport), at most 3 times. `model-retry.ts:153-182`. Title failures fall back to the primary model once, then give
  up silently (§6.1).

### 2.6 Doom loop

- [dead] No doom-loop detection in 2.2.5/0.5.1. `doom_loop` survives only as a V1 config permission key accepted by the
  parser (`core/src/v1/config/permission.ts:27`); nothing evaluates it.

### 2.7 Context overflow

- [0.5.1] A provider "too long" rejection before any output ⇒ overflow compaction (§5) once per step, then the step is
  re-run with the compacted history (captured `logs/overflow`: 400 `prompt is too long…` → compaction request →
  request with only the checkpoint). A second overflow in the same step surfaces as an error. `step.ts:155-167`,
  `llm.ts:304-310,367-370`.

---

## 3. Agents

Built-in agents (`core/src/plugin/agent.ts`, `plugin/plan.ts`); defaults from `schema/src/agent.ts:39-53`
(permissions `* allow`, `read *.env ask`, `read *.env.* ask`, `read *.env.example allow`).

| id | name | mode | hidden | system | extra permissions | notes |
|---|---|---|---|---|---|---|
| build | Build | primary | no | default prompt | `question * allow` | default agent [0.5.1] |
| plan | Plan | primary | no | default prompt | `question allow`, `edit * deny`, `edit <home>/.opencode/plan/* allow` | reminders §3.2 [0.5.1] |
| general | General | subagent | no | default | `question deny`, `subagent deny` | via subagent tool [0.5.1] |
| explore | Explore | subagent | no | `PROMPT_EXPLORE` | deny all, allow shell/grep/glob/webfetch/websearch/read/external_directory, .env ask, subagent deny | [0.5.1] |
| compaction | Compaction | primary | yes | none | — | model override hook only [0.5.1] |
| title | Title | primary | yes | `PROMPT_TITLE` | `* deny` | [0.5.1] |
| summary | Summary | primary | yes | `PROMPT_SUMMARY` | `* deny` | [dead] defined, no caller |

- [0.5.1] Descriptions verbatim in `verbatim/session/source-strings/agent-build-general-explore-plan-defs.ts.txt`.
- [0.5.1] Default agent: config `default_agent` if selectable (not subagent, not hidden), else `build`, else first
  selectable. `core/src/agent.ts:53-64`, `config/plugin/agent.ts:87-88`.
- [0.5.1] `anysola run --agent <id>` uses any existing agent, including subagent-mode ones (observed `--agent reviewer`
  and `--agent explore`).
- [0.5.1] Tab switching = TUI calls `session.switchAgent` (`server/src/handlers/session.ts:217-221`); emits `session.agent.selected` (TUI area).
- [0.5.1] Agent model: `agent.model` overrides the session model for that agent (observed: reviewer `model:
  anysola/claude-haiku-5-5` ran on Haiku). Tools are filtered by the agent's merged permissions (explore advertised
  `glob, grep, read, shell, skill, webfetch, websearch`).

### 3.1 Custom agents

- [0.5.1] Config `agents: {<id>: {model, request, system, description, mode, hidden, color, steps, disabled, permissions}}`
  (V2 schema `schema/src/config/agent.ts`). `disabled: true` removes the agent (built-ins too).
- [0.5.1] Markdown files in every config directory entry (`~/.config/opencode/`, project `.opencode/` dirs walked up,
  etc. — storage/config area): `{agent,agents}/**/*.md` (mode from frontmatter, default primary per `Info.default`) and
  `{mode,modes}/*.md` (forced `mode: primary`). Name = relative path without the dir prefix and `.md`
  (nested ⇒ `a/b`). Body (trimmed) = `system`. `config/plugin/agent.ts:21-34,163-215`.
- [0.5.1] Frontmatter: V2 keys `model, variant, request, system, description, mode, hidden, color, steps, disabled,
  permissions`; `model: provider/id` + `variant: x` are joined to `provider/id#x`. A file whose keys include anything
  else and none of `permissions|request|system|disabled` is parsed as V1 (`model, variant, temperature, top_p, prompt,
  tools, disable, description, mode, hidden, options, color, steps, maxSteps, permission`, unknown keys → options) and
  migrated; colour names become `#aaaaaa`. Invalid YAML with unquoted colons is retried with block scalars.
  `config/plugin/agent.ts:176-215`, `v1/config/agent.ts`, `v1/config/migrate.ts:34-74`, `config/markdown.ts`.
- [0.5.1] Global `permissions` from config documents are appended to every agent; `~/`/`$HOME` expanded for
  read/edit/external_directory resources. Hot reload on file change (100 ms debounce).
- [dead] `.claude/agents` / `.agents/agents` are not agent sources (only skills come from `.claude`/`.agents`).

### 3.2 Plan mode reminders

- [0.5.1] Enter reminder (synthetic user message, persisted) on session created with agent plan, or agent switch to
  plan; leave reminder on switch away from plan. Per request the context hook re-adds a missing/stale reminder just
  before the last user message (captured `requests/primary-plan-agent.json`: reminder as its own user turn before the
  prompt). Plan dir = `<home>/.opencode/plan`. Edit/write/patch permission blocks in plan become
  `Cannot use <tool> to modify files outside the Plan directory: <dir>`. `plugin/plan.ts`.

---

## 4. Instructions

### 4.1 Sources and order inside system block 2 — `session/context.ts:137-163`

1. Code Mode catalog (`# Code Mode …`, tools area for content) — `codemode/instructions.ts`.
2. MCP server instructions (integrations area).
3. References (`ConfigReferencePlugin`; integrations/config area).
4. Skill guidance (§7.1).
5. Discovered instruction files (§4.2).
6. `Today's date: <Date.toDateString()>` (e.g. `Wed Nov 04 2026`).
7. Environment block:
   ```
   Here is some useful information about the environment you are running in:
   <env>
     Working directory: <location.directory>
     Workspace root folder: <project root>
     Is directory a git repo: yes|no
     Platform: <process.platform>
     Prefer <global tmp, /tmp/opencode on Linux> over generic system temporary directories such as /tmp; it is pre-created and approved for external access.
   </env>
   ```
   `instructions/builtins.ts:22-56`. No git status, no directory tree.
8. Session instruction entries (`InstructionEntry`, per-session; integrations of plugins).
- Sources are joined with `\n\n`; a source that is empty/removed is omitted.

### 4.2 Instruction files — `core/src/config/plugin/instruction.ts`

- [0.5.1] ONLY `AGENTS.md` is discovered. `CLAUDE.md`, `CONTEXT.md`, `.cursorrules` etc. are NOT read (grep confirms no
  reference). Users expecting CLAUDE.md get nothing.
- [0.5.1] Order: global `~/.config/opencode/AGENTS.md` (`global.config`), then project `AGENTS.md` files from the cwd
  upward, nearest first (observed from `proj/sub`: `sub/AGENTS.md`, then `proj/AGENTS.md`); then configured
  `instructions` entries in config order. Walk bound: `stop = cwd under $HOME ? $HOME : project root`
  (`instruction.ts:40`) — so for the normal case (project under `$HOME`) the walk goes PAST the repo root up to and
  including `~/AGENTS.md` (log of the `my-app` run watched `<HOME>/AGENTS.md`); outside `$HOME` it stops at the project
  root. The walk only happens when cwd is inside the project root (`instruction.ts:39`); the global file is deduped.
- [0.5.1] `instructions` config entries: `http(s)` URLs fetched at config load (5 s timeout; failure keeps last copy or
  skips); `~/` expanded; absolute paths/globs scanned; relative globs searched upward from cwd to project root
  (`fs.globUp`); results sorted, deduped by real path. Missing files skipped; an access-denied project file makes the
  set "unavailable" (initial baseline then blocks the turn with `Instruction initialization blocked by unavailable
  sources: …`).
- [0.5.1] Render: `Instructions from: <absolute path or URL>\n<file content as-is>`, files joined by `\n\n`
  (content's own trailing newline is kept, hence blank lines). `instruction-discovery.ts:123-125`.
- [0.5.1] Files are watched; edits apply on the next step via §4.5.
- [0.5.1] `/init` command creates/updates AGENTS.md using `initialize.txt` (`${path}` = project dir).

### 4.3–4.5 Deltas mid-session — `instructions/index.ts`, `session/instruction-state.ts`

- [0.5.1] Each source is hashed (sha256 of canonical JSON); the first step stores the baseline. A later change produces
  one `system` message with the concatenated update renders (joined `\n\n`):
  - date: `Today's date is now: <date>`; env: `The environment you are running in is now:\n<env block>`;
  - instruction files: `The instructions from <path> no longer apply.` / `New instructions apply from:\n<render>` /
    for edits the shorter of `The instructions from <path> changed. Here's the diff:\n```diff\n<hunks>\n```` and
    `The instructions changed:\n<render>`; all files removed: `Previously loaded instructions no longer apply.`
    (observed: `The instructions changed:\nInstructions from: …/AGENTS.md\n<new content>`);
  - skills: see `source-strings/skill-guidance.ts.txt`; Code Mode catalog: `codemode/instructions.ts` update texts.
- [0.5.1] Wire form per §1.7 (native system for Claude ids, `<system-update>` user text for `auto`).

---

## 5. Compaction — `core/src/session/compaction.ts`

- [0.5.1] Settings: `compaction.auto` (default true), `compaction.keep.tokens` (default 15000), `compaction.buffer`
  ("N%" or tokens); `experimental.background_compaction` enables precompute (default off).
  `config/plugin/compaction.ts`, `compaction.ts:271`.
- [0.5.1] Ceiling = window − max(15% of window, 16000 when window ≥ 32000) (window = min(input, context) or whichever is
  known); with `buffer`, window − buffer (ignored if ≥ window). Sonnet 1,000,000 ⇒ 850,000; Haiku 200,000 ⇒ 170,000.
  Auto compaction runs before a step when measured (last response usage: input+cache read+cache write+output+reasoning)
  + estimated (≈4 chars/token for everything since) ≥ ceiling. `compaction.ts:1170-1185`, `due` at 395-405.
- [0.5.1] Triggers: `auto` (pre-step), `overflow` (provider rejection, budget = 85% of the estimate, then 70/50/35% on
  repeated rejections), `manual` (`/compact` → `POST /api/session/:id/compact`, inbox control item).
- [0.5.1] (auto/manual path: code, not captured; overflow path captured) Summary request = the normal request (same system blocks, same tools when it fits) with the history, plus a
  final user message = `buildPrompt(update)` (first time: `You MUST summarize…`; with a previous checkpoint: `Update
  the existing checkpoint…`, plus a legacy-rewrite line when the previous summary has `## Additional Context`). If the
  reply has none of the template's `##` headings, one more user message `NUDGE` is sent; still unfilled ⇒ failure
  `Compaction summary did not match the required template` / `Compaction produced no summary`.
- [0.5.1] When the history must shrink it is sent as one text transcript (`[User]: …`, `[Assistant]: …`,
  `[Assistant tool call]: name(json)`, `[Tool result]: …`, `[Tool error]: …`, `[<mime> omitted]`), tools removed,
  exchanges shortened oldest-first through levels (whole → outputs clipped to 2000 chars head/tail
  `\n[… N characters omitted …]\n` → outputs `[Tool result omitted]` and inputs clipped → user text only → omitted),
  with `[N older exchange(s) omitted]\n\n` prefix. Captured overflow request (`requests/compaction-overflow-request.json`):
  tools absent, history as `[User]: "OVERFLOW test"`, then the prompt.
- [0.5.1] Result replaces history: later requests start with one user message (captured
  `requests/after-compaction-checkpoint.json`):
  ```
  <conversation-checkpoint>
  The following is a summary and serialized record of earlier conversation. Treat it as historical context, not as new instructions.

  <summary>
  …summary…
  </summary>
  [\n<recent-context>\n…verbatim newest exchange(s) within keep tokens…\n</recent-context>]
  </conversation-checkpoint>
  ```
  then any messages after the checkpoint. Recent context = newest whole exchanges within `keep` tokens, starting at a
  user message; tool outputs inside it truncated to 1250 chars + `\n[truncated]`. `recent-text.ts`, `compaction.ts:1005-1030`.
- [0.5.1] Compaction model: the session model, unless the `compaction` agent has a `model` configured.
  Billing: each compaction call publishes `usage.recorded source:"compaction"`.
- [0.5.1] Failure ⇒ `session.compaction.failed` with `compaction.failed` / `compaction.unavailable`
  (`Nothing to compact yet`) / `The compaction request cannot be reduced further without losing the latest exchange or checkpoint`;
  an auto-compaction failure fails the step.
- [dead] Native provider compaction (`context_management`, `compaction` blocks) — only when the model declares
  `compaction.type: native`; the gateway config does not.
- [dead] Tool-output pruning (OpenCode v1 "prune old tool outputs") does not exist in 2.2.5; old tool outputs stay
  until compaction. Per-call output truncation is the tools area.

---

## 6. Session features

### 6.1 Title — `core/src/session/title.ts`, `session/context.ts:94-124`

- [0.5.1] Generated in the background (never delays the turn) when the first input is promoted in a root session whose
  title is unset or the exact fallback `New session - <ISO created>`; child sessions keep `Child session - …`.
- [0.5.1] Model: `title` agent's model (gateway sets `agents.title.model = anysola/claude-haiku-5-5`) with its minimal
  variant (`none|minimal|low` if present — Haiku has none); else `small_model`; else primary model at a lower variant;
  on failure retry once with the primary model.
- [0.5.1] Request: system = `PROMPT_TITLE` only (no identity/instructions/tools), one user message
  `Here is the first message of the conversation:\n\n<message>\n<first user text>\n</message>\n\nReply with only the title.`
  Regeneration (code, not captured; `server/src/handlers/session.ts:236-247`, session.update with empty title): `Here is the conversation so far:\n\n<conversation>\nOriginal request:\n<first 2000 chars>\n\nRecent conversation:\n<User:/Assistant: lines, last ≤8000 chars budget>\n</conversation>\n\nReply with only the title.`
- [0.5.1] Title = first non-empty trimmed line, truncated to 100 chars (`…` as `...`); dropped if the title changed
  meanwhile. Usage recorded as `source:"title"`.
- [0.5.1] `anysola run --title <t>` sets the title (empty ⇒ first 50 chars of the message + `...`) — CLI area.

### 6.2 Sharing

- [dead] No session sharing in the fork (no share code in core; README "No … session sharing").

### 6.3 Fork

- [0.5.1] `POST /api/session/:id/fork` (`server/src/handlers/session.ts:202`; TUI fork, `anysola run --fork` with `-c`/`-s`) — storage area for copy
  semantics; the forked session gets its own id/affinity (cache key not shared, TODO in `model-request.ts:306`).

### 6.4 Revert / undo / redo — snapshots

- [0.5.1] Snapshots: per-location git shadow repo at `<data>/snapshot/<projectID>/<hash(worktree)>`
  (`~/.local/share/opencode/snapshot/…`), `--git-dir <that> --work-tree <project>`; tree captured at each step start
  (in parallel with the request) and after the step; untracked files >2 MiB skipped; ignore rules from the real repo;
  periodic pack every 10 min. Only when the project is a git repo and `snapshots` config ≠ false. Changed files per step
  are stored on the assistant message (`snapshot.start/end/files`). `core/src/snapshot.ts`, `runner/step.ts:234-258`.
- [0.5.1] Revert stage (undo to a message): restores every file changed by assistant messages after that message to the
  start tree of the first step that changed it; records `{messageID, snapshot(original), files(diff)}`. Clear (redo)
  restores the original tree for those files. Commit (sending a new prompt while staged) truncates history after the
  message (`RevertEvent.Committed`; code, not captured). `session/revert.ts:22-112`, API `protocol/src/groups/session.ts:474-505`. UI is TUI area.

### 6.5 Child sessions (subagents)

- [0.5.1] Subagent tool / subagent commands create a child session (`parentID`, agent, model = command/agent model or
  parent model), first prompt `You are a subagent spawned by another session.\n<prompt>`, run as a background job; the
  parent receives a synthetic message
  `<subagent sessionID="<child>" state="completed|error|cancelled" description="<desc>">\n<text>\n</subagent>`
  (text = child's last assistant text parts joined `\n\n`, or `Subagent completed without a text response.`,
  `Subagent failed`, `Subagent cancelled`). `session/subagent-completion.ts`, `config/plugin/command.ts:92-117`.
- [0.5.1] Subagent tool description lists `- <id>: <description or "This subagent should only be called when explicitly requested.">`.

### 6.6 Continue / resume / queueing

- [0.5.1] `-c` continues the latest session, `-s <id>` continues or creates; history replays from the latest completed
  compaction onward (CLI/storage area; `cli/src/run/run.ts:94-125`, `session/context.ts:168-184`).
- [0.5.1] Input while busy: inbox items with delivery `steer` (default for prompts; promoted at the next step boundary
  mid-turn, i.e. injected into the running turn) or `queue` (only promoted at idle, starts a new turn; one at a time).
  Compaction and move items are controls promoted at boundaries. Delivery of a pending item can be switched
  (`PATCH …/inbox/:id`) or cancelled. `session/inbox.ts:52-57`, `llm.ts:61-207`.
- [0.5.1] Session over budget asks before the next turn (gateway/Anysola area).

### 6.7 Warming (keep-alive)

- [0.5.1, opt-in] `warming: true|{prompt, interval, duration}` sends a `generate` keep-alive (`This is a keep-alive request. Do not perform any work or use tools. Reply with exactly: OK`) every 4 min for 30 min after activity, root sessions only. Off by default. `plugin/warming.ts`.

### 6.8 Goal loop (`/goal`)

- [0.5.1] Built-in command `/goal <objective> | pause | resume | clear` (`keep working until an objective is met`):
  kickoff synthetic message, reviewer ("judge") one-shot generate at each idle, continuation synthetic messages, max 50
  rounds, 3 stalls, 30 min wait limit, objective ≤4000 chars, reminder re-added near the tail after compaction.
  Prompts: `verbatim/session/goal/*.txt`. `plugin/goal.ts`.

---

## 7. Skills and commands

### 7.1 Skills

- [0.5.1] Discovery locations: every config directory entry's `skill/` and `skills/` (global `~/.config/opencode`,
  project `.opencode` dirs); config `skills: [...]` (paths relative to cwd, `~/`, or `http(s)` index URLs fetched as
  `<url>/index.json` → `{skills:[{name, version?, files[]}]}`, cached in `<cache>/skills/<hash>`); compatibility dirs
  `~/.claude/skills`, `~/.agents/skills`, and `.claude/skills`, `.agents/skills` in every ancestor of cwd up to `/`
  (observed leaking `/home/ubuntu/.claude/skills` when cwd is under `/home/ubuntu`). Later sources override same id.
  `config/plugin/skill.ts`, `config/plugin/compatibility.ts`, `config/discovery.ts:59-145`, `skill/discovery.ts`.
- [dead] Built-in skills (OpenCode docs, issue report) removed by Anysola.
- [0.5.1] Scan: `SKILL.md` in a directory ends descent; at depth 0 any `*.md` file is a skill; depth ≤6; dot dirs and
  ignored dirs skipped. Id = file stem for top-level `*.md`, else parent dir name. Frontmatter `name`, `description`,
  `metadata["opencode/autoinvoke"]`, `disable-model-invocation`. `config/plugin/skill-file.ts`, `skill-scan.ts`.
- [0.5.1] (`skill/instructions.ts:79-96`) Listed in the system prompt only when it has a description, autoinvoke ≠ false and permission `skill:<id>` is
  not denied; sorted by id; format in `source-strings/skill-guidance.ts.txt` (captured in block 2). When none:
  the source is omitted.
- [0.5.1] Loading (skill tool or user `@skill` mention): `<skill_content name="…">\n# Skill: …\n\n<body>\n\nBase directory for this skill: <dir>\nRelative paths …\nNote: file list is sampled.\n\n<skill_files>\n<file>…</file>×≤10\n</skill_files>\n</skill_content>`.
  User-invoked skills are prepended to the user message. `skill.ts:36-74`, `session/prompt.ts:58-78`.

### 7.2 Commands

- [0.5.1] Built-ins: `init` (`guided AGENTS.md setup`), `review` (`review changes [commit|branch|pr], defaults to
  uncommitted`), `goal`, MCP prompts as `<server>:<prompt>` (non `[A-Za-z0-9_-]` → `_`). Built-in templates: `$ARGUMENTS`
  replaced by trimmed input, else input appended after `\n\n`. `plugin/command.ts`.
- [0.5.1] Markdown commands: `{command,commands}/**/*.md` in each config directory; name = relative path w/o `.md`;
  frontmatter `description, agent, model, subagent, subtask`; body = template. Config `commands: {name: {template,…}}`.
  V1 `variant` joined into model. `config/plugin/command.ts:137-181`.
- [0.5.1] Templating: `$ARGUMENTS` = raw input; `$1..$N` positional (args split on whitespace, quotes and `[Image N]`
  kept together, quotes trimmed); the highest placeholder swallows the rest; missing ⇒ empty. No placeholders and no
  `$ARGUMENTS` ⇒ template + `\n\n` + input. Then every ``!`cmd` `` is replaced by the command's combined stdout+stderr
  (configured shell, cwd = location dir, concurrency 2); failure aborts with `Shell interpolation failed for "<cmd>": …`.
  `config/plugin/command.ts:183-245`.
- [0.5.1] `@file` in command templates is NOT expanded (no such regex in core; stays literal text).
- [0.5.1] Execution: switches agent/model if the command sets them (emits agent/model switch), then prompts the same
  session; `subagent: true` or a subagent-mode agent ⇒ child session (§6.5) with title = description or name.
- [0.5.1] `anysola run` has no slash-command path: `/greet x` is sent as plain text. Commands run via TUI/ACP
  (`session.command` API).

---

## 8. File references and attachments

- [0.5.1] TUI `@path` (autocomplete) adds an attachment with `uri file://…` (+`?start=&end=` for `#L-L` ranges; name
  `path#a-b`) and a mention. Server reads it (`session/prompt.ts:92-245`): >20 MiB rejected
  (`Attachment exceeds the 20971520 byte limit: …`); directory ⇒ listing (dirs first, `name/`), mime
  `application/x-directory`; mime by magic bytes (png/jpeg/gif/bmp/webp/pdf/avif/heic/mp4…), else text vs
  `application/octet-stream`; images resized/normalized.
- [0.5.1] Model text: text file ⇒ extra text part `\n\nAttached file: <name or uri>[\nDescription: …]\n\n<content>`;
  line range keeps only those lines; directory ⇒ `\n\nAttached directory: <path>[\nDescription…]\n\n<listing>`; images/PDF ⇒
  `Attached file: <local path>` text (file:// sources only) + media block; binary other types ⇒ dropped; duplicate
  identical images with the same mention deduped. `to-llm-message.ts:18-103`.
- [0.5.1] `anysola run --file f` (CLI path, ≤10 MiB, no directories): text files are inlined into the message text as
  `<message>\n\n<file name="<basename>">\n<content>\n</file>`; images/PDF sent as inline attachments ⇒ image block with
  no `Attached file:` line (captured `requests/run-file-attachments.json`). `cli/src/run/noninteractive.ts:878-889`.
- [0.5.1] `anysola run` does not parse `@path` in the message.
- Cross-area (CLI): `anysola run a "b c"` sends `a "b c"` — every multi-word argv element is wrapped in double quotes
  (`cli/src/run/run.ts:244-247`); stdin is read and merged when not a TTY (`run.ts:79`), which hangs if stdin stays open.

---

## 9. Auto model and economy variant (client side)

- [0.5.1] Models, default (`model: anysola/claude-sonnet-5-5`), small model and title-agent model come from
  `GET /v1/code/config`; variants come from the config (`economy`) plus generated effort variants (§1.4).
- [0.5.1] `auto` is sent as `model: "auto"`; identity block shows `Name: Auto … Model ID: auto`; the gateway picks the
  real model. Client keeps the ref `anysola/auto` for replay, so thinking signatures are replayed across turns
  regardless of which model answered. Because `auto` does not parse as a Claude id: no native system updates (wrapped
  `<system-update>`), effort updates and thinking binding only via the config `compatibility` flags.
- [0.5.1] Economy: request settings `thinking {adaptive, summarized}`, `effort: low` ⇒ wire `output_config.effort:"low"`,
  plus header `x-anysola-variant: economy` (captured for both `claude-sonnet-5-5#economy` and `auto#economy`).
  Selected with `--model anysola/<id>#economy` or the TUI variant picker.
- [0.5.1] Title requests never carry the session variant (title model + its own minimal variant).

---

## Rust implementation notes

- Reproduce the wire body from `verbatim/session/requests/*.json`; diff against these byte-for-byte (minus session ids,
  trace headers, paths, base64). Key order in the JSON body is the TS literal order (§1.2).
- Build the system array exactly as: [agent prompt (+`\n\n`+anthropic.txt), identity, joined instructions, worktree
  hint]; cache marks on block 0 and block 3, last tool, tail message. Keep instruction baseline frozen per session and
  persist it (the TS stores hashes + blobs in SQLite); emit deltas as system messages.
- Thinking default must always include `block_binding` for Claude ≥5.1 ids and for `auto` (config flag); the beta
  header follows from the body.
- Tool calls must start executing while the stream is still open and run concurrently; preserve result order by call
  order in the replayed user turn.
- Retry schedule and classification tables are small and deterministic; port them literally.
- Compaction: port ceiling math, token estimate (chars/4 via `util/token.ts`), template detection by `##` headings,
  nudge, shrink steps, transcript levels and checkpoint wrapper text.
- Instruction discovery is AGENTS.md-only; skill discovery walks to `/` for `.claude/skills`/`.agents/skills`.
- Snapshot shadow repo uses the system `git` with `--git-dir/--work-tree`; reuse the same directory layout so existing
  sessions' snapshots stay valid.
- Effort marker messages (`role:"system", content:[], output_config`) must be emitted only for effort-like variants on
  the same model.

## Dead in 0.5.1

- Non-Anthropic system prompt variants (gpt, gpt-astra, kimi, trinity, meta) and verbosity/patch/tools-trim plugins.
- `summary` agent (defined, unused).
- Doom-loop detection (only a V1 permission key).
- Tool-output pruning (not in 2.2.5).
- Session sharing.
- Native compaction / `context_management`, `speed`, manual `budget_tokens` thinking, `temperature`/`top_p` on the wire.
- CLAUDE.md as an instruction file (never existed in 2.2.5; only AGENTS.md).
- Built-in skills (removed by Anysola).
- `.claude`/`.agents` agent and command directories (only skills are read from them).
- `@file` expansion inside command templates.

## Verbatim index (`spec/verbatim/session/`)

- `system.txt.template` — default prompt; `${OPENCODE_TOOL_GUIDANCE}` replaced per §1.3 (`source-strings/system-prompt-tool-guidance.ts.txt`).
- `anthropic.txt` — appended `# Code comments` text (selection rule §1.3).
- `system-block0-default-plus-anthropic.wire.txt`, `system-block1-identity.wire.txt`,
  `system-block2-instructions.wire.txt` (project with skills, global+project AGENTS.md, instructions glob, date, env;
  Code Mode catalog included), `system-block3-worktree-hint.wire.txt`, `system-block0-explore-agent.wire.txt` — exact
  wire bytes (paths/dates are from the capture; template variables: model name/id, tool list, paths, date, platform).
- `agent-title.system.wire.txt` — title system prompt as sent.
- `compaction-prompt-initial.wire.txt` — `buildPrompt(false)` as sent.
- `source-strings/*.ts.txt` — exact source snippets for every other text: agent prompts and descriptions, plan
  reminders, identity, max steps, continuation texts, image/media texts, env/date, instruction renders and deltas,
  skill guidance and skill content, Code Mode header, worktree hint, warming, `<system-update>` wrapper, history
  conversions (checkpoint, shell, location, attachments), compaction prompts/NUDGE/transcript texts, recent-text,
  title prompts, subagent completion, command built-ins and template evaluation, step error texts, stale tool texts,
  run file inlining and argv quoting, goal helpers.
- `initialize.txt` (`/init`, `${path}`), `review.txt` (`/review`, `${path}`, `$ARGUMENTS`), `schedule.txt` (schedule tool).
- `goal/{kickoff,continuation,judge,reminder}.txt` — `/goal` prompts.
- `dead-variants/*.txt` — unreachable provider prompts.
- `requests/*.json` — full captured requests (headers + body) for each scenario named in this file.
