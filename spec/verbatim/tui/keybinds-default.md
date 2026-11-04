# TUI keybindings — defaults, syntax, display (Anysola Code 0.5.1)

Paths relative to `packages/tui/src/`.

## Where keybinds live

- TUI settings (incl. `keybinds`, `leader.timeout`) are the `Config.Info` fields of the CLI config file
  `<XDG config>/opencode/cli.json` (`packages/cli/src/config/config.ts:31`, schema `packages/cli/src/config/schema.ts:6-9`,
  `$schema` URL `https://opencode.ai/v2/cli.json`). The file is watched; edits reload live (`config/index.tsx:360-369`).
- Key: `"keybinds": { "<command id>": <binding value> }` using the **v2 dotted ids** below (`config/index.tsx:78`).
- Unknown ids throw `Unrecognized keybind: <id>` / `Unrecognized keybinds: a, b` (`config/keybind.ts:341-344`).
- Legacy `tui.json` (v1, underscore names like `session_new`) is migrated into cli.json by
  `packages/cli/src/config/migrate.ts` using the v1 `CommandMap` (`config/v1/keybind.ts:251-414`). v1 is input-only;
  the running TUI reads only v2.
- Leader timeout: `"leader": { "timeout": <ms> }`, integer > 0, default **2000** (`config/index.tsx:82-88,316`;
  `context/keymap.tsx:140-147`). Settings dialog exposes it as "Leader timeout" 250–10000 step 250, shown `"<n> ms"`.

## Binding value syntax (`config/keybind.ts:8-33`)

- `false` or `"none"` → unbound.
- A string: one or more alternatives separated by `,` (`registerCommaBindings`, `context/keymap.tsx:81`),
  e.g. `"ctrl+c,ctrl+d,<leader>q"`.
- Within an alternative, a **sequence** is space-separated strokes or a `<leader>` token followed by a key
  (`"<leader>n"` = leader then `n`). Modifiers joined with `+`: `ctrl`, `shift`, `alt`/`meta`, `super`, `hyper`.
- Key-name aliases accepted on input (`context/keymap.tsx:82-90`): `enter`→`return`, `esc`→`escape`,
  `pgdown`→`pagedown`, `pgup`→`pageup` (case-insensitive).
- Object form: `{ "name": "x", "ctrl": true, "shift"?, "meta"?, "super"?, "hyper"? }` (KeyStroke) or
  `{ "key": <string|KeyStroke>, "event"?: "press"|"release", "preventDefault"?: bool, "fallthrough"?: bool, ...extra }`.
- Array of any of the above.
- Keyboard-layout fallback (`registerBaseLayoutFallback`): Kitty base-layout codes match Latin keys on other layouts.
- Pending sequences: `escape` clears a pending sequence, `backspace` pops one stroke
  (`registerEscapeClearsPendingSequence`, `registerBackspacePopsPendingSequence`, `context/keymap.tsx:92-93`).
- Leader: `registerTimedLeader({ trigger: <leader key>, name: "leader", timeoutMs })` (`context/keymap.tsx:140-149`).
- On Windows (`terminalSuspend: false`): `terminal.suspend` forced to `none`, and `ctrl+z` is prepended to
  `input.undo` unless the user set it (`config/index.tsx:294-302`).

## How shortcuts are displayed (`context/keymap.tsx:445-466`, `@opentui/keymap` extras formatting)

- Modifier order: `ctrl`, `shift`, `alt` (meta is displayed as `alt`), `super`, `hyper`, joined with `+`.
- Key name `return` displays as `enter`. Aliases: `up`→`↑`, `down`→`↓`, `left`→`←`, `right`→`→`,
  `pageup`→`pgup`, `pagedown`→`pgdn`, `delete`→`del`.
- `<leader>` token displays as the configured leader key string (default `ctrl+x`); sequence parts are joined by a
  single space: `<leader>n` → `ctrl+x n`.
- Multiple bindings of one command join with `, ` (palette footer column uses all bindings, deduped):
  `app.exit` → `ctrl+c, ctrl+d, ctrl+x q`. Footer hints in dialogs use only the first binding.

## Keymap modes (`context/keymap.tsx:41,467-500`)

`base` (default), `modal` (pushed while any dialog is open, `ui/dialog.tsx:320-324`), `menu` (tab context menu),
`global` layers ignore the mode. Inline bindings with `bind:` are hardcoded (not configurable).

## v2 defaults — verbatim (`config/keybind.ts:41-303`)

