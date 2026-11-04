# Composer (prompt) user-visible strings — Anysola Code 0.5.1

Source: fork `packages/tui/src/…` at tag of 0.5.1 (branch `anysola`). Strings are verbatim; `<x>` marks an interpolated value. `…` is U+2026, `·` is U+00B7.

## Placeholders
| String | Source |
|---|---|
| `Ask anything… "<example>"` | component/prompt/index.tsx:1636 |
| `Run a command… "<example>"` | component/prompt/index.tsx:1633 |
| normal examples: `Explain this project`, `Fix the failing tests`, `Review my last commit` | routes/home.tsx:22 |
| shell examples: `ls -la`, `git status`, `pwd` | routes/home.tsx:23 |

## Prompt chips (virtual text inserted in the textarea)
| String | Source |
|---|---|
| `[Pasted ~<N> lines]` (N = newlines + 1; used for ≥3 lines or >150 chars) | component/prompt/index.tsx:1486 |
| `[SVG: <filename or "image">]` | component/prompt/index.tsx:1467 |
| `[Image <N>]` / `[PDF <N>]` | prompt/attachment.ts:16-17, prompt/attachment.ts:96 |
| `@<file path>` / `@<path>#<start>` / `@<path>#<start>-<end>` / `@<agent>` / `@<skill>` / `@<reference>` | component/prompt/autocomplete.tsx:183-194, component/prompt/autocomplete.tsx:267-270 |

## Metadata row (below the textarea)
| String | Source |
|---|---|
| `<Agent>` (title-cased agent id) | component/prompt/index.tsx:1573 |
| `Shell` (shell mode) | component/prompt/index.tsx:1607, component/prompt/metadata.tsx:26 |
| `auto` (permission mode autoaccept) | component/prompt/metadata.tsx:49 |
| `·` separator | component/prompt/metadata.tsx:54, component/prompt/metadata.tsx:75 |
| provider label hidden when it is `Anysola` | component/prompt/index.tsx:1874 |
| `Limit reached` / `<$x> left` (dead under Anysola) | component/usage.tsx:66-67 |

## Footer row
| String | Source |
|---|---|
| `esc interrupt` / `esc again to interrupt` | component/prompt/index.tsx:141-143 |
| `[⋯]` (spinner when animations off) | component/prompt/index.tsx:1928 |
| spinner cells `■` / `⬝` (8 wide) | ui/spinner.ts:273, ui/spinner.ts:324 |
| `Creating worktree` / `Creating session` / `Submitting prompt` + `.`×1..3 | component/prompt/move.tsx:32, :53, :147; component/prompt/index.tsx:1947 |
| `(new worktree)` | component/prompt/index.tsx:1955 |
| `<~/dir>` or `<~/dir>:<branch>` | component/prompt/index.tsx:1654-1656 |
| `<file><#L or #L1-L2>[ +N]` (editor context) | component/prompt/index.tsx:158-159, :240, :251 |
| `<agent.cycle key> agents` (e.g. `shift+tab agents`) | feature-plugins/prompt/footer.tsx:115-116 |
| `<palette key> commands` (e.g. `ctrl+p commands`) | feature-plugins/prompt/footer.tsx:127-128 |
| `<↓> <N> subagent[s]` / `<N> shell[s]` joined by ` · ` | feature-plugins/prompt/footer.tsx:28, :35, :84-89 |
| `<tokens> (<pct>%)` context usage, `$<cost>` (USD), joined by ` · ` | util/session.ts:63-66, feature-plugins/prompt/footer.tsx:48-49, :104 |
| `◎ goal` / `◎ goal paused` / `◎ goal met` | feature-plugins/prompt/goal.tsx:56-60, :85 |
| `/btw` (spinner label while a side question runs) | feature-plugins/prompt/btw.tsx:35 |
| `esc exit shell mode` / `esc shell` (width < 44) | feature-plugins/prompt/footer.tsx:134-136 |
| `<N> queued · <first queued prompt>` (dock above the prompt) | routes/session/index.tsx:2493-2494 |
| `No preview` / `+<N> more` (image previews) | component/prompt/index.tsx:1742, :1775 |

## Autocomplete popup
| String | Source |
|---|---|
| row kind labels `skill`, `agent`, `reference` | component/prompt/autocomplete.tsx:871-875 |
| reference description ` <git repository or path>` | component/prompt/autocomplete.tsx:467 |
| `/cd` recents description `recent` | component/prompt/index.tsx:2012 |
| `Press ctrl+d to confirm` | component/prompt/index.tsx:2018 |
| `No matching commands` | component/prompt/autocomplete.tsx:858 |
| `Searching…` | component/prompt/autocomplete.tsx:860, :864 |
| `Could not search directories. Keep typing to try again.` | component/prompt/autocomplete.tsx:861 |
| `No matching directories` | component/prompt/autocomplete.tsx:862 |
| `Could not search files. Keep typing to try again.` | component/prompt/autocomplete.tsx:865 |
| `No matching files, agents, or references` | component/prompt/autocomplete.tsx:866 |

