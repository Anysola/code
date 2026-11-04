# TUI command registry (Anysola Code 0.5.1)

Source of truth: every `KeymapCommand` registered through `Keymap.createLayer` / `context.keymap.layer`.
Paths are relative to `packages/tui/src/`. "Bind" is the default from `config/keybind.ts` (v2 ids; `none` = unbound).
"Palette" = shown in the command palette (ctrl+p). A command is listed in `/` autocomplete only if it has a `slash`
name **and** is currently reachable (its layer enabled and its `enabled` not false) — `context/keymap.tsx:315-347`
(`visibility: "reachable"`). The autocomplete row shows `/<name>` (and one row per alias) with
`description ?? title` as the description (`component/prompt/autocomplete.tsx:491-519`). Rows are sorted by
`display.localeCompare` and the name column is padded to the longest name + 2.

Server-side commands (from core: custom commands, MCP prompts, skills-as-commands) are appended to the same `/`
list from `data.location.command.list(...)` with their server description; they are `queueable` and selecting one
inserts `/<name> ` (`autocomplete.tsx:503-511`). Their list must be captured live (see `slash-list*.txt` captures).

`arguments: true` commands: selecting from autocomplete inserts `/<name> ` and waits for text; Enter on
`/<name> <text>` runs the command with `<text>` as input (`autocomplete.tsx:483-489`, `keymap.tsx:70-79`).

## Slash commands (have a `/name`)

