# Anysola Code 0.5.1: tools and permissions parity checklist

Scope: everything the model and the user see from tools and permissions in the shipped 0.5.1 binary (fork of OpenCode
2.2.5, branch `anysola`). Paths are relative to `/home/ubuntu/anysola-code/build/fork/packages/` unless absolute.
Tags: `[0.5.1]` exercised by the shipped binary; `[conditional: <gate>]` present but off by default; `[dead]` disabled
or removed in the fork, with the reason.

Ground truth: request bodies captured from the binary (linux-arm64) against a scripted local gateway that logs
`/v1/code/messages` and answers with scripted tool calls. Captures and verbatim texts:
`/home/ubuntu/anysola-code/rust/spec/verbatim/tools/` (one `<tool>.md` per tool: source paths, description, JSON
schema, every model-visible output and error text, captured tool_results; plus `_captured-*.json` request bodies).

## Tools sent to the model (build agent, default config)

Order as sent: direct tools sorted by name, then `execute` last with `"cache_control": {"type": "ephemeral"}`
(`core/src/tool.ts:310-315`). Anthropic Messages format: `{name, description, input_schema}`.

1. `edit` 2. `glob` 3. `grep` 4. `question` 5. `read` 6. `shell` 7. `skill` 8. `subagent` 9. `webfetch`
10. `websearch` 11. `write` 12. `execute` (Code Mode; catalog in system block 2: `opencode` namespace with
`list_mcp_resources`, `models`, `read_mcp_resource`, `session_move`, `session_rename`, `worktree_create`,
`worktree_list`, `worktree_remove`).

Identical for `anysola/auto`, `anysola/claude-sonnet-5-5`, `anysola/claude-haiku-5-5` (diffed). Not sent by default:
`todowrite`, `schedule_*`, `artifact_*` (experimental flags), `experimental_image_generate`, `patch` (dead for Anysola
models). Per-agent sets are in "Tool sets per agent" below.

Every tool failure reaches the model as `is_error: true` with content
`{"error":{"type":"<type>","message":"<message>"},"content":[]}` (`core/src/session/runner/to-llm-message.ts:143-150`).

## File tools (read, edit, write, patch)

Verbatim descriptions, schemas and every output/error text: `verbatim/tools/{read,edit,write,patch}.md`. Paths below are under `packages/` of the fork.

