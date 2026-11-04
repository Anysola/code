# Anysola Code 0.5.1 — TUI parity inventory

Scope: everything a user sees or does in the interactive TUI of Anysola Code 0.5.1 (fork of OpenCode v2.2.5, branch
`anysola`, `/home/ubuntu/anysola-code/build/fork`). The Rust rewrite (ratatui + crossterm) must reproduce every
`[0.5.1]` item; `[dead: …]` items ship as code but no user can reach them, so they need not be ported.

Conventions
- Paths are relative to `packages/tui/src/` unless they start with `packages/`.
- Each item: `- <exact behavior> — <file:line> — [0.5.1]` or `[dead: <reason>]`.
- "Captured" means confirmed on the shipped linux-arm64 binary (`rust/testing/bin/anysola-0.5.1`) under a pty with a
  fake gateway (scripted gateway: todowrite → shell → edit → markdown answer with thinking; `/me`, `/usage`, `/tokens`).
- The CLI-side login/device flow (`anysola login`, first-run sign-in before the TUI starts) is in the CLI spec; inside the
  TUI the only sign-in surface is the sidebar "Getting started" card and `/connect` (section 4).

Verbatim artifacts (`rust/spec/verbatim/tui/`)
| file | content |
|---|---|
| `slash-commands.md` | every keymap command: id, slash name, aliases, title/description, category, default key, palette, condition, file:line, status |
| `keybinds-default.md` | full keybind config with defaults (v2 + v1 ids), leader key + timeout, syntax, display format |
| `dialogs.md` | every dialog's title, placeholder, options, footers, empty states, confirm texts, toasts; dead dialogs |
| `composer-strings.md` | placeholders, hints, paste/attachment chips, shell-mode texts, composer toasts |
| (none) | tool-rendering, permission and question strings are inline in section 1/3 below; permission/form texts also in `dialogs.md` §29–30 |
| `theme-anysola.json` | byte copy of `packages/tui/src/theme/assets/v2/anysola.json` (the default theme) |
| `theme-anysola.md` | every anysola token, dark + light expressions and resolved hex |
| `themes-bundled.md` | bundled theme list, theme JSON formats, discovery dirs, background detection, system theme |
| `wordmark.md` | ANYSOLA wordmark and symbol, exact characters |
| `screens/*.txt` | text captures of 0.5.1 (120x36 unless named otherwise); `screens/png/` colored PNGs of key screens |

Captures in `screens/`: `home-120x36-empty`, `home-120x36-recent`, `home-170x40`, `home-60x20-narrow`,
`home-low-balance-event-banner`, `slash-popup` (+ `slash-popup-scroll-0..6`, `slash-list-home-context`),
`at-mention-popup`, `shell-mode`, `command-palette` (+ `command-palette-60rows-scroll-0..7`), `model-picker`,
`agent-picker`, `theme-picker`, `help`, `credits`, `usage-7d`, `usage-30d`, `devices`, `budget-toast-home`,
`budget-set-toast`, `mcps`, `settings`, `connect`, `sessions-empty`, `session-list`, `debug`, `skills`, `plugins`,
`worktrees`, `open`, `update`, `permission-shell`, `permission-edit-diff`, `session-tools-edit-diff-markdown`,
`session-120x36-after-turn`, `session-170x40-sidebar`, `status-in-session`, `timeline`, `exit-summary`.

## 0. Observed screens (captured, 0.5.1)

- [ ] Home (120x36): tab strip row only when tabs exist (`<title>   <title>   + New session`); wordmark centered (3 rows) with
  `Code` right-aligned under it; prompt box with left `┃` rail and `╹▀▀▀…` bottom edge; placeholder `Ask anything… "<example>"`
  (examples rotate: "Fix the failing tests", "Review my last commit"); metadata row `Build · Claude Sonnet 5.5`; footer row
  `~/my-app:master` left, `731 credits  shift+tab agents  ctrl+p commands` right; then `Recent` list; bottom line
  `Team · renews 23 Nov` left and version `0.5.1` right — screens/home-120x36-recent.txt — [0.5.1]
