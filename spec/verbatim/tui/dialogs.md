# TUI dialogs — verbatim texts (Anysola Code 0.5.1)

Paths relative to `packages/tui/src/`. All strings are exact. "Legacy" = the default look; the `new-designs`
experiment (off by default, toggled only from the DevTools bar → Experiments) swaps footers/headers. Unless stated,
everything below describes the **legacy (default)** rendering, which is what 0.5.1 users see.

## 0. Dialog frame (shared)

- Overlay: full-screen box, background `RGBA(0,0,0,150)`, zIndex 3000; dialog panel top-aligned at
  `paddingTop = height/4` (or vertically centered when `centered`), horizontally centered (`ui/dialog.tsx:36-90`).
- Widths: medium 60, large 88, xlarge 116; `maxWidth = terminalWidth − 2`; panel bg `theme.background.base`
  (dialog surface), paddingTop 1 (`ui/dialog.tsx:24-34,80-83`).
- Clicking the backdrop closes all dialogs (unless the mouse-down started a text selection) (`ui/dialog.tsx:51-60`).
- `DialogLayout`: side padding 3 (`DIALOG_INSET`), gap 1 row between header/body/footer, 1 blank row at bottom
  (`ui/dialog.tsx:139-171`).
- `DialogHeader`: bold title (legacy) left, muted close label right — default `esc`, clickable (`ui/dialog.tsx:92-133`).
- `DialogFooter` (legacy): secondary actions left, primary right, gap 2; each action renders `"<title> <key>"` with
  the title in base text and key in muted; actions with no resolvable shortcut are hidden; below 60 cols the
  secondary group stacks vertically (`ui/dialog.tsx:209-304`).
- Keys: `escape`/`ctrl+c` close top dialog (see keybinds-default.md). Dialog stack: `replace` swaps the whole stack.
- `DialogSelect` (list dialogs, `ui/dialog-select.tsx`):
  - Header = title; below it a search input, placeholder **`Search`** unless the dialog passes `placeholder`
    (the `titlePlaceholder` values below are only used under new designs) (`:655,687`).
  - Fuzzy filter: `fuzzysort.go(needle, options, { keys: ["title","category","searchText"], scoreFn: title*2 + category + searchText, threshold })` (`:201-224`).
  - Categories render as a bold header in `hue.accent[200]` with 1 blank row before each non-first category (`:878-889`).
  - Row: current item marked `●` in the gutter; active row has `background.action.primary.focused` bg, bold,
    `text.action.primary.focused` fg; description follows the title after one space in muted (active: same as title);
    footer right-aligned; titles truncated to 61 cells by default (`:891-967`).
  - Visible list height `min(rows, floor(termHeight/2) − 6)` (`:247-262`). Navigation wraps.
  - Empty: `No items available`; no match: `No results found` (`:756-777`).
  - Choice widget (new designs only): `‹ value ›` active, `  value ↵` for pickers (`:870-876`).

## 1. Command palette (`component/command-palette.tsx`) — ctrl+p — [0.5.1]

- Title `Commands`; search placeholder (legacy) `Search` (new designs: `Search commands`); flat when filtering;
  `filterThreshold 0.7`.
- Options: every reachable command with `palette` set except `command.palette.show`; title = command title;
  description = command description; category = group; footer = all shortcuts joined `, `; searchText = id + description.
- With an empty query a **Suggested** category is prepended containing commands whose `suggested` is true
  (`/sessions` when sessions exist, `/new` in a session, `Switch model`, `Connect an integration` when nothing connected,
  `Open settings`) followed by all commands grouped by category.
- When the query is non-empty, every Settings entry is appended as an option (title = setting title,
  category = setting category, searchFooter `Settings · <Category>`); selecting opens Settings focused on it.

## 2. Model picker (`component/dialog-model.tsx`) — /models, `<leader>m` — [0.5.1]

- Title `Select model` (or the provider name when opened for one provider). Placeholder `Search`.
- Sections (no query, something connected): `Favorites`, `Recent`, then one section per provider name
  (Anysola: `Anysola`). Deprecated models hidden. Free-model section only under new designs.
