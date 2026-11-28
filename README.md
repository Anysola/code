# Anysola Code

A coding agent for Anysola accounts, as a desktop app for Windows and Linux.

Download: https://www.anysola.com/code. Account: https://account.anysola.com.

## Install

| Platform | File (from this repository's latest release) |
|---|---|
| Windows x64 | `anysola-code-desktop-windows-x64-setup.exe` (installs for the current user) |
| Linux x64 | `anysola-code-desktop-linux-x64.AppImage` or `anysola-code-desktop-linux-x64.deb` |

The Windows installer and the AppImage update themselves when a new version is out; a `.deb` install opens the
release page instead. The installers are not code-signed yet, so Windows shows SmartScreen's "unknown publisher"
on first run.

The first start opens a device sign-in at https://account.anysola.com/device in your browser.

## Coming from the terminal app

The `anysola` terminal app (CLI and TUI) is retired; 1.0.9 was its last version. Install the desktop app: your
sessions, sign-in and settings in `~/.anysola` (`%USERPROFILE%\.anysola` on Windows) carry over. Remove the old
binary with `anysola uninstall` or `npm rm -g @anysola/code`, whichever way you installed it.

## Configuration

Settings → Agent edits the agent's settings; the files behind it are read in this order: `~/.anysola/config.json`,
then `anysola.json` or `anysola.jsonc` in the project and its parent folders, then a project's `.anysola/` folder,
which also holds `agent/`, `command/` and `skills/`. `~/.anysola/` holds the global `AGENTS.md`, agents, commands
and skills the same way. For compatibility, the 0.5.x names are still read: `opencode.json[c]`, `.opencode/`,
`~/.config/opencode/` and each `OPENCODE_*` variable; where both exist, the `anysola` one wins.

## Hooks

Hooks are commands that run around the agent's work: `PreToolUse` and `PostToolUse` around each tool call,
`UserPromptSubmit` when you send a message, `Stop` when the agent would stop and `SubagentStop` when a subagent
would, `PreCompact` before the conversation is compacted, `Notification` when you are asked something (a
permission or a question), and `SessionStart` and `SessionEnd`. The format is Claude Code's, so existing hook
scripts work: the event comes as JSON on stdin, exit code 2 blocks with stderr as the reason, and JSON on stdout can
add context or deny a call. Settings → Hooks adds, edits and removes them.

What each new event can do: `SubagentStop` keeps a subagent going like `Stop` does the agent (at most 8 times in a
row in which the subagent calls no tool; rounds with a tool call don't count), and its matcher is the subagent's type. `PreCompact` can block a `/compact` (matcher `manual`); an automatic
compaction (matcher `auto`) can't be blocked, since the conversation could not go on, and a block is only shown.
`Notification` (matcher `permission_prompt` or `question`) runs alongside the question, never answers it and never
holds it up. `SessionEnd` runs when the app closes or signs you in or out, and when you delete a session (reason
`other`), after nothing can refuse the delete any more; all of them together get 1.5 seconds. Then each hook's
process group is ended: on Linux and macOS that takes what it left in the background (a `&` job, a double fork),
but not a process that leaves the group itself (`setsid`, a service it starts). On Windows the hook's process tree
is ended; what it started outside that tree ends only when the app exits (its job), not when a session is deleted.
`Notification` and `SessionEnd` can't block
anything; their exit code 2 shows stderr to you.

Hooks run in your `shell` setting; without one, on Windows in Git Bash when it is installed (so
`"$CLAUDE_PROJECT_DIR"/.claude/hooks/x.sh` works as written), else in the shell commands use. A hook can name its own with a
`shell` field next to its `command`, such as `"shell": "powershell"` for a PowerShell command; a project's hooks
can't (the field is ignored there, as a project's `shell` setting is). Settings → Hooks shows where each hook
runs and can set its shell.

Hooks only tighten: a hook can block a call or have you asked about it, never allow something your permissions
would not. Hooks in `~/.anysola/config.json` run right away; a project's hooks run only once you approve them in
the conversation, and again after they change. A project file approved in 1.5 or earlier that has hooks for one of
the events added in 1.6 is asked about again. The agent's file tools (edit, write, notebook) always ask before
changing your config files, the files your hooks run and the app's database.

## What a project can change

A repository's own config is read like yours, but it only tightens until you say otherwise. Its rules that ask
or deny apply at once; its rules that allow, its MCP servers, formatters, hooks and downloads (instruction URLs,
skill URLs, git references) wait for your approval, asked once per project file in the conversation and again
after a change. Settings → Permissions, Instructions, MCP servers, Advanced and Hooks list what waits and what you
allowed, with Stop allowing.

Some things a project can never do, allowed or not: lift a rule or policy of yours that refuses; choose the agent
your sessions run on when you chose one; turn off, hide or retype a built-in agent or one of yours; replace one of
your commands; choose the shell or where worktrees are created; turn the shell sandbox on or off, or turn LSP
diagnostics on (it can turn them off). Rules name files, not spellings: a rule on
`secrets/*` holds in a worktree, from a subfolder and through a symlink, and a symlink that leads out of the
project is an outside path. A subagent is bound by the rules of every session above it.

A project can still choose the model, replace an agent's prompt and turn snapshots off. Shell commands are judged
by your `shell` rules only: a `read` rule does not stop `cat`.

## Worktrees

In a Git folder a new session works in its own worktree, on a branch `anysola/<name>` made from the folder's
current commit, under `~/.anysola/worktrees`. Uncommitted changes of the main folder are not in it. Review shows
the session's changes against that commit; Apply puts them on the main folder's branch as one commit (your Git
hooks and identity apply; Git 2.40 or later), after which the session goes on from that commit. Discard deletes
the worktree and the branch. A branch made from an earlier message gets its own worktree with the files as they
were then. A new worktree gets the main folder's ignored `.env*` files, and the folder's setup command (Settings,
yours only, never the repository's) runs in it before the first message. If the main folder changed the same
lines, "Bring main in" merges its branch into the session's, where the conflicts are resolved before applying.

The session's file tools treat only its worktree as inside. They never write into the main folder or another
session's worktree, in any mode and whatever you allowed for a parent folder: apply the changes instead. Reading
there asks as for any outside folder, and your rules still name those files as they would in the main folder. The project's
config, hooks and MCP servers are the main folder's; an edit to the worktree's `anysola.json` counts once
applied. Hooks in a worktree session run from the worktree, with it as their project folder
(`$CLAUDE_PROJECT_DIR`). Local hook and config files under `.claude/` and `.anysola/` that Git ignores are
copied in from the main folder when the worktree is made; later changes to them in the
main folder aren't picked up until Recreate. The file tools also never write the worktree's own `.git`.

A worktree is not a sandbox. Only the file tools are held to it: shell commands run as anywhere else and can
reach any folder, MCP servers run with the main folder as their folder and aren't confined either, and Git in a
worktree shares the repository's branches and objects (it can move or push them).

An experimental shell sandbox (Linux only, with bubblewrap) is off by default; `"sandbox": true` in
`~/.anysola/config.json` turns it on (`{"network": false}` also cuts the network). It wraps the agent's shell tool,
scheduled `{run}` checks, and ``!`cmd` `` template placeholders in a private PID and session namespace
(`--unshare-pid --new-session`) with `/` read-only and only the session's folder (its worktree, or the main folder,
with `.git` itself mounted read-only so it cannot be edited, removed, or renamed) plus a per-session cache folder
(`XDG_CACHE_HOME`, `npm_config_cache`, `PIP_CACHE_DIR`, `GOCACHE`, `GOMODCACHE`, `UV_CACHE_DIR`, `TMPDIR`, cleaned
up when the session is deleted) writable. Reads and the network stay open unless `"network": false`, and because
`.git` is read-only `git add`/`commit` don't work inside the sandbox. Everything else that runs commands stays
outside it: hooks, formatters, LSP servers, MCP servers, your own `!` commands, the worktree setup command, and the
embedded terminal. When it is on but can't start (bubblewrap missing, user namespaces restricted, or not Linux),
commands are refused instead of run unconfined.

Post-edit LSP diagnostics are on by default for projects you approve: opening a folder with matching installed
language servers asks for approval before starting any server (since language servers run a project's own config,
plugins, and build tooling outside the shell sandbox). Adding a new language ecosystem asks again, revoking stops
running servers immediately, and `"lsp": false` in your own or project config disables LSP without asking.

A branch made from an older session (from before worktrees) starts from the main folder's current commit with
only that session's own files as they were then; other uncommitted changes of the main folder aren't in it.

## Notebooks

The agent reads a Jupyter notebook (`.ipynb`) as its cells: each cell's index, id, type and source, with text
outputs and error tracebacks shortened and images as a placeholder. It changes one cell at a time with the
`notebook` tool (`NotebookEdit` to hooks): replace a cell's source, insert a cell, or delete one. Every other cell
keeps its exact bytes; a file with a top-level key written twice is not changed, and replacing a code cell's source clears its outputs. It asks and is undone like any other
file edit.

## Side questions, import and notes

`/btw <question>` asks about the session without adding to it, also while the agent is working: the answer comes
from what the session already holds, uses no tools, and shows in a card over the message box until you close it
(Esc); it isn't stored. "Import session…" (the New session menu, the palette, or a project's menu as "Import
session here…") takes a file written by Export: it becomes a new session, with the conversation only, in that
project, else the open folder, else your default folder. Nothing in the file carries permissions, settings, hooks
or approvals, and snapshots from another machine are not taken, so an imported session can't revert files. What a
later request sends from it is rebuilt from checked values (images are checked as attached ones and limited per
file), and what the file says came from a hook or a subagent shows as "From the imported file", never as the app's
own note. ArrowUp brings back earlier messages, but a `/` line brought back (or put back by a rewind) runs nothing
on Enter until you change it, so a command in an imported history never runs by accident. `/memory <note>` adds a line to the project's `AGENTS.md` (in a worktree session, the worktree's) or to your
own `~/.anysola/AGENTS.md`, whichever you pick; `/memory` alone opens either in your editor. When the project's
`AGENTS.md` is a link, the note goes to the file it links to only if that is an `AGENTS.md` or `CLAUDE.md` of the
same checkout outside Git's data and not a file the agent's tools always ask about; a project `AGENTS.md` that is a
hard link (a second name for another file) takes no note. A `/memory` or `/btw` line is never sent as a message. Ctrl G jumps to any of the
session's messages, Ctrl Shift ↑ / ↓ to the previous or next.