| Slash | Aliases | Title / description (as shown) | Command id | Group | Default bind | Palette | Enabled when | Source | Status |
|---|---|---|---|---|---|---|---|---|---|
| /sessions | /resume, /continue | Switch session | session.list | Session | `<leader>l` | yes (Suggested when any session exists) | always | app.tsx:760-767 | [0.5.1] |
| /new | | New session | session.new | Session | `<leader>n` | yes (Suggested while in a session) | always | app.tsx:770-795 | [0.5.1] |
| /clear | | Clear session | session.clear | Session | (no config key) | yes | always; also closes the current tab | app.tsx:797-822 | [0.5.1] |
| /open | /projects, /project | Open session or project | open.menu | Session | `ctrl+o` | yes | always | app.tsx:824-834 | [0.5.1] |
| /models | | Switch model | model.list | Agent | `<leader>m` | yes (Suggested) | always | app.tsx:899-906 | [0.5.1] |
| /agents | | Switch agent | agent.list | Agent | `<leader>a` | yes | always | app.tsx:945-951 | [0.5.1] |
| /mcps | | MCP servers | mcp.list | Agent | none | yes | always | app.tsx:954-960 | [0.5.1] |
| /variants | /thinking, /effort | Switch model variant | variant.list | Agent | none | only when the current model has variants | always (toast "No variants available" if none) | app.tsx:980-995 | [0.5.1] |
| /connect | | Connect an integration | provider.connect | Integration | none | yes (Suggested when nothing connected) | always | app.tsx:1006-1018 | [0.5.1] (list content comes from server; see dialogs.md) |
| /highlights | | Show release highlights | opencode.highlights | System | none | yes | only if `app.highlights.length > 0` | app.tsx:1020-1029 | [dead: build no longer embeds OPENCODE_RELEASE_HIGHLIGHTS (commit 8ee8951f4d; cli/src/version.ts:13-21 → []), so never registered] |
| /settings | | Open settings | opencode.settings | System | none | yes (Suggested) | always | app.tsx:1031-1038 | [0.5.1] |
| /status | | View status | opencode.status | System | `<leader>s` | yes | always | app.tsx:1041-1047 | [0.5.1] |
| /update | | Update Anysola Code | opencode.update | System | none (v2 has no key; `opencode.update` is bound via appBindingCommands but no Definitions entry) | yes | when an updater is passed (default CLI handler always passes one: cli/src/commands/handlers/default.ts:145-160) | app.tsx:1050-1058 | [0.5.1] |
| /restart | | Restart service | service.restart | System | none | yes | only when the TUI runs against a managed service (`client.restart` defined) | app.tsx:1061-1079 | [0.5.1] (managed-service mode only) |
| /reload | | Reload configuration | location.reload | System | none | yes | always | app.tsx:1082-1095 | [0.5.1] |
| /debug | | View debug info | opencode.debug | System | none | yes | always | app.tsx:1098-1104 | [0.5.1] |
| /themes | | Switch theme | theme.switch | System | none | yes | always | app.tsx:1107-1113 | [0.5.1] |
| /help | | Help | help.show | System | none | yes | always | app.tsx:1138-1144 | [0.5.1] |
| /exit | /quit, /q | Exit the app | app.exit | System | `ctrl+c,ctrl+d,<leader>q` | yes | the key binding layer is enabled only when the prompt is unfocused or empty (app.tsx:1306-1313) | app.tsx:1156-1161 | [0.5.1] |
| /rename | | Rename session | session.rename | Session | `ctrl+r` (bound in the session view layer and as the session-list "rename" action) | yes | in a session route | routes/session/index.tsx:1041-1054 | [0.5.1] — `/rename <title>` renames directly; `/rename ` with empty text regenerates the title (`data.session.title.generate`); no argument opens the Rename dialog |
| /timeline | | Jump to message | session.timeline | Session | `<leader>g` | yes | in a session | routes/session/index.tsx:1056-1063 | [0.5.1] |
| /fork | | Fork session | session.fork | Session | none | yes | in a session | routes/session/index.tsx:1065-1080 | [0.5.1] |
| /compact | | Compact session | session.compact | Session | `<leader>c` | yes | in a session | routes/session/index.tsx:1082-1104 | [0.5.1] |
| /undo | | Undo previous message | session.undo | Session | `<leader>u` | yes | in a session | routes/session/index.tsx:1106-1131 | [0.5.1] |
| /redo | | Redo | session.redo | Session | `<leader>r` | yes | only when the session has a staged revert (`session.revert.messageID`) | routes/session/index.tsx:1133-1148 | [0.5.1] |
| /copy | | Copy last assistant message | messages.copy | Session | `<leader>y` | yes | in a session | routes/session/index.tsx:1259-1300 | [0.5.1] |
| /export | | Export session transcript | session.export | Session | `<leader>x` | yes | in a session | routes/session/index.tsx:1332-1374 | [0.5.1] |
| /terminal | | New terminal | session.terminal | Session | none | yes | in a session and server reports `capabilities.persistentPty !== false` (bun runtime: true) | component/session-frame.tsx:245-255 | [0.5.1] |
| /cd | | Change working directory | session.cd | (none) | none | no (palette not set) | prompt not disabled | component/prompt/index.tsx:270-300 | [0.5.1] — needs an argument; empty → toast "Directory is required" |
| /editor | | Open editor | prompt.editor | Session | `<leader>e` | yes | prompt not disabled | component/prompt/index.tsx:552-584 | [0.5.1] |
| /skills | | Skills | prompt.skills | Prompt | none | yes | prompt not disabled | component/prompt/index.tsx:585-623 | [0.5.1] |
| /worktrees | | Manage workspaces | session.move | Session | none | yes | prompt not disabled | component/prompt/index.tsx:624-632 | [0.5.1] |
| /btw | | Ask a side question — description shown: "One-shot answer from the session's context without adding to the conversation" | session.aside | Session | none | yes | always registered; outside a session → toast "Open a session first" | feature-plugins/prompt/btw.tsx:50-80 | [0.5.1] |
| /plugins | | Plugins | plugins.list | System | none | yes | always | feature-plugins/system/plugins.tsx:333-348 | [0.5.1] |
| /diff | | Open diff viewer | diff.open | VCS | none | yes | always (panel in a session, full-screen route outside) | feature-plugins/system/diff-viewer.tsx:1311-1355 | [0.5.1] |
| /credits | | Credits and plan | anysola.credits | Anysola | none | yes | always | feature-plugins/system/anysola.tsx:362-368 | [0.5.1] (Anysola; behavior in parent's Anysola section) |
| /usage | | Usage | anysola.usage | Anysola | none | yes | always | feature-plugins/system/anysola.tsx:370-376 | [0.5.1] |
| /budget | | Session budget | anysola.budget | Anysola | none | yes | always (`arguments: true`) | feature-plugins/system/anysola.tsx:378-384 | [0.5.1] |
| /devices | | Signed-in devices | anysola.devices | Anysola | none | yes | always | feature-plugins/system/anysola.tsx:386-392 | [0.5.1] |
| /account | | Open your Anysola account | anysola.account | Anysola | none | yes | always | feature-plugins/system/anysola.tsx:394-400 | [0.5.1] |
| /feedback | | Send feedback or report a problem | anysola.feedback | Anysola | none | yes | always | feature-plugins/system/anysola.tsx:402-408 | [0.5.1] |

## Palette / key-only commands (no slash name)

Palette "yes" means `palette: true`; app.tsx maps every entry to `palette: true` unless the entry sets
`palette: undefined` (app.tsx:1269-1278), session route likewise (routes/session/index.tsx:1420-1430),
prompt likewise (component/prompt/index.tsx:634-643).

| Title shown | Command id | Group | Default bind | Palette | Enabled when | Source | Status |
|---|---|---|---|---|---|---|---|
| Show command palette | command.palette.show | System | `ctrl+p` | no (excluded from its own list) | always | app.tsx:750-758 | [0.5.1] |
| Switch to session in quick slot N (N=1..9) | session.quick_switch.N | Session | `<leader>N` | no | only when session tabs are disabled | app.tsx:836-843 | [0.5.1] (tabs off only) |
| Next tab | session.tab.next | Session | `ctrl+tab,alt+down` | no | tabs enabled | app.tsx:844-851 | [0.5.1] |
| Previous tab | session.tab.previous | Session | `ctrl+shift+tab,alt+up` | no | tabs enabled | app.tsx:852-859 | [0.5.1] |
| Next unread tab | session.tab.next_unread | Session | `alt+shift+down` | no | tabs enabled | app.tsx:860-867 | [0.5.1] |
| Previous unread tab | session.tab.previous_unread | Session | `alt+shift+up` | no | tabs enabled | app.tsx:868-875 | [0.5.1] |
| Close tab | session.tab.close | Session | `<leader>w` | yes | tabs enabled | app.tsx:876-882 | [0.5.1] |
| Reopen closed tab | session.tab.reopen | Session | `ctrl+shift+t` | yes | tabs enabled | app.tsx:883-889 | [0.5.1] |
| Switch to tab N (N=1..10) | session.tab.select.N | Session | `<leader>N,ctrl+N` (10 → `<leader>0,ctrl+0`) | no | tabs enabled | app.tsx:890-897 | [0.5.1] |
| Model cycle | model.cycle_recent | Agent | `f2` | no | always | app.tsx:908-915 | [0.5.1] |
| Model cycle reverse | model.cycle_recent_reverse | Agent | `shift+f2` | no | always | app.tsx:917-924 | [0.5.1] |
| Favorite cycle | model.cycle_favorite | Agent | none | no | always | app.tsx:926-933 | [0.5.1] |
| Favorite cycle reverse | model.cycle_favorite_reverse | Agent | none | no | always | app.tsx:935-942 | [0.5.1] |
| Agent cycle | agent.cycle | Agent | `shift+tab` | no | always | app.tsx:962-969 | [0.5.1] |
| Agent cycle reverse | agent.cycle.reverse | Agent | none | no | always | app.tsx:996-1003 | [0.5.1] |
| Variant cycle | variant.cycle | Agent | `ctrl+t` | yes | always | app.tsx:971-977 | [0.5.1] |
| "Switch to light mode" / "Switch to dark mode" | theme.switch_mode | System | none | no | only when the theme supports the other mode | app.tsx:1115-1124 | [0.5.1] |
| "Lock theme mode" / "Unlock theme mode" | theme.mode.lock | System | none | no | always | app.tsx:1126-1135 | [0.5.1] |
| Open docs | docs.open | System | none | yes | opens `https://www.anysola.com/code` | app.tsx:1146-1153 | [0.5.1] |
| Toggle debug panel | app.debug | System | none | no | always | app.tsx:1162-1170 | [0.5.1] (OpenTUI debug overlay) |
| Toggle console | app.console | System | none | yes | always | app.tsx:1172-1179 | [0.5.1] (OpenTUI console overlay) |
| Suspend terminal | terminal.suspend | System | `ctrl+z` (forced `none` on Windows; Windows adds `ctrl+z` to input.undo) | no | not win32 | app.tsx:1181-1191, config/index.tsx:294-302 | [0.5.1] |
| "Disable terminal title" / "Enable terminal title" | terminal.title.toggle | System | none | no | always | app.tsx:1193-1207 | [0.5.1] |
| "Disable animations" / "Enable animations" | app.toggle.animations | System | none | no | always | app.tsx:1209-1221 | [0.5.1] |
| "Disable file context" / "Enable file context" | app.toggle.file_context | System | none | no | always | app.tsx:1223-1235 | [0.5.1] |
| "Disable diff wrapping" / "Enable diff wrapping" | app.toggle.diffwrap | System | none | no | always | app.tsx:1237-1252 | [0.5.1] |
| "Disable paste summary" / "Enable paste summary" | app.toggle.paste_summary | System | none | no | always | app.tsx:1254-1266 | [0.5.1] |
| Page up (scrolls height/2) | session.page.up | Session | `pageup,ctrl+alt+b` | no | session | routes/session/index.tsx:915-921 | [0.5.1] |
| Page down (height/2) | session.page.down | Session | `pagedown,ctrl+alt+f` | no | session | routes/session/index.tsx:922-928 | [0.5.1] |
| Line up | session.line.up | Session | `ctrl+alt+y` | no | session | routes/session/index.tsx:929-935 | [0.5.1] |
| Line down | session.line.down | Session | `ctrl+alt+e` | no | session | routes/session/index.tsx:936-942 | [0.5.1] |
| Half page up (height/4) | session.half.page.up | Session | `ctrl+alt+u` | no | session | routes/session/index.tsx:943-949 | [0.5.1] |
| Half page down (height/4) | session.half.page.down | Session | `ctrl+alt+d` | no | session | routes/session/index.tsx:950-956 | [0.5.1] |
| First message | session.first | Session | `ctrl+g,home,alt+home` (home only when no editor focused) | no | session | routes/session/index.tsx:960-1027 | [0.5.1] |
| Last message | session.last | Session | `ctrl+alt+g,end` | no | session | routes/session/index.tsx:1028-1036 | [0.5.1] |
| "Collapse thinking" / "Expand thinking" | session.toggle.thinking | Session | none | no | session | routes/session/index.tsx:1150-1166 | [0.5.1] |
| Toggle session scrollbar | session.toggle.scrollbar | Session | none | no | session | routes/session/index.tsx:1168-1180 | [0.5.1] |
| "Group related tool calls" / "Show tool calls individually" | session.toggle.exploration_grouping | Session | none | no | session | routes/session/index.tsx:1182-1194 | [0.5.1] |
| "Verbosity: Low/Medium/High" (cycles low→medium→high) | session.verbosity.cycle | Session | none | yes | session | routes/session/index.tsx:1196-1209 | [0.5.1] |
| Jump to last user message | session.messages_last_user | Session | `alt+end` | no | session | routes/session/index.tsx:1211-1229 | [0.5.1] |
| Next message | session.message.next | Session | none | no | session | routes/session/index.tsx:1231-1237 | [0.5.1] |
| Previous message | session.message.previous | Session | none | no | session | routes/session/index.tsx:1238-1244 | [0.5.1] |
| Next user message | session.message.user.next | Session | none | no | session | routes/session/index.tsx:1245-1251 | [0.5.1] |
| Previous user message | session.message.user.previous | Session | none | no | session | routes/session/index.tsx:1252-1258 | [0.5.1] |
| Copy session ID | session.copy.id | Session | none | yes | session | routes/session/index.tsx:1302-1313 | [0.5.1] |
| Copy session transcript | session.copy | Session | none | yes | session | routes/session/index.tsx:1314-1331 | [0.5.1] |
| Background blocking tools | session.background | Session | `ctrl+b` | no | session (route) / status running (prompt) | routes/session/index.tsx:1376-1384, component/prompt/index.tsx:535-551 | [0.5.1] |
| Toggle subagent picker | session.child.first | Session | `down` | yes | session | routes/session/index.tsx:1386-1395 | [0.5.1] |
| View queued prompts | session.queued_prompts | Prompt | `<leader>q` | yes | only with ≥1 queued prompt | routes/session/index.tsx:1396-1402 | [0.5.1] |
| Go to parent session | session.parent | Session | `up` | no | only in a child (subagent) session | routes/session/index.tsx:1403-1418 | [0.5.1] |
| Close sidebar (inline) | (inline) | Session | `escape,ctrl+c` | no | sidebar overlay visible and (width − vertical tabs width) ≤ 120 | routes/session/index.tsx:257-260 | [0.5.1] |
| "Show sidebar" / "Hide sidebar" | session.sidebar.toggle | Session | `<leader>b` | yes | session | component/session-frame.tsx:182-195 | [0.5.1] |
| "Show terminal pane" / "Hide terminal pane" | terminal.toggle | Session | `<leader>t` | yes | session + persistentPty | component/session-frame.tsx:197-218 | [0.5.1] |
| Select terminal | terminal.select | Session | `<leader>down` | yes | session + persistentPty | component/session-frame.tsx:219-231 | [0.5.1] |
| Close terminal pane | terminal.close | Session | `<leader>up` | yes | terminal pane open | component/session-frame.tsx:232-244 | [0.5.1] |
| Focus session pane | pane.focus.left | (none) | `<leader>left` | no | right pane is terminal/panel, no dialog open, not fullscreen | component/session-frame.tsx:164-180 | [0.5.1] |
| Focus right pane | pane.focus.right | (none) | `<leader>right` | no | same | component/session-frame.tsx:164-180 | [0.5.1] |
| Clear prompt | prompt.clear | Prompt | `ctrl+c` | no | prompt enabled | component/prompt/index.tsx:422-431 | [0.5.1] |
| Submit prompt | prompt.submit | Prompt | none (Enter is `input.submit`) | no | prompt enabled | component/prompt/index.tsx:432-446 | [0.5.1] |
| Queue prompt | prompt.queue | Prompt | `<leader>return` | yes | prompt enabled | component/prompt/index.tsx:447-460 | [0.5.1] |
| Remove editor context | prompt.editor_context.clear | Prompt | none | yes | an editor context chip is present | component/prompt/index.tsx:461-470 | [0.5.1] |
| Paste | prompt.paste | Prompt | `ctrl+v` (preventDefault: false) | no | prompt enabled | component/prompt/index.tsx:471-495 | [0.5.1] |
| View image attachments | prompt.images.view | Prompt | `<leader>i` | yes | ≥1 image attachment | component/prompt/index.tsx:496-502 | [0.5.1] |
| Interrupt session | session.interrupt | Session | `escape` | no | status running; must be pressed twice within 5 s (first press arms, second interrupts with `resume: true`); in shell mode it leaves shell mode instead | component/prompt/index.tsx:503-533 | [0.5.1] |
| Stash prompt | prompt.stash | Prompt | none | yes | prompt has text | component/prompt/index.tsx:885-896 | [0.5.1] |
| Stash pop | prompt.stash.pop | Prompt | none | yes | stash non-empty | component/prompt/index.tsx:897-912 | [0.5.1] |
| Stash list | prompt.stash.list | Prompt | none | yes | stash non-empty | component/prompt/index.tsx:913-929 | [0.5.1] |
| Previous prompt history | prompt.history.previous | Prompt | `up` | no | cursor at start of buffer, autocomplete hidden | component/prompt/index.tsx:1033-1053 | [0.5.1] |
| Next prompt history | prompt.history.next | Prompt | `down` | no | cursor at end of buffer | component/prompt/index.tsx:1069-1095 | [0.5.1] |
| Autocomplete prev/next/hide/select/complete | prompt.autocomplete.* | — | `up,ctrl+p` / `down,ctrl+n` / `escape` / `return` / `tab` | no | autocomplete visible | component/prompt/autocomplete.tsx:686-750 | [0.5.1] |
| Dismiss autocomplete | prompt.clear (rebound) | — | `ctrl+c` | no | autocomplete visible | component/prompt/autocomplete.tsx:715-721 | [0.5.1] |
| Confirm autocomplete action | prompt.autocomplete.destructive | — | `ctrl+d` (inline) | no | autocomplete visible | component/prompt/autocomplete.tsx:745-750 | [0.5.1] |
| Composer subagent up/down/select/interrupt | composer.subagent.* | Composer | `up` / `down` / `return` / `ctrl+d` | no | subagents tab active | routes/session/composer/subagents-tab.tsx:140-195 | [0.5.1] |
| Toggle active subagents | composer.subagent.toggle-activity | Composer | `ctrl+a` (inline; no config key) | no | subagents tab active | subagents-tab.tsx:175-185 | [0.5.1] |
| Composer shell up/down/select/kill | composer.shell.* | Composer | `up` / `down` / `return` / `ctrl+d` | no | shell tab active | routes/session/composer/shell-tab.tsx:66-105 | [0.5.1] |
| Composer terminal up/down/select | composer.terminal.* | Composer | `up,k` / `down,j` / `return` | no | terminals tab active | routes/session/composer/terminals-tab.tsx:44-66 | [0.5.1] |
| Diff viewer commands | diff.close … diff.help | VCS | see keybinds-default.md | no | diff viewer focused | feature-plugins/system/diff-viewer.tsx:585-785 | [0.5.1] |

## Keybind names with no registered command (dead config keys)

| Config key | Default | Reason |
|---|---|---|
| `app.clear` | `ctrl+l` | "Clear the screen in mini" — mini frontend deleted (commit 12bab1edde); no layer binds it. [dead] |
| `app.scrap` | none | no command registered anywhere in tui/src. [dead] |
| `server.pair` | none | `/pair` and dialog-pair deleted (12bab1edde). [dead] |
| `usage.show` | none | OpenCode Console `/usage` deleted (12bab1edde); Anysola `/usage` is `anysola.usage`. [dead] |
| `session.tab.history.back` / `session.tab.history.forward` | none / `ctrl+i` | not in any binding layer (only history helpers exist in context/session-tabs-model.ts). [dead] |
| `session.share` / `session.unshare` | none | descriptions say "Deprecated: session sharing was removed". [dead] |
| `diff.toggle`, `diff.expand`, `diff.expand_all`, `diff.collapse`, `diff.switch_focus` | none | "Deprecated: file tree is mouse-controlled" / "keyboard navigation always controls the diff". [dead] |
| `which-key.*` (11 keys) | ctrl+alt+… | no which-key panel in tui/src nor in @opentui/keymap 0.8.0. [dead] |
| `model.dialog.usage` | `ctrl+u` | action hidden unless OpenCode Console limits load (`limits.info()`), which never happens against Anysola. [dead in practice] |
| `opencode.update` / `diff.open` in `appBindingCommands` | — | listed in app.tsx:162-196 for binding; `opencode.update` has no Definitions entry so it never gets a key. |
