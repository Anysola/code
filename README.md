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
`UserPromptSubmit` when you send a message, `Stop` when the agent would stop, and `SessionStart`. The format is
Claude Code's, so existing hook scripts work: the event comes as JSON on stdin, exit code 2 blocks with stderr as
the reason, and JSON on stdout can add context or deny a call. Settings → Hooks adds, edits and removes them.

Hooks only tighten: a hook can block a call or have you asked about it, never allow something your permissions
would not. Hooks in `~/.anysola/config.json` run right away; a project's hooks run only once you approve them in
the conversation, and again after they change. The agent's edit and write tools always ask before changing your
config files, the files your hooks run and the app's database.

## What a project can change

A repository's own config is read like yours, but it only tightens until you say otherwise. Its rules that ask
or deny apply at once; its rules that allow, its MCP servers, formatters, hooks and downloads (instruction URLs,
skill URLs, git references) wait for your approval, asked once per project file in the conversation and again
after a change. Settings → Permissions, Instructions, MCP servers, Advanced and Hooks list what waits and what you
allowed, with Stop allowing.

Some things a project can never do, allowed or not: lift a rule or policy of yours that refuses; choose the agent
your sessions run on when you chose one; turn off, hide or retype a built-in agent or one of yours; replace one of
your commands; choose the shell or where worktrees are created. Rules name files, not spellings: a rule on
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
(`$CLAUDE_PROJECT_DIR`). Local hook and config files under `.claude/` and `.anysola/` that Git doesn't track
(ignored or untracked) are copied in from the main folder when the worktree is made; later changes to them in the
main folder aren't picked up until Recreate. The file tools also never write the worktree's own `.git`.

A worktree is not a sandbox. Only the file tools are held to it: shell commands run as anywhere else and can
reach any folder, MCP servers run with the main folder as their folder and aren't confined either, and Git in a
worktree shares the repository's branches and objects (it can move or push them).

A branch made from an older session (from before worktrees) starts from the main folder's current commit with
only that session's own files as they were then; other uncommitted changes of the main folder aren't in it.

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