```ts
export const LeaderDefault = "ctrl+x"

const keybind = (value: Definition["default"], description: string): Definition => ({ default: value, description })

export const Definitions = {
  leader: keybind(LeaderDefault, "Leader key for keybind combinations"),

  "app.exit": keybind("ctrl+c,ctrl+d,<leader>q", "Exit the application"),
  "app.clear": keybind("ctrl+l", "Clear the screen in mini"),
  "app.debug": keybind("none", "Toggle debug panel"),
  "app.console": keybind("none", "Toggle console"),
  "app.scrap": keybind("none", "Open scrap screen"),
  "app.toggle.animations": keybind("none", "Toggle animations"),
  "app.toggle.file_context": keybind("none", "Toggle file context"),
  "app.toggle.diffwrap": keybind("none", "Toggle diff wrapping"),
  "app.toggle.paste_summary": keybind("none", "Toggle paste summary"),
  "command.palette.show": keybind("ctrl+p", "List available commands"),
  "help.show": keybind("none", "Open help dialog"),
  "docs.open": keybind("none", "Open documentation"),
  "opencode.settings": keybind("none", "Open settings"),
  "server.pair": keybind("none", "Pair device"),
  "service.restart": keybind("none", "Restart service"),
  "location.reload": keybind("none", "Reload configuration"),
  "diff.open": keybind("none", "Open diff viewer"),
  "diff.close": keybind("escape,q", "Close diff viewer"),
  "diff.down": keybind("j,down", "Move diff viewer down"),
  "diff.up": keybind("k,up", "Move diff viewer up"),
  "diff.page.down": keybind("pagedown,ctrl+f", "Page diff viewer down"),
  "diff.page.up": keybind("pageup,ctrl+b", "Page diff viewer up"),
  "diff.half_page.down": keybind("ctrl+d", "Scroll diff viewer down half a page"),
  "diff.half_page.up": keybind("ctrl+u", "Scroll diff viewer up half a page"),
  "diff.first": keybind("gg,home", "Go to the start of the diff"),
  "diff.last": keybind("shift+g,end", "Go to the end of the diff"),
  // Retain shipped configuration names without registering the removed tree navigation commands.
  "diff.toggle": keybind("none", "Deprecated: file tree is mouse-controlled"),
  "diff.expand": keybind("none", "Deprecated: file tree is mouse-controlled"),
  "diff.expand_all": keybind("none", "Deprecated: file tree is mouse-controlled"),
  "diff.collapse": keybind("none", "Deprecated: file tree is mouse-controlled"),
  "diff.switch_focus": keybind("none", "Deprecated: keyboard navigation always controls the diff"),
  "diff.next_hunk": keybind("]", "Jump to next diff hunk"),
  "diff.previous_hunk": keybind("[", "Jump to previous diff hunk"),
  "diff.next_file": keybind("n,alt+down", "Jump to next diff file"),
  "diff.previous_file": keybind("p,alt+up", "Jump to previous diff file"),
  "diff.toggle_file_tree": keybind("b", "Toggle diff viewer file tree"),
  "diff.single_patch": keybind("s", "Toggle single patch view"),
  "diff.switch_source": keybind("d", "Switch diff viewer source"),
  "diff.toggle_view": keybind("v", "Toggle diff viewer split or unified view"),
  "diff.toggle_fullscreen": keybind("f", "Toggle diff viewer full screen"),
  "diff.mark_reviewed": keybind("m", "Toggle selected diff file reviewed"),
  "diff.help": keybind("?,shift+?,shift+/", "Show more diff viewer shortcuts"),

  "prompt.editor": keybind("<leader>e", "Open external editor"),
  "theme.switch": keybind("none", "List available themes"),
  "theme.switch_mode": keybind("none", "Switch between light and dark theme mode"),
  "theme.mode.lock": keybind("none", "Lock or unlock theme mode"),
  "session.sidebar.toggle": keybind("<leader>b", "Toggle sidebar"),
  "pane.focus.left": keybind("<leader>left", "Focus session pane"),
  "pane.focus.right": keybind("<leader>right", "Focus right pane"),
  "terminal.select": keybind("<leader>down", "Select terminal"),
  "terminal.toggle": keybind("<leader>t", "Toggle terminal pane"),
  "terminal.close": keybind("<leader>up", "Close terminal pane"),
  "session.toggle.scrollbar": keybind("none", "Toggle session scrollbar"),
  "opencode.status": keybind("<leader>s", "View status"),
  "opencode.debug": keybind("none", "View debug info"),

  "session.export": keybind("<leader>x", "Export session to editor"),
  "session.copy": keybind("none", "Copy session transcript"),
  "session.copy.id": keybind("none", "Copy session ID"),
  "session.move": keybind("none", "Manage workspaces"),
  "session.new": keybind("<leader>n", "Create a new session"),
  "session.list": keybind("<leader>l", "List all sessions"),
  "open.menu": keybind("ctrl+o", "Open recent sessions and projects"),
  "session.tab.next": keybind("ctrl+tab,alt+down", "Switch to next open session tab"),
  "session.tab.previous": keybind("ctrl+shift+tab,alt+up", "Switch to previous open session tab"),
  "session.tab.history.back": keybind("none", "Go back in session tab history"),
  "session.tab.history.forward": keybind("ctrl+i", "Go forward in session tab history"),
  "session.tab.next_unread": keybind("alt+shift+down", "Switch to next unread session tab"),
  "session.tab.previous_unread": keybind("alt+shift+up", "Switch to previous unread session tab"),
  "session.tab.close": keybind("<leader>w", "Close current session tab"),
  "session.tab.reopen": keybind("ctrl+shift+t", "Reopen last closed session tab"),
  "session.timeline": keybind("<leader>g", "Show session timeline"),
  "session.fork": keybind("none", "Fork session from message"),
  "session.rename": keybind("ctrl+r", "Rename session"),
  "session.delete": keybind("ctrl+d", "Delete session"),
  // Retain shipped configuration names; session sharing was removed.
  "session.share": keybind("none", "Deprecated: session sharing was removed"),
  "session.unshare": keybind("none", "Deprecated: session sharing was removed"),
  "session.interrupt": keybind("escape", "Interrupt current session"),
  "session.background": keybind("ctrl+b", "Background blocking session tools"),
  "session.compact": keybind("<leader>c", "Compact the session"),
  "session.aside": keybind("none", "Ask a side question"),
  "session.cd": keybind("none", "Change working directory"),
  "session.queued_prompts": keybind("<leader>q", "Manage queued prompts"),
  "queued_prompt.undo": keybind("ctrl+u", "Undo queued prompt"),
  "queued_prompt.delete": keybind("ctrl+d", "Delete queued prompt"),
  "session.toggle.exploration_grouping": keybind("none", "Toggle related tool call grouping"),
  "session.verbosity.cycle": keybind("none", "Cycle transcript verbosity"),
  "session.child.first": keybind("down", "Toggle subagent picker"),
  "session.child.next": keybind("right", "Go to next child session"),
  "session.child.previous": keybind("left", "Go to previous child session"),
  "session.parent": keybind("up", "Go to parent session"),
  "session.pin.toggle": keybind("ctrl+f", "Pin or unpin session in the session list"),
  "session.quick_switch.1": keybind("<leader>1", "Switch to session in quick slot 1"),
  "session.quick_switch.2": keybind("<leader>2", "Switch to session in quick slot 2"),
  "session.quick_switch.3": keybind("<leader>3", "Switch to session in quick slot 3"),
  "session.quick_switch.4": keybind("<leader>4", "Switch to session in quick slot 4"),
  "session.quick_switch.5": keybind("<leader>5", "Switch to session in quick slot 5"),
  "session.quick_switch.6": keybind("<leader>6", "Switch to session in quick slot 6"),
  "session.quick_switch.7": keybind("<leader>7", "Switch to session in quick slot 7"),
  "session.quick_switch.8": keybind("<leader>8", "Switch to session in quick slot 8"),
  "session.quick_switch.9": keybind("<leader>9", "Switch to session in quick slot 9"),
  "session.tab.select.1": keybind("<leader>1,ctrl+1", "Switch to session tab 1"),
  "session.tab.select.2": keybind("<leader>2,ctrl+2", "Switch to session tab 2"),
  "session.tab.select.3": keybind("<leader>3,ctrl+3", "Switch to session tab 3"),
  "session.tab.select.4": keybind("<leader>4,ctrl+4", "Switch to session tab 4"),
  "session.tab.select.5": keybind("<leader>5,ctrl+5", "Switch to session tab 5"),
  "session.tab.select.6": keybind("<leader>6,ctrl+6", "Switch to session tab 6"),
  "session.tab.select.7": keybind("<leader>7,ctrl+7", "Switch to session tab 7"),
  "session.tab.select.8": keybind("<leader>8,ctrl+8", "Switch to session tab 8"),
  "session.tab.select.9": keybind("<leader>9,ctrl+9", "Switch to session tab 9"),
  "session.tab.select.10": keybind("<leader>0,ctrl+0", "Switch to session tab 10"),

  "stash.delete": keybind("ctrl+d", "Delete stash entry"),
  "model.dialog.provider": keybind("ctrl+a", "Open provider list from model dialog"),
  "model.dialog.favorite": keybind("ctrl+f", "Toggle model favorite status"),
  "model.dialog.usage": keybind("ctrl+u", "Open usage limits from model dialog"),
  "model.list": keybind("<leader>m", "List available models"),
  "model.cycle_recent": keybind("f2", "Next recently used model"),
  "model.cycle_recent_reverse": keybind("shift+f2", "Previous recently used model"),
  "model.cycle_favorite": keybind("none", "Next favorite model"),
  "model.cycle_favorite_reverse": keybind("none", "Previous favorite model"),
  "mcp.list": keybind("none", "List MCP servers"),
  "usage.show": keybind("none", "View usage limits"),
  "provider.connect": keybind("none", "Connect integration"),
  "agent.list": keybind("<leader>a", "List agents"),
  "agent.cycle": keybind("shift+tab", "Next agent"),
  "agent.cycle.reverse": keybind("none", "Previous agent"),
  "variant.cycle": keybind("ctrl+t", "Cycle model variants"),
  "variant.list": keybind("none", "List model variants"),

  "session.page.up": keybind("pageup,ctrl+alt+b", "Scroll messages up by one page"),
  "session.page.down": keybind("pagedown,ctrl+alt+f", "Scroll messages down by one page"),
  "session.line.up": keybind("ctrl+alt+y", "Scroll messages up by one line"),
  "session.line.down": keybind("ctrl+alt+e", "Scroll messages down by one line"),
  "session.half.page.up": keybind("ctrl+alt+u", "Scroll messages up by half page"),
  "session.half.page.down": keybind("ctrl+alt+d", "Scroll messages down by half page"),
  "session.first": keybind("ctrl+g,home,alt+home", "Navigate to first message"),
  "session.last": keybind("ctrl+alt+g,end", "Navigate to last message"),
  "session.message.next": keybind("none", "Navigate to next message"),
  "session.message.previous": keybind("none", "Navigate to previous message"),
  "session.message.user.next": keybind("none", "Navigate to next user message"),
  "session.message.user.previous": keybind("none", "Navigate to previous user message"),
  "session.messages_last_user": keybind("alt+end", "Navigate to last user message"),
  "messages.copy": keybind("<leader>y", "Copy message"),
  "session.undo": keybind("<leader>u", "Undo message"),
  "session.redo": keybind("<leader>r", "Redo message"),
  "session.toggle.thinking": keybind("none", "Toggle thinking blocks visibility"),

  "prompt.submit": keybind("none", "Submit prompt"),
  "prompt.queue": keybind("<leader>return", "Queue prompt"),
  "prompt.editor_context.clear": keybind("none", "Clear editor context"),
  "prompt.images.view": keybind("<leader>i", "View image attachments"),
  "prompt.skills": keybind("none", "Open skill selector"),
  "prompt.stash": keybind("none", "Stash prompt"),
  "prompt.stash.pop": keybind("none", "Pop stashed prompt"),
  "prompt.stash.list": keybind("none", "List stashed prompts"),

  "prompt.clear": keybind("ctrl+c", "Clear input field"),
  "prompt.paste": keybind({ key: "ctrl+v", preventDefault: false }, "Paste from clipboard"),
  "input.submit": keybind("return", "Submit input"),
  "input.newline": keybind("shift+return,ctrl+return,alt+return,ctrl+j", "Insert newline in input"),
  "input.move.left": keybind("left,ctrl+b", "Move cursor left in input"),
  "input.move.right": keybind("right,ctrl+f", "Move cursor right in input"),
  "input.move.up": keybind("up", "Move cursor up in input"),
  "input.move.down": keybind("down", "Move cursor down in input"),
  "input.select.left": keybind("shift+left", "Select left in input"),
  "input.select.right": keybind("shift+right", "Select right in input"),
  "input.select.up": keybind("shift+up", "Select up in input"),
  "input.select.down": keybind("shift+down", "Select down in input"),
  "input.line.home": keybind("ctrl+a", "Move to start of line in input"),
  "input.line.end": keybind("ctrl+e", "Move to end of line in input"),
  "input.select.line.home": keybind("ctrl+shift+a", "Select to start of line in input"),
  "input.select.line.end": keybind("ctrl+shift+e", "Select to end of line in input"),
  "input.visual.line.home": keybind("alt+a", "Move to start of visual line in input"),
  "input.visual.line.end": keybind("alt+e", "Move to end of visual line in input"),
  "input.select.visual.line.home": keybind("alt+shift+a", "Select to start of visual line in input"),
  "input.select.visual.line.end": keybind("alt+shift+e", "Select to end of visual line in input"),
  "input.buffer.home": keybind("none", "Move to start of buffer in input"),
  "input.buffer.end": keybind("none", "Move to end of buffer in input"),
  "input.select.buffer.home": keybind("shift+home", "Select to start of buffer in input"),
  "input.select.buffer.end": keybind("shift+end", "Select to end of buffer in input"),
  "input.delete.line": keybind("ctrl+shift+d", "Delete line in input"),
  "input.delete.to.line.end": keybind("ctrl+k", "Delete to end of line in input"),
  "input.delete.to.line.start": keybind("ctrl+u", "Delete to start of line in input"),
  "input.backspace": keybind("backspace,shift+backspace", "Backspace in input"),
  "input.delete": keybind("ctrl+d,delete,shift+delete", "Delete character in input"),
  "input.undo": keybind("ctrl+-,super+z", "Undo in input"),
  "input.redo": keybind("ctrl+.,super+shift+z", "Redo in input"),
  "input.word.forward": keybind("alt+f,alt+right,ctrl+right", "Move word forward in input"),
  "input.word.backward": keybind("alt+b,alt+left,ctrl+left", "Move word backward in input"),
  "input.select.word.forward": keybind("alt+shift+f,alt+shift+right", "Select word forward in input"),
  "input.select.word.backward": keybind("alt+shift+b,alt+shift+left", "Select word backward in input"),
  "input.delete.word.forward": keybind("alt+d,alt+delete,ctrl+delete", "Delete word forward in input"),
  "input.delete.word.backward": keybind("ctrl+w,ctrl+backspace,alt+backspace", "Delete word backward in input"),
  "input.select.all": keybind("super+a", "Select all in input"),
  "prompt.history.previous": keybind("up", "Previous history item"),
  "prompt.history.next": keybind("down", "Next history item"),

  "composer.subagent.up": keybind("up", "Previous subagent"),
  "composer.subagent.down": keybind("down", "Next subagent"),
  "composer.subagent.select": keybind("return", "Navigate to subagent"),
  "composer.subagent.interrupt": keybind("ctrl+d", "Interrupt subagent"),
  "composer.shell.up": keybind("up", "Previous shell"),
  "composer.shell.down": keybind("down", "Next shell"),
  "composer.shell.select": keybind("return", "View shell output"),
  "composer.shell.kill": keybind("ctrl+d", "Kill shell command"),
  "composer.terminal.up": keybind("up,k", "Previous terminal"),
  "composer.terminal.down": keybind("down,j", "Next terminal"),
  "composer.terminal.select": keybind("return", "Select terminal"),

  "dialog.select.prev": keybind("up,ctrl+p", "Move to previous dialog item"),
  "dialog.select.next": keybind("down,ctrl+n", "Move to next dialog item"),
  "dialog.select.page_up": keybind("pageup", "Move up one page in dialog"),
  "dialog.select.page_down": keybind("pagedown", "Move down one page in dialog"),
  "dialog.select.home": keybind("home", "Move to first dialog item"),
  "dialog.select.end": keybind("end", "Move to last dialog item"),
  "dialog.select.submit": keybind("return", "Submit selected dialog item"),
  "dialog.prompt.submit": keybind("return", "Submit dialog prompt"),
  "dialog.integration.rename": keybind("ctrl+r", "Rename integration account"),
  "dialog.integration.delete": keybind("ctrl+d", "Delete integration account"),
  "dialog.worktree.generate": keybind("tab", "Generate worktree name"),
  "dialog.move_session.new": keybind("ctrl+a", "New worktree"),
  "dialog.move_session.move": keybind("ctrl+m", "Move session to worktree"),
  "dialog.move_session.delete": keybind("ctrl+d", "Delete worktree"),
  "dialog.move_session.refresh": keybind("ctrl+r", "Refresh worktrees"),
  "prompt.autocomplete.prev": keybind("up,ctrl+p", "Move to previous autocomplete item"),
  "prompt.autocomplete.next": keybind("down,ctrl+n", "Move to next autocomplete item"),
  "prompt.autocomplete.hide": keybind("escape", "Hide autocomplete"),
  "prompt.autocomplete.select": keybind("return", "Select autocomplete item"),
  "prompt.autocomplete.complete": keybind("tab", "Complete autocomplete item"),
  "permission.prompt.fullscreen": keybind("ctrl+f", "Toggle permission prompt fullscreen"),
  "plugins.toggle": keybind("return", "Toggle plugin"),
  "dialog.mcp.toggle": keybind("space", "Toggle MCP server"),
  "dialog.plugins.error": keybind("space", "View plugin error"),
  "dialog.plugins.update": keybind("ctrl+u", "Update plugin from plugin dialog"),
  "dialog.plugins.check": keybind("ctrl+r", "Check for plugin updates from plugin dialog"),

  "terminal.suspend": keybind("ctrl+z", "Suspend terminal"),
  "terminal.title.toggle": keybind("none", "Toggle terminal title"),
  "plugins.list": keybind("none", "Open plugin manager dialog"),

  "which-key.toggle": keybind("ctrl+alt+k", "Toggle which-key panel"),
  "which-key.layout.toggle": keybind("ctrl+alt+shift+k", "Switch which-key layout"),
  "which-key.pending.toggle": keybind("ctrl+alt+shift+p", "Toggle which-key pending preview"),
  "which-key.group.previous": keybind("ctrl+alt+left,ctrl+alt+[", "Previous which-key group"),
  "which-key.group.next": keybind("ctrl+alt+right,ctrl+alt+]", "Next which-key group"),
  "which-key.scroll.up": keybind("ctrl+alt+up,ctrl+alt+p", "Scroll which-key up"),
  "which-key.scroll.down": keybind("ctrl+alt+down,ctrl+alt+n", "Scroll which-key down"),
  "which-key.page.up": keybind("ctrl+alt+pageup", "Page which-key up"),
  "which-key.page.down": keybind("ctrl+alt+pagedown", "Page which-key down"),
  "which-key.home": keybind("ctrl+alt+home", "Jump to first which-key binding"),
  "which-key.end": keybind("ctrl+alt+end", "Jump to last which-key binding"),
} satisfies Record<string, Definition>
```