- Row: title = model name; description = **Anysola note** for the model id from the remote config
  (`Account.notes()[model.id]`), else provider name (sections) / `(Favorite)` for favorites (provider sections)
  (`:86-87,126,161`).
- Footer = `~<credits> credit/turn` when the remote config sends a per-turn estimate (`estimate === 1` → singular
  `credit`), else `Free` for free models (`:89-95`). Credits formatted by `formatCredits` (anysola/store).
- Searching: fuzzysort over `title`,`category`, favorites first.
- Footer actions (legacy, shortcut shown after title): `Connect an integration ctrl+a` (or `View all integrations`
  when nothing is connected), `Usage ctrl+u` (hidden: only with OpenCode Console limits — never under Anysola),
  `Favorite ctrl+f` (hidden when nothing connected).
- Selecting a model: sets it (recorded as recent); if the model has variants and the current variant is not valid,
  opens **Select variant**; else closes.

## 3. Select variant (`component/dialog-variant.tsx`) — /variants /thinking /effort — [0.5.1]

- Title `Select variant`; flat; options `Default` (value default) then each variant id (e.g. `economy`,
  effort names). Current marked `●`. Filter shown (legacy).
- No variants → toast (info) title `No variants available`, message `The current model does not support any variants.`

## 4. Agent picker (`component/dialog-agent.tsx`) — /agents, `<leader>a` — [0.5.1]

- Title `Select agent`; options title = agent id, description = agent description; current marked.

## 5. Session list (`component/dialog-session-list.tsx`) — /sessions /resume /continue, `<leader>l` — [0.5.1]

- Size large. Title view: bold `Sessions` + muted ` for <project name>` when scoped to the current project.
  Placeholder `Search` (new designs: `Search sessions in <project>` / `Search sessions`).
- Scope: stored pref `session-list.allProjects`, initial `true` unless tabs.scope is `cwd` (default `cwd` → current
  directory). `ctrl+a` toggles; footer hint right side: `all projects ctrl+a` / `current directory ctrl+a`.
- Server search (debounced 150 ms, limit 50, desc, top-level only). Status texts: `Loading sessions…`,
  `Searching sessions…`, `Could not load sessions.`, `Could not search sessions. Change the search to try again.`,
  `No sessions available`, `No sessions found` (error ones in error color).
- Categories: `Pinned` (only when tabs disabled), `Today`, else `Date.toDateString()` (e.g. `Tue Nov 03 2026`).
- Row title = session title with timestamped fallback; footer = basename of the session root when outside the project
  (truncated 25); gutter: `!` (permission) / `?` (question) bold warning, spinner when running, quick-slot number
  (accent) when tabs disabled.
- Actions: `pin/unpin ctrl+f` (hidden with tabs on), `delete ctrl+d` (first press turns the row into
  `Press ctrl+d again to confirm` with destructive bg; second deletes), `rename ctrl+r` (opens Rename session).
  Quick-switch hint (tabs off, legacy): `switch ctrl+x 1-9` (`<first> through <last>` if not a 1..9 range).
- Delete failure toast: `Failed to delete session: <error>` (5 s).

## 6. Rename session (`component/dialog-session-rename.tsx` + `ui/dialog-prompt.tsx`) — [0.5.1]

- Title `Rename session`; placeholder `Session title`; prefilled with current title; single-line textarea.
- Legacy footer: `submit enter` (left). Busy state shows `processing…`. Empty value ignored.
- Failure toast: `Failed to rename session: <error>` (5 s).
- Generic DialogPrompt defaults: placeholder `Enter text`, busy spinner text `Working…`.

## 7. Theme list (`component/dialog-theme-list.tsx`) — /themes — [0.5.1]

- Title `Themes`; options = all theme names sorted case-insensitively; current marked; **live preview**: moving the
  selection applies the theme, typing applies the first match, empty query restores the original; Esc/close
  reverts unless confirmed with Enter.

## 8. Settings (`component/dialog-config.tsx`) — /settings — [0.5.1]