### Shared: error envelope and path resolution
- [0.5.1] Tool failure reaches the model as a `tool_result` with `is_error: true` and string content `{"error":{"type":"<type>","message":"<msg>"},"content":[]}`; type `tool.execution` (plain ToolFailure), `permission.rejected` (BlockedError / CorrectedError / non-interactive decline), `unknown` (ToolFailure wrapping an error with its own message; that message replaces the tool's). `core/src/session/to-session-error.ts:73-81`. Captured.
- [0.5.1] Invalid input: `Invalid arguments for tool "<name>":\n- <path>: <issue>` (max 5 issues, then `- ...and N more issue(s)`) + `\n\nArguments provided:\n<JSON.stringify(input,null,2)>\n\nUpdate the arguments and call the tool again.` `core/src/tool/runtime.ts:88-100`. Captured (`offset: Expected a value greater than or equal to 0`).
- [0.5.1] Path resolution: `~`, `~/x` expanded to home (and `~\x` on Windows); relative paths resolve against the session Location directory. `core/src/file-access.ts:68-78`.
- [0.5.1] Windows: `/c/...`, `/c:/...`, `/cygdrive/c/...`, `/mnt/c/...` rewritten to `C:/...` before resolution (only when platform is win32). `util/src/fs-util.ts:275-282`.
- [0.5.1] Internal = inside Location dir, or inside the project worktree unless the worktree is a filesystem root. Internal resource = Location-relative path with `/` separators (`.` for the dir itself); external resource = absolute path with `\` replaced by `/`. `core/src/file-access.ts:95-123`.
- [0.5.1] External path: permission `external_directory` asked first with resource `<dir>/*` (dir = the directory itself for directories, else dirname) and save pattern `<project root of that dir or the dir>/*`; deduped per call. Default rules allow it for build (`*` allow), so no prompt by default; captured external read and write succeeded without `--auto`. `core/src/file-access.ts:107-142`.
- [0.5.1] `read` permission asked with resource = resource string, save `["*"]`; `.env` / `*.env.*` ask by default (`*.env.example` allowed). Non-interactive `run` without `--auto` rejects with `This non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action.` (captured); with `--auto` it reads. `core/src/file-access.ts:144-161`, `schema/src/agent.ts:46-51`.
- [0.5.1] Mutations serialize per resolved path with a process-local keyed mutex (edit/patch lock the whole read-modify-write). `core/src/file-mutation.ts:59-78`.

### read
- [0.5.1] One tool for files and directories; `codemode: false`; permission action `read`. `core/src/tool/plugin/read.ts:40-46`.
- [0.5.1] Media detection by magic bytes of the first 256 KiB chunk: PNG, JPEG, GIF, WebP, PDF are media; BMP/AVIF/HEIC/MP4 are not readable media (BMP fell to `Cannot read binary file`, captured). `core/src/mime.ts:3-32`, `core/src/tool/read-filesystem.ts:21,136`.
- [0.5.1] Media > 20 MiB (20971520 bytes): error `Media exceeds 20971520 byte ingestion limit: <resource>`. Otherwise whole file base64, content `[text "Image read successfully"|"PDF read successfully", file]`. `read-filesystem.ts:136-149`, `read.ts:203-212`.
- [0.5.1] Unpaged mode: file size <= 51200 bytes and no `offset`/`limit` given -> whole file, NO line cap (captured: 3000-line 25 KB file returned all 3000 lines), each line clamped to 2000 chars, `\r` kept (captured `1: alpha\r`). Binary iff any NUL byte. `read-filesystem.ts:151-162`.
- [0.5.1] Paged mode (file > 51200 bytes, or offset/limit present): offset 1-based (0 or missing = 1), limit default and cap 2000 (0 = 2000), page also stops before exceeding 51200 bytes of output lines (bytes of clamped lines + 1 per separator); trailing `\r` stripped per line; streamed in 256 KiB chunks; binary iff a NUL byte within the consumed range. `read-filesystem.ts:164-224,291-324`.
- [0.5.1] Offset past the end of a file: error `Offset <o> is out of range for this file (<n> line|lines)`; past the end of a directory: `Read directory <p>, 0 entries` (no error). Captured. `read-filesystem.ts:246-247`.
- [0.5.1] Line-number format `<n>: <line>`, header `Read file <path>, lines <a>-<b>` / `Read file <path>, 0 lines`, continuation `[Output truncated. Continue reading with offset: <next>]`. A trailing newline does not create an extra line; blank trailing lines are shown as `n: `. `read.ts:227-237`.
- [0.5.1] Line clamp: >2000 UTF-16 units -> first 2000 + `... (line truncated to 2000 chars)`. `read-filesystem.ts:18-20,326`.
- [0.5.1] Invalid UTF-8 without NUL is decoded with U+FFFD replacement (TextDecoder non-fatal). `read-filesystem.ts:158,294`.
- [0.5.1] Directory listing: offset/limit apply to entries (limit cap 2000); directories first with trailing `path.sep`, then files+symlinks, each sorted by JS `localeCompare` (case-insensitive-like; Rust needs an ICU/locale collation to match); `other` types hidden; dotfiles/`.git/` shown; no ignore rules. `read-filesystem.ts:258-289`.
- [0.5.1] Generic tool-output truncation (2000 lines / 50 KiB) is skipped for read because read sets `metadata.truncated` itself. `read.ts:113`, `core/src/tool-output.ts:70`.
- [0.5.1] Filename recovery: on NotFound, if exactly one file in the same dir matches after NFC + NBSP/U+202F -> space + curly single quotes -> `'`, it is read instead (permission `read` re-asked for it, external approval shared) and the header shows the real name. Captured (`Screenshot 2026\u202fat 10.txt`). `read.ts:132-144,58-79`.
- [0.5.1] Not found: `File not found: <input>` + up to 3 `Did you mean one of these?` suggestions (case-insensitive substring either way); if the parent folder is missing, names the first missing folder and nearest existing parent (absolute) plus suggestions. Captured. `read.ts:154-198`.
- [0.5.1] After a successful internal read, AGENTS.md files between the read file's dir (or the listed dir) and the Location root (exclusive) are loaded as session instructions (cross-area: session instructions). External reads skip this. `read.ts:82-107`.
- [0.5.1] Windows: directory entries end with `\`; resources use `/`.

### edit
- [0.5.1] Inputs `path`, `oldString`, `newString`, `replaceAll?`; permission action `edit` (shared with write/patch). `edit.ts:24-34,274`.
- [0.5.1] Order of checks: identical strings -> empty oldString -> resolve + external_directory approval -> read file (NotFound / directory errors) -> compute matches -> `edit` permission asked (with diff preview metadata when 1 match or replaceAll) -> not-found / multiple-match errors -> no-op errors -> write. So the edit permission prompt appears even for edits that then fail on matching. `edit.ts:286-363`.
- [0.5.1] Matching tiers, first tier with any hit wins (`edit.ts:168-188`):
  1. Endings-only edit (oldString and newString equal after removing all `\r`): exact byte search on the raw file, replacement verbatim.
  2. Otherwise on the LF view (all `\r\n` -> `\n` in file and in both strings): exact substring, non-overlapping, left to right.
  3. Typographic normalization of both sides, one-to-one char mapping: `‘’‚‛`->`'`, `“”„‟`->`"`, `‐‑‒–—―−`->`-`, U+00A0, U+2002-U+200A, U+202F, U+205F, U+3000 -> space. Captured: `let z = "hi"` matched a file with curly quotes.
  4. Line-wise: each line compared after `trimEnd()` + tier-3 normalization; oldString ending in `\n` requires the last matched line to end with a newline; overlapping candidates dropped. Captured: `foo\nbar` matched `foo   \nbar`.
  Offsets map back from LF view to the raw file.
- [0.5.1] Replacement line endings: each replacement line reuses the ending of the source line it keeps (LCS on trimEnd+normalized keys) or replaces; extra lines use the dominant ending of the matched region, else of the file (ties -> `\n`). Captured: CRLF file stayed CRLF for inserted lines. `edit.ts:104-162`.
- [0.5.1] The matched region is replaced with newString as given (normalized quotes are not preserved from the file).
- [0.5.1] replaceAll true replaces every match; false with >1 match fails. Replacements applied right-to-left.
- [0.5.1] Miss hint (only when `read` of the file is already allowed, never asks): already-applied hint, or "closest lines" by whitespace-insensitive line voting (>= half of the distinctive oldString lines must agree), up to 12 lines, 200-char cut with `…`. `edit.ts:190-241,337-351`.
- [0.5.1] UTF-8 BOM kept if the original had one or the result starts with one; at most one BOM. Captured. `edit.ts:364-369`, `util/src/bom.ts`.
- [0.5.1] Success text `Edited <resource> (<n> replacement[s])`; metadata carries a unified diff (createTwoFilesPatch, after formatter) for UI only.
- [0.5.1] No must-read-before-edit and no "modified since last read" check: grep for FileTime / "modified since" / "has been modified" in core/cli finds nothing; edit reads the file fresh each call. [dead in v2: OpenCode v1 FileTime rule is not present in 2.2.5 core].
- [0.5.1] Multiple edit calls in one assistant turn run and all apply (captured 13 parallel edits).

### write
- [0.5.1] Inputs `path`, `content`; permission action `edit`; external_directory approval first; diff preview (status `added`/`modified`) in permission metadata. `write.ts:64-92`.
- [0.5.1] Parent directories created; existing UTF-8 BOM kept (or added if content starts with one), at most one; content written as given otherwise (no line-ending conversion). `file-mutation.ts:103-117`.
- [0.5.1] Output `Created file successfully: <resource>` / `Wrote file successfully: <resource>`; all failures surface as `Unable to write <path>` unless the cause has its own message. Captured (writing to a directory -> `Unable to write sub`).
- [0.5.1] No must-read-first rule for overwrite.

### patch
- [dead] Only offered when model id contains `gpt-` and neither `oss` nor `gpt-4`; then edit/write are hidden. Never for Anysola's three models (captured tool lists have no `patch`). `core/src/tool/plugin/patch.ts:331-342`.
- [dead] Grammar (for completeness): `*** Begin Patch` / `*** End Patch` envelope (input trimmed, optional `cat <<'EOF'` heredoc stripped, CR stripped), optional first `*** Environment ID: x` line ignored, hunks `*** Add File:` (+lines, newline appended), `*** Delete File:`, `*** Update File:` with optional `*** Move to:`, `@@ [context]` chunks, ` `/`-`/`+` lines, `*** End of File`. Chunk location: seek forward with comparators exact, rstrip, trim, typographic-normalized; retry without a trailing empty old line. All hunks verified before any write; `edit` permission once for all targets; writes sequential, partial application reported. `util/src/patch.ts`, `patch.ts:85-317`.

### Formatter after edits
- [conditional: config `formatter` is `true` or an object] Default 0.5.1 config has no `formatter` key, so no formatter runs. `core/src/config/plugin/formatter.ts:29-31,64-75`.
- When enabled: after edit/write/patch, formatters whose extension list contains `path.extname(file)` are tried in order gofmt, mix, oxfmt, prettier, biome, zig, clang-format, ktlint, ruff, air, uv, rubocop, standardrb, htmlbeautifier, dart, ocamlformat, terraform, latexindent, gleam, shfmt, nixfmt, rustfmt, pint, ormolu, cljfmt, dfmt; each enabled only if its binary/config is found (rules in `core/src/formatter/builtins.ts`); first exit-0 wins, failures logged and next tried. cwd = Location dir, stdin/stdout/stderr ignored, `$FILE` substituted. Config object entries can add `command`, `extensions`, `environment`, `disabled`. `core/src/formatter.ts:56-94`.
- Formatter output is never shown to the model; the model-visible success text is unchanged. BOM state re-synced after formatting. Formatted content only affects the UI diff metadata.

### File-time / must-read rule
- [dead] Not present in 2.2.5 core (no FileTime service, no stale-read error). Edits match against current disk content.

### LSP diagnostics
- [dead] No LSP runtime in 2.2.5 core: `edit.ts:258`, `write.ts:44`, `file-mutation.ts:131` carry `TODO: Add LSP notification and diagnostics after LSP runtime exists.`; no `lsp` module in `core/src`; the `lsp` config key is only parsed (`core/src/config/normalize.ts:575`). Nothing is appended to edit/write output.

### Images and PDFs as tool results
- [0.5.1] Every tool result `file` item with `image/*` mime and a base64 data URI is normalized through photon (wasm, worker thread, falls back to main thread). `core/src/tool.ts:92-119`.
- [0.5.1] Limits default: autoResize true, max 2000x2000 px, max base64 5 MiB (5242880). Config `media.image.{auto_resize,max_width,max_height,max_base64_bytes}`. `core/src/image.ts:61-68`, `core/src/config/plugin/image.ts`.
- [0.5.1] Rules (`core/src/image/photon-normalize.ts`): fits limits and (not PNG, or PNG base64 <= 384 KiB, or PNG <= 65536 px) -> unchanged; fits but large PNG -> re-encode JPEG q80 only if smaller and the image is not transparent (>5% of a 64x64 sample grid with alpha < 250); too big -> scale to fit max dims (aspect kept), then try sizes shrinking by 0.75 per step (up to 32), encoders PNG-first if transparent or <= 65536 px else JPEG q80,70,55,40 then PNG, first under the byte limit wins (Lanczos3). Captured: 3000x1000 noise PNG -> 2000x667 JPEG; 800x800 1.9 MB PNG -> JPEG.
- [0.5.1] Decode failure -> image dropped + `[1 image omitted: could not be decoded.]`; cannot fit -> `[N image(s) omitted: could not be resized below the image size limit.]`; resizer unavailable -> passed through unchanged. Captured decode case.
- [0.5.1] PDFs are not normalized. Media the model does not support (per model capabilities from the gateway config) is replaced by `ERROR: Cannot read "<name>" (this model does not support <modality> input). Inform the user.` Captured for PDF with the fake gateway config (no pdf input declared); depends on the real gateway's model capabilities (cross-area: gateway config). `core/src/session/model-request.ts:125-151`.
- [0.5.1] Request-level bound: when inline image base64 in the transcript exceeds 25 MiB, oldest images are replaced by the "[This image was removed to reduce the request size ...]" text until <= 15 MiB. `model-request.ts:44-47,195-222`.
- [0.5.1] On the Anthropic wire images in tool results are `{"type":"image","source":{"type":"base64","media_type":...,"data":...}}` inside the tool_result content array, preceded by the text item.

## shell tool (verbatim: verbatim/tools/shell.md)

### shell: registration and description
- [ ] Tool name `shell`, `codemode: false` (direct tool, never inside `execute`); permission action `shell`. tool/plugin/shell.ts:21,189 [0.5.1]
- [ ] Input `{command: string, workdir?: string, timeout?: int>=0, background?: bool}`; schema/descriptions verbatim in shell.md. shell.ts:46-59 [0.5.1]
- [ ] Description rebuilt per request by session `context`/`compaction`/`generate` hooks with the OS + compat shell clause `Commands run on ${OS} using ${name}.`; registration-time text has no clause. shell.ts:26-44,277-285 [0.5.1]
- [ ] Declared output schema `{exit?, signal?, shellID?, truncated, timeout?, output, status?: completed|running}` (not sent to the model; used as structured output/metadata). shell.ts:61-73 [0.5.1]

### shell: shell selection
- [ ] Compat selection: config `shell` -> `$SHELL` -> OS fallback; fish and nu are never used by the tool (META deny), falling through to the fallback. shell/select.ts:13-23,116-123,179-189 [0.5.1]
- [ ] Fallback: macOS `/bin/zsh`; Linux/other `bash` on PATH else `/bin/sh`; Windows first of `pwsh`, `powershell`, Git Bash, `%COMSPEC%`/`cmd.exe`. select.ts:95-108,134-139 [0.5.1]
- [ ] Git Bash: `OPENCODE_GIT_BASH_PATH`, else `<git.exe dir>/../../bin/bash.exe` if it exists with non-zero size. select.ts:125-132; cli/src/server-process.ts:137; server/src/routes.ts:143 [0.5.1, Windows]
- [ ] Default (non-config) resolution is cached per process; config change reloads via config observer. select.ts:176-193; config/plugin/shell.ts [0.5.1]
- [ ] Args: `cmd` -> `/c <command>`; pwsh/powershell -> `-NoLogo -NoProfile -NonInteractive -Command <command>`; everything else `-c <command>` (not a login shell; `login` flag only used for PTY terminals, pty.ts:169). select.ts:164-169 [0.5.1]
- [ ] Windows paths `/c/..`, `/c:/..`, `/cygdrive/c/..`, `/mnt/c/..` normalized to `C:/..` for shell paths and file-access resolution. util/src/fs-util.ts:275-282 [0.5.1, Windows]

### shell: execution
- [ ] cwd = `workdir` resolved against the session directory (`~`, `~/` expanded; `~\` on Windows), default session directory; no cd persistence between calls (verified: `cd sub; pwd` then `pwd` prints the project root). shell.ts:120-121; file-access.ts:68-78 [0.5.1]
- [ ] Errors before spawn: `Working directory does not exist: <abs>` / `Working directory is not a directory: <abs>` (checked after approval). shell.ts:141-147 [0.5.1]
- [ ] Env = base env + `TERM=xterm-256color`, `OPENCODE_TERMINAL=1`, `AGENT=1`, `OPENCODE=1`, `AI_AGENT` (kept if already set, else `opencode`), `OPENCODE_SESSION_ID=<session id>`. shell.ts:207-210; shell.ts(core):278-282 [0.5.1]
- [ ] Base env: with the background service, the CLI's own environment (minus `OPENCODE_PASSWORD`/`OPENCODE_SERVER_PASSWORD`) stored per session; with `--standalone` the server's process env; none in a workspace location. core/shell.ts:269-272; cli/src/env.ts:15-22; cli/src/run/run.ts:105 [0.5.1]
- [ ] stdin is `/dev/null` (ignore); not a TTY (verified `tty` -> `not a tty`, `cat` gets EOF). core/shell.ts:314 [0.5.1]
- [ ] Unix: spawned detached (own process group/session, verified pgid=pid); kill = SIGTERM to the group, SIGKILL after 3 s; all groups killed when the server process exits. Windows: not detached, kill via `taskkill /pid <pid> /T /F`. core/shell.ts:315-316,385; util/src/cross-spawn-spawner.ts:325-328,376-395 [0.5.1]
- [ ] stdout+stderr merged into one stream in arrival order and written to the output file; no CRLF conversion. core/shell.ts:333-345 [0.5.1]
- [ ] After the shell exits, at most 1 s more is waited for pipes held by descendants (e.g. `cmd &`), then output capture stops. cross-spawn-spawner.ts:347-358 [0.5.1]
- [ ] Timeout: foreground default 120000 ms; `0` disables; no maximum; background default none (`timeout ?? 0`). Timeout race: process killed, status `timeout`. shell.ts:22,195; core/shell.ts:366-393 [0.5.1]
- [ ] Interrupt/abort of the turn cancels the job and removes the shell (process killed, output file deleted). shell.ts:231,256; core/shell.ts:167-180 [0.5.1]
- [ ] Each command is a Job (`type: shell`, title = command) with recovery info, so the TUI can list/background it. shell.ts:233-246 [0.5.1]

### shell: output
- [ ] Result blocks: output text (`(no output)` if empty) + optional notice `Timed out before completion` / `Killed by <SIGNAL>` / `Exited with code <n>` (non-zero). shell/result.ts:30-34; shell.ts:77-80 [0.5.1]
- [ ] Timeout appends `\n\nCommand exceeded timeout of <ms> ms. Retry with a larger timeout if the command is expected to take longer.` to the output. shell.ts:223-225 [0.5.1]
- [ ] Failed tool shown to the model as `{"error":{"type":...,"message":...},"content":[]}` with is_error (observed; serialization owned by session area). [0.5.1]

### shell: truncation
- [ ] Tail-kept: last `tool_output.max_bytes` (default 51200) bytes, then last `tool_output.max_lines` (default 2000) lines; notice `\n\n[showing lines A-B of N; full output saved to <file>]` (A computed from total newlines; wrong when the byte cut is mid-line, keep as is). core/shell.ts:244-259 [0.5.1]
- [ ] Generic tool-output truncation skipped for shell (metadata.truncated always present). tool-output.ts:70 [0.5.1]
- [ ] Output file `<data>/opencode/shell/<projectID>/sh_<id>.out`, kept 7 days (hourly sweep of `sh_[0-9a-f]{12}*.out`), last 25 exited shells kept in memory. core/shell.ts:32-34,91-119,144,289 [0.5.1]

### shell: background
- [ ] `background: true` returns immediately with `Command moved to the background (shell ID: <id>).\nOutput is streaming to: <file>` + the BACKGROUND_INSTRUCTION block. shell.ts:24-25,94-99,248-252 [0.5.1]
- [ ] On finish a synthetic user message `<shell id=".." state="completed|cancelled|error" command="..">\n<text>\n</shell>` is added and wakes the session (verified). shell.ts:151-183; shell/result.ts:45-63 [0.5.1]
- [ ] ctrl+b in the TUI moves running foreground shells to the background: same tool result, timeout cleared, plus synthetic "User requested that active blocking work be moved to the background..." message. shell.ts:257-261; session.ts:509-524; tui keybind `session.background` [0.5.1]
- [ ] `anysola run` does not wait for background shells: the run ends when the turn ends and the private server kills them (verified: `sleep 4; touch marker` never ran to completion). [0.5.1]

### shell: permissions parsing
- [ ] Hand-written scanner (no tree-sitter) with bash/zsh/posix dialects plus a PowerShell scanner; commands it cannot analyze FAIL the tool with `Portable shell scanner cannot analyze command: <reason>` and are not run (verified for an unterminated quote). shell/parse.ts:164-172; shell/scan.ts:3-12,221-233,1844 [0.5.1]
- [ ] Resources = source text of every simple command (pipelines, lists, substitutions, if/for/while/function bodies flattened); declarations skipped; redirects kept. Save = ARITY prefix + ` *` (table verbatim in shell.md). parse.ts:23-161,174-248,605-611 [0.5.1]
- [ ] Static `cd`-family targets (and `workdir`) checked as `external_directory` (`<dir>/*`, save `<project root>/*`) before the `shell` ask; static cds are not shell resources; dynamic cds stay as resources. Other path arguments (`ls /etc`, `git -C /tmp`) are NOT checked. parse.ts:221-226,253-269,456-565; shell.ts:120-139 [0.5.1]
- [ ] No `shell` ask when the command list is empty (e.g. only `export A=1` or `cd ~/x`). shell.ts:131 [0.5.1]
- [ ] Default build agent allows `shell` and `external_directory` (`*` allow); explore agent allows `shell`. schema/agent.ts:46-51; plugin/agent.ts:82 [0.5.1]
- [ ] `anysola run` without `--auto`: stderr `! permission requested: <action> (<resources>); auto-rejecting`, reply `reject` with feedback "This non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action." (model continues); `--auto` replies `once`. cli/src/run/noninteractive.ts:73-74,160-189 [0.5.1]
- [ ] "Always" stores the request's `save` list for the project (e.g. `git status *`). permission.ts:332-338 [0.5.1]

### shell: Windows
- [ ] Description says `Windows using pwsh|powershell|bash|cmd`. [0.5.1, Windows]
- [ ] `cmd.exe` commands are scanned with the posix dialect (cmd syntax like `&` or `%VAR%` gets bash rules). parse.ts:170 [0.5.1, Windows]
- [ ] PowerShell: `-Path`/`-LiteralPath`/`-PSPath` flags (all prefixes) and `$env:`, `$HOME`, `$PWD`, `$PSHOME` expansion for cd targets; `-StackName` makes the cd incomplete; env var lookup case-insensitive. parse.ts:13-16,479-542,599-603 [0.5.1, Windows]
- [ ] Output decoded as UTF-8; no CRLF normalization. [0.5.1]

### shell: Rust notes
- Port shell/scan.ts + parse.ts as a hand-written scanner (same opaque reasons, same resource/save strings); tree-sitter-bash would produce different resources and different opaque cases. Keep a golden-test corpus generated by running the TS `ShellParse.scan` (bun can import the fork source directly).
- Process spawn: `std::process::Command` with `setsid`/`process_group(0)` on Unix, `killpg` SIGTERM then SIGKILL after 3 s, kill all groups on exit; Windows `taskkill /T /F` (or a Job Object), stdin null, stdout+stderr into one file in arrival order (one pipe for both, or `os_pipe` dup).
- Output file with streaming byte and newline counters; tail read of max_bytes then line cut, notice text as in shell.md.
- `which`-style PATH lookup (`which` crate) and `/etc/shells` read for listing.

## Search and web tools (grep, glob, webfetch, websearch)

Verbatim: `verbatim/tools/{grep,glob,webfetch,websearch}.md` (all captured from the binary).

### grep
- [0.5.1] Permission order: `external_directory` for the search path (if outside the project), then `grep` with resource = the pattern, save `["*"]`. `core/src/tool/plugin/grep.ts:84-102`.
- [0.5.1] `path` may be a file (search that one file) or a directory; missing -> `Search path does not exist: <path or .>`. `grep.ts:103-109`.
- [0.5.1] rg args: `--no-config --json --hidden --no-messages [--fixed-strings] [--ignore-case] [--glob=<include>] --glob=!**/.git/** -- <pattern> <file or .>`, cwd = search dir. Hidden files ARE searched; `.gitignore`/`.ignore` are honoured (rg defaults); `.git/` excluded. `core/src/ripgrep.ts:219-235`.
- [0.5.1] Default limit 100 (`FileSystem.DEFAULT_SEARCH_LIMIT`), runs with limit+1 to detect truncation; 30 s timeout. `grep.ts:110-130`, `core/src/filesystem.ts:66-67`.
- [0.5.1] Match line text capped at 2000 chars + `...`; at most 100 submatches kept. `ripgrep.ts:20-21,244-252`.
- [0.5.1] Output: absolute paths, grouped per file in rg output order, `  Line N: <text>` where text keeps rg's trailing newline (extra blank lines, verbatim quirk). Header `Found N matches` / `No matches found`; truncation notice. `grep.ts:44-61`. Captured.
- [0.5.1] rg exit 2 with "regex parse error"/"error parsing regex" in stderr -> `Invalid regex pattern: <stderr>`; other exit codes other than 0/1/2 -> `ripgrep failed with code N` or stderr. `ripgrep.ts:100-101,144-150`.
- [0.5.1] grep/glob set `metadata.truncated`, so the generic 2000-line/50 KiB bounding is skipped for them (`tool-output.ts:70`).

### glob
- [0.5.1] Permission: external_directory (kind directory) then `glob` with resource = pattern, save `["*"]`. `glob.ts:65-82`.
- [0.5.1] Path must be a directory: `Search path is not a directory: <path>`. `glob.ts:88-92`.
- [0.5.1] rg args: `--no-config --files [--hidden] --glob=<pattern> [--glob=!**/.*  unless hidden] --glob=!**/.git/** .`; respects gitignore, but a positive glob that matches an ignored directory re-includes it (captured `**/*` returned a gitignored file). Order = rg output order (unsorted). `ripgrep.ts:163-190`.
- [0.5.1] Output: absolute path per line; `No files found`; truncation notice. Default limit 100, 30 s timeout.

### ripgrep binary
- [0.5.1] Resolution: `rg`/`rg.exe` on PATH (plus the app `bin` dir) first; otherwise downloads ripgrep 15.1.0 from `https://github.com/BurntSushi/ripgrep/releases/download/15.1.0/ripgrep-15.1.0-<target>.<tar.gz|zip>` into `<data>/bin`. Targets: aarch64/x86_64 apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-musl, aarch64/i686/x86_64-pc-windows-msvc. `core/src/ripgrep/binary.ts:15-142`.
- [0.5.1] Windows: zips extracted with `%SystemRoot%\System32\tar.exe` (bsdtar) by path, not PowerShell, not Git's tar; output paths normalized to `/`. `ripgrep/binary.ts:91-95`, `ripgrep.ts normalizePath`.