- [ ] Home recent list: header `Recent` (faint) + right `ctrl+x l all sessions`; up to 3 rows `<title>` … `now`/`1m ago` — `anysola/home.tsx:6-95` — [0.5.1]
- [ ] Narrow home (60x20): wordmark replaced by the A symbol centered with `Anysola Code` under it; footer dir truncated `…/my-ap…`; recent list hidden (height < 24); version hidden — screens/home-60x20-narrow.txt, `component/logo.tsx:12-31` — [0.5.1]
- [ ] Session (120x36): no sidebar; footer right `728 credits · this turn 12 · session 12  1.3K (0%)  ctrl+p commands` — screens/session-120x36-after-turn.txt — [0.5.1]
- [ ] Session (170x40): sidebar on the right with title, `Context`, `1,260 tokens`, `0% used`, and `~/my-app:master` at the bottom; no `$ spent` line (Anysola cost = 0); edit diff switches to split view (`1 - Helo wrold` | `1 + Hello world`) — screens/session-170x40-sidebar.txt — [0.5.1]
- [ ] Tool rows seen: `⚙ Todo update failed` (todowrite error, inline), running shell `⠏ cat hello.txt && ls` then block `$ cat hello.txt && ls` + output, `Press ctrl+b to move running work to the background` hint while running, `← Edit hello.txt` block with unified diff `1 - …` / `1 + …`, `+ Thought · 4ms` collapsed reasoning, assistant markdown (heading, list, inline code, fenced ts code, grid table with box chars, `│` quote), footer `Build · Claude Sonnet 5.5 · 7.1s · 638.3 tok/s` — screens/session-tools-edit-diff-markdown.txt — [0.5.1]
- [ ] Permission prompt: `△ Permission required`, body `$ <command>` (shell) or `→ Edit <file>` + diff (edit), buttons ` Allow once   Always allow   Reject `, right hint `ctrl+f fullscreen  ⇆ select  enter confirm` — screens/permission-shell.txt, permission-edit-diff.txt — [0.5.1]
- [ ] Tab strip marks a tab needing attention with `!` before the title (`! Fix typo in hello.txt`) — screens/permission-shell.txt — [0.5.1]
- [ ] Exit summary printed on the main screen after leaving the alternate screen (format in section 5) — screens/exit-summary.txt — [0.5.1]
- [ ] `/debug` dialog reports `Version    2.2.5 (latest)` (upstream version, not 0.5.1) — screens/debug.txt, `component/dialog-debug.tsx` — [0.5.1] (parity quirk: keep or fix deliberately)
- [ ] `@` popup lists skills discovered by walking up from the project dir (here the user's `~/.claude/skills` were found from outside the isolated HOME), rows `@<name> <description…>   skill` — screens/at-mention-popup.txt — [0.5.1]
- [ ] `/` popup at home lists 41 rows (aliases are separate rows, server commands `/init` "guided AGENTS.md setup", `/review` "review changes [commit|branch|pr], defaults to uncommitted" included); session-only commands (`/compact`, `/undo`, `/redo`, `/rename`, `/timeline`, `/fork`, `/copy`, `/export`, …) appear only inside a session — screens/slash-list-home-context.txt — [0.5.1]


## 1 and 3. Routes, session layout, sidebar and message rendering

Paths are relative to `packages/tui/src/` unless they start with `packages/`. "content width" = session pane width minus 4 (`routes/session/index.tsx:259`). Config defaults quoted below come from `config/index.tsx:300-345` and the `?? default` at each use site.

### Layout regions (top to bottom, left to right)

- App root is a column filling the terminal on `theme.background.base`; order: [vertical tab column] | [horizontal tab strip, then route body] ; then DevTools bar (only when `debug.devtools`), startup loading overlay, reconnecting overlay, migration overlay, toasts — `app.tsx:1383-1462` — [0.5.1]
- Horizontal session tab strip is row 1 above the session (e.g. `Latest Bun version            +`) when tabs are enabled (`tabs.mode` auto/on, not `HERDR_ENV=1`), at least one tab exists, route is not a plugin route and no full-screen panel is open; tab strip itself is specified in the tabs/dialogs section — `app.tsx:610-618`, `app.tsx:1417-1419`, `component/session-tabs.tsx` — [0.5.1]
- Vertical tabs replace the strip when `tabs.layout = "vertical"` and terminal width ≥ tabsWidth + 64; tabs column default width 42, clamp [5, min(72, width-44)], resizable by mouse drag on its right edge, persisted as `layout.verticalTabsWidth` — `app.tsx:536-550`, `ui/layout.ts:1-20` — [0.5.1]
- Session route = `SessionFrame`: a row of [session pane (flex-grow)] + [right pane: sidebar | terminal | plugin panel] — `component/session-frame.tsx:261-398` — [0.5.1]
- Session pane = column: transcript scrollbox (flex-grow) → 1-row status row (right-aligned "Loading session history…" or "Jump to latest ↓") → bottom block: queued-prompt dock, `session.composer.top` plugin slot (Anysola banners live here), Composer tab panel, then exactly one of: permission prompt / form (question) prompt / "Session location unavailable" question / the prompt input — `routes/session/index.tsx:1502-1650` — [0.5.1]
- No separate session header row exists in 0.5.1: the session title is shown only in the tab strip and in the sidebar — `routes/session/index.tsx:1502`, `routes/session/sidebar.tsx:33-48` — [0.5.1]
- Session pane horizontal padding is 2 cells each side, 1 when terminal width < 44; bottom padding 1 — `routes/session/index.tsx:1504-1507` — [0.5.1]
- Every transcript row has `marginTop={1}` (one blank line between rows) — `routes/session/index.tsx:1669` — [0.5.1]
- Most transcript content is indented 3 cells (`paddingLeft={3}`); inline tool rows reserve a 2-cell icon column (`INLINE_TOOL_ICON_WIDTH = 2`) — `routes/session/message-parts.tsx:19`, `:222` — [0.5.1]
- Left "split border" glyph used for user messages, block tools, reasoning, prompts: `┃` (vertical only, no corners) — `ui/border.ts:15-21` — [0.5.1]
- Child (subagent) sessions: never show the sidebar; composer opens on the "Subagents" tab by default and the prompt input is hidden (closing the composer navigates to the parent) — `component/session-frame.tsx:108`, `routes/session/index.tsx:1585-1598` — [0.5.1]

### Responsive breakpoints

- Sidebar docks to the right (width 42) when `session.sidebar` is `auto` (default) and (terminal width − vertical-tabs width) > 120 — `component/session-frame.tsx:106-111`, `ui/layout.ts:1` — [0.5.1]
- At ≤ 120 the sidebar is hidden; toggling it shows it as an overlay: full-screen box with backdrop `RGBA(0,0,0,70)`, sidebar right-aligned; `escape` or `ctrl+c` closes it (layer priority 10, title "Close sidebar") — `component/session-frame.tsx:387-398`, `routes/session/index.tsx:254-258` — [0.5.1]
- Sidebar toggle `session.sidebar.toggle` default `<leader>b` (ctrl+x b); palette title "Show sidebar"/"Hide sidebar"; persists `session.sidebar` = "hide"/"auto" to TUI config; opening it closes an open panel or terminal — `component/session-frame.tsx:118-130`, `:186-195`, `config/keybind.ts:96` — [0.5.1]
- Assistant footer: model name shown only when terminal width ≥ 28; duration shown when width < 28 or ≥ 36 — `routes/session/index.tsx:2042-2047` — [0.5.1]
- Edit/patch diffs: `diffs.view` `auto` (default) → split when content width > 120, else unified; `split`/`unified` force — `routes/session/index.tsx:3444-3451`, `:3524-3528` — [0.5.1]
- Permission-prompt diff uses terminal width > 120 (not content width) for auto split — `routes/session/permission.tsx:32-37` — [0.5.1]
- Permission/form/location prompts: action footer stacks vertically (column, left-aligned) when terminal width < 80 — `routes/session/permission.tsx:283`, `:427`, `:513-523` — [0.5.1]
- Form header shows field tabs only when Σ(min(len(label),24)+3) + len("Submit")+3 ≤ width − 8; otherwise "Field N of M · A/M completed" — `routes/session/form.tsx:119-122`, `:783-845` — [0.5.1]
- Docked plugin panel (diff viewer) needs width ≥ 108 (64 preferred + 44 min) to split, else full screen; right pane width default floor(total/2), clamp min(24, half) … max(half, total−44), persisted `layout.paneWidth`, drag handle between panes — `ui/layout.ts:5-26`, `component/session-frame.tsx:41-56`, `:378-380` — [0.5.1]
- Inline images height = clamp(floor(terminal height / 4), 4, 8), width = 2×height — `routes/session/index.tsx:2647` — [0.5.1]
- Sidebar "Getting started" card hidden when terminal height < 22 — `feature-plugins/sidebar/footer.tsx:19` — [0.5.1]
- Crash screen: content width min(84, max(24, width−4)); subtext only when height ≥ 18; footer only when height ≥ 20 — `component/error-component.tsx:104-106` — [0.5.1]

### Sidebar

- Box: width 42, background `background.raised.base`, padding 1 top/bottom, 2 left/right; title row (bold, `title_shimmer`: animated shimmer while an auto-title is pending, dim instead when `animations=false`); untitled sessions show `New session - <ISO timestamp>` — `routes/session/sidebar.tsx:22-48`, `packages/util/src/session-title-fallback.ts:14-36` — [0.5.1]
- Middle is a vertical-only scrollbox (1-col absolute scrollbar) with `sidebar.content` plugin slots, gap 1; bottom `sidebar.footer` slot — `routes/session/sidebar.tsx:50-76` — [0.5.1]
- Slot order (builtins order): Context, MCP, Todo; footer: Getting started card, then location — `plugin/builtins.ts:16-33` — [0.5.1]
- Context section (hidden when no usage and cost 0): bold "Context"; "<N> tokens" (`toLocaleString`, e.g. `12,345 tokens`) = input+output+reasoning+cache.read+cache.write of the last assistant message with usage before the revert boundary; "<P>% used" = round(tokens / model.limit.context × 100) when the model has a context limit; "$X.XX spent" (en-US USD) only when session family cost > 0 — `feature-plugins/sidebar/context.tsx:10-41`, `util/session.ts:46-61` — [0.5.1]
- "$ spent" line effectively never shows with Anysola models because gateway model configs carry no pricing (cost stays 0); credits live in the status line instead — `feature-plugins/sidebar/context.tsx:36` — [dead: Anysola models report no cost; verify in capture]
- MCP section (only when ≥ 1 server): bold "MCP"; when > 2 servers a `▼`/`▶` toggle (persisted plugin storage `view.open`, mouse-down toggles) and collapsed suffix "(N active, M error(s))"; each row `•` dot (connected=success, failed=error, needs_auth=warning, disabled/other=muted) + bold name (truncated) + status text "Connected"/"Connecting"/"Error"/"Disabled"/"Sign in"/raw status; click opens MCP dialog for that server — `feature-plugins/sidebar/mcp.tsx:5-93` — [0.5.1]
- Todo section (only when the latest completed `todowrite` before the revert boundary has items): bold "Todo"; > 2 items gets `▼`/`▶` toggle and collapsed suffix "(N remaining)" (pending+in_progress); items via TodoItem — `feature-plugins/sidebar/todo.tsx:5-49`, `util/todo.ts:33-43` — [0.5.1]
- TodoItem: marker `[✓]` completed, `[•]` in_progress (warning color), `[-]` cancelled (faint + strikethrough), `[ ]` pending; content word-wrapped — `component/todo-item.tsx:5-26`, `util/todo.ts:25-30` — [0.5.1]
- Footer "Getting started" card (raised.high bg): `⬖` + bold "Getting started" + `✕` (click dismisses, persisted `getting-started.dismissed`), text "Sign in with your Anysola account to start.", row "Sign in" … "/connect" (click dispatches `provider.connect`); shown only when no provider is connected — `feature-plugins/sidebar/footer.tsx:9-72` — [0.5.1]
- Footer location: `<dir>:<branch>` (home abbreviated, branch from VCS), FilePath truncation max 38, hover → base color, click opens working-directory actions (move session/workspaces) — `feature-plugins/sidebar/footer.tsx:74-111` — [0.5.1]
- No "Modified files" list and no LSP status in the 0.5.1 sidebar (upstream v2 dropped them; the OpenCode Console limit section was deleted by Anysola) — `feature-plugins/sidebar/` (context, mcp, todo, footer only), commit 12bab1edde — [dead: not present]

### Transcript mechanics

- Transcript is a scrollbox with sticky-scroll to bottom (`stickyStart="bottom"`); stops following while the reader is away from bottom or during message navigation — `routes/session/index.tsx:1512-1546` — [0.5.1]
- Optional vertical scrollbar (`session.scrollbar`, default false; command "Toggle session scrollbar", no default key) — `routes/session/index.tsx:239`, `:1168-1180`, `:1520-1528` — [0.5.1]
- Mouse wheel speed 3 lines per tick by default; `scroll.speed` overrides; `scroll.acceleration=true` uses macOS-style acceleration — `util/scroll.ts:20-29` — [0.5.1]
- Tail-first mounting: only the newest 40 weighted rows mount on open; older rows mount in chunks of 60 as the reader nears the top; at the very top older history pages load from the server and the viewport is kept in place — `routes/session/index.tsx:147-148`, `:418-470`, `routes/session/history.ts` — [0.5.1]
- "Jump to latest ↓" (action.secondary color, hover color) at the right of the row under the transcript when away from bottom; click scrolls to bottom — `routes/session/index.tsx:1561-1578` — [0.5.1]
- "Loading session history…" (info color) shown in that row while "First message" loads all history — `routes/session/index.tsx:1560-1563` — [0.5.1]
- Scroll position per session tab is saved as an anchor and restored on return — `routes/session/index.tsx:531-617` — [0.5.1]
- Scroll keys (global, palette-hidden): page up/down = half the viewport (`pageup,ctrl+alt+b` / `pagedown,ctrl+alt+f`); half page = quarter viewport (`ctrl+alt+u` / `ctrl+alt+d`); line (`ctrl+alt+y` / `ctrl+alt+e`) — `routes/session/index.tsx:914-957`, `config/keybind.ts:181-186` — [0.5.1]
- First message `ctrl+g,home,alt+home` (loads full history, then scrolls to 0); last message `ctrl+alt+g,end`; last user message `alt+end`; next/previous message and next/previous user message exist with no default key — `routes/session/index.tsx:959-1036`, `:1210-1258`, `config/keybind.ts:187-193` — [0.5.1]
- Message navigation stops on user messages with text and assistant messages with text (assistant aligned 1 row above); adds bottom slack so the target can reach the top — `routes/session/message-navigation.ts:8-48` — [0.5.1]
- Streaming: assistant markdown renders incrementally (`streaming` true until the message completes; trailing block stays unstable); rows appear on `session.text.delta` / `reasoning.delta` / `tool.input.started` events — `routes/session/message-parts.tsx:186-196`, `routes/session/rows.ts:274-301` — [0.5.1]
- No per-message timestamps are rendered in the transcript in 0.5.1; time appears only as turn duration in the footer and in dialogs — `routes/session/index.tsx:2000-2058` — [0.5.1]

### Grouping and verbosity

- `session.grouping` `auto` (default): adjacent read/glob/grep/webfetch/websearch tools collapse into one exploration row; "Show tool calls individually"/"Group related tool calls" palette command toggles to `none` — `routes/session/grouping/session.ts:78-91`, `routes/session/index.tsx:1182-1195` — [0.5.1]
- Exploration row: running → spinner + "Exploring: <counts>"; done → icon `→` + "Explored: <counts>"; counts map grep/glob/websearch→"search(es)", webfetch→"fetch(es)", read→"read(s)", e.g. `Explored: 2 reads, 1 search`; click toggles expansion (shows the individual tool rows); hover → base color — `routes/session/group-view.tsx:115-158` — [0.5.1]
- `session.verbosity` medium (default) / low / high; "Verbosity: Medium" palette command cycles low→medium→high; high opens exploration and instruction groups by default — `routes/session/index.tsx:243-249`, `:1196-1209` — [0.5.1]
- Low verbosity: every run of non-text parts becomes one activity row `+`/`−` (U+2212) + "<n> command(s), <n> edit(s), <n> thought(s), <n> read(s), <n> tool(s), <n> instruction(s)" (finished items only; while nothing finished, the running item's label: "Thinking…", "Preparing <noun>…", "Running <noun>…", noun command/code/tool name) — `routes/session/group-view.tsx:201-232`, `routes/session/activity-summary.ts:47-95` — [0.5.1]
- Instruction loads (synthetic messages with `metadata.instruction.paths`) group as `◈ Instructions: N file(s)`; click expands — `routes/session/group-view.tsx:234-265` — [0.5.1]
- Question tool calls never group — `routes/session/grouping/session.ts:86` — [0.5.1]
- Permission-blocked tools stay visible below a collapsed group — `routes/session/group-view.tsx:292-309` — [0.5.1]
- Group expansion state is kept per session tab (session-tabs store) — `routes/session/index.tsx:1433-1442` — [0.5.1]

### Thinking / reasoning

- `session.thinking` default `hide`: consecutive reasoning parts form one group row: running → spinner + "Thinking" or "Thinking: <title>"; done → `+`/`-` icon + "Thought" + ": <title>" (collapsed only) + " · N steps" (when > 1) + " · <duration>"; colors: running base, done warning at 60% alpha, hover/expanded full warning — `routes/session/group-view.tsx:162-189` — [0.5.1]
- Title = first `**bold**` block at the start of the reasoning text (OpenAI summary style), stripped from body — `context/thinking.ts:9-14` — [0.5.1]
- Expanded thoughts render each part as markdown via `<code filetype="markdown">` inside a left `┃` border, every syntax style recolored to `text.muted` — `routes/session/group-view.tsx:358-403`, `routes/session/thinking-syntax.ts` — [0.5.1]
- `show` mode: each reasoning part rendered open with left border; header "Thought: <duration>" (warning 60% alpha), spinner "Thinking"/"Thinking: <title>" while streaming — `routes/session/message-parts.tsx:43-171` — [0.5.1]
- Palette command "Expand thinking"/"Collapse thinking" (`session.toggle.thinking`, no default key) flips config `session.thinking` — `routes/session/index.tsx:1150-1166`, `config/keybind.ts:197` — [0.5.1]
- `[REDACTED]` placeholder removed from reasoning text; redacted-only reasoning renders nothing — `routes/session/message-parts.tsx:119-122` — [0.5.1]

### Assistant text (markdown)

- Assistant text parts render with OpenTUI `<markdown>` (marked lexer, `internalBlockMode="top-level"`), fg `theme.markdown.text`, bg `background.base`, indented 3 — `routes/session/message-parts.tsx:173-200` — [0.5.1]
- `session.markdown` `rendered` (default) conceals markers (`**`, `*`, `` ` ``, `~~`, link brackets); `source` shows them — `routes/session/index.tsx:240`, `packages/tui/node_modules/@opentui/core/index.node.js:11780-11880` — [0.5.1]
- Supported blocks: headings/paragraphs (tree-sitter markdown highlight with conceal), fenced code (CodeRenderable, language from info string, highlighted by tree-sitter), lists (markers `-` or `N.` right-padded to the widest marker), blockquote (left border + `markup.quote` style), hr (1-row top border in conceal color), tables (TextTable, `style: "grid"`, `cellPaddingX: 1`, single border, word wrap, header cells use `markup.heading` style) — `routes/session/message-parts.tsx:192`, `@opentui/core/index.node.js:11999-12260`, `:12520-12560` — [0.5.1]
- Inline: strong, em, del (strikethrough), codespan (`markup.raw`), links (label in `markup.link.label`; when the terminal lacks OSC 8 hyperlinks the URL is appended " (url)"), images shown as alt text or "image", `br` — `@opentui/core/index.node.js:11780-11880` — [0.5.1]
- Links are clickable: a plain left click on a hyperlink cell opens the URL (ctrl/alt/shift clicks are left to the terminal) — `ui/link.tsx:41-47`, `app.tsx:1396-1399` — [0.5.1]
- User messages are plain text (no markdown) — `routes/session/index.tsx:2412` — [0.5.1]
- Code highlighting = tree-sitter (web-tree-sitter WASM in a worker). Bundled grammars: javascript, typescript, typescriptreact (tsx), markdown, markdown_inline, zig. Downloaded on first use from GitHub URLs and cached under `$XDG_DATA_HOME/opentui`: python, rust, go, cpp, csharp, bash, c, java, kotlin, ruby, php, scala, html, vue, hcl, json, yaml, haskell, css, julia, lua, ocaml, clojure, swift, toml, nix, diff (aliases udiff, patch), elixir, fsharp, r, make (alias makefile), vim, xml, agda — `parsers-config.ts:6-386`, `routes/session/index.tsx:139`, `@opentui/core/assets/*`, `@opentui/core/parser.worker.js:3231-3313` — [0.5.1]
- File-path → filetype map for tool output (`.ts/.tsx/.js/.jsx/.mjs/.cjs` → typescript, etc.; unknown → undefined, no path → "none") — `util/filetype.ts:3-135` — [0.5.1]
- ```` ```mermaid ```` blocks with flowchart/gantt/gitGraph/sequence/state/timeline syntax render as box-drawing diagrams (compact, folded to min(120, renderer width)), 1-row top margin, not selectable, horizontal drag/scroll pans; on parse error keeps the last good frame while streaming, else falls back to the code block — `packages/merman/src/plugin.ts:5-14`, `packages/merman/src/markdown.ts:258-320`, `packages/merman/src/detect.ts` — [0.5.1]
- ```` ```latex ```` and ```` ```math ```` blocks render as 2-D Unicode math (display mode, strict), brackets/box chars in muted color, relation symbols bold, horizontally scrollable, 1-row top margin; invalid/oversized (> 100 000 chars, nesting > 256) input falls back to the code block. Inline `$…$` is not rendered — `packages/latex/src/plugin.ts:4-13`, `packages/latex/src/markdown.ts:27-120`, `packages/latex/src/limits.ts:3-4` — [0.5.1]
- Mermaid/LaTeX renderers apply to assistant text and compaction summaries (they use `plugins.markdown()`), not to thinking blocks — `routes/session/message-parts.tsx:188`, `routes/session/index.tsx:2213`, `plugin/markdown.ts` — [0.5.1]

### User message

- Box with left `┃` border in the session agent's color (border.base while still pending delivery), background `raised.base` (darker on hover), padding 1/1/2; plain text — `routes/session/index.tsx:2341-2416` — [0.5.1]
- Attachments: images (only when `session.image_preview` is true; default off) above the text; skill chips ` skill ` (accent bg, bold) + ` <name> `; file chips ` file `/` dir ` + ` <name|uri|"attachment"> ` — `routes/session/index.tsx:2368-2463` — [0.5.1]
- Click (without text selection) opens "Message Actions": Jump to / Revert / Copy / Fork (descriptions "view message in session", "undo messages and file changes", "message text to clipboard", "create a new session"); a pending steer opens "Pending steer": "Move to queue" / "Delete" — `routes/session/index.tsx:2384-2407`, `routes/session/dialog-message.tsx:24-88` — [0.5.1]
- Shell messages (`!` prompt mode) render like the bash tool box; errors "Command cancelled", "Command timed out", "Command exited with code N" — `routes/session/index.tsx:2322-2339` — [0.5.1]

### Assistant footer, errors, retries

- Footer after a terminal step: "<Agent>" (titlecased, agent color; muted on error) + " · <model name>" + " · <duration>" + " · <x.x> tok/s" (when `session.tps`, default true, and every step streamed) + " · interrupted" (error message "Step interrupted"); e.g. `Build · Claude Sonnet 5.5 · 112ms · 129.0 tok/s` — `routes/session/index.tsx:2000-2058`, `config/index.tsx:326` — [0.5.1]
- Duration = message completion − turn input creation; format `850ms`, `12.4s`, `3m 5s`, `2h 10m`, `1d 4h` — `packages/util/src/session-turn.ts:22-45`, `packages/util/src/duration.ts` — [0.5.1]
- Step error (not interrupted, not retrying): "Error: <message>" in error color above the footer, footer gets marginTop 1 — `routes/session/index.tsx:2021-2036` — [0.5.1]
- Retry line (warning color): "⚠ Retrying in Ns · attempt N · <error>" counting down each second, then "⚠ Retry due · attempt N · <error>" — `routes/session/index.tsx:2528-2553` — [0.5.1]
- Turn-level failure (idle message with an error different from the one already shown): "Error: <message>" — `routes/session/index.tsx:1953-1960`, `routes/session/rows.ts:381-388` — [0.5.1]
- Free-limit notice "You've used today's free messages…" / "Upgrade to OpenCode Go → /connect · Switch model → /models" and the Go upsell dialog — `routes/session/index.tsx:626-737`, `:2512-2526` — [dead: needs `provider.free-limit`, produced only for OpenCode Zen `FreeUsageLimitError` bodies; Anysola gateway does not send them]
- OpenCode Go / Console budget notices and "Running low on <model>" toasts (`UsageLimitNotice`, `useUsageAlerts`) — `component/usage.tsx:213-329` — [dead: require OpenCode Console limits (`servedByConsole`) or Go/Console error bodies; Anysola is the only provider]
- Agent/model switch notices: "Switched agent from X to Y", "Switched agent to Y", "Switched model to <Name> (<variant>)", "Switched model to <provider/id>", "Switched variant to <v|default>", "↳ Moved to <dir>" — `routes/session/index.tsx:2060-2088`, `util/model.ts:10-21` — [0.5.1]
- Background completion notices: "↳ <Agent> finished · <description>" (info color, click opens the child session) / "! <Agent> failed" (error) / cancelled (warning); shell actor "Shell"; other synthetic/system notices render as `◈`/`◎` (goal) rows with the description or "Instructions updated" — `routes/session/index.tsx:2090-2147` — [0.5.1]
- Skill message: `→ Skill <name>` — `routes/session/index.tsx:2149-2156` — [0.5.1]
- Background hint 3 s after a shell/subagent tool starts running: "Press <ctrl+b> to move running work to the background" (`session.background`, default `ctrl+b`) — `routes/session/index.tsx:1884-1916`, `config/keybind.ts:129` — [0.5.1]
- Turn token table (`debug.turn_tokens`, off by default): "+ Tokens: N steps · latest: a new · b cached · c total" with cache-bust warnings — `routes/session/index.tsx:1724-1863` — [0.5.1] (debug only)

### Tool rendering (common)

- Tool names canonicalized: bash→shell, task→subagent, apply_patch→patch; displays: shell, glob, read, grep, webfetch, websearch, write, edit, subagent, execute, patch, todowrite, question, skill; anything else (MCP tools, `opencode`, `session_rename`, `artifact_*`, `schedule_*`, `list_mcp_resources`, `experimental_image_generate`, …) → generic — `util/tool-display.ts:1-6`, `routes/session/message-parts.tsx:21-41` — [0.5.1]
- Inline tool row: icon (2 cols) + text; until `complete` is known shows spinner + pending text; failed → icon/text in error color, click toggles the error message below; denied (error contains "QuestionRejectedError", "rejected permission", "specified a rule", "user dismissed") → strikethrough, muted; awaiting permission → warning color; hover (when clickable) → base color — `routes/session/index.tsx:2789-2858`, `routes/session/message-parts.tsx:202-279` — [0.5.1]
- Block tool: left border in `background.base`, background `raised.base` (darker on hover when clickable), padding 1/1/2, gap 1; header = label + FilePath (truncated to content width − 4 − label) or title; header in warning color while awaiting permission; error text below in error color — `routes/session/index.tsx:2871-2965` — [0.5.1]
- Spinner: braille frames `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` at 80 ms; with `animations=false` → `⋯` + text — `component/spinner.tsx:12-49`, `component/spinner-frames.ts` — [0.5.1]
- StatusBadge ` Background ` (muted on darker bg) — `routes/session/index.tsx:2860-2869` — [0.5.1]
- Tool-result images (data:image or media refs) render under the tool only when `session.image_preview` is true; max 3 + "+N more", click opens image preview dialog, "No preview" on load failure — `routes/session/index.tsx:2632-2719` — [0.5.1]

### Tool rendering (per tool)

- shell/bash: block; header-less; command line "$ <cmd>" (prefixed "cd <workdir> && " when workdir ≠ "."), spinner + command while running (no "$ "), output muted below; before the command streams: "Writing command…" (spinner while running) — `routes/session/index.tsx:2969-3141` — [0.5.1]
- shell collapse: max 10 lines and 10 × max(20, content width − 6 − (running?2:0)) chars; command limited to 2 wrapped lines ending "…"; output shows the tail with a first line "(N earlier line(s))"; click toggles full view (only when overflowing or a background shell) — `routes/session/index.tsx:3079-3090`, `util/collapse-tool-output.ts:24-93` — [0.5.1]
- shell background: completed tool whose metadata.status is "running" shows the ` Background ` badge; output polled every 1 s from the shell service (1 MiB cap, "[earlier output omitted]" prefix) only when expanded — `routes/session/index.tsx:2967`, `:3016-3076`, `:3135-3137` — [0.5.1]
- read: `→ Read <path>` + `:<offset>-<end>` when offset/limit given; spinner while running; pending "Reading file…"; extra lines "↳ Loaded <path>" for instruction files the read pulled in — `routes/session/index.tsx:3192-3228` — [0.5.1]
- glob: `✱ Glob "<pattern>" in <path> (N match(es))`; pending "Finding files…" — `routes/session/index.tsx:3179-3190` — [0.5.1]
- grep: `✱ Grep "<pattern>" in <path> (N match(es))`; pending "Searching content…" — `routes/session/index.tsx:3230-3241` — [0.5.1]
- webfetch: `% WebFetch <url>`; pending "Fetching from the web…" — `routes/session/index.tsx:3243-3249` — [0.5.1]
- websearch: `◈ Web Search via Anysola "<query>"` (provider from metadata, id "anysola" → "Anysola"; provider name cross-fades 80 ms out/60 ms resize/80 ms in when it changes); without provider: `◈ Web Search "<query>"`; pending "Searching web…" — `routes/session/index.tsx:3251-3274`, `util/tool-display.ts:42-51`, `component/retry-provider.tsx:19-21`, `packages/core/src/plugin/websearch/anysola.ts:16` — [0.5.1]
- write: completed → block "# Wrote <path>" with the full file content, line numbers (min width 3), syntax highlighted, plus diagnostics; otherwise inline `← Write <path>`, pending "Preparing write…" — `routes/session/index.tsx:3143-3177` — [0.5.1]
- edit: block "← Edit <path>" with PatchDiff of metadata.files[0].patch (line numbers, split/unified, wrap `diffs.wrap` default "word", padding 1 left / 3 right) + diagnostics; before the diff exists: "← Edit <path>" or title "# Preparing edit…" (spinner "Preparing edit…" while streaming) — `routes/session/index.tsx:3439-3501` — [0.5.1]
- patch/apply_patch: one block per file: "# Created <path>" / "← Patched <path>" with diff, "# Deleted <path>" with "-N line(s)"; fallback from metadata.applied (path only); in progress "Patching <path>"/"Patching"; failed "# Patch failed" (header error color, error text muted) — `routes/session/index.tsx:3503-3621` — [0.5.1]
- Diagnostics under write/edit: up to 3 severity-1 entries for that file: "Error [line:col] <message>" (1-based) — `routes/session/index.tsx:3694-3718`, `:3800-3816` — [0.5.1]
- todowrite: completed with items → block "# Todos" + TodoItem list; otherwise inline `⚙` "Cleared todos" / "Todo update failed"; pending "Updating todos…" — `routes/session/index.tsx:3623-3647` — [0.5.1]
- subagent/task: inline row; icon `│` + spinner while the child runs, `↳` for continuation, `✓` done; text "<Agent> Subagent — <description> · <model (variant)>" or "Continue subagent — …" (agent default "General"); pending "Delegating…"; ` Background ` badge for background subagents; click navigates into the child session — `routes/session/index.tsx:3276-3337` — [0.5.1]
- question: answered → block "# Questions" listing each question (muted) and answer joined ", " or "(no answer)"; else `→ Asked N question(s)`; pending "Asking questions…" — `routes/session/index.tsx:3649-3683` — [0.5.1]
- skill: `→ Skill "<name>"`; pending "Loading skill…" — `routes/session/index.tsx:3685-3692` — [0.5.1]
- execute (code mode): row `✓`/`✗`/`│` "execute"; nested calls `› <tool> [k=v, …]` (`✗` on error, click expands key: value lines); runtime error shows first 4 lines "↳ …" in error color; click opens the Execute dialog ("Receiving code…", "Running", "No output", "Waiting for output…", copy code `c`, copy output `o`) — `routes/session/index.tsx:3339-3437`, `routes/session/dialog-execute.tsx:70-174` — [0.5.1] (only when the execute tool is enabled; see tools spec)
- generic (MCP and other tools): `✓`/`✗` + "<tool> [key=value, …]" (primitive inputs only, whitespace collapsed); spinner with the tool name while running; click expands "key: value" lines (objects as 2-space JSON) and "output: <text>" (ANSI stripped) — `routes/session/index.tsx:2721-2776`, `util/tool-display.ts:13-20` — [0.5.1]
- No dedicated LSP tool rendering in 0.5.1 (only a permission presentation for action "lsp") — `util/permission.ts:104-119` — [0.5.1]

### Diff rendering

- PatchDiff splits a patch into hunks; each hunk is an OpenTUI DiffRenderable; between hunks a 1-row separator " @@ -a,b +c,d @@" in `diff.text.hunkHeader` on the line-number background — `component/patch-diff.tsx:37-112`, `util/diff.ts:40-84` — [0.5.1]
- Gutter: padding 1, line-number digits = width of the largest line number across hunks; colors from theme `diff.*` (added/removed/context backgrounds, sign colors, line-number fg/bg) — `component/patch-diff.tsx:46-55`, `routes/session/index.tsx:3462-3481` — [0.5.1]
- Split view rows = max(additions, deletions) per change block — `util/diff.ts:56-84` — [0.5.1]
- Added-only patches > 3000 lines are virtualized in 128-line chunks (diff viewer only, when a scroll container is passed) — `component/patch-diff.tsx:24-25`, `:116-202` — [0.5.1]
- `app.toggle.diffwrap` palette command "Disable/Enable diff wrapping" flips `diffs.wrap` — `app.tsx:1239`, `config/keybind.ts:55` — [0.5.1]

### Diff viewer (`/diff`)

- `/diff` ("Open diff viewer", group VCS) toggles a docked side panel in a session (full screen when `diffs.fullscreen` or not enough width), or a full-screen plugin route outside a session — `feature-plugins/system/diff-viewer.tsx:1310-1360` — [0.5.1]
- Sources: "All" (Branch + local changes), "Committed" (Branch commits only), "Uncommitted" (Local changes only), "Last turn" (Latest session turn); base branch picker "Base branch" / "Search local and remote branches" — `feature-plugins/system/diff-viewer.tsx:80-87`, `:270-300` — [0.5.1]
- Layout: file tree (width clamp(floor(w/4), 30, 40), hidden in compact layout < 88 cols or docked), patch pane, split view when patch pane ≥ 100 cols; reviewed counter "r/n"; states "Loading diff…", "No changes to show", "Could not load diff. …" — `feature-plugins/system/diff-viewer.tsx:44-53`, `:325-352`, `:880-930` — [0.5.1]
- Keys (defaults): close `escape,q`; down/up `j,down`/`k,up`; page `pagedown,ctrl+f`/`pageup,ctrl+b`; half page `ctrl+d`/`ctrl+u`; first/last `gg,home`/`shift+g,end`; hunks `]`/`[`; files `n,alt+down`/`p,alt+up`; tree `b`; single patch `s`; source `d`; view `v`; fullscreen `f`; reviewed `m`; help `?` ("Diff shortcuts" dialog); right-click file menu "Mark complete"/"Mark incomplete" — `config/keybind.ts:64-90`, `feature-plugins/system/diff-viewer.tsx:1225-1270`, `feature-plugins/system/diff-viewer-file-menu.tsx:17-28` — [0.5.1]

### Compaction, revert, queue

- Compaction row: horizontal rule with centered label "Compaction" ("Provider compaction" for provider-native), spinner while running, `✗` + error color on failure, "· cancelled", "· <in> in · <out> out" (K/M abbreviations), then the summary as markdown indented 3 — `routes/session/index.tsx:2158-2224`, `util/locale.ts:19-27` — [0.5.1]
- Queued compaction: rule with "◇ Compaction queued" — `routes/session/index.tsx:2226-2238` — [0.5.1]
- `/compact` ("Compact session", `<leader>c`) compacts with the current model/variant — `routes/session/index.tsx:1082-1106`, `config/keybind.ts:130` — [0.5.1]
- Revert banner at the end of the transcript: raised box with "N message(s) reverted", per file "A|M|D <path> +a -d", and "<redo key> or /redo to restore"; click = redo — `routes/session/index.tsx:2240-2320`, `component/dialog-workspace-file-changes.tsx:23-27` — [0.5.1]
- `/undo` ("Undo previous message", `<leader>u`): restores the last user message text into the prompt, interrupts, waits, stages a revert; "Nothing to undo" toast when none — `routes/session/index.tsx:1106-1132` — [0.5.1]
- `/redo` ("Redo", `<leader>r`, enabled only while a revert is staged) clears the revert — `routes/session/index.tsx:1134-1148` — [0.5.1]
- Queued prompts dock above the composer: "N queued · <first prompt>" (click opens "Queued prompts" dialog with search "Search queued prompts", footer "i of n", enter = steer, actions delete/undo) — `routes/session/index.tsx:764-840`, `:2468-2500` — [0.5.1]

### Permission prompt

- Replaces the prompt input when the session (or a descendant) has a pending permission and `session.permissions` is `prompt` (default); `autoaccept` answers every request "once" automatically — `routes/session/index.tsx:207-213`, `:268-286`, `:1600-1609` — [0.5.1]
- Box: left border `background.action.primary.focused`, raised background, max height 15 rows; header `△` (warning) "Permission required", then (except shell) "<icon> <title>"; body per action; footer (darker bg): option buttons, right side "<ctrl+f> fullscreen" (or "minimize"), "⇆ select", "enter confirm" — `routes/session/permission.tsx:200-215`, `:461-608` — [0.5.1]
- Options: "Allow once", "Always allow" (only when the request has `save` patterns), "Reject"; first option preselected; `left,h` / `right,l` cycle; `return` selects; `escape` and `app.exit` (`ctrl+c,ctrl+d,<leader>q`) reject (escape first leaves fullscreen); mouse hover selects, click confirms — `routes/session/permission.tsx:232-257`, `:427-475`, `util/permission.ts:46-52` — [0.5.1]
- Selecting "Always allow" switches the body to "This will always allow <action> for this project." (save = ["*"]) or "This will always allow the following patterns for this project." + "- <pattern>" lines — `routes/session/permission.tsx:224-230`, `util/permission.ts:38-44` — [0.5.1]
- Body: edit → scrollable diff (always word-wrapped) or raw patch or "No diff provided"; external_directory → "Patterns" + lines; others → lines ("$ <command>", "Path: …", "Pattern: …", "URL: …", "Query: …", "◉ <description>", "Tool: <name>") — `routes/session/permission.tsx:22-111`, `:168-198`, `util/permission.ts:23-128` — [0.5.1]
- Fullscreen (`permission.prompt.fullscreen`, default `ctrl+f`): prompt moves to a portal covering the screen (top −height+1, bottom 1, left/right 2) — `routes/session/permission.tsx:443-452`, `:497-507`, `config/keybind.ts:278` — [0.5.1]
- Reject inside a child (subagent) session opens "Reject permission" with textarea "Tell Anysola Code what to do differently", "enter confirm", "esc cancel"; ctrl+c clears the text first — `routes/session/permission.tsx:248-251`, `:272-402` — [0.5.1]
- Tool rows awaiting permission are tinted warning — `routes/session/index.tsx:2778-2787`, `:2830` — [0.5.1]

### Question / form prompt

- Question tool and server forms (e.g. web search provider consent) render as a FormPrompt replacing the input: title (muted), optional message, field tabs or "Field N of M · A/M completed", body, footer hints "⇆ tab", "↑↓ select"/"↑↓ scroll", "enter <action>", "c copy" (external), "esc dismiss|close", error text right — `routes/session/form.tsx:767-1175` — [0.5.1]
- Options: numbered "1." … ; single-select marks the chosen row " ✓"; multiselect shows "[✓]"/"[ ]"; extra "Type your own answer" row when custom answers are allowed; digit keys 1-9 pick, `up,k`/`down,j` move, `return` select, `space` toggles (multiselect), `left,h`/`right,l`/`tab`/`shift+tab` switch fields, `escape`/app.exit dismiss — `routes/session/form.tsx:640-760`, `:930-1060` — [0.5.1]
- Action label: "submit" (review / single question), "confirm", "toggle", "edit", "done", "open link", "I finished", "continue" — `routes/session/form.tsx:194-209` — [0.5.1]
- Text answers: placeholders "Type your answer", "name@example.com", "https://example.com", "YYYY-MM-DD", "YYYY-MM-DDTHH:MM:SSZ", "<min>-<max>", "at least N", "at most N" — `routes/session/form.tsx:167-184` — [0.5.1]
- Review tab "Submit": each field "<label>: <value>" or "(required)" / "(not answered)" / validation error; external fields "Acknowledged"/"(acknowledgement required)" — `routes/session/form.tsx:1060-1110` — [0.5.1]
- In-progress answers survive tab switches (kept per form id) — `routes/session/form.tsx:53-56` — [0.5.1]

### Composer tab panel (above the prompt)

- Bordered raised panel with tabs "Subagents", "Shell", "Terminals" (when PTYs are available); single tab shows its label bold; "esc" closes (also `ctrl+c`); `left`/`right` switch tabs; footer hints "<label> <key>" and "tabs ←/→" — `routes/session/composer/index.tsx:77-153` — [0.5.1]
- Subagents tab (`session.child.first`, default `down` when the input is empty… bound in the base layer): tree of the session family "<prefix><Agent>: <title>" with "Running"; max 5 rows; `ctrl+a` toggles active/inactive filter ("show inactive"/"show active"); `ctrl+d` interrupts; `return` navigates; empty "No active subagents"/"No inactive subagents" — `routes/session/composer/subagents-tab.tsx:41-255`, `util/session.ts:10-44` — [0.5.1]
- Shell tab: background shell commands, `return` view output, `ctrl+d` kill, empty "No shell commands" — `routes/session/composer/shell-tab.tsx:52-120` — [0.5.1]
- Child-session navigation keys: `up` parent, `left`/`right` previous/next child (session.parent / session.child.*) — `config/keybind.ts:138-141`, `routes/session/index.tsx:1403-1418` — [0.5.1]

### Terminal pane (right pane)

- When the server reports `persistentPty` capability: `/terminal` "New terminal", `<leader>t` toggle, `<leader>down` select, `<leader>up` close, `<leader>left`/`<leader>right` focus panes; terminal shows in the right pane (resizable); "Unable to load terminal" toast on failure — `component/session-frame.tsx:164-257`, `context/session-terminals.tsx:67-82`, `config/keybind.ts:97-101` — [0.5.1]

### Copy, selection, mouse

- Mouse capture on by default (`mouse` config) — `config/index.tsx:316` — [0.5.1]
- `terminal.copy` default `select` (non-Windows) copies the selection on mouse release after a drag; `manual` (Windows default) copies on right-click; toast "Copied to clipboard" (info); selection is not cleared after copy — `app.tsx:582-583`, `:1388-1400`, `util/selection.ts:34-63` — [0.5.1]
- With a selection: `ctrl+c` copies (or clears when copy-on-select already copied), `escape` clears; any other key clears unless the selection is inside the focused editor — `util/selection.ts:65-101`, `app.tsx:586-592` — [0.5.1]
- Clickable rows ignore the click when text is selected (selection wins) — e.g. `routes/session/index.tsx:2381`, `:2906` — [0.5.1]
- `/copy` ("Copy last assistant message", `<leader>y`): toasts "Message copied to clipboard!", "No assistant messages found", "No text content found in last assistant message", "Failed to copy to clipboard"; palette-only "Copy session ID" ("Session ID copied to clipboard!" / "Failed to copy session ID") and "Copy session transcript" ("Session transcript copied to clipboard!" / "Failed to copy session transcript") — `routes/session/index.tsx:1259-1330`, `config/keybind.ts:107-108`, `:195` — [0.5.1]
- `/export` ("Export session transcript", `<leader>x`): export options dialog; copy → "Copied to clipboard"; file → `<tmpdir>/session-<uuid>.md|json` then export-result dialog; failure "Failed to export session"; markdown format: `# <title>`, `**Session ID:**`, `**Created:**`, `**Updated:**`, `---`, `## User`, `## Assistant`, `## Shell`, `_Thinking:_`, `**Tool: <name>**`, `**Input:**` JSON block — `routes/session/index.tsx:1332-1375`, `:3728-3762` — [0.5.1]

### Session-level commands registered by the session route

- `/rename [title]` (`ctrl+r`; empty argument regenerates the title), `/timeline` ("Jump to message", `<leader>g`), `/fork` ("Fork session"; toast "Forked session"), `/compact`, `/undo`, `/redo`, `/copy`, `/export`; palette-only: Expand/Collapse thinking, Toggle session scrollbar, grouping toggle, Verbosity, Copy session ID, Copy session transcript, Background blocking tools (`ctrl+b`), Toggle subagent picker, View queued prompts (`<leader>q`), Go to parent session — `routes/session/index.tsx:1039-1421`, `routes/session/dialog-fork.tsx:49` — [0.5.1]
- "Session not found: <id>" error toast (5 s) closes the tab or returns home — `routes/session/index.tsx:347-357` — [0.5.1]

### Overlays

- Reconnecting overlay (dim backdrop RGBA(0,0,0,150), 48-wide box): spinner "Restarting service…" + "Your session will resume automatically." (managed service) or "Connection lost…" + "Reconnecting to the server automatically." — `component/reconnecting.tsx:5-40` — [0.5.1]
- Crash screen: "Anysola Code crashed", "An unexpected error stopped the session.", " Error " box, " Stack trace " box, actions "Copy report"/"✓ Copied"/"Copy failed", `r` Restart, `q` Quit, footer "Copy the report and open a GitHub issue to help us fix this.", version line "Anysola Code <ver> (OpenCode 2.2.5)"; issue URL `https://github.com/Anysola/code/issues/new?template=bug-report.yml` — `component/error-component.tsx:44-266` — [0.5.1]
- Session location missing: question "Session location unavailable", "<dir>", "Choose another directory to continue this session.", option "Choose directory" → workspaces dialog; toasts "Creating workspace failed", "Failed to move session" — `routes/session/location-missing.tsx:20-81` — [0.5.1]

## 2. Prompt / composer

All paths are under `packages/tui/src/`. "state dir" = `$XDG_STATE_HOME/opencode` (`~/.local/state/opencode`); per-channel TUI storage = `<state>/<channel>/tui/<key>.json` (`context/storage.tsx:48-58`, channel is `latest` in shipped builds, see the home template `.local/state/opencode/latest/tui/`).

### Layout of the prompt box
- Prompt box = outer box with a single left border `┃` (`SplitBorder.customBorderChars.vertical`), bottom-left corner `╹`; border colour = theme `border.base` tinted toward the highlight colour by the agent-label fade-in alpha — `component/prompt/index.tsx:1694-1703`, `ui/border.ts:15-20`, `component/prompt/index.tsx:1622` — [0.5.1]
- Highlight colour: `border.base` while muted (leader key pending or `muted` prop), `text.action.primary.selected` in shell mode, otherwise the current agent's colour — `component/prompt/index.tsx:1602-1606` — [0.5.1]
- Agent colour: `agent.color` from config if set, else categorical theme palette step 200 (step 400 under the opt-in "new-designs" experiment), de-duplicated, indexed by the agent's position among visible primary agents — `context/local.tsx:77-83`, `context/local.tsx:128-135` — [0.5.1]
- Inner box: background `theme.decrease(background.raised.base)`; paddingLeft/Right 2 (1 when terminal width < 44); paddingTop 1 — `component/prompt/index.tsx:1690`, `component/prompt/index.tsx:1704-1712` — [0.5.1]
- Textarea: min height 1 row, max height `max(6, floor(termHeight/3))` rows, then it scrolls; cursor colour `text.base` (`background.raised.base` when disabled); cursor style from config — `component/prompt/index.tsx:1688`, `component/prompt/index.tsx:1781-1867` — [0.5.1]
- Below the textarea: metadata row (paddingTop 1), then a 1-row strip whose bottom border is `▀` drawn in the prompt background colour (gives a half-block bottom edge; a blank when the bg is transparent) with `╹` on the left rail — `component/prompt/index.tsx:1868-1911` — [0.5.1]
- Below that, the footer row (not inside the box): status area left (grows), plugin slots, editor-file label, right-side hints — `component/prompt/index.tsx:1912-1996` — [0.5.1]
- Prompt focus: auto-focuses whenever visible, enabled and no dialog is open; blurs and becomes unfocusable while a dialog is open — `component/prompt/index.tsx:741-753` — [0.5.1]
- Session route: prompt hidden (replaced) while a permission request, a question form, or a "location missing" notice is pending, or while the composer dock is open / the session is a child session — `routes/session/index.tsx:1600-1648` — [0.5.1]
- Home route: prompt disabled while a question form is open (form floats above, absolute, bottom 1) — `routes/home.tsx:103`, `routes/home.tsx:111-117` — [0.5.1]

### Placeholder
- Home prompt placeholder: `Ask anything… "<example>"`, example picked at random from `Explain this project`, `Fix the failing tests`, `Review my last commit`; re-rolled when the sessionID prop changes — `routes/home.tsx:21-24`, `component/prompt/index.tsx:1629-1641`, `component/prompt/index.tsx:410-418` — [0.5.1]
- Home shell-mode placeholder: `Run a command… "<example>"` from `ls -la`, `git status`, `pwd`; re-rolled each time shell mode is entered — `routes/home.tsx:23`, `component/prompt/index.tsx:984` — [0.5.1]
- Session prompt has no placeholder (no `placeholders` prop passed) — `routes/session/index.tsx:1633-1646` — [0.5.1]
- Placeholder is cut to width: `termWidth-5` when width < 44, else `min(75, termWidth-4) - 5`, trailing spaces trimmed; colour `text.muted` — `component/prompt/index.tsx:1639-1640`, `component/prompt/index.tsx:1784` — [0.5.1]

### Metadata row (agent · model · variant)
- Normal mode shows: `<Agent>` (agent id title-cased, agent colour) then optional `auto` (muted, when permission mode is `autoaccept`: `--auto` flag or settings Permissions) then `·` (muted) then model display name (base colour, muted when muted) then provider name (muted) then `·` + variant (bold, `text.feedback.warning.base`) then optional usage-limit text — `component/prompt/metadata.tsx:39-101`, `component/prompt/index.tsx:1869-1882`, `context/permission.tsx` — [0.5.1]
- Provider label is blanked when it equals `Anysola` (Anysola fork change), so the row reads e.g. `Build · Claude Sonnet 5.5 · economy` — `component/prompt/index.tsx:1874` — [0.5.1]
- Responsive fit: candidates tried in order (full with `auto`; without `auto`; provider reduced to its last ` / ` segment; without provider); width budget = termWidth − 13 (− 9 when < 44); if none fits, model name is truncated with `…` (min 9 cols) keeping agent prefix and variant/limit suffix — `component/prompt/metadata.tsx:117-166` — [0.5.1]
- Terminal width < 44: agent label and provider hidden — `component/prompt/metadata.tsx:127-128` — [0.5.1]
- Fade-in: agent, model and variant labels fade in (alpha animation) the first time per app instance, only when animations are enabled — `component/prompt/index.tsx:1608-1621` — [0.5.1]
- While catalogs load, the last resolved labels stay on screen — `component/prompt/index.tsx:1551-1587` — [0.5.1]
- Shell mode: row shows only `Shell` in the highlight colour — `component/prompt/metadata.tsx:26`, `component/prompt/index.tsx:1607` — [0.5.1]
- Usage-limit suffix (`$X left` / `Limit reached`) from OpenCode Console limits — `component/prompt/index.tsx:1588-1601`, `component/usage.tsx:62-68` — [dead: only shown for providers with `integrationID === "opencode"` (`util/usage-limits.ts:36-38`); Anysola is the only provider]

### Footer row under the prompt
- Status area, idle: working-directory label `~/path` (home abbreviated) plus `:<branch>` when the location is a VCS repo; for a session it shows only when the session is idle; on home it shows where the next session will be created; middle-truncated as a file path to the measured width; muted, base colour on hover — `component/prompt/index.tsx:1642-1663`, `component/prompt/index.tsx:1959-1977` — [0.5.1]
- Clicking the directory label opens a `Working directory` menu: `Copy path` (description = path; toast `Path copied to clipboard`), `Open folder` (`in system file manager`), `Workspaces` (`to another working directory`) — `ui/working-directory-actions.tsx:16-57`, `component/prompt/index.tsx:1664-1667` — [0.5.1]
- Status area, running: knight-rider spinner (8 cells, `■` active / `⬝` inactive, agent colour, 40 ms frames; `[⋯]` when animations are off) then `esc interrupt`; after one Esc it becomes `esc again to interrupt` in warning colour with a short flash, reverting after 5 s — `component/prompt/index.tsx:1925-1941`, `component/prompt/index.tsx:110-147`, `ui/spinner.ts:273-324` — [0.5.1]
- Status area during workspace move: spinner + `Creating worktree` / `Creating session` / `Submitting prompt` plus 1–3 animated dots (1 s step); `(new worktree)` in accent colour when a new worktree is chosen but not yet created — `component/prompt/index.tsx:1942-1958`, `component/prompt/move.tsx:30-62`, `component/prompt/move.tsx:146-167` — [0.5.1]
- Plugin slot `prompt.footer.status` appends: `/btw` spinner while side questions run (`feature-plugins/prompt/btw.tsx:28-40`), Anysola credit status line (`feature-plugins/system/anysola.tsx:261-300`, Anysola area) — [0.5.1]
- Editor-file label (`prompt.footer.file` slot): `<file><range>` where file = basename (or `parent/index.ext` for `index.*` files), range = `#L` or `#L1-L2` plus ` +N` for extra ranges; middle-truncated to `max(12, min(48, floor(width/3)))`; accent colour while pending, muted once sent — `component/prompt/index.tsx:235-257`, `component/prompt/index.tsx:1981-1994` — [0.5.1] (only when an editor connection exists, see Editor context)
- Right side (`prompt.footer` slot, PromptFooter plugin), normal mode, nothing to report: `<agent.cycle key> agents` (e.g. `shift+tab agents`) when width ≥ 44, then `<palette key> commands` (`ctrl+p commands`); dimmed when the prompt does not own input — `feature-plugins/prompt/footer.tsx:109-130` — [0.5.1]
- Right side when the session has data: `↓ N subagent(s) · N shell(s)` (running children / running shell commands; clickable, opens the subagent picker), `◎ goal…` status, then context usage `<tokens> (<pct>%)` and `$cost` joined with ` · `; shortcut hints drop out when they do not fit (`promptFooterLayout`, budget `width-8` minus `min(28, width/2)`) — `feature-plugins/prompt/footer.tsx:23-108`, `feature-plugins/prompt/footer.tsx:156-166`, `util/session.ts:46-66` — [0.5.1]
- Dollar cost `$0.00` format (`Intl.NumberFormat en-US USD`) only when `session.cost > 0` — `feature-plugins/prompt/footer.tsx:10-13`, `feature-plugins/prompt/footer.tsx:46-49` — [0.5.1] (likely never shown: gateway models carry no $ price; verify live)
- Details (usage/hints) hidden while an interrupt is armed and width < 80 — `component/prompt/index.tsx:1623-1627` — [0.5.1]
- Shell mode footer: `esc exit shell mode` (`esc shell` when width < 44) — `feature-plugins/prompt/footer.tsx:132-139` — [0.5.1]
- Queued prompts dock above the prompt: `N queued · <first queued text>` (newlines → spaces), click opens queued-prompt manager — `routes/session/index.tsx:1581-1583`, `routes/session/index.tsx:2468-2499` — [0.5.1]

### Multiline input, submit, newline, editing keys
- `return` submits (`input.submit`); submit is double-deferred (two `setTimeout 0`) so IME composition flushes — `config/keybind.ts:210`, `component/prompt/index.tsx:1804-1809` — [0.5.1]
- Newline: `shift+return`, `ctrl+return`, `alt+return`, `ctrl+j` — `config/keybind.ts:211` — [0.5.1]
- Textarea editing keys (OpenTUI textarea, defaults): left/ctrl+b, right/ctrl+f, up, down, shift+arrows select, ctrl+a / ctrl+e line home/end, ctrl+shift+a / ctrl+shift+e select to line start/end, alt+a / alt+e visual line home/end, alt+shift+a / alt+shift+e select visual line, shift+home / shift+end select to buffer start/end, ctrl+shift+d delete line, ctrl+k delete to line end, ctrl+u delete to line start, backspace/shift+backspace, ctrl+d/delete/shift+delete delete char, alt+f/alt+right/ctrl+right word forward, alt+b/alt+left/ctrl+left word back, alt+shift+f/alt+shift+right and alt+shift+b/alt+shift+left select word, alt+d/alt+delete/ctrl+delete delete word forward, ctrl+w/ctrl+backspace/alt+backspace delete word back, super+a select all; buffer home/end unbound — `config/keybind.ts:212-245` — [0.5.1]
- In-input undo `ctrl+-` / `super+z`; redo `ctrl+.` / `super+shift+z` — `config/keybind.ts:237-238` — [0.5.1]
- `prompt.submit` command exists with no default key (`none`) — `config/keybind.ts:199`, `component/prompt/index.tsx:432-446` — [0.5.1]
- Double-submit guard: overlapping submits are ignored — `component/prompt/index.tsx:1099-1115` — [0.5.1]
- Typing exactly `exit`, `quit` or `:q` and pressing Enter exits the app — `component/prompt/index.tsx:1130`, `component/prompt/index.tsx:1147-1150` — [0.5.1]
- Empty Enter on home or in shell mode: does nothing (calls `onEmptySubmit`, home passes none) — `component/prompt/index.tsx:1127-1129` — [0.5.1]
- Empty Enter in an existing session: commits the composer's agent and model choice to the session and, if prompts are queued, promotes the first queued prompt to steer now — `component/prompt/index.tsx:1315-1323`, `routes/session/index.tsx:1639-1643` — [0.5.1]
- Default delivery of Enter is "steer" (a prompt sent while the agent runs steers the running turn); `<leader>return` (ctrl+x then Enter) sends with delivery "queue" — `component/prompt/index.tsx:1100`, `component/prompt/index.tsx:447-460`, `config/keybind.ts:200` — [0.5.1]
- Queue refused with warning toast `This prompt cannot be queued` for shell mode, exit words, and slash commands that take arguments; `Editor context cannot be queued` when an unsent editor selection is attached — `component/prompt/index.tsx:1143-1146`, `component/prompt/index.tsx:1162-1165` — [0.5.1]
- Submit clears the composer synchronously before any network await; on failure the snapshot is restored only if the user has not typed something new — `component/prompt/index.tsx:1194-1217` — [0.5.1]
- First submit on home creates the session optimistically (client-minted ID) and navigates to it immediately; text typed meanwhile is moved into the new session's composer draft — `component/prompt/index.tsx:1234-1291`, `component/prompt/index.tsx:1396-1414` — [0.5.1]
- No model selected: warning toast `Connect an integration to send prompts` (3000 ms) and, if no provider is connected, opens the integration dialog — `component/prompt/index.tsx:320-329`, `component/prompt/index.tsx:1168-1172` — [0.5.1]
- Selected model missing in the location: warning toast title `Model unavailable`, message `<providerID>/<modelID> is not available in this session's location` — `component/prompt/index.tsx:1173-1181` — [0.5.1]
- Over-budget session asks before a new turn (`Account.allowTurn`, Anysola dialog); choosing Stop leaves the composer untouched — `component/prompt/index.tsx:1183-1192` — [0.5.1] (dialog itself: Anysola area)
- Pending revert is committed (`revert.commit`) before the new prompt is admitted — `component/prompt/index.tsx:1356-1360` — [0.5.1]
- Error toasts on submit paths: `Failed to set up session` / `Creating a session failed`, `Failed to prepare session`, `Failed to commit revert`, `Failed to send editor context`, `Failed to run shell command`, `Failed to run command`, `Failed to send prompt`; model switch failure message `Failed to switch model: <msg>` — `component/prompt/index.tsx:1271-1275`, `component/prompt/index.tsx:1306-1309`, `component/prompt/index.tsx:1318`, `component/prompt/index.tsx:1329`, `component/prompt/index.tsx:1348`, `component/prompt/index.tsx:1358`, `component/prompt/index.tsx:1372`, `component/prompt/index.tsx:1392` — [0.5.1]
- `ctrl+c` with text: clears the prompt (`prompt.clear`, only enabled while text is non-empty); the cleared text is saved to history first when it is ≥ 20 trimmed chars or has pastes/files/agents — `config/keybind.ts:208`, `component/prompt/index.tsx:957-963`, `component/prompt/index.tsx:1536-1549`, `component/prompt/index.tsx:102` — [0.5.1]
- `ctrl+z` suspends the terminal (textarea-managed) — `config/keybind.ts:288`, `prompt/traits.ts:15` — [0.5.1]
- `tui.prompt.append` server event inserts text into the prompt (for the matching directory) and moves to buffer end — `component/prompt/index.tsx:342-353` — [0.5.1]
- Per-tab in-memory draft: leaving a session (tab switch / remount) keeps the unsent text and cursor; restored on return; not persisted across restarts — `component/prompt/draft-stash.ts:5-20`, `component/prompt/index.tsx:715-739` — [0.5.1]

### Interrupt / background
- `escape` while the session runs: first press arms (shows `esc again to interrupt`), second press within 5 s interrupts (`session.interrupt` with `resume: true`); Esc in shell mode leaves shell mode instead; ignored while autocomplete is open — `config/keybind.ts:128`, `component/prompt/index.tsx:502-533` — [0.5.1]
- `ctrl+b` while running: "Background blocking tools" (moves blocking tool calls such as a long shell to the background) — `config/keybind.ts:129`, `component/prompt/index.tsx:534-550` — [0.5.1]

### Prompt history
- File `<state>/prompt-history.jsonl`, one JSON `PromptInfo` per line (`text`, `files`, `agents`, `skills`, `pasted`, `mode`); keeps the last 50; rewritten on load to drop corrupt lines; an entry identical to the last one is not appended — `prompt/history.tsx:64-147` — [0.5.1]
- `up` (`prompt.history.previous`): if the cursor is not at offset 0 it moves the cursor up a visual row, or to offset 0 when already on the top row; at offset 0 it recalls the previous entry (restores text, mode — shell entries reopen shell mode —, mentions/pastes) with cursor at start — `config/keybind.ts:246`, `component/prompt/index.tsx:1023-1057` — [0.5.1]
- `down` (`prompt.history.next`): cursor down / to end; at the end recalls the next entry, cursor at end; past the newest entry returns an empty prompt — `config/keybind.ts:247`, `component/prompt/index.tsx:1059-1097`, `prompt/history.tsx:110-120` — [0.5.1]
- History navigation only works when the input is empty or still equals the entry being shown; otherwise the key falls through — `prompt/history.tsx:114` — [0.5.1]
- When `down` has nothing to recall it falls through to `session.child.first` (also `down`), opening the subagent picker dock — `component/prompt/index.tsx:1086-1087`, `config/keybind.ts:138`, `routes/session/index.tsx:1386-1394` — [0.5.1] (verify live)
- Disabled while autocomplete is visible (arrows then move the autocomplete selection) — `component/prompt/index.tsx:1029` — [0.5.1]

### Paste handling (text)
- Bracketed paste: bytes decoded, `\r\n` and lone `\r` normalised to `\n`, terminal default insertion suppressed, handled through a serial paste queue that drops the paste if the session, mode or text changed meanwhile — `component/prompt/index.tsx:1810-1833`, `component/prompt/index.tsx:376-395` — [0.5.1]
- Empty bracketed paste (0 bytes, old Windows Terminal image-only clipboard) triggers a clipboard read (`prompt.paste`) — `component/prompt/index.tsx:1821-1826` — [0.5.1]
- ANSI escape sequences are stripped from pasted text — `component/prompt/index.tsx:1460` — [0.5.1]
- Large paste collapse: when the trimmed paste has ≥ 3 lines or > 150 chars and setting `prompt.paste` is not `full`, a virtual placeholder `[Pasted ~N lines]` (N = newline count + 1) is inserted, followed by a space, styled with `extmark.paste` — `component/prompt/index.tsx:1475-1487`, `component/prompt/index.tsx:1419-1444` — [0.5.1]
- Pasting the same text again right after its placeholder (cursor at placeholder end or end+1) expands the placeholder into the full text instead of adding a second one — `component/prompt/index.tsx:1477-1485`, `component/prompt/index.tsx:1446-1457` — [0.5.1]
- Clicking (left mouse down) on a paste placeholder expands it in place — `component/prompt/index.tsx:1850-1863` — [0.5.1]
- Copying text out of the prompt expands placeholders to the real pasted text — `component/prompt/index.tsx:1836-1838`, `prompt/part.ts:4-17` — [0.5.1]
- On submit, placeholders are replaced by the full pasted text — `component/prompt/index.tsx:1131-1141`, `prompt/part.ts:19-35` — [0.5.1]
- Smaller pastes are inserted verbatim — `component/prompt/index.tsx:1490-1496` — [0.5.1]
- Setting: `/settings` → Input → `Large pastes` = `compact` (default) / `full` (config `prompt.paste`) — `component/dialog-config.tsx:270-278`, `config/index.tsx:144-146` — [0.5.1]
- Toggle command `Disable paste summary` / `Enable paste summary` (`app.toggle.paste_summary`): not in the palette and unbound by default, reachable only through a user keybind — `app.tsx:1254-1266`, `config/keybind.ts:56` — [0.5.1]

### Paste handling (files and images)
- A pasted string that is a local path (quotes stripped, `\x` unescaped on non-Windows, `file://` URLs decoded; `http(s)://` excluded) to `.avif .gif .jpeg .jpg .png .svg .webp .pdf` becomes an attachment; up to 32 whitespace/quote-separated paths in one paste, all must resolve; 20 MiB total budget — `component/prompt/local-attachment.ts:5-6`, `component/prompt/local-attachment.ts:30-61`, `component/prompt/local-attachment.ts:73-82`, `component/prompt/local-attachment.ts:93-170` — [0.5.1]
- Image/PDF attachment shows as a virtual chip `[Image N]` / `[PDF N]` (N = highest existing number of that kind + 1; re-pasting the same image reuses its label) followed by a space — `prompt/attachment.ts:15-19`, `prompt/attachment.ts:77-97`, `component/prompt/index.tsx:1499-1534` — [0.5.1]
- SVG file paste is inlined as text with chip `[SVG: <filename>]` — `component/prompt/index.tsx:1464-1469` — [0.5.1]
- `ctrl+v` (`prompt.paste`, `preventDefault: false`): reads the system clipboard preferring `image/png` then `text/plain`; an image becomes `[Image N]` with filename `clipboard` as a base64 `data:` URI; text goes through the text paste path — `config/keybind.ts:209`, `component/prompt/index.tsx:471-494`, `clipboard.ts:22-52` — [0.5.1]
- Clipboard read errors surface as error toasts: `Clipboard read timed out`, `Clipboard content exceeded configured read or image conversion limits` — `clipboard.ts:29-35`, `component/prompt/index.tsx:391-393` — [0.5.1]
- Duplicate identical images (same uri, name, label) are de-duplicated — `prompt/attachment.ts:41-51` — [0.5.1]
- Image previews above the textarea (setting `/settings` → Input → `Image previews`, config `prompt.image_preview`, default off): up to 3 thumbnails, height `max(4, min(8, floor(termHeight/4)))`, width 2×height, `+N more` tile, `No preview` on decode failure; click opens the image viewer — `component/prompt/index.tsx:397-408`, `component/prompt/index.tsx:1713-1780`, `component/dialog-config.tsx:279-288` — [0.5.1]
- Image viewer dialog: title `Image i of n`, ←/→ (`previous` / `next`), footer shows the chip label or `Image i`, `No preview` on error; `<leader>i` (`prompt.images.view`, palette "View image attachments", enabled when images are attached) — `component/dialog-image-preview.tsx:33-82`, `config/keybind.ts:202`, `component/prompt/index.tsx:495-501` — [0.5.1]
- Local file attachments keep their bytes; when the server is local the path is sent as a `file://` source instead — `component/prompt/local-attachment.ts:63-71` — [0.5.1]

### `@` mention autocomplete
- Trigger: the last `@` before the cursor that is at start or preceded by whitespace, with no whitespace after it — `prompt/display.ts:22-31`, `component/prompt/autocomplete.tsx:836-841` — [0.5.1]
- Popup closes when the cursor moves before the trigger or whitespace appears between trigger and cursor — `component/prompt/autocomplete.tsx:814-824` — [0.5.1]
- Sources: skills (`@<id>`, description, right label `skill`), references (`@<name>`, description ` <git repository or path>`, label `reference`), non-primary non-hidden agents (`@<id>`, label `agent`), then files/directories from server `file.find` (limit 20, already fuzzy-ranked server side) — `component/prompt/autocomplete.tsx:424-481`, `component/prompt/autocomplete.tsx:378-405` — [0.5.1]
- Empty query: list = skills, references, agents, files in that order (unbounded, popup scrolls) — `component/prompt/autocomplete.tsx:557-567` — [0.5.1]
- Non-empty query: skills/references/agents filtered with `fuzzysort.go` on keys [display or value minus `#range`, aliases], threshold 0.5, limit 10; score ×2 when the target starts with `@` + query; then files appended unsorted; total capped at 10 — `component/prompt/autocomplete.tsx:569-592` — [0.5.1]
- Frecency: inserting a file mention records `{path, frequency+1, lastOpen}` in `<state>/frecency.jsonl` (max 1000 entries, newest kept); score = `frequency / (1 + daysSinceLastOpen)` — `prompt/frecency.tsx:10-79`, `component/prompt/autocomplete.tsx:258` — [0.5.1]
- Frecency multiplier `score × (1 + frecency)` in ranking — `component/prompt/autocomplete.tsx:586-587` — [dead: only options with `path` get a frecency value, and file options (the only ones with `path`) are never passed through fuzzysort; ranking is unaffected]
- `#<line>` / `#<start>-<end>` suffix on the query: keeps the line range on the inserted file mention (`path#12-20`, URI `?start=12&end=20`); end ignored unless > start — `prompt/parse.ts:1-15`, `component/prompt/autocomplete.tsx:261-291` — [0.5.1]
- Reference alias shortcut: if the text up to the first `/` equals a reference name, only that reference is shown and the file search is skipped — `component/prompt/autocomplete.tsx:295-301`, `component/prompt/autocomplete.tsx:551-553` — [0.5.1]
- Selecting inserts `@<text>` plus a space (unless the next char is already a space) as a styled virtual extmark (`extmark.file` / `extmark.agent` / `extmark.skill`); a file already mentioned is re-pointed, a skill already mentioned is not added twice — `component/prompt/autocomplete.tsx:171-259` — [0.5.1]
- Directory rows: Tab replaces the token with `@<dir>/` and keeps the popup open for the next level; Enter inserts it as a mention — `component/prompt/autocomplete.tsx:657-682`, `component/prompt/autocomplete.tsx:730-743` — [0.5.1]
- Mentions are atomic extmarks that move with edits; deleting part of one removes the mention — `component/prompt/index.tsx:811-881` — [0.5.1]
- Paths are shown relative to the location directory when inside it, else absolute; long rows middle-truncated to popup width − 4 — `component/prompt/autocomplete.tsx:303-313`, `component/prompt/autocomplete.tsx:394` — [0.5.1]

### `/` command autocomplete
- Opens only when `/` is the first character of the input and the token has no whitespace or second `/` — `prompt/display.ts:33-41`, `component/prompt/autocomplete.tsx:829-834`, `component/prompt/autocomplete.tsx:561-563` — [0.5.1]
- Rows: every keymap command with `slash` contributes one row per name and per alias (`/name`, description = command description or title); every server command (custom/MCP/skill commands, core `/goal`, …) contributes `/name` with its server description and is marked queueable — `component/prompt/autocomplete.tsx:491-511` — [0.5.1]
- Sorted by display (`localeCompare`), display padded to the longest name + 2 so descriptions line up — `component/prompt/autocomplete.tsx:513-520` — [0.5.1]
- Filtering: fuzzysort keys [name, description, aliases], no threshold, limit 10, ×2 for names starting with `/` + query — `component/prompt/autocomplete.tsx:569-592` — [0.5.1]
- Selecting a TUI command without arguments removes the `/token` and runs the command immediately; a command with arguments (or any server command) inserts `/name ` for typing arguments — `component/prompt/autocomplete.tsx:483-499`, `component/prompt/autocomplete.tsx:632-643`, `component/prompt/autocomplete.tsx:761-777` — [0.5.1]
- On Enter: `/name args` for a TUI command that takes arguments runs it with the argument string; `/name …` matching a server command sends `session.command` (with files/agents/skills); anything else is a normal prompt — `component/prompt/index.tsx:175-185`, `component/prompt/index.tsx:1151-1159`, `component/prompt/index.tsx:1332-1349` — [0.5.1]
- `<leader>return` while the popup shows a queueable (server) command completes it and queues it — `component/prompt/index.tsx:455`, `component/prompt/autocomplete.tsx:792-796` — [0.5.1]
- Argument autocomplete: only `/cd` has one (directory mode); after `/cd ` lists, for an empty query, recent directories of the project (description `recent`, max 10, `ctrl+d` twice removes one: first press shows `Press ctrl+d to confirm`) then subdirectories (`~`, `~/…`, `/…`, `../` supported; prefix match case-insensitive; dot-dirs hidden unless typed) — `prompt/directory-completion.ts:7-97`, `prompt/directory-recents.ts:12-35`, `component/prompt/index.tsx:1998-2023` — [0.5.1]
- `/cd <dir>`: on home sets where the next session is created; in a session moves the session (`session.move`); toasts `Directory is required`, `Failed to change directory` — `component/prompt/index.tsx:266-316` — [0.5.1]

### Autocomplete popup (shared)
- Positioned above the prompt box, same x and width, z-index 100; height `min(10, rows, rows available above)`; left+right `┃` border (classic design) on `background.raised.high`; rows padded 1 — `component/prompt/autocomplete.tsx:846-851`, `component/prompt/autocomplete.tsx:877-901` — [0.5.1]
- Row: display (base; selected = `text.action.primary.focused` on `background.action.primary.focused`), then ` <description>` (muted, whitespace collapsed), then right-aligned kind label — `component/prompt/autocomplete.tsx:923-1002` — [0.5.1]
- "New designs" experiment variant: per-row left rail `┃`, selected row rail in `text.formfield.focused` — `component/prompt/autocomplete.tsx:1009-1023`, `config/index.tsx:391-394` — [0.5.1] (opt-in experiment, off by default)
- Keys: `up`/`ctrl+p` and `down`/`ctrl+n` move (wrapping), `return` select, `tab` complete (expands directories), `escape` hide (keeps typed text), `ctrl+c` dismiss and delete the `/token`, `ctrl+d` destructive action with confirm; mouse hover moves, click selects; selection resets to 0 on every keystroke — `config/keybind.ts:276-280`, `component/prompt/autocomplete.tsx:595-599`, `component/prompt/autocomplete.tsx:684-752`, `component/prompt/autocomplete.tsx:960-962` — [0.5.1]
- Empty states: `No matching commands`; `Searching…`; `Could not search directories. Keep typing to try again.`; `No matching directories`; `Could not search files. Keep typing to try again.` (error colour); `No matching files, agents, or references` — `component/prompt/autocomplete.tsx:856-868` — [0.5.1]
- While the popup is open the textarea captures escape/navigate/submit/tab (they do not reach other bindings) — `prompt/traits.ts:16-28` — [0.5.1]

### `!` shell mode
- Typing `!` at offset 0 in normal mode with the popup closed switches to shell mode; the `!` is not inserted — `component/prompt/index.tsx:965-990` — [0.5.1]
- Exit shell mode: `escape`; `ctrl+c` when the input is empty; `backspace` at offset 0 — `component/prompt/index.tsx:992-1021` — [0.5.1]
- Look: rail colour `text.action.primary.selected`, metadata shows `Shell`, footer `esc exit shell mode`, textarea status trait `SHELL`, home placeholder `Run a command… "…"` — `component/prompt/index.tsx:1604`, `component/prompt/index.tsx:1607`, `feature-plugins/prompt/footer.tsx:132-139`, `prompt/traits.ts:24` — [0.5.1]
- Enter runs the text through `session.shell` (creates a session first on home), records it in history with `mode: "shell"`, and returns to normal mode — `component/prompt/index.tsx:1325-1331` — [0.5.1]
- Output appears in the transcript as a shell message: `$ <command>` (`cd <workdir> && ` prefix when set), muted output collapsed to 10 lines, click expands; errors `Command cancelled` / `Command timed out` / `Command exited with code N`; running shells count in the footer `N shell(s)` — `routes/session/index.tsx:2322-2339`, `routes/session/index.tsx:2968-3140`, `feature-plugins/prompt/footer.tsx:30-36` — [0.5.1] (rendering details: session section)
- Shell mode skips model/budget checks — `component/prompt/index.tsx:1173`, `component/prompt/index.tsx:1185-1192` — [0.5.1]

### External editor
- `/editor` or `<leader>e` (`ctrl+x e`), palette "Open editor": opens `$VISUAL` or `$EDITOR` (split on spaces; Windows via shell) on a temp file `<os.tmpdir()>/<Date.now()>.md` containing the prompt with pastes expanded; renderer suspended while the editor runs; cwd = project directory (not `/`) else location directory else launch cwd — `config/keybind.ts:92`, `component/prompt/index.tsx:551-583`, `editor.ts:14-39` — [0.5.1]
- Result: file content replaces the prompt; a single trailing newline is dropped for one-line text; mentions are re-aligned by searching their text; paste placeholders are dropped (text is already expanded); empty file → no change; temp file deleted — `component/prompt/index.tsx:571-582`, `prompt/content.ts:1-13`, `prompt/mention.ts:20-83` — [0.5.1]
- Errors: toast title `Failed to open editor`, message `Set $VISUAL or $EDITOR to open an external editor` or `Editor exited with code N` / `Editor exited with signal S` — `editor.ts:16`, `editor.ts:31`, `component/prompt/index.tsx:568-570` — [0.5.1]

### Editor context (IDE / Zed selection)
- Connection: env `CLAUDE_CODE_SSE_PORT` or `OPENCODE_EDITOR_SSE_PORT` → `ws://127.0.0.1:<port>`; else the best `~/.claude/ide/<port>.lock` whose `workspaceFolders` contain the cwd (longest folder, then newest mtime; `transport` must be `ws`; `authToken` sent as `x-claude-code-ide-authorization`) — `context/editor.ts:117-121`, `context/editor.ts:361-374`, `editor.ts:41-81`, `context/editor.ts:390-398` — [0.5.1]
- Protocol: JSON-RPC 2.0 over WebSocket; sends `initialize` (`protocolVersion "2025-11-25"`, `clientInfo {name: "opencode", version: "0.0.0"}`) then `notifications/initialized`; handles `selection_changed` (selection) and `at_mentioned` (`{filePath, lineStart, lineEnd}` → inserts a file mention `@path#a-b` into the prompt); reconnect backoff 1 s doubling to 10 s — `context/editor.ts:9`, `context/editor.ts:217-282`, `component/prompt/autocomplete.tsx:315-327`, `component/prompt/autocomplete.tsx:779-786` — [0.5.1]
- Zed: when `ZED_TERM=true` or `TERM_PROGRAM=zed` and no socket, polls Zed's SQLite DB every 1 s (`$OPENCODE_ZED_DB`, `~/Library/Application Support/Zed/db/0-stable/db.sqlite`, `~/.local/share/zed/db/0-stable/db.sqlite`), picks the active editor of the workspace containing the cwd, reads selections (byte offsets → line/char, 1-based) and buffer contents (DB `contents` or the file) — `context/editor.ts:121`, `context/editor.ts:177-211`, `editor-zed.ts:41-277` — [0.5.1]
- Context sent before the prompt as a hidden synthetic message: `<system-reminder>Note: The user opened the file "<path>". This may or may not be relevant to the current task.</system-reminder>` (no range) or `<system-reminder>Note: The user selected [Selection i: ]#L1-L2 from "<path>". ```<text>```\n\n … This may or may not be relevant to the current task.</system-reminder>`; marked sent afterwards (label turns muted) — `component/prompt/index.tsx:162-173`, `component/prompt/index.tsx:1361-1373`, `component/prompt/index.tsx:1393` — [0.5.1]
- "Remove editor context" palette command (enabled when context exists) dismisses the current selection — `component/prompt/index.tsx:461-470`, `config/keybind.ts:201` — [0.5.1]
- Setting `/settings` → Input → `Editor context` on/off (config `prompt.editor`, default on); toggle command `Disable file context`/`Enable file context` not in palette, unbound — `component/dialog-config.tsx:260-269`, `app.tsx:1223-1235`, `config/keybind.ts:54` — [0.5.1]

### Agent switch
- `shift+tab` (`agent.cycle`) cycles forward through primary agents (mode ≠ `subagent`, not hidden), wrapping; `agent.cycle.reverse` unbound by default — `config/keybind.ts:176-177`, `app.tsx:963-970`, `app.tsx:997-1004`, `context/local.tsx:59-60`, `context/local.tsx:117-126` — [0.5.1]
- Plain `tab` in the prompt is captured and does nothing (no agent binding) unless the popup is open (complete) — `prompt/traits.ts:16-22` — [0.5.1]
- `/agents` or `<leader>a` opens the agent picker — `app.tsx:945-952`, `config/keybind.ts:175` — [0.5.1]
- Unknown agent id → warning toast `Agent not found: <id>` (3000 ms) — `context/local.tsx:100-105` — [0.5.1]
- Agent choice is a per-session draft until the next submit commits it (`switchAgent`) — `context/local.tsx:107-115`, `component/prompt/index.tsx:1294-1302` — [0.5.1]

### Model and variant cycling
- `f2` / `shift+f2`: next / previous recently used model (recent list max 10, most recent first, stored in `<state>/model.json` `recent`) — `config/keybind.ts:168-169`, `app.tsx:908-925`, `context/local.tsx:415-426`, `model-preference.ts:49-60`, `context/local.tsx:157` — [0.5.1]
- Favorite cycling (`model.cycle_favorite[_reverse]`) unbound by default; with no favorites toast `Add a favorite model to use this shortcut` — `config/keybind.ts:170-171`, `context/local.tsx:427-454` — [0.5.1]
- `<leader>m` / `/models` opens the model picker — `config/keybind.ts:167`, `app.tsx:898-906` — [0.5.1]
- `ctrl+t` (`variant.cycle`, palette "Variant cycle"): default → first named variant → … → last → back to default — `config/keybind.ts:178`, `app.tsx:971-978`, `model-preference.ts:67-75` — [0.5.1]
- `/variants` (aliases `/thinking`, `/effort`): dialog `Select variant` with `Default` plus variant ids; current marked; with no variants toast title `No variants available`, message `The current model does not support any variants.`; palette entry hidden when the model has no variants — `app.tsx:979-996`, `component/dialog-variant.tsx:7-43` — [0.5.1]
- Variant is remembered per model in `model.json` `variant` map (`default` stored explicitly) and per session draft — `context/local.tsx:487-496`, `model-preference.ts:174-181` — [0.5.1]
- Model choice is committed to the session (`switchModel`) on submit, not on selection — `component/prompt/index.tsx:1303-1314` — [0.5.1]

### Stash
- Palette commands `Stash prompt` (enabled with text; saves and clears), `Stash pop` (restores the newest entry), `Stash list` (dialog); all unbound by default — `component/prompt/index.tsx:883-947`, `config/keybind.ts:204-206` — [0.5.1]
- Stored in `<state>/prompt-stash.jsonl` as `{prompt, timestamp}` lines, max 50 — `prompt/stash.tsx:9-93` — [0.5.1]
- Dialog title `Stash`, newest first; row title = first line trimmed and truncated to 50; description = relative time `just now` / `Nm ago` / `Nh ago` / `Nd ago` / locale datetime after 7 days; footer `~N lines` for multi-line; Enter restores and removes; `ctrl+d` (`stash.delete`) shows `Press ctrl+d again to confirm`, second press deletes; action label `delete` — `component/dialog-stash.tsx:9-90`, `config/keybind.ts:163` — [0.5.1]

### Undo / redo
- Textarea undo/redo keys: see "Multiline input" — `config/keybind.ts:237-238` — [0.5.1]
- `/undo` / `<leader>u` (session): reverts to before the last user message and appends that message's text (with images read back; missing ones dropped with warning `An image in this message is no longer stored, so it can't be restored` / `N images in this message are no longer stored, so they can't be restored`) to the composer after a blank line; `Nothing to undo` error toast — `routes/session/index.tsx:1106-1131`, `component/prompt/draft-stash.ts:37-47`, `prompt/history.tsx:36-62`, `prompt/restore.ts:14-35` — [0.5.1]
- `/redo` / `<leader>r` clears the staged revert; enabled only while a revert is staged — `routes/session/index.tsx:1133-1148`, `config/keybind.ts:195-196` — [0.5.1]
- Undoing a queued prompt while in shell mode with text: error toast `Leave shell mode before undoing a queued prompt` — `routes/session/index.tsx:809-810` — [0.5.1]

### Skills
- `/skills` (palette "Skills"): dialog `Skills` listing name (padded) + description; selecting inserts `@<skill>` + space as a skill mention; empty/loading/error texts `No skills available`, `Loading skills…`, `Could not load skills` + error + `Close and reopen Skills to try again.`, filter miss `No skills found` — `component/prompt/index.tsx:584-622`, `component/dialog-skill.tsx:40-93` — [0.5.1]

### Workspaces
- `/worktrees` (palette "Manage workspaces"): opens the workspaces dialog; choosing a new worktree before the first prompt defers creation to submit; toasts `Unable to determine current project`, `Creating workspace failed` — `component/prompt/index.tsx:623-632`, `component/prompt/move.tsx:12-181` — [0.5.1] (dialog contents: dialogs section)

### Composer dock (Subagents / Shell / Terminals tabs)
- A dock that replaces the prompt in a session; opened by `down` (`session.child.first`, "Toggle subagent picker") or by clicking the footer `↓ N subagents` item; always open (Subagents tab) in a child session — `routes/session/index.tsx:1386-1394`, `routes/session/index.tsx:1584-1601`, `feature-plugins/prompt/footer.tsx:73-91` — [0.5.1]
- Header: tab labels (active bold, others muted, gap 2) or the single label in bold; `esc` at right (click closes); footer hints `<label> <key>` and `tabs ←/→` when > 1 tab; left/right switch tabs, `escape`/`ctrl+c` close (in a child session closing navigates to the parent) — `routes/session/composer/index.tsx:21-157`, `routes/session/index.tsx:1589-1596` — [0.5.1]
- Subagents tab: family tree rows `<tree prefix><Agent>: <title>` with `Running` at right; filter active/inactive toggled by `ctrl+a` (hint `show inactive` / `show active`), `ctrl+d` interrupts (hint `interrupt`), Enter navigates, up at top closes; empty `No active subagents` / `No inactive subagents`; max 5 rows visible — `routes/session/composer/subagents-tab.tsx:26-259`, `config/keybind.ts:249-252` — [0.5.1]
- Shell tab: running shell commands of the session (first line of command); Enter `output` opens the shell output dialog, `ctrl+d` `kill`; empty ` No shell commands` — `routes/session/composer/shell-tab.tsx:12-149`, `config/keybind.ts:253-256` — [0.5.1]
- Terminals tab: shown only when the server offers persistent terminals; rows = terminal foreground process or title, plus `+ New terminal`; up/k, down/j, Enter selects/creates; error toast `Unable to load terminal` — `routes/session/composer/terminals-tab.tsx:9-114`, `component/session-frame.tsx:294`, `context/session-terminals.tsx:72-73`, `config/keybind.ts:257-259` — [0.5.1]

### /btw side question
- `/btw [question]` (palette "Ask a side question", description `One-shot answer from the session's context without adding to the conversation`, unbound): in a session sends a one-shot `session.generate` with fixed instructions (no tools) plus the question; without argument asks via prompt dialog titled `/btw` with placeholder `Ask anything`; outside a session toast `Open a session first` — `feature-plugins/prompt/btw.tsx:17-86`, `config/keybind.ts:131` — [0.5.1]
- While pending a spinner labelled `/btw` shows in the footer status — `feature-plugins/prompt/btw.tsx:28-40` — [0.5.1]
- Answer dialog (large, centered): header `/btw` + question (muted, one line), markdown answer in a scroll box (max 60% height; ↑/↓/j/k, pgup/pgdn, home/end), `c` copies (`copy response` → `✓ copied`) — `feature-plugins/prompt/btw.tsx:88-189` — [0.5.1]

### Goal
- Footer status `◎ goal` (muted) / `◎ goal paused` (warning) / `◎ goal met` (success) when the session has a goal set by the server `/goal` command (core goal plugin, description `keep working until an objective is met [pause|resume|clear]`); hover brightens; click opens dialog `Goal`: objective, `◎ Active|Paused|Met`, reason, actions `pause` (p), `resume` (r), `clear` (x) that send `/goal pause|resume|clear`; error toast `Failed to update the goal` — `feature-plugins/prompt/goal.tsx:12-171`, `feature-plugins/prompt/footer.tsx:93-100`, `packages/core/src/plugin/goal.ts:344-346` — [0.5.1]
- Toast `No goal is set. Start one with /goal <objective>.` — `feature-plugins/prompt/goal.tsx:94-97` — [dead: the clickable status only renders when a goal exists, so this branch is unreachable from the UI]

### Prompt part highlighting
- Mentions and chips are virtual extmarks styled by syntax-theme scopes `extmark.file`, `extmark.agent`, `extmark.skill`, `extmark.paste` (paste style also used for image/PDF/SVG chips) — `component/prompt/index.tsx:335-338` — [0.5.1]

## 4. Dialogs (checklist; exact texts in `verbatim/tui/dialogs.md`, commands in `slash-commands.md`, keys in `keybinds-default.md`)

### Shared frame
- [ ] Overlay backdrop `RGBA(0,0,0,150)`; panel top-aligned at height/4, centered; widths 60/88/116, max width terminal−2; header bold title + muted `esc` (clickable); backdrop click closes all — `ui/dialog.tsx:24-171` — [0.5.1]
- [ ] `DialogSelect`: `Search` input, fuzzysort over title/category/searchText (score title×2), bold accent category headers, `●` current marker, focused row highlight, right-aligned footer column, list height `min(rows, floor(h/2)−6)`, wrap-around navigation, `No items available` / `No results found` — `ui/dialog-select.tsx:201-967` — [0.5.1]
- [ ] Footer actions rendered `<title> <key>` (e.g. `delete ctrl+d  rename ctrl+r`, `connect space`, `change ←/→`) — `ui/dialog.tsx:209-304`; captured in screens/sessions-empty.txt, mcps.txt, settings.txt — [0.5.1]
- [ ] `DialogConfirm` / `DialogAlert` / `DialogPrompt` generic dialogs (`Enter text`, `Working…`) — `ui/dialog-confirm.tsx`, `ui/dialog-alert.tsx`, `ui/dialog-prompt.tsx` — [0.5.1]

### Per dialog (all [0.5.1] unless marked)
- [ ] Command palette `Commands` (ctrl+p): `Suggested` group first, then categories (Session, Agent, Integration, Prompt, VCS, Anysola, System, Plugins …), footer = all shortcuts joined `, ` (e.g. `ctrl+c, ctrl+d, ctrl+x q`); non-empty query also lists every Settings entry — `component/command-palette.tsx:1-74`; screens/command-palette*.txt — [0.5.1]
- [ ] Model picker `Select model`: group `Anysola`; row `<name> <Anysola note>` and right `~6 credits/turn` / `~1 credit/turn`; footer `Connect an integration ctrl+a  Favorite ctrl+f`; Favorites/Recent sections; selecting a model with variants may open `Select variant` — `component/dialog-model.tsx:86-95,201`; screens/model-picker.txt — [0.5.1]
- [ ] `Select variant` (`/variants`, `/thinking`, `/effort`; `ctrl+t` cycles): `Default` + variant ids; toast `No variants available` / `The current model does not support any variants.` — `component/dialog-variant.tsx`, `app.tsx:973-996` — [0.5.1]
- [ ] `Select agent` (`/agents`, `ctrl+x a`): `build`, `plan` with descriptions — `component/dialog-agent.tsx`; screens/agent-picker.txt — [0.5.1]
- [ ] Session list `Sessions for <project>` (`/sessions`, `/resume`, `/continue`, `ctrl+x l`): `Today`/date groups, `!`/`?`/spinner gutter, `delete ctrl+d` (press twice: `Press ctrl+d again to confirm`), `rename ctrl+r`, `all projects ctrl+a` toggle, server search debounced 150 ms, limit 50 — `component/dialog-session-list.tsx`; screens/session-list.txt — [0.5.1]
- [ ] `Rename session` (`/rename [title]`, ctrl+r in session; empty argument regenerates title) — `component/dialog-session-rename.tsx`, `routes/session/index.tsx` — [0.5.1]
- [ ] `Fork session` (`/fork`; toast `Forked session`), `Timeline` (`/timeline`, `ctrl+x g`; rows `<user prompt>  <time>`), message actions dialog (click on a user message: revert/copy/fork) — `routes/session/dialog-fork.tsx`, `dialog-timeline.tsx`, `dialog-message.tsx`; screens/timeline.txt — [0.5.1]
- [ ] `Themes` (`/themes`): live preview while moving, Esc reverts; 35 entries (see section 6) — `component/dialog-theme-list.tsx`; screens/theme-picker.txt — [0.5.1]
- [ ] `Settings` (`/settings`): 30+ settings across Appearance/Session/Tabs/Diffs/Input/Debug with values cycled by ←/→/Enter, written to `cli.json` (full list in dialogs.md §8) — `component/dialog-config.tsx`; screens/settings.txt — [0.5.1]
- [ ] `Status` (`/status`, `ctrl+x s`): `No MCP servers` or `<n> MCP server(s)` + one `•` line per server (`Connected` / error / `Disabled in configuration` / `Needs authentication: …`) — `component/dialog-status.tsx`; screens/status-in-session.txt — [0.5.1]
- [ ] `MCP servers` (`/mcps`): list with toggle `connect space`, auth/error details — `component/dialog-mcp.tsx`, `component/dialog-error-details.tsx`; screens/mcps.txt — [0.5.1]
- [ ] `Help` (`/help`): `Press ctrl+p to see all available actions and commands in any context.` + `ok`, close `esc/enter` — `ui/dialog-help.tsx`; screens/help.txt — [0.5.1]
- [ ] `Debug` (`/debug`): Version/Date/OS/Terminal/Session ID/Model, `Share this when reporting an issue.`, `copy enter` (shows upstream `2.2.5 (latest)`) — `component/dialog-debug.tsx`; screens/debug.txt — [0.5.1]
- [ ] Export (`/export`, `ctrl+x x`): `Export session`: `Export as:` `◉ Markdown`/`○ JSON`, `[x] Include thinking`, `[x] Include tools` (markdown) or `[ ] Sanitize sensitive data` (JSON), buttons `Copy` / `Export`; result `Session exported` + path + `Close` — `ui/dialog-export-options.tsx`, `ui/dialog-export-result.tsx` — [0.5.1]
- [ ] Stash list (palette only), Skills (`/skills`, list of discovered skills with descriptions), Open (`/open`, `/projects`, `/project`, `ctrl+o`: `Search sessions and projects…`, sections Sessions/Projects with `<project> · 5h`), Worktrees (`/worktrees`: `Current`, `new ctrl+a  delete ctrl+d  refresh ctrl+r`), Plugins (`/plugins`: `show internal ctrl+a`), image preview (`ctrl+x i`), shell output dialog — `component/dialog-stash.tsx`, `dialog-skill.tsx`, `dialog-open.tsx`, `dialog-workspaces.tsx`, `feature-plugins/system/plugins.tsx`, `dialog-image-preview.tsx`, `dialog-shell-output.tsx`; screens/skills.txt, open.txt, worktrees.txt, plugins.txt — [0.5.1]
- [ ] `Connect an integration` (`/connect`): `Services` group, `✓ Anysola  $ANYSOLA_TOKEN` when signed in by env; only Anysola is listed under the provider lock — `component/dialog-integration.tsx`; screens/connect.txt — [0.5.1]
- [ ] `Update` (`/update`): `Anysola Code is already up to date.` + `Skip`; titles `Update` / `Update available` / `Updating Anysola Code`, buttons `Skip` + `Update`/`Restart`; a newer `GET /v1/code/version` shows the home notice `/update to install v<version>` under the logo — `component/dialog-update.tsx`, `context/update-notification.tsx`; screens/update.txt — [0.5.1]
- [ ] Permission prompt and question/form UI — see section 3 (`routes/session/permission.tsx`, `routes/session/form.tsx`) — [0.5.1]
- [ ] `/btw` answer panel (`One-shot answer from the session's context without adding …`) and `/goal` (`keep working until an objective is met [pause|resume|clear…`) — `feature-plugins/prompt/btw.tsx`, `feature-plugins/prompt/goal.tsx` — [0.5.1]
- [ ] Experiments dialog — `component/dialog-experiments.tsx` — [0.5.1, hidden: only via the DevTools bar, off by default]
- [ ] Highlights `What's new in <version>` — `component/dialog-highlights.tsx` — [dead: highlights list is always empty since 8ee8951f4d]
- [ ] OpenCode Go upsell — `component/dialog-go-upsell.tsx`, `go-upsell-art.ts` — [dead: needs `FreeUsageLimitError` from OpenCode Console; the Anysola gateway never sends it]
- [ ] `Free models` — `component/dialog-model-free.tsx` — [dead: only under the off-by-default `new-designs` experiment with nothing connected]
- [ ] Console usage dialog — `component/usage.tsx` `DialogUsage` — [dead: model-picker `Usage ctrl+u` action hidden unless OpenCode Console limits load]
- [ ] Session share/unshare — [dead: removed upstream; deprecated keybind ids kept as no-ops]

### Home route, tabs, keymap
- [ ] Home layout and recent sessions — section 0 and `routes/home.tsx`, `anysola/home.tsx` — [0.5.1]
- [ ] Home footer: MCP count `⊙ N` (click → `/mcps`), `N plugin(s) failed` (click → `/plugins`), `home.footer.status` slot (Anysola plan + banners), version right; hidden when height < 14 or width < 44 — `feature-plugins/home/footer.tsx:16-117` — [0.5.1]
- [ ] Session tabs: strip with titles, `+ New session`, `!` permission / `?` question / spinner indicators, unread tracking, close (`ctrl+x w`), reopen (`ctrl+shift+t`), next/prev, quick slots 1–9, persisted per cwd in `<state>/latest/tui/tabs.json`; vertical layout option — `component/session-tabs.tsx`, `context/session-tabs*.ts(x)` — [0.5.1]
- [ ] Leader key `ctrl+x`, timeout 2000 ms; key display `ctrl+x m` (leader expanded); full defaults in keybinds-default.md — `config/keybind.ts`, `context/keymap.tsx:140-147` — [0.5.1]

## 5. Anysola UI

All Anysola account UI lives in the TUI plugin `anysola.account` (`feature-plugins/system/anysola.tsx`) plus the shared store
`anysola/store.ts`, formatting `anysola/format.ts` (shared with `anysola usage`/`anysola devices` CLI), `anysola/home.tsx`,
`anysola/wordmark.ts`, `component/logo.tsx`, `util/presentation.ts`. Data comes from the server's `anysola.account` RPC
(`packages/core/src/plugin/provider/anysola-account.ts:135-138`: `GET /v1/code/me`, `GET /v1/code/usage?days=7|30`,
`GET /v1/code/tokens`, `POST /v1/code/tokens/<id>/revoke`, plus `session`, `budget`, `budgetDefault`, `estimates`, `notes`
and a `credits` event stream). The gateway protocol itself is in the gateway spec.

### State and refresh
- [ ] Account state: `me` (plan, access paid/free, credits.available/renewsAt, freeQuota, event{label,until}, usageToday, lowBalance), per-session credits (`turn`, `total`, `turns`, `budget`, `remaining`), per-model `estimates` and `notes` — `anysola/store.ts:11-77` — [0.5.1]
- [ ] `me` refreshed on app start, on `server.connected`, every 5 min, and 1.5 s (debounced) after every `credits` event; `estimates`+`notes` on start and reconnect — `feature-plugins/system/anysola.tsx:202-248` — [0.5.1]
- [ ] Balance shown = the session's latest `remaining` (gateway header) else any session's `remaining`, else `me.credits.available` — `anysola/store.ts:50-57` — [0.5.1]
- [ ] Low balance = not free AND (`me.lowBalance` OR remaining < 100) (`LOW_BALANCE = 100`) — `anysola/store.ts:22,72-76` — [0.5.1]
- [ ] Every RPC failure becomes inline text, never the crash screen: `Account details are unavailable (<detail>). If you just updated, run \`anysola service restart\`.`, `Unexpected reply from the Anysola Code server`; HTTP 404 → `This server does not offer it yet.`; other → server message or `Request failed (HTTP <n>)`; status 0 → `Cannot reach Anysola` — `feature-plugins/system/anysola.tsx:26-31,62-105` — [0.5.1]

### Status line (composer footer, slot `prompt.footer.status`)
- [ ] Text `"<balance> credits"` + `" · this turn <n>"` (only when turn > 0) + `" · session <total>"` or `" · session <total>/<budget>"` (when total > 0 or a budget is set); numbers `Math.round` + en-US grouping (`1,234`) — `anysola/store.ts:79-96`, `anysola/format.ts:5-7` — [0.5.1]
- [ ] Free accounts with a quota: `today <usedTurns>/<dailyTurns>` instead — `anysola/store.ts:83-86` — [0.5.1]
- [ ] Hidden when no balance is known; muted color, warning color when low or over budget; hover appends ` · renews <D Mon>` (or the free quota reset date); click opens `/credits` — `feature-plugins/system/anysola.tsx:261-299` — [0.5.1]
- [ ] Reopening a session loads its stored total and budget from the server before it spends (captured: `731 credits · session 12`) — `feature-plugins/system/anysola.tsx:267-269`; screens/status-in-session.txt — [0.5.1]
- [ ] Footer row order (captured): `~/dir:branch` left; right side `<status line>  <context tokens> (<pct>%)  ctrl+p commands` in a session, `<status line>  shift+tab agents  ctrl+p commands` at home, `<status line>  esc exit shell mode` in shell mode — screens/*.txt, `feature-plugins/prompt/footer.tsx` — [0.5.1]

### Banners
- [ ] Event banner (info color): `<event.label>` + ` · until <Mon D, HH:MM AM>` when `until` is set — `feature-plugins/system/anysola.tsx:306-313` — [0.5.1]
- [ ] Low-balance banner (warning color): `Low balance: <n> credits left · /credits` — `feature-plugins/system/anysola.tsx:314-320` — [0.5.1]
- [ ] Placement: above the composer in a session (slot `session.composer.top`); on home in the bottom footer row after the plan line (captured: `Team · renews 23 Nov  Double credits weekend · until Nov 10, 12:00 AM  Low balance: 64 credits left · /credits    0.5.`) — `feature-plugins/system/anysola.tsx:343-352`; screens/home-low-balance-event-banner.txt — [0.5.1]
- [ ] Home plan line (faint, click opens `/credits`): `<Plan>` or `<Plan> · renews <D Mon>` (`team` → `Team`; date as `23 Nov`, year added when not this year) — `feature-plugins/system/anysola.tsx:325-342`, `anysola/format.ts:18-32` — [0.5.1]
- [ ] Date formats: `formatDate` = en-US `Nov 23, 03:00 AM` (local time zone); `formatShortDate` = `23 Nov` / `5 Jan 2027`; `formatAgo` = `now`, `40m ago`, `5h ago`, `2d ago`, then short date — `anysola/format.ts:10-42` — [0.5.1]

### Commands (group `Anysola`, all in palette and `/`)
- [ ] `/credits` "Credits and plan" → dialog `Credits`: label column width 14, rows `Plan <Plan>[ (free)]`, `Balance <n> credits`, `Renews <D Mon>`, `Free quota <u>/<d> turns today`, `Resets <date>`, `Today <r> requests · <c> credits`, `This session <t> turns · <c> credits`, `Event <label>[ until <date>]`; then `Upgrade or top up: https://account.anysola.com/billing` (warning color when low); while loading `Account  Loading…` or the error — `feature-plugins/system/anysola.tsx:362,415-462`; screens/credits.txt — [0.5.1]
- [ ] `/usage` "Usage" → dialog `Usage · last 7 days` / `Usage · last 30 days`; `t` toggles (footer action `30 days t` / `7 days t`); body = `usageLines`: `Totals` table (`requests`, `credits`, row `all`), `  Cache hit rate <p>%` when present, `By day` (`date`), `By model` (`model`), `Balance <n> credits · renews <date>`; tables left-align column 1, right-align numbers, 3-space gaps, 2-space indent; indented lines base color, headings muted — `feature-plugins/system/anysola.tsx:372,464-497`, `anysola/format.ts:44-82`; screens/usage-7d.txt, usage-30d.txt — [0.5.1]
- [ ] `/budget` "Session budget" (takes arguments): no arg → toast `Session budget <spent>/<b> credits` or `No budget · <spent> credits spent. Set one: /budget <credits>`; `/budget <n>` (integer ≥ 1) → success toast `Session budget <spent>/<n> credits`; `/budget off` → `Budget removed`; `/budget default` → `No default budget` or `New sessions start with a <n> credit budget`; `/budget default <n>|off` → same / `Default budget removed`; bad input → warning `Usage: /budget <credits> | off | default <credits>` or `Usage: /budget default <credits> | off`; no session → warning `Open a session first, or set one for new sessions: /budget default <credits>`; failure → error `Budget unavailable (<detail>). If you just updated, run \`anysola service restart\`.` — `feature-plugins/system/anysola.tsx:153-200,379-384`; screens/budget-set-toast.txt, budget-toast-home.txt — [0.5.1]
- [ ] Budget gate: before a new turn in a session whose total ≥ budget, a select dialog `Budget reached · <total>/<budget> credits` with `Continue once` (Ask again before the next prompt), `Add <budget> credits` (Budget <raised>, raised = max(budget,total)+budget), `Remove the budget`, `Stop` (Keep the prompt in the composer); a running turn is never cut; a failing gate never blocks — `feature-plugins/system/anysola.tsx:131-151`, `anysola/store.ts:67-71`, `component/prompt/index.tsx:1189-1190` — [0.5.1]
- [ ] `/devices` "Signed-in devices" → `DialogSelect` `Devices`: row `<name|Unnamed device>[ (this device)]`, description = client, footer `used <date>` / `never used`; selecting the current device → toast `This device: sign out with \`anysola logout\`.`; others → confirm `Revoke device` / `Sign out "<title>"? It will need to log in again.` / button `Revoke` → toast `Device signed out` (success) or the error, then the list reopens; loading `Loading…` / error text in a `Devices` dialog — `feature-plugins/system/anysola.tsx:386-391,499-553`; screens/devices.txt — [0.5.1]
- [ ] `/account` "Open your Anysola account" → opens `https://account.anysola.com` in the browser and toasts the URL (or `Open <url>` when no browser could be opened) — `feature-plugins/system/anysola.tsx:16,254-258,393-399` — [0.5.1]
- [ ] `/feedback` "Send feedback or report a problem" → same with `https://www.anysola.com/contact` — `feature-plugins/system/anysola.tsx:18,401-407` — [0.5.1]

### Model picker additions
- [ ] Row description = the model's Anysola note from the remote config (`Picks Haiku or Sonnet per turn`, `Default`, `Fastest`), footer `~<n> credits/turn` (singular `credit` for 1) from `credits_per_turn_estimate` — `component/dialog-model.tsx:86-95,201`; screens/model-picker.txt — [0.5.1]

### Wordmark, version, exit summary
- [ ] Wordmark: 3 rows of quadrant blocks (exact text in `verbatim/tui/wordmark.md`), bold, base text color, unselectable, `Code` muted right-aligned below; A-symbol + `Anysola Code` when width < 64 (wordmark width 60 + 4); nothing when height < 12 — `anysola/wordmark.ts:5-15`, `component/logo.tsx:9-31` — [0.5.1]
- [ ] Version: home footer right shows `ANYSOLA_VERSION` (`0.5.1`) — `feature-plugins/home/footer.tsx:98-102` — [0.5.1]
- [ ] Exit summary (printed after leaving the alternate screen, only when a session was open or an update was installed): wordmark indented 2 (bold, `\x1b[1m`), `Code` dimmed (`\x1b[90m`) under its right edge, blank line, optional `  Run anysola to use <version>` + blank, then rows `  <label padded to 10, dim><value bold>`: `Session   <title>`, `Spent     <n> turn(s) · <c> credits` (only when the session spent > 0), `Continue  anysola -s <sessionID>`, blank — `util/presentation.ts:4-35`, `anysola/store.ts:98-103`; screens/exit-summary.txt — [0.5.1]
- [ ] Recent sessions under the composer on home: up to 3 top-level sessions of this directory/project, newest first, title (truncated to fit, timestamped fallback for untitled) + `formatAgo` time; hover brightens, click opens; hidden when height < 24 or none or the read fails; width ≤ 75 — `anysola/home.tsx:6-95` — [0.5.1]
- [ ] Sidebar "Getting started" card when nothing is connected: `Getting started`, `Sign in with your Anysola account to start.`, `Sign in` … `/connect` — `feature-plugins/sidebar/footer.tsx:9-72` — [0.5.1]
- [ ] No in-TUI device-flow screen: sign-in runs in the CLI before the TUI starts (`packages/cli/src/commands/handlers/anysola/account.ts:1-3`); inside the TUI `/connect` → Anysola uses the integration's OAuth method — [0.5.1] (CLI spec)

## 6. Themes and 7. Other UX (checklist)

Paths: `packages/<pkg>/src/...` under `/home/ubuntu/anysola-code/build/fork/`. OpenTUI = `node_modules/.bun/@opentui+core@0.8.0+eff4d38d6a4a2cd9/node_modules/@opentui/core` (JS) plus `libopentui.so` (native Zig, not readable as source). Verbatim: `verbatim/tui/theme-anysola.json`, `theme-anysola.md`, `themes-bundled.md`.

### Themes

- Default theme name is `anysola` when config `theme.name` is unset; unknown names fall back to `anysola` — `packages/tui/src/context/theme.tsx:113-118,139,300` — [0.5.1]
- `anysola` is a v2 document ("Grafit"): neutral = accent = `grafit` hue, interactive = `sand` hue; dark bg `#110f0d`, text `#e8e4df`, muted `#aba39b`, border `#2e2b27`, dialogs on `#1a1815`; light bg `#fbfaf8`, text `#24211e`. Full token table in `theme-anysola.md` — `packages/tui/src/theme/assets/v2/anysola.json:1-415` — [0.5.1]
- Theme load failure: toast variant `error`, title `Failed to load theme: <name>`, message = error text (decode errors: `Invalid theme: <name> <value> is an invalid value`); active theme switches to `opencode` — `packages/tui/src/component/theme-error-toast.tsx:5-19`, `packages/tui/src/context/theme.tsx:301-313`, `packages/theme/src/tui/resolve.ts:39-43` — [0.5.1]
- `/themes` (command `theme.switch`, "Switch theme", category System, key none) opens dialog titled `Themes`, search in the title row with placeholder `Search themes`, 35 entries sorted case-insensitively (list in `themes-bundled.md`), current theme marked — `packages/tui/src/app.tsx:1106-1114`, `packages/tui/src/component/dialog-theme-list.tsx:7-55` — [0.5.1]
- Live preview: moving the highlight applies that theme; filtering previews the first match; empty filter restores the original; Esc/close without Enter restores the original; Enter keeps it (from Settings it returns to the Settings dialog at the Theme row) — `packages/tui/src/component/dialog-theme-list.tsx:20-52`, `packages/tui/src/component/dialog-config.tsx:28-38` — [0.5.1]
- Every theme change (including preview moves) writes `theme.name` to the TUI config file — `packages/tui/src/context/theme.tsx:345-355` — [0.5.1]
- Bundled set: 32 v1 themes + `opencode` (v2) + `anysola` (v2) + `system` (generated) — `packages/tui/src/theme/index.ts:50-62`, `packages/tui/src/theme/v1.ts:37-70` — [0.5.1]
- `opencode2` theme only while TUI config `experimental["new-designs"] === true` (experiment toggle lives in the devtools bar's Experiments dialog); the same flag switches v1 migration to `migrateV1NewDesigns` — `packages/tui/src/context/theme.tsx:291-294`, `packages/tui/src/theme/index.ts:41-48,78-88`, `packages/tui/src/component/dialog-experiments.tsx:19-21` — [0.5.1] (hidden experiment)
- v1 asset `packages/tui/src/theme/assets/opencode.json` is never imported — `packages/tui/src/component/error-component.tsx:17` — [dead: not bundled; v2 `opencode` replaces it]
- Legacy v1 resolver `resolveTheme`/`resolveThemeColors` in the TUI package has no caller — `packages/tui/src/theme/index.ts:112-119`, `packages/tui/src/theme/resolve.ts:4-52` — [dead: unused helper]
- Custom themes: `themes/*.json` in `~/.config/opencode` (XDG_CONFIG_HOME) then `.opencode/` of every ancestor of cwd, root first; deeper dirs override; name = basename; JSON object with `theme` (v1) or `base` (v2) key, anything else ignored — `packages/tui/src/util/config-directories.ts:3-5`, `packages/tui/src/theme/discovery.ts:4-20`, `packages/tui/src/theme/index.ts:73-100` — [0.5.1]
- Custom-theme discovery error resets active theme to `anysola`; discovery runs once after mount (first paint uses bundled themes) — `packages/tui/src/context/theme.tsx:160-175` — [0.5.1]
- `SIGUSR2` reloads: palette re-probe at 250 ms and 1000 ms, custom theme rescan at 1000 ms — `packages/tui/src/context/theme.tsx:72-75,78,270-280` — [0.5.1]
- Theme v2 format (base + light/dark, hue scales 100..900, `$path` references, stateful colors `$hovered/$focused/$pressed/$selected/$disabled`, `@dialog`/`@footer` surfaces) and resolution rules — `packages/theme/src/tui/schema.ts:1-310`, `packages/theme/src/tui/expand.ts`, `packages/theme/src/tui/resolve.ts`, `packages/theme/src/tui/select.ts` — [0.5.1]
- v1 themes migrate to v2 at load (OKLCH hue inference, generated 9-step scales, neutral interpolation, literal-to-reference rewrite) — `packages/theme/src/tui/v1-migrate.ts:52-465`, `packages/theme/src/tui/color.ts` — [0.5.1]
- Initial color mode: config lock (`theme.mode` dark/light) else terminal reply to OSC 10/11 within 1000 ms (brightness `(299r+587g+114b)/1000 > 128` → light), else `dark` — `packages/tui/src/app.tsx:313`, `packages/tui/src/context/theme.tsx:133-142`, OpenTUI `chunk-bun-6kxdhkdc.js:20703-20800` — [0.5.1]
- Follows terminal theme changes (DEC 2031 `CSI ? 997 ; 1|2 n` → re-query OSC 11) unless locked — `packages/tui/src/context/theme.tsx:257-268` — [0.5.1]
- A theme missing the requested mode renders its only mode — `packages/tui/src/context/theme.tsx:400-405` — [0.5.1]
- `theme.switch_mode` ("Switch to light mode"/"Switch to dark mode", palette-hidden, key none) pins and persists the other mode; enabled only if the theme has it — `packages/tui/src/app.tsx:1115-1125`, `packages/tui/src/context/theme.tsx:235-244,340-344` — [0.5.1]
- `theme.mode.lock` ("Lock theme mode"/"Unlock theme mode", palette-hidden, key none): lock persists current mode, unlock persists `system` and re-applies the terminal mode — `packages/tui/src/app.tsx:1126-1135`, `packages/tui/src/context/theme.tsx:246-255` — [0.5.1]
- Settings rows: "Theme" (Appearance, "Color theme used across the interface", default `anysola`), "Color mode" (Appearance, "Dark or light variant, or follow the system", `system`/`dark`/`light`, default `system`) — `packages/tui/src/component/dialog-config.tsx:28-48` — [0.5.1]
- `system` theme: generated from the terminal's 16-color palette + default fg/bg (OSC 4/10/11 via `renderer.getPalette({size:16})`), transparent background, ANSI-derived accents, computed gray scale and muted text; probed only while `system` is active, after first paint; palette cached in TUI storage `system-theme`; before any palette it renders as `anysola` — `packages/tui/src/theme/system.ts:4-144`, `packages/tui/src/context/theme.tsx:177-225`, `packages/tui/src/theme/index.ts:60` — [0.5.1]
- Syntax highlight colors: fixed capture-scope → token mapping (`generateSyntax`), incl. prompt extmark styles (`extmark.file` warning bold, `extmark.agent` categorical[0] bold, `extmark.skill` categorical[1] bold, `extmark.paste` inverse-ish on warning bg) — `packages/theme/src/tui/syntax.ts:4-86` — [0.5.1]
- Renderer background = `background.base` — `packages/tui/src/context/theme.tsx:322` — [0.5.1]
- Crash screen uses a hard-coded palette (dark: bg `#0a0a0a`, element `#1e1e1e`, border `#3c3c3c`, text `#eeeeee`, muted `#808080`, primary `#fab283`, error `#e06c75`, success `#7fd88f`; light: `#ffffff`, `#f5f5f5`, `#d4d4d4`, `#1a1a1a`, `#8a8a8a`, `#3b7dd8`, `#d1383d`, `#3d9a57`), not the anysola theme — `packages/tui/src/component/error-component.tsx:17-42` — [0.5.1]

### Attention & notifications

- Config `attention`: `notifications` (default false), `sound` (default false), `volume` 0..1 (default 0.4), `sounds` per-event file overrides, `sound_pack` (default `opencode.default`) — `packages/tui/src/config/index.tsx:99-112,306-312` — [0.5.1]; `sound_pack` has no reader — [dead: parsed, never used]
- Settings rows (category Alerts): "Notifications" ("Whether to notify when a session needs attention", off/on), "Sounds" ("Whether a sound plays when a session needs attention", off/on), "Volume" ("Volume of alert sounds", step 0.1, shown as `40%`) — `packages/tui/src/component/dialog-config.tsx:301-332` — [0.5.1]
- Both off → nothing happens (`attention_disabled`) — `packages/tui/src/attention.ts:131` — [0.5.1]
- Events → messages/sounds (plugin `opencode.notifications`): session finished (idle, not failed, root has no running direct child) → message `Session done`, sound `done` (root) or `subagent_done` (child); form/question created → `Input needs response`, sound `question`, title = form title; permission asked → `Permission needs input`, sound `permission`; execution failed → message = error message, sound `error`, plus toast `Session failed` (variant error, message = error) except for `provider.free-limit`/`provider.go-limit`; a failed session gets no "Session done"; each form/permission notifies once — `packages/tui/src/feature-plugins/system/notifications.ts:4-102` — [0.5.1]
- Notification title = session title (fallback `Anysola Code`), limited to 80 chars; message ANSI-stripped, newlines collapsed to spaces, control chars removed, limited to 240 chars — `packages/tui/src/attention.ts:35-63,144-147` — [0.5.1]
- OS notification only for root sessions and only when the terminal reported focus-out (focus events); unknown focus → skipped; sound plays regardless of focus — `packages/tui/src/feature-plugins/system/notifications.ts:16-17`, `packages/tui/src/attention.ts:78-83,138-161` — [0.5.1]
- Notification escape chosen natively by OpenTUI (`renderer.triggerNotification`): `libopentui.so` contains OSC 9 (`ESC ] 9 ;`), OSC 777 (`ESC ] 777 ; notify ;`) and OSC 99 (kitty, `i=opentui-notifications`) forms; selection logic is in native code — `packages/tui/src/attention.ts:144`, OpenTUI `chunk-bun-6kxdhkdc.js:16879-16885,22187-22191` — [0.5.1] (exact per-terminal choice: open question)
- No terminal bell (BEL) is emitted for attention — grep of `packages/tui/src` — [0.5.1]
- Sounds: built-in MP3s `default`/`done` = `bip-bop-01.mp3`, `question` = `bip-bop-03.mp3`, `permission` = `staplebops-06.mp3`, `error` = `nope-03.mp3`, `subagent_done` = `yup-01.mp3` (from `packages/ui/src/assets/audio/`); config override path tried first, then built-in; played in-process by OpenTUI `Audio` (native) at the configured volume — `packages/tui/src/attention-sounds.bun.ts:2-8`, `packages/tui/src/attention.ts:38-45,104-126`, `packages/tui/src/audio.ts:7-46` — [0.5.1]
- Program status records (OSC 7501, capability-probed natively with `ESC ] 7501 ; ?`): per open tab/viewed session `working` or `blocked` (kind `permission`/`question`, message `Permission needs input`/`Input needs response`), root summary `N sessions need input` / `N sessions working` / `idle`, app id `opencode`, titles clipped to 192 bytes, messages 2048 bytes; cleared on exit — `packages/tui/src/context/program-status.ts:1-94`, `packages/tui/src/context/session-tabs.tsx:180-200` — [0.5.1]

### Clipboard

- Write = host clipboard (OpenTUI native backend, X11/Wayland/macOS/Windows APIs, no shelling to `xclip`/`wl-copy`/`pbcopy`) and OSC 52 (`destination: "all-available"`); success if either worked; NUL bytes stripped — `packages/tui/src/clipboard.ts:13-20,53-62` — [0.5.1]
- Read (paste image): prefers `image/png`, then `text/plain`; image returned base64 — `packages/tui/src/clipboard.ts:24-52` — [0.5.1]
- Copy behavior config `terminal.copy`: `select` (default except Windows) copies when a mouse drag selection is released; `manual` copies with right-click; both show toast `Copied to clipboard` (variant info) — `packages/tui/src/app.tsx:581-582,1388-1400`, `packages/tui/src/util/selection.ts:34-63`, `packages/tui/src/component/dialog-config.tsx:343-351` — [0.5.1]
- Keys with an active selection: Ctrl+C copies (in select mode outside an editor it just clears the selection); Esc clears it (and swallows the key only if text was selected); any other key clears it unless the selection is inside the focused editor; selection stays after copying — `packages/tui/src/util/selection.ts:44-101`, `packages/tui/src/app.tsx:584-594` — [0.5.1]
- Dialogs copy on selection release the same way — `packages/tui/src/ui/dialog.tsx:449-469` — [0.5.1]
- Clicking a link (mouse up) opens the URL in the browser — `packages/tui/src/app.tsx:1396-1398` — [0.5.1]
- OpenTUI console: Ctrl+Y copies the console selection — `packages/tui/src/app.tsx:258`, `:596-606` — [0.5.1] (console hidden by default)

### Terminal title

- Enabled unless `terminal.title: false` (Settings "Window title", Terminal category, "Whether the window title shows the current session") — `packages/tui/src/app.tsx:608`, `packages/tui/src/component/dialog-config.tsx:333-342` — [0.5.1]
- Home: `Anysola Code`; session without a real title (missing or `New session - <ISO>`/`Child session - <ISO>`): `Anysola Code`; titled session: `OC | <title>` with titles over 40 chars cut to 37 chars + `…`; plugin route: `OC | <route name>` — `packages/tui/src/app.tsx:631-658`, `packages/util/src/session-title-fallback.ts:3,25-27` — [0.5.1] (the `OC | ` prefix is upstream branding left in 0.5.1; parity keeps it)
- Toggle command `terminal.title.toggle` ("Disable terminal title"/"Enable terminal title", palette-hidden, key none) persists `terminal.title`; disabling sets the title to empty — `packages/tui/src/app.tsx:1193-1207` — [0.5.1]
- Title set to empty on exit; not changed while suspended (external editor) — `packages/tui/src/util/renderer.ts:5-15` — [0.5.1]
- Sequence emitted natively (`nativeSession.control({kind:"title"})`; `ESC ] 0 ;` present in `libopentui.so`) — OpenTUI `chunk-bun-6kxdhkdc.js:23850-23852` — [0.5.1] (OSC 0 vs 2: open question)

### Toasts

- One toast at a time, absolute top 1 / right 2, max width `min(60, termWidth-6)`, left and right borders `┃` in the variant's feedback color (info/success/warning/error), body on `background.raised.high` with padding 2 horizontal / 1 vertical — `packages/tui/src/ui/toast.tsx:20-101`, `packages/tui/src/ui/border.ts:15-21` — [0.5.1]
- With title: bold title row + right affordance, then word-wrapped message; without title: message + affordance on one row. Affordance `x` (muted) or `› <action label>` (bold when hovered) — `packages/tui/src/ui/toast.tsx:34-93` — [0.5.1]
- Default duration 5000 ms; hover pauses, leaving resumes with the remaining time; click (mouse up with no text selected) dismisses and runs the action — `packages/tui/src/ui/toast.tsx:104-187` — [0.5.1]
- Queue: a new toast replaces the visible one immediately unless the visible one is hovered (paused) or others are queued, in which case it queues; queued count shown as `+N more` (muted) — `packages/tui/src/ui/toast.tsx:94-98,159-167` — [0.5.1]
- `toast.error(err)` = variant error, message = formatted error — `packages/tui/src/ui/toast.tsx:168-170` — [0.5.1]
- Toast texts in this area: `Copied to clipboard` (info); `Failed to load theme: <name>` (error); `MCP server needs authentication` / `Connect "<name>" to use its tools.` (warning, action `Open MCP servers`); `MCP server failed: <name>` / `Run /mcps to view details.` (error, action `Open MCP servers`); `Session failed` (error); `Invalid model format: <arg>` (warning, 3000 ms) — `packages/tui/src/app.tsx:565-579,661-668`, `packages/tui/src/feature-plugins/system/notifications.ts:88-93` — [0.5.1]

### Zed

- When running inside Zed's terminal (`ZED_TERM=true` or `TERM_PROGRAM=zed`), the TUI polls Zed's state DB every 1000 ms (`$OPENCODE_ZED_DB`, else `~/Library/Application Support/Zed/db/0-stable/db.sqlite`, else `~/.local/share/zed/db/0-stable/db.sqlite`) read-only, picks the active editor of the workspace containing cwd, and exposes its file and selection (byte ranges converted to line/col) as editor context for the prompt — `packages/tui/src/editor-zed.ts:1-277`, `packages/tui/src/editor.ts:85`, `packages/tui/src/context/editor.ts:121,178-209,284-288` — [0.5.1]
- Toggle via `app.toggle.file_context` ("Disable file context"/"Enable file context") / config `prompt.editor` (default true) — `packages/tui/src/app.tsx:1223-1235` — [0.5.1] (prompt-side rendering is in the composer section)

### Mini

- Mini frontend (`anysola mini`): entry, footer, prompt, permission UI deleted in 12bab1edde — `packages/tui/src/mini/` — [dead: no command reaches it]
- `mini/splash.ts` (exit banner with the A symbol, eb54ab6257) and `mini/theme.ts`, `mini/mono.ts` are only imported by the storybook — `packages/tui/src/feature-plugins/system/storybook/one-cell-spinner.tsx:7` — [dead: storybook-only]
- `mini/tool.ts` tool formatting is used outside the TUI by `anysola run` and ACP (`packages/cli/src/run/run.ts:11`, `packages/cli/src/run/noninteractive.ts:13`, `packages/cli/src/acp/tool.ts:4`) — [0.5.1] (belongs to the CLI/run spec)
- Config `mini.*` keys (thinking, tools, shell_output, turn_summary, footer, splash, work_spinner, mono, replay, replay_limit) still accepted by the schema, read by nothing — `packages/tui/src/config/index.tsx:204-237` — [dead: no reader]

### Simulation

- Drive mode: `OPENCODE_DRIVE` swaps the renderer for a scripted one with a WebSocket control server (`OPENCODE_DRIVE_RENDERER=headless` for headless), prints `opencode drive ui websocket: <url>` to stderr; `OPENCODE_SIMULATE` replaces backend services with simulated ones; `SimulationSemantics` tags renderables for it — `packages/tui/src/app.tsx:272-275`, `packages/simulation/src/frontend/simulation.ts:9-30`, `packages/cli/src/server-process.ts:114`, `packages/tui/src/simulation/semantics.ts:1-12` — [dead: developer test harness behind env vars; not user-facing]
- `OPENCODE_ROUTE` (JSON initial route) and `OPENCODE_STORY` (storybook) — `packages/tui/src/app.tsx:366-380`, `packages/tui/src/plugin/builtins.ts:31` — [dead: developer-only env switches]

### Devtools

- DevTools bar (bottom row: Server status icon + panels Server/Runtime(CPU, Memory, Loop braille graphs)/Theme(name, mode, init timing)/Tools("Write debug snapshot")/registered groups/Experiments) shown when config `debug.devtools` is true, or by default only on the `local` build channel; release 0.5.1 is channel `latest` (DB file `opencode.db`) so it is off by default — `packages/tui/src/app.tsx:493,1450-1452`, `packages/tui/src/component/devtools-bar.tsx:1-611`, `packages/script/src/index.ts:23-31` — [0.5.1] (opt-in)
- Settings row "Developer tools" (Debug, "Whether the developer tools bar is shown", default off) — `packages/tui/src/component/dialog-config.tsx:352-361` — [0.5.1]
- Experiments dialog: `New designs` ("Preview the new TUI designs"), `TUI tools` ("Let the agent open terminals in this TUI (requires server support)") — `packages/tui/src/component/dialog-experiments.tsx:17-28` — [0.5.1] (reachable only via the devtools bar)
- `app.debug` "Toggle debug panel" (OpenTUI debug overlay, palette-hidden, key none) and `app.console` "Toggle console" (OpenTUI console, in palette, key none) — `packages/tui/src/app.tsx:1162-1180`, `packages/tui/src/config/keybind.ts:50-51` — [0.5.1]
- `debug.timing` shows "Time to first draw"; `debug.turn_tokens` per-turn token diagnostics — `packages/tui/src/config/index.tsx:238-245`, `packages/tui/src/component/devtools-bar.tsx:439` — [0.5.1] (opt-in)

### Animations

- Config `animations`: runtime default **on** (`?? true` everywhere) — `packages/tui/src/app.tsx:1211`, `packages/tui/src/component/spinner.tsx:18,26` — [0.5.1]
- Settings row "Animations" (Appearance, "Whether spinners and transitions animate") declares default `false`, so with the key unset Settings shows `off` while animations actually run (display bug kept for parity or fixed deliberately) — `packages/tui/src/component/dialog-config.tsx:49-57,387` — [0.5.1]
- Command `app.toggle.animations` ("Disable animations"/"Enable animations", palette-hidden, key none) persists the inverse — `packages/tui/src/app.tsx:1209-1221` — [0.5.1]
- Spinner: braille frames `⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏` at 80 ms, muted color, optional shimmer; animations off → static `⋯` (with label `⋯ <label>`) — `packages/tui/src/component/spinner.tsx:12-50`, `packages/tui/src/component/spinner-frames.ts:1` — [0.5.1]
- Animation engine: spring/tween (smoothstep default) driven by a 16 ms interval; consumers: prompt (tween 0.22 s), session tabs pulses, sidebar, fade-in text, title shimmer; animations off → static rendering (`[⋯]` in the prompt, `⋯` in the transcript) — `packages/tui/src/ui/animation.ts:1-235`, `packages/tui/src/component/prompt/index.tsx:120,224,1928`, `packages/tui/src/routes/session/index.tsx:2185,3266`, `packages/tui/src/component/session-tabs.tsx:560,1287`, `packages/tui/src/component/fade-in-text.tsx:99` — [0.5.1]

### Locale helpers (formatting used across the TUI)

- `time()` = locale short time; `datetime()` = `<time> · <locale date>`; `number()` = `1.2K` / `3.4M` (one decimal, 999950+ → M); `truncate` adds `…`; width-aware truncation by grapheme — `packages/tui/src/util/locale.ts:7-79` — [0.5.1]

## Rust implementation notes

- Rendering engine: 0.5.1 runs on OpenTUI (Zig core, Solid reconciler) with flexbox layout (Yoga-like), z-indexed overlays, RGBA alpha blending for backdrops (`RGBA(0,0,0,150)` dialogs, `RGBA(0,0,0,70)` sidebar overlay) and per-cell hit testing for mouse. ratatui has no layout engine with flex-grow/shrink + absolute overlays and no alpha: plan a small layout layer (e.g. `taffy`) and a compositor that dims cells under overlays.
- Markdown: OpenTUI's renderer (marked lexer + tree-sitter markdown highlighting with marker concealment, list marker padding, grid tables with box chars and word-wrapped cells, `│` blockquote, OSC 8 links with ` (url)` fallback, streaming top-level block mode). Port with `pulldown-cmark` (or `comrak`) + a custom block layouter; table layout and streaming re-layout of the last block are the hard parts. `session.markdown = source` mode must keep markers.
- Syntax highlighting: tree-sitter with nvim-treesitter `highlights.scm` queries. Bundled in 0.5.1: javascript, typescript, tsx, markdown, markdown_inline, zig; downloaded on demand (WASM, cached under `$XDG_DATA_HOME/opentui`): python, rust, go, cpp, csharp, bash, c, java, kotlin, ruby, php, scala, html, vue, hcl, json, yaml, haskell, css, julia, lua, ocaml, clojure, swift, toml, nix, diff, elixir, fsharp, r, make, vim, xml, agda (`parsers-config.ts`). Rust has native `tree-sitter` crates, so static linking of all grammars is simpler than the download path; capture→theme-token mapping (`theme-anysola.md` syntax rules) must match.
- Diffs: unified vs split by width (> 120 content width; permission prompt uses terminal width), word wrap on by default, line numbers, add/remove backgrounds, plus the full diff viewer plugin (`/diff`, file tree, single-patch mode, docked pane ≥ 108 cols). Needs a Myers/patience diff (`similar` crate) and a careful side-by-side wrapper.
- Mermaid (`packages/merman`) and LaTeX (`packages/latex`) render inside markdown code fences as box-drawing diagrams / 2-D Unicode math; both are large (~12k and ~2k lines) and have no Rust equivalent — a straight port is required for parity.
- Mouse: click targets everywhere (tabs, recent sessions, status line → `/credits`, links, sidebar sections, buttons), drag-select with copy-on-select (OSC 52 + native clipboard), scroll acceleration option, drag handles for pane widths. Text selection across wrapped, styled cells over a virtualized transcript is the hardest UX piece; crossterm gives raw mouse events only.
- Keymap: leader sequences (`ctrl+x` + key, 2000 ms timeout), comma alternatives, layered modal/global commands with priorities, Kitty keyboard protocol (shift+return, ctrl+return, base-layout fallback). crossterm supports the Kitty enhancement flags; the layer/priority model must be rebuilt.
- Terminal queries: OSC 10/11 background/foreground with a 1000 ms wait, the 16-colour palette for the `system` theme, live theme-change following, OSC 8 hyperlink detection, kitty graphics / image preview. crossterm has no query helpers for these.
- Spinners/shimmer/animations (title shimmer, `⠏` braille spinners, fade-in), toasts with hover-pause, and a 60 fps render loop: use a tick-driven redraw with dirty tracking.
- Theme resolution: v2 theme JSON with primitives/hue steps and the v1→v2 colour-math migration (`v1-migrate.ts`); hover/press shades depend on the hue step a colour came from, so colours must carry their origin (see `themes-bundled.md`).
- East-Asian width / emoji width (`string-width`, `get-east-asian-width`): use `unicode-width` with the same rules.

## Dead in 0.5.1 (code present, unreachable by users)

- Mini frontend (`src/mini/*`: splash, theme, mono, tool renderers, `mini.*` config keys) — the `mini` command was deleted (67cbb0e156, 12bab1edde); only `mini/tool.public.ts` type exports remain.
- Simulation / `OPENCODE_DRIVE` / `OPENCODE_ROUTE` / `OPENCODE_STORY` storybook routes — developer env switches only.
- Release highlights dialog — the highlights array is always empty (8ee8951f4d); `/highlights` never registers.
- OpenCode Go upsell (`dialog-go-upsell.tsx`, `go-upsell-art.ts`) — needs an OpenCode Console `FreeUsageLimitError`.
- `Free models` dialog — only under the hidden `new-designs` experiment with nothing connected.
- OpenCode Console usage dialog (`component/usage.tsx`) and the `$x left` / `Limit reached` usage-limit label in the prompt metadata row — Console providers only.
- Model picker `Usage ctrl+u` action — hidden for the same reason.
- Sidebar `$ spent` line — Anysola models report no cost (captured: absent).
- Session share/unshare — removed upstream; deprecated keybind ids are no-ops.
- Prompt frecency ordering — `frecency.jsonl` is written but the score never changes the order.
- `No goal is set` toast — unreachable from the UI.
- `sound_pack` setting; the v1 `opencode.json` theme asset; the TUI's own old theme resolver.
- Upstream sidebar "Modified files" and LSP sections — not present in v2.2.5/0.5.1.
- Hidden but reachable (not dead): DevTools bar and Experiments dialog (Settings → Debug → Developer tools).

## Parity quirks to keep or fix deliberately

- `/debug` shows `Version 2.2.5 (latest)` (upstream version).
- Terminal title prefix `OC | <title>` (upstream branding) for titled sessions; `Anysola Code` otherwise.
- Settings shows Animations `off` when unset while animations actually run.
- `<leader>q` is bound to both `app.exit` and `session.queued_prompts`.
- Crash screen links to `https://github.com/Anysola/code/issues/new?template=bug-report.yml` and prints `Anysola Code <ver> (OpenCode 2.2.5)`.