## Toasts emitted from composer files
| Variant | Title | Message | Source |
|---|---|---|---|
| error | — | `Directory is required` | component/prompt/index.tsx:276 |
| error | `Failed to change directory` | `<error>` | component/prompt/index.tsx:292, :309 |
| warning (3000 ms) | — | `Connect an integration to send prompts` | component/prompt/index.tsx:321-325 |
| error | `Failed to open editor` | `<error>` (e.g. `Set $VISUAL or $EDITOR to open an external editor`, `Editor exited with code <n>`, `Editor exited with signal <s>`) | component/prompt/index.tsx:569, editor.ts:16, editor.ts:31 |
| warning | — | `This prompt cannot be queued` | component/prompt/index.tsx:1144 |
| warning | — | `Editor context cannot be queued` | component/prompt/index.tsx:1163 |
| warning | `Model unavailable` | `<providerID>/<modelID> is not available in this session's location` | component/prompt/index.tsx:1175-1179 |
| error | `Failed to set up session` / `Creating a session failed` | `<error>` | component/prompt/index.tsx:1272 |
| error | `Failed to prepare session` | `<error>` | component/prompt/index.tsx:1318, :1352 |
| error | `Failed to commit revert` | `<error>` | component/prompt/index.tsx:1358 |
| error | `Failed to send editor context` | `<error>` | component/prompt/index.tsx:1372 |
| error | `Failed to run shell command` | `<error>` | component/prompt/index.tsx:1329 |
| error | `Failed to run command` | `<error>` | component/prompt/index.tsx:1348 |
| error | `Failed to send prompt` | `<error>` | component/prompt/index.tsx:1392 |
| error | — | `Failed to switch model: <error>` (inside the above) | component/prompt/index.tsx:1308 |
| error | — | `Clipboard read timed out` | clipboard.ts:31 |
| error | — | `Clipboard content exceeded configured read or image conversion limits` | clipboard.ts:33 |
| error | — | `Unexpected clipboard MIME type: <mime>` | clipboard.ts:51 |
| error | `Creating workspace failed` | `<error>` (e.g. `Unable to determine current session location`, `Unable to determine current project`, `No worktree directory returned`) | component/prompt/move.tsx:36-59 |
| error | — | `Unable to determine current project` | component/prompt/move.tsx:67 |
| warning | — | `An image in this message is no longer stored, so it can't be restored` / `<N> images in this message are no longer stored, so they can't be restored` | prompt/restore.ts:27-33 |
| warning | — | `Open a session first` | feature-plugins/prompt/btw.tsx:61 |
| info | — | `No goal is set. Start one with /goal <objective>.` (unreachable) | feature-plugins/prompt/goal.tsx:96 |
| error | `Failed to update the goal` | `<error>` | feature-plugins/prompt/goal.tsx:112-116 |
| error | — | `Unable to load terminal` | routes/session/composer/terminals-tab.tsx:14 |
| info | — | `Path copied to clipboard` | ui/working-directory-actions.tsx:32 |
| warning (3000 ms) | — | `Agent not found: <id>` | context/local.tsx:101-105 |
| info (3000 ms) | — | `Add a favorite model to use this shortcut` | context/local.tsx:430-434 |
| info | `No variants available` | `The current model does not support any variants.` | app.tsx:986-990 |
| error | — | `Nothing to undo` (3000 ms) | routes/session/index.tsx:1115 |
| error | — | `Leave shell mode before undoing a queued prompt` | routes/session/index.tsx:810 |