- Title `Settings`; `filterThreshold 0.7`; footer hint `change ←/→`; Enter cycles forward, ←/→ cycle; numeric
  settings step and clamp. Footer column shows the current value label. Options in order (category · title ·
  values (labels) · default · description):
  - Appearance · Theme · (all themes) · `anysola` · Color theme used across the interface
  - Appearance · Color mode · system/dark/light · `system` · Dark or light variant, or follow the system
  - Appearance · Animations · off/on · `off` (display default; the app treats unset as on) · Whether spinners and transitions animate
  - Session · Sidebar · hide/auto · `auto` · Whether the sidebar shows when there is room
  - Session · Scrollbar · off/on · `off` · Whether a scrollbar is shown next to the chat
  - Session · Thinking · hide/show · `hide` · Whether model thinking is shown in the chat
  - Session · Markdown · source/rendered · `rendered` · Whether chat markdown is rendered or shown as source
  - Session · Tool grouping · none/auto · `auto` · Whether consecutive reads and searches are grouped
  - Session · Verbosity · low/medium/high · `medium` · How much tool activity is shown in the chat
  - Session · Transcript images · off/on · `off` · Whether images are shown in the chat
  - Session · TPS · off/on · `on` · Whether tokens per second are shown for responses
  - Session · New session location · launch/inherit (launch directory/active session) · `launch` · Directory new sessions start in
  - Session · Permissions · prompt/autoaccept (prompt/auto accept) · `prompt` · Whether permission requests ask you or auto accept
  - Tabs · Mode · off/on/auto · `auto` · Whether sessions open in tabs
  - Tabs · Scope · cwd/global (current directory/global) · `cwd` · Whether tabs are per directory or shared everywhere
  - Tabs · Layout · horizontal/vertical · `horizontal` · Whether tabs run along the top or down the side
  - Tabs · Indicators · status/numbers (status icons/always show numbers) · `status` · Whether tabs show status icons or always numbers
  - Diffs · Layout · auto/split/unified · `auto` · Whether diffs are side by side or in one column
  - Diffs · Wrapping · none/word · `word` · Whether long lines in diffs wrap
  - Diffs · File tree · off/on · `on` · Whether a file tree is shown next to diffs
  - Diffs · Single patch · off/on · `off` · Whether diffs show one file at a time
  - Input · Scroll speed · 0.25–10 step 0.25, shown `toFixed(2)` · `3` · How far each scroll step moves
  - Input · Acceleration · off/on · `off` · Whether scrolling speeds up the faster you scroll
  - Input · Mouse · off/on · `on` · Whether the interface responds to the mouse
  - Input · Editor context · off/on · `on` · Whether your editor's selection is added to prompts
  - Input · Large pastes · compact/full · `compact` · Whether large pastes are summarized or shown in full
  - Input · Image previews · off/on · `off` · Whether attached images are previewed in the prompt
  - Input · Leader timeout · 250–10000 step 250, shown `<n> ms` · `2000` · How long the leader key waits for the next key
  - Alerts · Notifications · off/on · `off` · Whether to notify when a session needs attention
  - Alerts · Sounds · off/on · `off` · Whether a sound plays when a session needs attention
  - Alerts · Volume · 0–1 step 0.1, shown `<n>%` · `0.4` · Volume of alert sounds
  - Terminal · Window title · off/on · `on` · Whether the window title shows the current session
  - Terminal · Copy behavior · manual/select · `select` (`manual` on Windows) · Whether selecting text copies it
  - Debug · Developer tools · off/on · `off` · Whether the developer tools bar is shown
  (descriptions are only displayed under new designs; in legacy they are search keywords only via `keywords`).

## 9. Status (`component/dialog-status.tsx`) — /status, `<leader>s` — [0.5.1]

- Title `Status`; body: `No MCP servers` or `<n> MCP server(s)` then one line per server:
  `•` (green connected / red failed / yellow needs_auth / muted) + bold name + muted status:
  `Connected` | `<error>` | `Disabled in configuration` | `Needs authentication: <error>` | raw status.

## 10. MCP servers (`component/dialog-mcp.tsx`) — /mcps — [0.5.1]

- Title `MCP servers`; options sorted by name; footer status: `Connecting …`, bold `Connected ✓`, `Failed !`,
  `Sign in required →`, `Disabled ○` colored by status.
- Action `space` with title `disconnect` / `retry` / `sign in` / `connect` by status; hint `view error enter` when
  the focused server has an error. Enter on failed → error details (title `MCP server: <name>`, context
  `Status: <s>\nConfiguration: mcp.servers.<name>[\nIntegration: <id>]`, fallback error `Unknown MCP connection error`).