## Usage limits

Anysola Code and Anysola Chat share two usage windows per account: a 5-hour window that starts with your first
request and a weekly one. Settings → Usage shows both with their reset times. When a window is full you can wait
for it to reset or spend a banked reset (at most 3, each with an expiry date).

## History

1.0 is a rewrite in Rust of 0.5.x, which was a fork of [OpenCode](https://github.com/anomalyco/opencode) 2.2.5 in
TypeScript (kept on the [`legacy-0.5`](https://github.com/Anysola/code/tree/legacy-0.5) branch). 1.0.0 to 1.0.9
also shipped a terminal app; from 1.1.0 the desktop app is the product. The system prompts, tool descriptions and
other texts the model sees come from OpenCode and are used under the MIT License; see [NOTICE](NOTICE). Anysola Code
is not affiliated with or endorsed by the OpenCode project.

## Development

Rust 1.99.0 (`RUST_VERSION` in `.github/workflows/ci.yml`) and Node 24. On Linux the desktop app needs WebKitGTK
(`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf`).

```sh
cargo test --workspace                 # the agent: crates/core, crates/mcp
cd desktop && npm ci && npm run build  # the window
cd src-tauri && cargo test             # the desktop app's Rust side
cd .. && npx tauri dev                 # run it
```

`ANYSOLA_API_URL` (default `https://api.anysola.com`) points sign-in, config and the model at another API.
[ARCHITECTURE.md](ARCHITECTURE.md) has the shape of the code; [RELEASING.md](RELEASING.md) the release steps.

## License

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