## Palette / keymap command titles defined by the composer
| Title | Command id | Default key | In palette | Source |
|---|---|---|---|---|
| `Clear prompt` | prompt.clear | ctrl+c | no | component/prompt/index.tsx:423 |
| `Submit prompt` | prompt.submit | none | no | component/prompt/index.tsx:433 |
| `Queue prompt` | prompt.queue | `<leader>return` | yes | component/prompt/index.tsx:448 |
| `Remove editor context` | prompt.editor_context.clear | none | yes (when context) | component/prompt/index.tsx:462 |
| `Paste` | prompt.paste | ctrl+v | no | component/prompt/index.tsx:472 |
| `View image attachments` | prompt.images.view | `<leader>i` | yes (when images) | component/prompt/index.tsx:496 |
| `Interrupt session` | session.interrupt | escape | no | component/prompt/index.tsx:503 |
| `Background blocking tools` | session.background | ctrl+b | no | component/prompt/index.tsx:535 |
| `Open editor` (`/editor`) | prompt.editor | `<leader>e` | yes | component/prompt/index.tsx:552-555 |
| `Skills` (`/skills`) | prompt.skills | none | yes | component/prompt/index.tsx:585-588 |
| `Manage workspaces` (`/worktrees`) | session.move | none | yes | component/prompt/index.tsx:624-628 |
| `Change working directory` (`/cd <dir>`) | session.cd | none | — | component/prompt/index.tsx:271-273 |
| `Stash prompt` | prompt.stash | none | yes (when text) | component/prompt/index.tsx:886 |
| `Stash pop` | prompt.stash.pop | none | yes (when stash) | component/prompt/index.tsx:898 |
| `Stash list` | prompt.stash.list | none | yes (when stash) | component/prompt/index.tsx:914 |
| `Shell mode` | (bind `!`) | `!` at offset 0 | no | component/prompt/index.tsx:980-981 |
| `Exit shell mode` | (bind escape / ctrl+c / backspace) | — | no | component/prompt/index.tsx:998-1018 |
| `Previous prompt history` / `Next prompt history` | prompt.history.previous / .next | up / down | no | component/prompt/index.tsx:1033-1034, :1069-1070 |
| `Ask a side question` (`/btw`), description `One-shot answer from the session's context without adding to the conversation` | session.aside | none | yes | feature-plugins/prompt/btw.tsx:52-57 |
| Autocomplete: `Previous autocomplete item`, `Next autocomplete item`, `Hide autocomplete`, `Dismiss autocomplete`, `Select autocomplete item`, `Complete autocomplete item`, `Confirm autocomplete action` | prompt.autocomplete.* | up/ctrl+p, down/ctrl+n, escape, ctrl+c, return, tab, ctrl+d | no | component/prompt/autocomplete.tsx:690-750 |
| Composer dock: `Previous tab`, `Next tab`, `Close composer` | — | left, right, escape/ctrl+c | no | routes/session/composer/index.tsx:77-80 |

## Dialogs opened from the composer
| Dialog | Strings | Source |
|---|---|---|
| Stash | title `Stash`; row description `just now` / `<n>m ago` / `<n>h ago` / `<n>d ago` / locale datetime; footer `~<N> lines`; confirm `Press <key> again to confirm`; action `delete` | component/dialog-stash.tsx:17-21, :43-52, :60, :77 |
| Select variant | title `Select variant`; first option `Default` | component/dialog-variant.tsx:15, :37 |
| Skills | title `Skills`; `No skills available`; `Could not load skills`; `Close and reopen Skills to try again.`; `Loading skills…`; `No skills found` | component/dialog-skill.tsx:58-88 |
| Image viewer | title `Image <i> of <n>`; actions `previous` (←), `next` (→); legacy footer `← previous` / `next →`; `No preview`; fallback label `Image <i>` | component/dialog-image-preview.tsx:41-67 |
| Working directory | title `Working directory`; `Copy path` (description = path); `Open folder` / `in system file manager`; `Workspaces` / `to another working directory` | ui/working-directory-actions.tsx:22-52 |
| /btw prompt | title `/btw`; placeholder `Ask anything` | feature-plugins/prompt/btw.tsx:65 |
| /btw answer | header `/btw` + question; action `copy response` (key `c`) → `✓ copied`; legacy footer `copy` (c), `scroll` (`j/k ↑/↓`) | feature-plugins/prompt/btw.tsx:128-161 |
| Goal | title `Goal`; status `◎ Active` / `◎ Paused` / `◎ Met`; actions `pause` (p), `resume` (r), `clear` (x) | feature-plugins/prompt/goal.tsx:122-162 |
| Composer dock | tab labels `Subagents`, `Shell`, `Terminals`; `esc`; hint `tabs ←/→`; subagent hints `interrupt <key>`, `show inactive <key>` / `show active <key>`; `Running`; `No active subagents` / `No inactive subagents`; shell hints `output <key>`, `kill <key>`; ` No shell commands`; `+ New terminal`; subagent row `<prefix><Agent>: <title>`, agent fallback `Subagent` | routes/session/composer/index.tsx:123-149, subagents-tab.tsx:55-56, :120-133, :203, :209, shell-tab.tsx:51-58, :116, terminals-tab.tsx:20, :105 |
| Shell output | hints `scroll ↑/↓`, `follow end`, `back esc`; `Earlier output omitted · showing recent output` | component/dialog-shell-output.tsx:120-133 |

## Hidden text sent to the model (not shown, but wire-visible)
| String | Source |
|---|---|
| `<system-reminder>Note: The user opened the file "<path>". This may or may not be relevant to the current task.</system-reminder>\n` | component/prompt/index.tsx:165 |
| `<system-reminder>` + per range `Note: The user selected <"Selection i: " when >1>#<L>[-<L2>] from "<path>". ```<text>```\n\n` joined by `\n` + ` This may or may not be relevant to the current task.</system-reminder>\n` | component/prompt/index.tsx:167-172 |
| /btw instructions: `The user is asking a quick side question about the conversation so far. Answer directly and concisely in markdown from what you already know. Do not call any tools and do not take any actions.` + `\n\n` + question | feature-plugins/prompt/btw.tsx:17-21, :69 |