- App toasts on status change (`app.tsx:556-579`): warning `MCP server needs authentication` / `Connect "<name>" to use
  its tools.`; error `MCP server failed: <name>` / `Run /mcps to view details.`; action label `Open MCP servers`
  (toast shows `› Open MCP servers`).

## 11. Error details (`component/dialog-error-details.tsx`) — reused by MCP/plugins — [0.5.1]

- Bold truncated title, optional source path line; scrollable error text (max 20 rows); `Reference: <ref>`.
- Legacy footer: left `investigate i`, `copy details c` (after copy `✓ copied` green); right `scroll ↑/↓`.
- `i` navigates home with a prefilled "investigate" prompt built by `util/error-details.ts`.

## 12. Help (`ui/dialog-help.tsx`) — /help — [0.5.1]

- Title `Help`, close label `esc/enter`; body (muted):
  `Press ctrl+p to see all available actions and commands in any context.` (shortcut resolved live);
  legacy footer button `ok` (primary focused bg, right-aligned).

## 13. Debug (`component/dialog-debug.tsx`) — /debug — [0.5.1]

- Size large; title `Debug`; rows label padded to 10: `Version` `<version> (<channel>)`, `Date` ISO,
  `OS`, `Terminal`, `Session ID` (or `n/a`), `Model` `<provider>/<model>` (or `n/a`).
- Legacy footer: left muted `Share this when reporting an issue.`, right `copy enter` → `✓ copied`;
  toast `Debug info copied to clipboard`.

## 14. Timeline (`routes/session/dialog-timeline.tsx`) — /timeline, `<leader>g` — [0.5.1]

- Size large; title `Timeline`; user messages newest first, title = text with newlines→spaces, footer
  `Locale.time(created)`; moving scrolls the transcript to that message; Enter opens Message Actions.

## 15. Message actions (`routes/session/dialog-message.tsx`) — [0.5.1]

- Title `Message Actions`; options (title + description):
  `Jump to` view message in session · `Revert` undo messages and file changes · `Copy` message text to clipboard ·
  `Fork` create a new session.
- Opened from Timeline or by clicking a user message (`routes/session/index.tsx:2405`).

## 16. Fork session (`routes/session/dialog-fork.tsx`) — /fork — [0.5.1]

- Size large; title `Fork session`; first option `Full session`, then user messages newest first with time footer.
  Pending state: same title + spinner `Forking session…`. Success toast `Forked session` (success, 4 s); navigates to
  the fork and restores the chosen user message into the prompt.

## 17. Export (`ui/dialog-export-options.tsx`, `ui/dialog-export-result.tsx`) — /export, `<leader>x` — [0.5.1]

- Title `Export session`; body `Export as:` with radio `◉`/`○` `Markdown` / `JSON`; markdown shows `[x]/[ ]`
  `Include thinking` (default **true** from the command), `Include tools` (default true); JSON shows
  `Sanitize sensitive data` (default false). Buttons right: `Copy`, `Export` (tab/shift+tab cycle, Enter activates).
- Copy → toast `Copied to clipboard` (success). Export → writes `<tmpdir>/session-<uuid>.md|json`, then dialog title
  `Session exported`, body = path, button `Close` (`c` copies the path). Failure toast `Failed to export session`.
- Related: `Copy session transcript` toast `Session transcript copied to clipboard!` / `Failed to copy session transcript`;
  `/copy` toast `Message copied to clipboard!` / `No assistant messages found` / `No text content found in last assistant message`
  / `Failed to copy to clipboard`; `Copy session ID` toast `Session ID copied to clipboard!` / `Failed to copy session ID`.
- Sharing: removed — `session.share/unshare` keybinds are "Deprecated: session sharing was removed". [dead]

## 18. Stash (`component/dialog-stash.tsx`) — "Stash list" — [0.5.1]

- Title `Stash`; newest first; title = first line truncated 50; description relative time `just now` / `<n>m ago` /
  `<n>h ago` / `<n>d ago` / locale datetime (≥7 d); footer `~<n> lines` when multi-line.