The managed textarea layer (prompt and other textareas) binds these ids (`context/keymap.tsx:94-138`):
`input.move.*, input.select.*, input.line.*, input.visual.line.*, input.buffer.*, input.delete.*, input.backspace,
input.delete, input.newline, input.undo, input.redo, input.word.*, input.select.word.*, input.delete.word.*,
input.select.all, input.submit` — enabled only when the focused editor is a Textarea (not a single-line Input).

## v1 legacy names — verbatim (`config/v1/keybind.ts:41-238`) — migration input only

```ts
export const LeaderDefault = "ctrl+x"

const keybind = (value: Definition["default"], description: string): Definition => ({ default: value, description })

export const Definitions = {
  leader: keybind(LeaderDefault, "Leader key for keybind combinations"),

  app_exit: keybind("ctrl+c,ctrl+d,<leader>q", "Exit the application"),
  app_debug: keybind("none", "Toggle debug panel"),
  app_console: keybind("none", "Toggle console"),
  app_heap_snapshot: keybind("none", "Write heap snapshot"),
  app_toggle_animations: keybind("none", "Toggle animations"),
  app_toggle_file_context: keybind("none", "Toggle file context"),
  app_toggle_diffwrap: keybind("none", "Toggle diff wrapping"),
  app_toggle_paste_summary: keybind("none", "Toggle paste summary"),
  command_list: keybind("ctrl+p", "List available commands"),
  help_show: keybind("none", "Open help dialog"),
  docs_open: keybind("none", "Open documentation"),
  diff_open: keybind("none", "Open diff viewer"),
  diff_close: keybind("escape,q", "Close diff viewer"),
  diff_toggle: keybind("enter,space", "Toggle diff viewer item"),
  diff_expand: keybind("right", "Expand diff viewer item"),
  diff_expand_all: keybind("E", "Expand all diff viewer folders"),
  diff_collapse: keybind("left", "Collapse diff viewer item"),
  diff_switch_focus: keybind("tab", "Switch diff viewer focus"),
  diff_next_hunk: keybind("]", "Jump to next diff hunk"),
  diff_previous_hunk: keybind("[", "Jump to previous diff hunk"),
  diff_next_file: keybind("n", "Jump to next diff file"),
  diff_previous_file: keybind("p", "Jump to previous diff file"),
  diff_toggle_file_tree: keybind("b", "Toggle diff viewer file tree"),
  diff_single_patch: keybind("s", "Toggle single patch view"),
  diff_switch_source: keybind("d", "Switch diff viewer source"),
  diff_toggle_view: keybind("v", "Toggle diff viewer split or unified view"),
  diff_help: keybind("?", "Show more diff viewer shortcuts"),

  editor_open: keybind("<leader>e", "Open external editor"),
  theme_list: keybind("<leader>t", "List available themes"),
  theme_switch_mode: keybind("none", "Switch between light and dark theme mode"),
  theme_mode_lock: keybind("none", "Lock or unlock theme mode"),
  sidebar_toggle: keybind("<leader>b", "Toggle sidebar"),
  scrollbar_toggle: keybind("none", "Toggle session scrollbar"),
  status_view: keybind("<leader>s", "View status"),
  debug_view: keybind("none", "View debug info"),

  session_export: keybind("<leader>x", "Export session to editor"),
  session_copy: keybind("none", "Copy session transcript"),
  session_move: keybind("none", "Move session"),
  session_new: keybind("<leader>n", "Create a new session"),
  session_list: keybind("<leader>l", "List all sessions"),
  open_menu: keybind("ctrl+o", "Open recent sessions and projects"),
  session_tab_next: keybind("ctrl+tab,alt+down", "Switch to next open tab"),
  session_tab_previous: keybind("ctrl+shift+tab,alt+up", "Switch to previous open tab"),
  session_tab_next_unread: keybind("alt+shift+down", "Switch to next unread tab"),
  session_tab_previous_unread: keybind("alt+shift+up", "Switch to previous unread tab"),
  session_tab_close: keybind("<leader>w", "Close current tab"),
  session_tab_reopen: keybind("ctrl+shift+t", "Reopen last closed tab"),
  session_timeline: keybind("<leader>g", "Show session timeline"),
  session_fork: keybind("none", "Fork session from message"),
  session_rename: keybind("ctrl+r", "Rename session"),
  session_delete: keybind("ctrl+d", "Delete session"),
  session_share: keybind("none", "Deprecated: session sharing was removed"),
  session_unshare: keybind("none", "Deprecated: session sharing was removed"),
  session_interrupt: keybind("escape", "Interrupt current session"),
  session_background: keybind("ctrl+b", "Background blocking session tools"),
  session_compact: keybind("<leader>c", "Compact the session"),
  session_queued_prompts: keybind("<leader>q", "View queued prompts"),
  queued_prompt_delete: keybind("ctrl+d", "Delete queued prompt"),
  session_child_first: keybind("down", "Toggle subagent picker"),
  session_parent: keybind("up", "Go to parent session"),
  session_pin_toggle: keybind("ctrl+f", "Pin or unpin session in the session list"),
  session_quick_switch_1: keybind("<leader>1", "Switch to session in quick slot 1"),
  session_quick_switch_2: keybind("<leader>2", "Switch to session in quick slot 2"),
  session_quick_switch_3: keybind("<leader>3", "Switch to session in quick slot 3"),
  session_quick_switch_4: keybind("<leader>4", "Switch to session in quick slot 4"),
  session_quick_switch_5: keybind("<leader>5", "Switch to session in quick slot 5"),
  session_quick_switch_6: keybind("<leader>6", "Switch to session in quick slot 6"),
  session_quick_switch_7: keybind("<leader>7", "Switch to session in quick slot 7"),
  session_quick_switch_8: keybind("<leader>8", "Switch to session in quick slot 8"),
  session_quick_switch_9: keybind("<leader>9", "Switch to session in quick slot 9"),
  session_tab_select_1: keybind("<leader>1,ctrl+1", "Switch to tab 1"),
  session_tab_select_2: keybind("<leader>2,ctrl+2", "Switch to tab 2"),
  session_tab_select_3: keybind("<leader>3,ctrl+3", "Switch to tab 3"),
  session_tab_select_4: keybind("<leader>4,ctrl+4", "Switch to tab 4"),
  session_tab_select_5: keybind("<leader>5,ctrl+5", "Switch to tab 5"),
  session_tab_select_6: keybind("<leader>6,ctrl+6", "Switch to tab 6"),
  session_tab_select_7: keybind("<leader>7,ctrl+7", "Switch to tab 7"),
  session_tab_select_8: keybind("<leader>8,ctrl+8", "Switch to tab 8"),
  session_tab_select_9: keybind("<leader>9,ctrl+9", "Switch to tab 9"),
  session_tab_select_10: keybind("<leader>0,ctrl+0", "Switch to tab 10"),

  stash_delete: keybind("ctrl+d", "Delete stash entry"),
  model_provider_list: keybind("ctrl+a", "Open provider list from model dialog"),
  model_favorite_toggle: keybind("ctrl+f", "Toggle model favorite status"),
  model_list: keybind("<leader>m", "List available models"),
  model_cycle_recent: keybind("f2", "Next recently used model"),
  model_cycle_recent_reverse: keybind("shift+f2", "Previous recently used model"),
  model_cycle_favorite: keybind("none", "Next favorite model"),
  model_cycle_favorite_reverse: keybind("none", "Previous favorite model"),
  mcp_list: keybind("none", "List MCP servers"),
  provider_connect: keybind("none", "Connect an integration"),
  agent_list: keybind("<leader>a", "List agents"),
  agent_cycle: keybind("shift+tab", "Next agent"),
  agent_cycle_reverse: keybind("none", "Previous agent"),
  variant_cycle: keybind("ctrl+t", "Cycle model variants"),
  variant_list: keybind("none", "List model variants"),

  messages_page_up: keybind("pageup,ctrl+alt+b", "Scroll messages up by one page"),
  messages_page_down: keybind("pagedown,ctrl+alt+f", "Scroll messages down by one page"),
  messages_line_up: keybind("ctrl+alt+y", "Scroll messages up by one line"),
  messages_line_down: keybind("ctrl+alt+e", "Scroll messages down by one line"),
  messages_half_page_up: keybind("ctrl+alt+u", "Scroll messages up by half page"),
  messages_half_page_down: keybind("ctrl+alt+d", "Scroll messages down by half page"),
  messages_first: keybind("ctrl+g,home,alt+home", "Navigate to first message"),
  messages_last: keybind("ctrl+alt+g,end", "Navigate to last message"),
  messages_next: keybind("none", "Navigate to next message"),
  messages_previous: keybind("none", "Navigate to previous message"),
  messages_next_user: keybind("none", "Navigate to next user message"),
  messages_previous_user: keybind("none", "Navigate to previous user message"),
  messages_last_user: keybind("alt+end", "Navigate to last user message"),
  messages_copy: keybind("<leader>y", "Copy message"),
  messages_undo: keybind("<leader>u", "Undo message"),
  messages_redo: keybind("<leader>r", "Redo message"),
  display_thinking: keybind("none", "Toggle thinking blocks visibility"),

  prompt_submit: keybind("none", "Submit prompt"),
  prompt_queue: keybind("<leader>return", "Queue prompt"),
  prompt_editor_context_clear: keybind("none", "Clear editor context"),
  prompt_images_view: keybind("<leader>i", "View image attachments"),
  prompt_skills: keybind("none", "Open skill selector"),
  prompt_stash: keybind("none", "Stash prompt"),
  prompt_stash_pop: keybind("none", "Pop stashed prompt"),
  prompt_stash_list: keybind("none", "List stashed prompts"),

  input_clear: keybind("ctrl+c", "Clear input field"),
  input_paste: keybind({ key: "ctrl+v", preventDefault: false }, "Paste from clipboard"),
  input_submit: keybind("return", "Submit input"),
  input_newline: keybind("shift+return,ctrl+return,alt+return,ctrl+j", "Insert newline in input"),
  input_move_left: keybind("left,ctrl+b", "Move cursor left in input"),
  input_move_right: keybind("right,ctrl+f", "Move cursor right in input"),
  input_move_up: keybind("up", "Move cursor up in input"),
  input_move_down: keybind("down", "Move cursor down in input"),
  input_select_left: keybind("shift+left", "Select left in input"),
  input_select_right: keybind("shift+right", "Select right in input"),
  input_select_up: keybind("shift+up", "Select up in input"),
  input_select_down: keybind("shift+down", "Select down in input"),
  input_line_home: keybind("ctrl+a", "Move to start of line in input"),
  input_line_end: keybind("ctrl+e", "Move to end of line in input"),
  input_select_line_home: keybind("ctrl+shift+a", "Select to start of line in input"),
  input_select_line_end: keybind("ctrl+shift+e", "Select to end of line in input"),
  input_visual_line_home: keybind("alt+a", "Move to start of visual line in input"),
  input_visual_line_end: keybind("alt+e", "Move to end of visual line in input"),
  input_select_visual_line_home: keybind("alt+shift+a", "Select to start of visual line in input"),
  input_select_visual_line_end: keybind("alt+shift+e", "Select to end of visual line in input"),
  input_buffer_home: keybind("none", "Move to start of buffer in input"),
  input_buffer_end: keybind("none", "Move to end of buffer in input"),
  input_select_buffer_home: keybind("shift+home", "Select to start of buffer in input"),
  input_select_buffer_end: keybind("shift+end", "Select to end of buffer in input"),
  input_delete_line: keybind("ctrl+shift+d", "Delete line in input"),
  input_delete_to_line_end: keybind("ctrl+k", "Delete to end of line in input"),
  input_delete_to_line_start: keybind("ctrl+u", "Delete to start of line in input"),
  input_backspace: keybind("backspace,shift+backspace", "Backspace in input"),
  input_delete: keybind("ctrl+d,delete,shift+delete", "Delete character in input"),
  input_undo: keybind("ctrl+-,super+z", "Undo in input"),
  input_redo: keybind("ctrl+.,super+shift+z", "Redo in input"),
  input_word_forward: keybind("alt+f,alt+right,ctrl+right", "Move word forward in input"),
  input_word_backward: keybind("alt+b,alt+left,ctrl+left", "Move word backward in input"),
  input_select_word_forward: keybind("alt+shift+f,alt+shift+right", "Select word forward in input"),
  input_select_word_backward: keybind("alt+shift+b,alt+shift+left", "Select word backward in input"),
  input_delete_word_forward: keybind("alt+d,alt+delete,ctrl+delete", "Delete word forward in input"),
  input_delete_word_backward: keybind("ctrl+w,ctrl+backspace,alt+backspace", "Delete word backward in input"),
  input_select_all: keybind("super+a", "Select all in input"),
  history_previous: keybind("up", "Previous history item"),
  history_next: keybind("down", "Next history item"),

  "dialog.select.prev": keybind("up,ctrl+p", "Move to previous dialog item"),
  "dialog.select.next": keybind("down,ctrl+n", "Move to next dialog item"),
  "dialog.select.page_up": keybind("pageup", "Move up one page in dialog"),
  "dialog.select.page_down": keybind("pagedown", "Move down one page in dialog"),
  "dialog.select.home": keybind("home", "Move to first dialog item"),
  "dialog.select.end": keybind("end", "Move to last dialog item"),
  "dialog.select.submit": keybind("return", "Submit selected dialog item"),
  "dialog.prompt.submit": keybind("return", "Submit dialog prompt"),
  "dialog.project_copy.generate": keybind("tab", "Generate project copy name"),
  "dialog.move_session.new": keybind("ctrl+m", "New project copy"),
  "dialog.move_session.delete": keybind("ctrl+d", "Delete project copy"),
  "dialog.move_session.refresh": keybind("ctrl+r", "Refresh project copies"),
  "prompt.autocomplete.prev": keybind("up,ctrl+p", "Move to previous autocomplete item"),
  "prompt.autocomplete.next": keybind("down,ctrl+n", "Move to next autocomplete item"),
  "prompt.autocomplete.hide": keybind("escape", "Hide autocomplete"),
  "prompt.autocomplete.select": keybind("return", "Select autocomplete item"),
  "prompt.autocomplete.complete": keybind("tab", "Complete autocomplete item"),
  "permission.prompt.fullscreen": keybind("ctrl+f", "Toggle permission prompt fullscreen"),
  "plugins.toggle": keybind("return", "Toggle plugin"),
  "dialog.mcp.toggle": keybind("space", "Toggle MCP server"),

  terminal_suspend: keybind("ctrl+z", "Suspend terminal"),
  terminal_title_toggle: keybind("none", "Toggle terminal title"),
} satisfies Record<string, Definition>
```