### webfetch
- [0.5.1] Only http/https (`URL must use http:// or https://`); permission `webfetch` with resource = URL, save `["*"]`; every redirect target re-checked with `assertNotDenied` (deny rules apply, never asks); manual redirects, max 10. `core/src/tool/plugin/webfetch.ts:76-110,140-162`.
- [0.5.1] Format default markdown; `Accept`, `Accept-Language`, `User-Agent` exactly as in the verbatim file; Cloudflare challenge (403 + `cf-mitigated: challenge`) retried once with UA `opencode`. `webfetch.ts:46-73,164-168`.
- [0.5.1] Timeout default 30 s, max 120 (schema `exclusiveMinimum 0, maximum 120`), message `Request timed out`. `webfetch.ts:14-16,175-180`.
- [0.5.1] Body limit 5 MiB (5242880) by Content-Length or while streaming: `Response too large (exceeds 5242880 byte limit)`. `webfetch.ts:113-118`, `tool/http-body.ts`.
- [0.5.1] Image responses rejected (`Unsupported fetched image content type: <mime>`, except svg+xml and vnd.fastbidsheet which count as textual only if textual); non-textual mime rejected (`Unsupported fetched file content type: <mime>`). Textual = empty mime, text/*, application/json, *+json, application/xml, *+xml, application/javascript, application/x-javascript. `webfetch.ts:120-122,170-174`, `core/src/mime.ts:34-42`.
- [0.5.1] Conversion only when content-type contains `text/html`: markdown -> custom htmlparser2-based converter (`tool/html-markdown.ts`, 658 lines: omits script/style/noscript/iframe/object/embed/meta/link/template; GFM tables, fenced code with `language-*` class, lists, blockquotes `> ` up to depth 8, links, details; output capped at 5 MiB); text -> concatenated text nodes trimmed (`extractTextFromHTML`); html -> raw. Captured outputs in verbatim file. `webfetch.ts:123-128,204-222`.
- [0.5.1] Non-2xx -> `StatusCode: non 2xx status code (<status> GET <url>)` (type `unknown`). Captured.
- [0.5.1] Large text goes through the generic truncation (2000 lines / 50 KiB) -> second text block `[showing lines 1-N of M; full output saved to ...]`. Captured.

### websearch (tool side)
- [0.5.1] Only provider: Anysola gateway `POST <api>/v1/code/search` `{query}`; default selection, so the provider-choice form ("Web Search" / "Allow Anysola Code to search the web for up-to-date information?") is never shown while the provider exists. `core/src/plugin/websearch/anysola.ts`, `tool/plugin/websearch.ts:67-149`.
- [dead] Exa, Firecrawl, Parallel, Tavily, TinyFish providers: removed in commit 2510688b10. `plugin/websearch/index.ts`.
- [0.5.1] Permission `websearch`, resource = query, save `["*"]`, metadata = input (TUI title `Web Search via Anysola "<query>"`). Default allowed. Config v1 `permission.websearch` accepts only a plain action string. `websearch.ts:50-58`, `core/src/v1/config/permission.ts:27`.
- [0.5.1] `"websearch": false` in config removes the tool from requests (context/compaction/generate hooks). `websearch.ts:188-198`.
- [0.5.1] Token: active Anysola connection (oauth access or key); none -> `Web search needs an Anysola sign-in. Run `anysola login`.` Headers: authorization Bearer, `x-anysola-client: opencode`, `x-anysola-version: 0.5.1`, accept json; 30 s timeout; 1 MiB response limit. Captured. `anysola.ts:80-125`.
- `[1.0]` The response carries no `credits`; nothing is recorded per search (the next usage event or `/limits` poll shows it). 0.5.1: `credits` in the response is charged to the session that searched (`AnysolaSearchCredits.note`, shown in run output as `Anysola · 2 credits`). Captured.
- [0.5.1] Output format, no-results text and all error texts: verbatim file (captured 401/402/429/500/bad JSON/empty/multi).
- [0.5.1] Description year = process-start year (`new Date().getFullYear()`), local time.

## Agent-facing tools (question, skill, subagent, todowrite, execute)

Verbatim: `verbatim/tools/{question,skill,subagent,todowrite,execute}.md`.

### question
- [0.5.1] Permission `question` resource `*` (no save -> no "Always allow" option). Build and plan add `question allow`; general and explore deny it (tool not sent to them). `tool/plugin/question.ts:62-71`, `plugin/agent.ts:60,69`, `plugin/plan.ts:38`.
- [0.5.1] Shown as one form titled `Questions`, field per question (header as title, question as description, options label/description, custom answer always allowed, `multiple` -> multiselect). `question.ts:118-131`.
- [0.5.1] Answer text: `User has answered your questions: "<q>"="<a1, a2>"[, ...]. You can now continue with the user's answers in mind.`; unanswered -> `Unanswered`. `question.ts:38-46`.
- [0.5.1] `anysola run`: cancelled with feedback `This non-interactive run cannot ask the user questions, so the question was cancelled. Continue without an answer; make reasonable assumptions and state them.` (model continues; captured). TUI dismissal without message ends the step (tool marked aborted). `cli/src/run/noninteractive.ts:70-71,191-211`, `question.ts:89-96`.

### skill
- [0.5.1] Lookup by id; unknown -> `Unable to load skill <id>`. Permission `skill` resource/save = skill id. `tool/plugin/skill.ts:46-60`.
- [0.5.1] Output `<skill_content name="...">` block with `# Skill: <name>`, body, base directory, "Note: file list is sampled." and up to 10 `<file>` entries. Captured. `core/src/skill.ts:40-70`.
- [0.5.1] Discovery (observed in logs): `~/.config/opencode/skill(s)`, `~/.agents/skills`, `~/.claude/skills`, and `.claude/skills` / `.opencode/skill(s)` in the project and every ancestor directory up to home (the capture listed the real `/home/ubuntu/.claude/skills` because the test project lives under `/home/ubuntu`). Detailed rules belong to the session/config area. `core/src/skill/discovery.ts`.
- [dead] Built-in skills (upstream OpenCode docs / GitHub issue skills): removed (`plugin/internal.ts:236`, deleted `packages/core/src/plugin/skill`).

### subagent
- [0.5.1] Description gets an `Available subagents:` list built per request from the caller's permissions (rule in verbatim file). Default list: explore, general. `subagent.ts:286-312`.
- [0.5.1] Depth limit: config `experimental.subagent_depth`, default 1 (a subagent cannot spawn another; general and explore also deny `subagent`). `subagent.ts:126-139`.
- [0.5.1] Permission `subagent`, resource/save = agent id. `subagent.ts:144-157`.
- [0.5.1] New child session: parentID = caller, title = description, agent, model = `model` param ?? agent.model ?? parent model; first prompt `You are a subagent spawned by another session.\n<prompt>`. `subagent.ts:187-213`.
- [0.5.1] Foreground blocks until the child finishes; output `<subagent sessionID=".." state="completed">\n<text>\n</subagent>`. If the job is moved to background while blocking, the background text is returned instead. Background returns the 4-line text immediately and later injects `<subagent sessionID=".." state=".." description="..">` as a synthetic user message that wakes the parent. Captured foreground. `subagent.ts:19-28,232-272`, `session/subagent-completion.ts`.
- [0.5.1] `sessionID` continues an existing child (must be a direct child): prompt sent as-is (steers a running child).
- [0.5.1] Interrupting the parent tool call interrupts and cancels the child. `subagent.ts:239-244`.

### todowrite
- [conditional: config `experimental.todo: true`, off by default] registered as direct tool `todowrite`; captured with the flag on. Permission `todowrite` resource/save `*`. Output templates in verbatim file. `tool/plugin/todo.ts:43-82`.

### execute (Code Mode) and the opencode namespace
- [0.5.1] `execute` is always last in `tools` with `cache_control: ephemeral`; its description is static; the catalog of Code-Mode-only tools is system block 2 ("# Code Mode ..."). Captured. `core/src/tool.ts:274-315`, `core/src/codemode/tool.ts:75-82`, `core/src/codemode/instructions.ts`.
- [0.5.1] Tools with `codemode !== false` go into Code Mode; every built-in direct tool sets `codemode: false`. The `opencode` namespace (8 tools by default: list_mcp_resources, models, read_mcp_resource, session_move, session_rename, worktree_create, worktree_list, worktree_remove) is Code-Mode-only. `tool/plugin/opencode.ts`, `tool/plugin/mcp-resource.ts`.
- [0.5.1] If `execute` is wholly denied for an agent, Code Mode tools become direct tools named `opencode_<tool>` (`tool.ts:276-278`). Explore's `*` deny disables `execute` and every namespace tool, so its captured list has neither.
- [0.5.1] Runtime: custom JavaScript interpreter on acorn 8.15.0 (`packages/codemode/src/interpreter`, ~5600 lines), not V8 eval; `fetch` available; no imports/fs/timers; `console.log` captured into a `Logs:` section; `search(...)` function for tool discovery; limits `timeoutMs`/`maxToolCalls`/`maxOutputBytes` exist but the session execute passes none (no default timeout). `packages/codemode/src/codemode.ts:26-171`.
- [0.5.1] Result: `JSON.stringify(value, null, 2)` + optional `\n\nLogs:\n...`; runtime errors are plain (non-error) tool results such as `Unknown tool 'opencode.nope'.\nUse search to find available tools.` Captured.
- [0.5.1] Catalog rendering rule (budget 2000 tokens, 120-char description cut, `(N tools, k shown)`): verbatim execute.md. `core/src/codemode/catalog.ts:47-176`.
- [0.5.1] System block 3 always adds: "When you create a worktree outside the current working directory and intend to use it as your primary working directory, consider using `execute` to call `tools.opencode.session_move` and make the worktree the session's working directory." (captured).
- [0.5.1] MCP tools join through one stable transform; MCP servers' tools default into Code Mode (`plugin/mcp-codemode-defaults.ts`), named `<server namespace>.<tool>` in the catalog. Details: integrations area.

### Gated tools
- [conditional: `experimental.schedules: true`] `schedule_*` tools (schedule_create and others) in the opencode Code Mode namespace; the captured catalog shows schedule_delete/pause/resume, the rest hidden by the catalog budget and reachable via `search`. `tool/plugin/schedule.ts:55-155`. Off by default.
- [conditional: `experimental.artifacts: true`] artifact_create/read/update/delete in the opencode namespace. `tool/plugin/artifact.ts:64-69`. Off by default.
- [dead] `experimental_image_generate`: registered only while an image model is in the catalog; the Anysola gateway config exposes only chat models (auto, sonnet, haiku). `tool/plugin/image-generate.ts:227-251`.
- [dead] `patch` (apply_patch grammar): offered only for `gpt-` model ids; never for Anysola models (see File tools).
- [dead] `lsp` tool and LSP diagnostics after edits: not present in core 2.2.5 (TUI still has an `lsp` permission presentation, `tui/src/util/permission.ts:104-119`).
- [dead] `list`/`ls`, `multiedit`, `batch` tools: do not exist in 2.2.5 (directory listing is `read` on a directory).

## Tool sets per agent (captured request bodies)

| agent | tools sent (order as sent) | source of difference |
|---|---|---|
| build (default) | edit, glob, grep, question, read, shell, skill, subagent, webfetch, websearch, write, execute | `*` allow + question allow |
| plan | same 12 as build; edit/write fail outside `~/.opencode/plan` with `Cannot use <tool> to modify files outside the Plan directory: <dir>`; plan reminder injected as a user message | `plugin/plan.ts` |
| general (subagent) | edit, glob, grep, read, shell, skill, webfetch, websearch, write, execute | denies question, subagent |
| explore (subagent) | glob, grep, read, shell, skill, webfetch, websearch | `*` deny + allows; system prompt PROMPT_EXPLORE |
| title / summary | none (`*` deny) | hidden agents |

- Same tool list for anysola/auto, claude-sonnet-5-5, claude-haiku-5-5 (diffed; only the "# Your Model" system block differs). Linux capture; the shell description's OS clause differs per platform (shell.md).
- With `experimental.todo/schedules/artifacts` on: build adds direct `todowrite`; namespace grows to 17 tools (14 shown).
- Captured files: `verbatim/tools/_captured-request-build-sonnet.json`, `_captured-request-explore-subagent.json`, `_captured-request-general-subagent.json`, `_captured-request-build-experimental-flags.json`.
- Unknown tool name from the model: `No tool named "<name>" is currently available. Please use a tool from the available tool list.`; tool removed by a context hook: `Tool is not available for this request: <name>`. `core/src/tool.ts:327-336`.
- Invalid arguments: template in `core/src/tool/runtime.ts:88-100` (see File tools shared section).

## Shared output bounding
- [0.5.1] After execution the session runner bounds every result whose metadata has no `truncated` key: 2000 lines / 51200 bytes (config `tool_output` can change both). Kept prefix; full text written to `<data>/tool-output/tool_<id>`; marker `[showing lines 1-N of M; full output saved to <file>]` (or `0 lines`) appended as its own text block. Files older than 7 days deleted hourly. `core/src/tool-output.ts:13-159`. Captured via webfetch.
- [0.5.1] Tools that bound themselves and set `truncated` (grep, glob, read, shell) skip this step. Shell keeps the tail instead (shell part).
- [0.5.1] Image normalization of tool-result images and the `[N image(s) omitted: could not be decoded.]` / `... could not be resized below the image size limit.]` notes: `core/src/tool.ts:92-119` (details in File tools section).
- [0.5.1] Single text block -> sent as a string; several blocks/images -> content array. `session/runner/to-llm-message.ts:127-141`.

## Permission system

### Rule model and evaluation
- [0.5.1] Rule = `{action, resource, effect: allow|ask|deny}`; ruleset is an ordered array; LAST matching rule wins (`findLast`); no match -> `ask`. `core/src/permission.ts:86-96`.
- [0.5.1] Wildcard match (`core/src/util/wildcard.ts`): input and pattern have `\` replaced by `/`; `*` = any chars incl. `/` and newline (`.*`, dotAll), `?` = one char; other regex chars escaped; a pattern ending in ` *` also matches without the trailing part (`git *` matches `git`); case-insensitive on Windows only. Used for actions too (`*` action matches every tool).
- [0.5.1] Effective rules for a request = agent.permissions ++ session.permissions. Deny check first on these configured rules only: any resource denied -> deny (saved "always" rules cannot override a deny). Then saved project rules (all `allow`) are appended; per resource evaluate; any `ask` -> ask, else allow. `permission.ts:161-202`.
- [0.5.1] Plugin hook `permission.evaluate` may change the effect/message (used by managed `policies`, below). `permission.ts:182-190`.
- [0.5.1] `Unattended` reference (schedule checks only): an ask becomes allow only if the unattended rules allow every resource, else deny with `Permission <action> needs approval, and nobody is present to give it`. `permission.ts:51-57,191-201`.
- [0.5.1] Missing agent -> `[* * deny]`. `permission.ts:19`.
- [0.5.1] Whole-tool hiding: a tool is removed from the request when the last rule matching its action has resource `*` and effect deny (`core/src/tool.ts:381-384`); `edit`, `write`, `patch` share the action `edit`. Captured: `{"permission":{"webfetch":"deny"}}` removes webfetch.

### Defaults (no config; the Anysola gateway config sets no permissions)
- [0.5.1] Every agent starts with `[* * allow, read *.env ask, read *.env.* ask, read *.env.example allow]`. `schema/src/agent.ts:39-52`.
- [0.5.1] build: + `question * allow`. plan: + `question allow`, `edit * deny`, `edit <home>/.opencode/plan/* allow`. general: + `question deny`, `subagent deny`. explore: + `* deny`, allow shell/grep/glob/webfetch/websearch/read, read .env asks as above, `subagent deny`, `external_directory * allow`. title/summary: `* deny`. compaction: defaults (hidden). `core/src/plugin/agent.ts:52-117`, `core/src/plugin/plan.ts:33-42`.
- [0.5.1] Consequence: by default shell, edit, write, webfetch, websearch, external directories are NOT asked; only `.env` reads ask. Captured: `/etc/hostname` read allowed without prompt.
- [dead] doom_loop detection: no code in core 2.2.5 asks `doom_loop`; the action is only accepted in v1 config (`v1/config/permission.ts:27`) and has a TUI presentation ("Continue after repeated failures" / "This keeps the session running despite repeated failures.", `tui/src/util/permission.ts:131-137`) but nothing triggers it.

### Config schema
- [0.5.1] Config files `opencode.json` / `opencode.jsonc` (discovery: storage/config area). `core/src/config/discovery.ts:12`.
- [0.5.1] Native: top-level `permissions: [{action, resource, effect}]` and per agent `agents.<id>.permissions` (same shape). Top-level rules are appended to EVERY agent after its built-ins; agent rules after those. `schema/src/config.ts:58-60`, `core/src/config/plugin/agent.ts:82-126`.
- [0.5.1] Legacy v1 accepted and migrated in order `tools` rules, then `permission` rules, then native `permissions` (`core/src/config/normalize.ts:179-183`):
  - `"permission": "allow"|"ask"|"deny"` -> `[* * effect]`.
  - `"permission": {"<tool>": "<effect>"}` -> `[<tool> * effect]`; `{"<tool>": {"<pattern>": "<effect>", ...}}` -> one rule per key in written order.
  - `"tools": {"<tool>": true|false}` -> allow/deny `*`.
  - Action renames: `write`, `patch`, `apply_patch` -> `edit`; `task` -> `subagent`; `bash` -> `shell`. `core/src/v1/config/migrate.ts:27-32`.
  - `question`, `websearch`, `doom_loop` must be plain strings in v1 (`v1/config/permission.ts:27-40`).
- [0.5.1] Path actions `read`, `edit`, `external_directory`: resources starting `~/`, `$HOME/` (or `$HOME\`) or equal to `~`/`$HOME` are expanded to the home dir; shell resources are NOT expanded. `core/src/config/plugin/agent.ts:140-161`.
- [0.5.1] Resources compared against: read/edit internal = Location-relative path (`src/a.ts`), external = absolute path with `/`; external_directory = `<dir>/*`; shell = parsed command strings (shell part); webfetch = URL; websearch = query; grep/glob = pattern; skill = skill id; subagent = agent id; question/todowrite = `*`.
- [0.5.1] Managed `policies: [{action: "tool.use", resource: "<tool>:<resource>", effect: allow|deny}]` (also provider.use, integration.*, mcp.use, skill.use, plugin.use). A matching `tool.use` deny turns any evaluation into deny with message `Blocked by configuration policy` (or `Blocked by <org>'s policy` / `Blocked by your organization's policy` for integration documents). Earlier-loaded documents win. Captured with `shell:git *`. `core/src/config/plugin/policy.ts:94-109`, `core/src/managed-policy.ts`.

### Asking, replies and persistence
- [0.5.1] Request = `{id: per_..., sessionID, action, resources[], save?[], metadata?, source {type:"tool", messageID, id}, message?}`; published as `permission.asked`. Replies `once | always | reject` (+ optional message). `schema/src/permission.ts:17-54`.
- [0.5.1] `reject` without message -> DeclinedError: the step ends, the tool is marked aborted with `The user declined this tool call`, step `Step interrupted`; ALL other pending asks of the same session are rejected too. `reject` with message -> CorrectedError: model sees `permission.rejected` with the message as text and continues. `permission.ts:309-330`, `session/runner/step.ts:206-225`.
- [0.5.1] `always` stores each `save` pattern as an allow rule in SQLite table `permission` (project_id, action, resource; unique index) for the current project, then auto-approves other pending asks that now evaluate allow. Saved rules apply to every session of the project. `permission.ts:332-356`, `core/src/permission/saved.ts`, `permission/sql.ts`.
- [0.5.1] Save patterns per tool: read/edit/glob/grep/webfetch/websearch/todowrite `["*"]`; skill/subagent = the id; external_directory = `<project root of that dir, or the dir>/*`; shell = per-command patterns (shell part).
- [0.5.1] Deny reaches the model as `{"error":{"type":"permission.rejected","message":"Permission denied: <action>"},"content":[]}` (BlockedError message, `permission.ts:69-78`). Captured.
- [0.5.1] Pending asks are rejected when the Location closes (`Interaction cancelled because the location shut down`).

### anysola run (non-interactive)
- [0.5.1] Without `--auto`: each ask from the run's session or its child sessions is rejected with feedback `This non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action.`; stdout gets `! permission requested: <action> (<resources joined by ", ">); auto-rejecting`; the model continues; exit code 0. Captured. `cli/src/run/noninteractive.ts:73-74,161-189`.
- [0.5.1] `--auto` (hidden aliases `--yolo`, `--dangerously-skip-permissions`): every ask answered `once`; denies still deny. `cli/src/commands/commands.ts:35-45`, `commands/handlers/run.ts:31`.
- [0.5.1] Questions/forms in run: question forms cancelled with the question feedback; other forms (MCP elicitation) cancelled and the exit code set to 1.

### TUI prompt (texts)
- [0.5.1] Header `△ Permission required`, then `<icon> <title>` (not for shell). Options `Allow once`, `Always allow` (only when the request has `save` patterns), `Reject`; Esc = reject. Highlighting `Always allow` shows `This will always allow <action> for this project.` (save = `["*"]`) or `This will always allow the following patterns for this project.` + `- <pattern>` lines. `tui/src/routes/session/permission.tsx:200-261`, `tui/src/util/permission.ts:154-168`.
- [0.5.1] Titles/lines per action (`tui/src/util/permission.ts:23-144`): edit `→ Edit <path>` + diff (`No diff provided` if none); read `→ Read <path>` / `Path: <path>`; glob/grep `✱ Glob|Grep "<pattern>"` / `Pattern: <p>`; shell title `Shell command`, line `$ <command>`; subagent `# <Agent> Subagent` / `◉ <description>`; webfetch `% WebFetch <url>` / `URL: <url>`; websearch `◈ Web Search via Anysola "<q>"` / `Query: <q>`; external_directory `← Access external directory <dir>` + `Patterns` + `- <resource>`; other `⚙ Call tool <action>` / `Tool: <action>`.
- [0.5.1] Reject in a CHILD (subagent) session opens `Reject permission` / `Tell Anysola Code what to do differently` with a reason field (`Rejection reason`), Confirm/Cancel; in a root session reject is immediate. `permission.tsx:241-247,315-395`.
- [0.5.1] Auto-accept mode: `--auto` on the TUI or TUI config `session.permissions: "autoaccept"` (setting "Permissions": prompt / auto accept) replies `once` to every ask automatically. `tui/src/context/permission.tsx`, `tui/src/routes/session/index.tsx:268-285`, `tui/src/config/index.tsx:180`.
- ACP permission mapping lives in `cli/src/acp/permission.ts` (integrations area).

## Rust implementation notes

- Tool definitions: emit `tools` exactly as captured (same key order in schemas, `additionalProperties: false`, no `$defs`, integer `minimum` without safe-int bounds). Simplest parity path: embed the captured JSON schemas and descriptions as static data, with the dynamic parts generated (shell OS clause, websearch year, subagent list, `execute` last with `cache_control`). Golden-test against `verbatim/tools/_captured-request-*.json`.
- Input validation errors must reproduce Effect Schema's issue texts (`Expected a value greater than or equal to 0`, `Missing key`, ...) inside the `Invalid arguments for tool "<name>":` template; collect real messages per schema keyword from the binary into a golden table rather than relying on a generic JSON-schema validator's wording (`jsonschema` crate messages differ).
- Search: either bundle/download rg 15.1.0 and drive it with the same argv and `--json` parsing (exact parity, including ordering and ignore quirks), or use the `ignore` + `grep-searcher` + `grep-regex` + `globset` crates. The crates reproduce rg semantics only when configured like rg (hidden, overrides from `--glob`, `.git` exclusion); output ORDER is rg's parallel walk order, so bundling rg is the lower-risk option. Keep the `Invalid regex pattern: <rg stderr>` text, which depends on rg's own stderr.
- HTML to markdown: port `tool/html-markdown.ts` (658 lines, htmlparser2 events) line by line on top of `html5ever`/`lol_html` tokenizer events; `html2md`/`htmd` crates do not produce the same output. Golden-test with the captured page.
- HTTP: `reqwest` with redirects disabled (manual loop, max 10, re-checking deny rules), streaming body cap 5 MiB, the exact Accept/User-Agent headers.
- Code Mode: the upstream runtime is a confined acorn-based interpreter, not a JS engine. Options: embed `rquickjs` (QuickJS) or `boa_engine` with no timers/imports and an injected `tools` proxy, `fetch`, `search`, `console`; result/log/error text formats must match `codemode/tool.ts`. Error texts like `Unknown tool 'x'.\nUse search to find available tools.` come from `packages/codemode/src`; port them verbatim.
- Shell permission parsing: a hand-written portable scanner (`core/src/shell/scan.ts`, ~2600 lines), not tree-sitter; port it (or use `tree-sitter-bash` plus a conformance suite generated from the fork's ShellParse, see shell part). Unparseable commands must fail with `Portable shell scanner cannot analyze command: <reason>` before running.
- Wildcard: build the same regex (`regex` crate, `(?s)`, `(?i)` on Windows), with the trailing ` *` optional-suffix rule.
- Permission store: SQLite table `permission(id, project_id, action, resource, time_*)` with unique `(project_id, action, resource)` (shared schema with the storage area).
- Images: `image`/`fast_image_resize` + JPEG/PNG/WebP encoders to reproduce photon's resize (max edge 2000 px observed: 3000x1000 -> 2000x667 JPEG); see File tools part for limits.
- Tool output files: `<data>/tool-output/tool_<id>` with 7-day retention swept hourly; shell uses its own `<data>/shell/<projectID>/sh_<id>.out`.
- Windows: path rewriting `/c/...`, `/cygdrive/c/...`, `/mnt/c/...` -> `C:/...`; `\` normalised to `/` in permission resources; shell fallback pwsh -> powershell -> Git Bash -> cmd; rg.exe download via System32 `tar.exe`.

## Dead in 0.5.1

- `patch` tool (apply_patch grammar): only for model ids containing `gpt-` (excluding `oss`, `gpt-4`); never for auto/sonnet/haiku.
- `experimental_image_generate`: needs an image model in the catalog; the gateway exposes none.
- LSP tool and LSP diagnostics appended after edits: not in core 2.2.5 (only TODOs; TUI keeps an `lsp` permission renderer).
- Must-read-before-edit / "file modified since last read" (FileTime): not in 2.2.5.
- doom_loop permission: accepted by v1 config and rendered by the TUI, never raised.
- Web search providers Exa, Firecrawl, Parallel, Tavily, TinyFish: removed (commit 2510688b10); Anysola gateway is the only provider, so the provider-choice form never appears.
- Built-in skills (OpenCode docs / GitHub issue skills): removed (`packages/core/src/plugin/skill` deleted).
- `list`/`ls`, `multiedit`, `batch`, `todoread` tools: do not exist in 2.2.5.
- Formatters after edits: present but only run when the config has a `formatter` key (`[conditional]`, not dead).
- `todowrite`, `schedule_*`, `artifact_*`: `[conditional]` on `experimental.todo|schedules|artifacts: true`; not sent by default.

## Cross-area notes (not owned here)
- `anysola run "say hi"` sends the user text wrapped in literal double quotes (`"\"say hi\""`) (CLI area).
- `anysola run` hangs when stdin is an open pipe that never closes (it reads stdin); run tests with `</dev/null`.
- Skill discovery walks ancestor directories (picked up `/home/ubuntu/.claude/skills` and `.agents/skills`) (session/config area).