- Action `delete ctrl+d` (two-press confirm: `Press ctrl+d again to confirm`). Selecting restores and removes it.

## 19. Skills (`component/dialog-skill.tsx`) — /skills — [0.5.1]

- Size large; title `Skills`; option title = name padded to the longest, description = collapsed description.
  States: `Loading skills…`, `No skills available`, `No skills found`, error `Could not load skills` + message +
  `Close and reopen Skills to try again.` Selecting inserts `@<skill> ` as a virtual mention.

## 20. Open (`component/dialog-open.tsx`) — /open /projects /project, `ctrl+o` — [0.5.1]

- Size large; title `Open`, placeholder `Search sessions and projects…`; section navigation alt+↑/↓.
- `Sessions` section: up to 8 recent sessions not already in a tab (plus any awaiting input); title with timestamp
  fallback; footer `<project · dir> · <ago>` where ago = `now`/`<n>m`/`<n>h`/`<n>d`/`<n>mo`/`<n>y`; gutter `!`/`?`/
  spinner/`▪` (open tab, accent).
- `Projects` section: project name, footer = `~`-abbreviated path (`→` suffix for git projects), `●` for current.
- `→` on a git project: worktree view titled `<Project> / Worktrees`, placeholder `Search worktrees…`, hint
  `new worktree ctrl+n`, `←` returns. New worktree prompt: `<Project> / New worktree`, placeholder
  `Worktree name (optional)`, description `Leave blank for a random name.`, busy `Creating worktree…`.
- Texts: `No recent sessions or projects`, `Refreshing sessions and projects…`, `Loading worktrees…`,
  `No matching worktrees`, `Searching sessions and projects…`, `No matches · search all sessions with ctrl+x l`
  (or `No matches`), `Could not refresh sessions|projects|sessions and projects.`
- Toasts: `Loading worktrees failed` / `Creating worktree failed` (+ error).

## 21. Worktrees / workspaces (`component/dialog-workspaces.tsx`, `prompt/move.tsx`) — /worktrees — [0.5.1]

- Size xlarge; title `Worktrees` (+ spinner while working); categories `Current` / `Other`; titles are
  `~`-abbreviated paths truncated from the left, subdirectory suffix muted.
- Actions: `move ctrl+m` (in a session), `new ctrl+a`, `delete ctrl+d` (two-press `Press ctrl+d again to confirm`;
  while deleting `Deleting <path>`), `refresh ctrl+r`.
- States: `Loading worktrees…`, `No worktrees available`, `No worktrees found`, `Could not load worktrees` +
  `Close and reopen Worktrees to try again.`
- Delete with changes → `Delete worktree?` / `This worktree has file changes. Do you want to delete it anyway?`
  (File changes dialog default title `File Changes Found`, message `Do you want to move these changes with the
  session?`, buttons cancel/confirm). Name prompt `Name worktree` placeholder `Worktree name`, action
  `generate one tab`. Toasts: `Failed to delete worktree`, `Creating workspace failed`, `Unable to determine current project`.
  Progress labels: `Creating worktree`, `Creating session`, `Submitting prompt`.

## 22. Image preview (`component/dialog-image-preview.tsx`) — `<leader>i` — [0.5.1]

- Size xlarge; title `Image <i> of <n>`; image rendered via terminal graphics (`protocol="auto"`); legacy footer
  `← previous` / name (or `No preview` in red) / `next →`.

## 23. Shell output (`component/dialog-shell-output.tsx`) — from composer Shell tab — [0.5.1]

- Size xlarge; bold command title + status `Running` / `Timed out` / `Killed` / `Exited` / `Exited · code <n>`;
  footer `scroll ↑/↓`, `follow end`, `back esc`; texts `Earlier output omitted · showing recent output`,
  `Loading output…`, `No captured output. Output redirected to files is not shown here.`,
  errors `Shell output is no longer available.` / `Unable to read shell output. Retrying…`.

## 24. Execute details (`routes/session/dialog-execute.tsx`) — [0.5.1 if the `execute` tool exists; see tools spec]

- Status `Receiving code…` / `Running`; footer `copy code c`, `copy output o`, `scroll ↑/↓`, `back esc`;
  `Waiting for code…`, `No output`, `Waiting for output…`.