## Hardcoded (non-configurable) keys

| Where | Keys | Action | Source |
|---|---|---|---|
| Any dialog | `escape`, `ctrl+c` | Close top dialog (if a mouse selection exists: clear it first; `ctrl+c` clears a non-empty focused input instead of closing) | ui/dialog.tsx:345-386 |
| DialogSelect lists | `up,ctrl+p` / `down,ctrl+n` / `pageup` / `pagedown` / `home` / `end` / `return` (via `dialog.select.*` config) | Move/submit (wraps around) | ui/dialog-select.tsx:476-536 |
| DialogSelect with onCancel | `escape` | "Back" | ui/dialog-select.tsx:544-558 |
| DialogSelect with sectionNavigation (Open dialog) | `alt+up` / `alt+down` | Previous/next section | ui/dialog-select.tsx:559-575 |
| Help | `return`, `escape` | Close | ui/dialog-help.tsx:10-16 |
| Alert | `return` | Confirm | ui/dialog-alert.tsx:20-30 |
| Confirm (legacy) | `return` / `left` / `right` | Choose / toggle cancel↔confirm (starts on confirm) | ui/dialog-confirm.tsx:35-66 |
| Prompt dialog | `escape` | Back (calls onCancel) | ui/dialog-prompt.tsx:84-102 |
| Session list | `ctrl+a` | Toggle "all projects" ↔ current directory | component/dialog-session-list.tsx:221-232 |
| Open dialog | `right` / `left` / `ctrl+n` | Show project worktrees / Return to projects / New worktree | component/dialog-open.tsx:384-417 |
| Settings | `left` / `right` | Previous/next value | component/dialog-config.tsx:456-469 |
| Experiments | `left` / `right` | Toggle | component/dialog-experiments.tsx:77-94 |
| Plugins | `ctrl+a` | Toggle internal plugins | feature-plugins/system/plugins.tsx:192-201 |
| Debug | `return` (legacy) / `c` (new designs) | Copy debug info | component/dialog-debug.tsx:51-54 |
| Error details | `escape` / `c` / `i` / `up` `down` `pageup` `pagedown` `home` `end` | Back / copy details / investigate / scroll | component/dialog-error-details.tsx:55-78 |
| Image preview | `left` / `right` | Previous/next image | component/dialog-image-preview.tsx:30-35 |
| Shell output | `up` `down` `pageup` `pagedown` `home` `end` | Scroll / follow | component/dialog-shell-output.tsx:175-184 |
| Execute details | `up` `down` `pageup` `pagedown` `home` `end` `c` `o` | Scroll / copy code / copy output | routes/session/dialog-execute.tsx:90-99 |
| Export options (legacy) | `tab` / `shift+tab` / `return` | Move / activate | ui/dialog-export-options.tsx:170-182 |
| Export result | `return` / `c` | Close / copy path | ui/dialog-export-result.tsx:30-45 |
| Update | `return`; legacy also `left` `right` `tab` `shift+tab` | Run active button / move | component/dialog-update.tsx:68-87 |
| Tab context menu | `escape,ctrl+c` | Close menu | component/session-tabs.tsx:378-382 |
| Session sidebar overlay | `escape,ctrl+c` | Close sidebar (when overlaying, width ≤ 120) | routes/session/index.tsx:257-260 |
| Permission prompt | `left,h` / `right,l` / `return` / `escape` | Previous / next option / select / reject | routes/session/permission.tsx:440-495 |
| Permission reject reason | `escape` / `return` | Cancel / confirm rejection | routes/session/permission.tsx:285-306 |
| Question form | `tab`/`shift+tab`, `left`/`h`, `right`/`l`, `up`/`k`, `down`/`j`, digits `1..n`, `return`, `escape`, `c` | Field nav, answer select, submit, dismiss, copy link | routes/session/form.tsx:540-760 |
| Selection (global, priority 101) | `ctrl+c` with a selection → copy (toast "Copied to clipboard"); `escape` → clear selection; any other key clears a non-editor selection | util/selection.ts:65-101, app.tsx:584-594 |
| OpenTUI console | `ctrl+y` | copy-selection inside console overlay | app.tsx:256-259 |
| Mouse | right-click copies the selection when `terminal.copy` is `manual`; release after drag copies when `select` (default except Windows) | app.tsx:1388-1403, ui/dialog.tsx:457-471 |