## 25. Update (`component/dialog-update.tsx`) — /update and home notice — [0.5.1]

- Centered. Title: `Update` / `Update available` / `Updating Anysola Code`.
- Bodies: spinner `Checking for updates…`; `An update is available. After installing, you'll be prompted to restart
  Anysola Code.`; spinner `Installing Anysola Code <v>…`; `Update successful! Restart closes Anysola Code, then run
  anysola to use the new version.`; `Anysola Code is already up to date.`; unavailable message; error text in red.
- Buttons (legacy, right): `Skip` + `Update` (available) or `Restart` (installed); active = last.
- Check = Anysola `GET /v1/code/version` → available when latest > current (packages/cli/src/services/updater.ts:237-244);
  install = npm/install command.
- Home notice under the logo (`routes/home.tsx:127-178`): `/update` (accent) + ` to install v<version>` or `/exit` +
  ` restart to use v<version>`; remote: `remote server update available`. Click opens the dialog / exits.

## 26. Plugins (`feature-plugins/system/plugins.tsx`) — /plugins — [0.5.1]

- Title `Plugins`; categories `TUI` / `Server`; internal plugins hidden unless `ctrl+a` (`show internal ctrl+a` /
  `hide internal ctrl+a`); footer per row: status (if not active), `local`, version (hash shortened to 7),
  `update available`, or `updating`.
- Actions: `check for updates ctrl+r` (`checking for updates`), `view error space`, `enable`/`disable` (`return`),
  `update ctrl+u`. Error details title `TUI plugin error` / `Server plugin error`. Toast `Failed to update plugin <id>`.

## 27. Connect an integration (`component/dialog-integration.tsx`) — /connect — [0.5.1 (contents from server)]

- Title `Connect an integration`; categories `Popular` (ids opencode, opencode-go, openai, github-copilot,
  anthropic, google) / `Services`; MCP-sourced integrations excluded. Row description `Environment only` when no
  connect methods; footer `Sign in required →` (warning) or connection summary; gutter `✓` when connected.
  Empty `No integrations available`; no match `No integrations found`.
- Manage view: title = integration name; `Add account`; `Connected accounts` section; actions `rename ctrl+r`
  (prompt `Rename account` / `Account name`), `delete ctrl+d` (two-press confirm). Connect method picker title
  `Connect <name>`; API key prompt placeholder `API key`; OAuth view footer `open o`, `copy c`
  (`copy code`/`copy link`), messages `Connecting…`, `Starting authorization…`, `Waiting for authorization…`,
  `Starting command…`, `Waiting for command to finish…`, code prompt `Authorization code`.
- Toasts: `Connected <name>`, `Disconnected <name>`, `Authentication expired`, `Authorization expired`,
  `Could not open the browser. Copy the URL and continue manually.`, `Copied to clipboard`.
- Under the Anysola provider lock the server lists no third-party providers; what remains (if anything) must be
  captured live.

## 28. /btw answer (`feature-plugins/prompt/btw.tsx`) — [0.5.1]

- Without an argument: prompt dialog title `/btw`, placeholder `Ask anything`. While pending the prompt footer shows a
  spinner `/btw`. Answer dialog (large, centered): bold `/btw` + muted question; markdown answer; footer
  `copy c` (→ `✓ copied`), `scroll j/k ↑/↓`.

## 29. Permission prompt (`routes/session/permission.tsx`, `util/permission.ts`) — [0.5.1]

- Inline (above composer), not a modal. Header `△` (warning) `Permission required`; second line `<icon> <title>`
  (not for shell): icons `→` Edit/Read/List/LSP, `✱` Glob/Grep, `#` Shell command / `<Agent> Subagent`,
  `%` WebFetch, `◈` web search, `←` external directory, `⟳` doom loop, `⚙` other tool (`Call tool <name>`).
- Options: `Allow once`, `Always allow` (only when the request has save patterns), `Reject`; ←/h →/l move,
  Enter selects, Esc rejects; `ctrl+f` toggles fullscreen. Always-allow body: `This will always allow <action> for
  this project.` or `This will always allow the following patterns for this project.` + `- <pattern>` lines.
- Edit body shows the diff (or `No diff provided`); external_directory shows `Patterns`.
- Reject in a **child** session asks for a reason: `△ Reject permission`, `Tell Anysola Code what to do differently`,
  Enter confirms, Esc cancels.

## 30. Question / form tool UI (`routes/session/form.tsx`) — [0.5.1]

- Inline panel: muted form title, message, `Field <i> of <n>` · `<a>/<n> completed` or tab strip per field + review;
  numbered answers `1.` …, `Type your own answer` custom row; placeholders `Type your answer`, `YYYY-MM-DD`,
  `YYYY-MM-DDTHH:MM:SSZ`; external link fields `open link` / `I finished`, `(acknowledgement required)` /
  `Acknowledged`; footer hints `⇆ tab`, `↑↓ select`, `↑↓ scroll`, `enter <action>`, `c copy`, `esc dismiss|close`.
  Errors: `External action must be acknowledged: <label>`, `Invalid answer`,
  `Could not open the browser. Copy the URL and continue manually.`; toast `Copied URL to clipboard`.

## 31. Overlays and toasts

- Toast (`ui/toast.tsx`): top-right (top 1, right 2), max width `min(60, width−6)`, left+right `┃` border in the
  variant color, bg `background.raised.high`, padding 2×1. With title: bold title row + `x` (or `› <action label>`)
  on the right, message below; without title: message + `x`. Queue indicator `+<n> more`. Default duration 5000 ms;
  hover pauses; click dismisses (or runs the action). Variants info/success/warning/error.
- Startup (`component/startup-loading.tsx`): after 500 ms of not-ready, bottom-centered spinner `Loading plugins…` /
  `Finishing startup…`, held ≥3 s once shown.
- Reconnecting (`component/reconnecting.tsx`): dimmed full screen, 48-wide box: spinner `Restarting service…` +
  `Your session will resume automatically.` (managed) or `Connection lost…` + `Reconnecting to the server
  automatically.`; shown after 1 s (reconnecting) / 5 s (initial) (`app.tsx:1357-1384`).
- Migration (`component/migration-overlay.tsx`): top-right spinner `<label> <n>/<m>`; error toast
  `Data migration failed` (10 s).
- Unknown plugin route: `Unknown plugin route: <id>/<name>` + `go home` button.
- App toasts: `Copied to clipboard` (selection copy), `Invalid model format: <arg>` (3 s), `Restarting service…` (30 s)
  → `Service restarted`, `Reloading configuration…` (30 s) → `Configuration reloaded`,
  `Session "<title>" was deleted` / `The current session was deleted`, `Nothing to undo` (3 s), server
  `tui.toast.show` events verbatim. Tab menu: `Session ID copied to clipboard`. Terminal: `Unable to load terminal`.
- Composer `/cd`: `Directory is required`, `Failed to change directory` + error.

## 32. Dead dialogs in 0.5.1

- `component/dialog-highlights.tsx` (`What's new in <version>`, `scroll ↑/↓`, `ok`) — [dead: highlights array is
  always empty; build stopped embedding release highlights (8ee8951f4d)].
- `component/dialog-go-upsell.tsx` + `go-upsell-art.ts` (OpenCode Go, `https://opencode.ai/go`, actions
  `don't show again` / `dismiss` / `subscribe`) — [dead: opened only on a `provider.free-limit` step error, produced
  only when the provider error body contains `FreeUsageLimitError` (core/src/session/to-session-error.ts:44), an
  OpenCode Console response the Anysola gateway does not send].
- `component/dialog-model-free.tsx` (`Free models`, `Connect a provider for access to more models`) — [dead: only
  under the off-by-default `new-designs` experiment and only when nothing is connected].
- `component/usage.tsx` `DialogUsage` (OpenCode Console limits) — [dead: reachable only via the model picker's
  `Usage` action, hidden unless Console limits load].
- `component/dialog-experiments.tsx` (`Experiments`: `New designs` / `TUI tools`, `No experiments available`) —
  [0.5.1 but hidden: reachable only from the DevTools bar, which shows only with Settings → Debug → Developer tools on].
- Session share/unshare — [dead: removed upstream; keybinds kept as deprecated no-ops].
- Pair dialog, stats route, Console limit sidebar — deleted in 12bab1edde (no file).
