# Changelog

Anysola Code releases, newest first. OpenCode's changelog up to 2.2.5 is in
[UPSTREAM-CHANGELOG.md](https://github.com/Anysola/code/blob/legacy-0.5/UPSTREAM-CHANGELOG.md) on the `legacy-0.5`
branch.

## Unreleased

- In a Git folder a new session works in its own worktree and branch, so sessions don't change each other's
  files or the main folder. Review shows its changes; Apply makes them one commit on the main folder's branch and
  the session goes on from there; Discard deletes the worktree and branch. Choose "Main folder" to work as before.
- Hooks in a worktree session run from the worktree. Local hook and config files under `.claude/` and
  `.anysola/` that Git ignores are copied in when the worktree is made; later changes to them in the main
  folder count after Recreate.
- A new worktree gets the main folder's ignored `.env` files and runs the folder's setup command (Settings), with
  its output in the conversation; "Start now" doesn't wait for it.
- When the main folder moved on in the same lines, "Bring main in" merges it into the session's branch so the
  conflicts can be resolved there. Edits in the main folder to files an apply touches can be set aside and put
  back.
- A session in its own worktree never writes into the main folder or another session's worktree, in any mode.
  Only its file tools are held to the worktree: shell commands and MCP servers are not confined.
- Applying never overwrites a file the main folder has outside Git (an ignored `.env`): it is set aside with the
  other edits in the way and put back. A crash during an apply puts those edits back at the next start, or says
  which stash holds them.
- Deleting a session whose worktree has changes not applied asks first.
- A branch from an earlier message gets its own worktree with the files as they were at that message.
- A session moved into a worktree by the agent now rewinds and reviews that worktree's files, not the main
  folder's.

## 1.1.8 (2026-11-24)

- A turn that stops at its budget sends a notification, like a finished one.
- A branch keeps its source's title; the branch icon marks it.
- Deleting a session also drops its usage record.

## 1.1.7 (2026-11-24)

- Ctrl K opens a palette: recent sessions first, then commands with their keys (new session, modes, review
  changes, branch, budget, settings).
- A session can have a budget, a share of the 5-hour window. It shows in the header, subagents count toward it,
  and every model call stops there, `/compact` included. "Budget reached" offers to raise or remove it.
- Branch from any of your messages but the first: the conversation up to it is copied into a new session. Files
  are not rewound; the folder stays as it is. A branch keeps its mark after a rename.
- Pressing Escape twice stops a running turn wherever the focus is, not only in the message box.
- Shortcuts work on non-Latin keyboard layouts, and on Turkish Q, Ctrl ç no longer stops a turn.

## 1.1.6 (2026-11-21)

- Settings > Usage names limits plainly: "5-hour limit" and "Weekly limit", "Resets in …", and each saved reset
  says what it resets ("Resets 5-hour and weekly limits") instead of "Starts both windows over".
- A pass over the app's wording: one word for each thing (Git, sign-in, subagents, the task list), and notices
  cut to what you need. The commit button no longer reads "to this commit" when no branch is checked out, the
  hooks notice names the button that stops them, and a session busy elsewhere says "another copy of Anysola Code".

## 1.1.5 (2026-11-20)

Fixes from a second look at 1.1.4's streaming.

- A call cut off while it was being written is kept with its file or command, but a long text in it is shortened
  to its start: a file cut off by the length limit no longer rides along, whole, in every later request.
- A tool call whose input the model wrote with a raw tab or line break inside a string is read as it meant, instead
  of failing and being written again (tool inputs are no longer checked by the API while they stream).
- A call that finishes while the model is still writing is stored before the window hears of it, so its row can't
  stay spinning.
- A long reply grows in fewer, larger steps, which keeps typing and scrolling smooth while it arrives; stopping a
  reply shows what was written at once.

## 1.1.4 (2026-11-20)

The conversation streams the way the model writes.

- A file being written fills in as the model writes it. 1.1.2 drew the lines as they arrived, but the API held a
  tool's input back until each field was whole, so a large file showed only its name for as long as the model
  took to write it (24 seconds for an 8 KB page) and then landed at once. Tool inputs now stream as they are
  generated.
- A command's output shows under its row while it runs: its last lines and a line count, with progress bars and
  colours as a terminal shows them. It used to be a spinner until the command ended.
- A call that finishes while the model is still writing the next one settles at once, instead of spinning until
  the whole message has arrived (1.1.2's known issue).
- A reply's text appears at an even pace instead of in the clumps it arrives in. With "Reduce motion" on it shows
  as it arrives.
- A call stopped or cut off while the model was writing it keeps what was written: its row still names the file or
  the command.
- A conversation started before this version goes on normally: its earlier hidden reasoning is dropped once, since
  the tool definitions changed.

## 1.1.3 (2026-11-20)

Review shows long changes whole.

- With several long diffs open, Review squeezed every file to fit the panel and cut its diff off, and the list
  didn't scroll. Each file keeps its full height now and the list scrolls to the last line of the last file.
- The same for the list of background commands when more than six run, and for the button under a long notice.

## 1.1.2 (2026-11-19)

Tool calls show while the model writes them.

- A file being written shows its name, a line count and its last lines as they arrive; an edit shows its new
  text the same way; a command appears as it is typed, and other tools fill in their one-line summary. It used
  to be a nameless spinner and then the whole call at once. When the call is ready the row becomes the usual one,
  in place. A program the agent writes for itself shows as "Writing a script" with its length, not its text.
- The status line says "Thinking" while the model reasons, and the conversation keeps following new output when
  a row grows on its own.
- A call that was stopped before it ran says what it would have done ("Write") instead of "Wrote", and a call cut
  off when the app closed is shown as interrupted after reopening.
- The window's minimize, maximize and close buttons light up as round buttons like the rest of the header: Close
  was cut by the panel's rounded corner.
- Known: a call that has finished keeps its spinner until the model's whole message has arrived.

## 1.1.1 (2026-11-19)

What a project's files can change about your own rules, closed further, and the approval pages finished.

- Your "deny" always wins. A project's rule can no longer turn a refusal of yours into a question (1.1.0's known
  limit), and a project's allow rule cannot lift it even after you allowed that project's rules. The same holds for
  policies: a project's policy never lifts one of yours that refuses. A project's rules that ask or deny apply at
  once, as before.
- Subagents can no longer do more than the session that started them. Each request of a subagent is decided by
  the strictest of its own rules and those of every session above it, so a General subagent started in Plan mode
  cannot change files outside the plan folder, and your rules for Build or for an agent of your own also hold for
  the subagents it starts. A question raised that way says it was asked by a subagent. The mode you are in now
  governs the whole tree: switching Plan to Build also lets subagents still running make changes.
- A project cannot change which agent your sessions run on: its `default_agent` is ignored when you set one
  yourself, and it cannot turn off, hide or change the kind of a built-in agent or one of yours. A project's
  command that names another agent runs as a separate task under your session, bound by it, instead of switching
  your session (for example out of Plan mode); your own commands switch as before. A project cannot replace one
  of your own commands by defining one with the same name. A command that runs as a separate task shows what
  you typed and then the task as a subagent you can open.
- Shell commands inside a project's command template (``!`…` ``) follow your permissions: a refused one does not
  run, and one your rules ask about asks you first. They used to run without a question. Stop while that
  question waits cancels the whole command, and a command with a step you declined says so in its message.
- Rules that name files from the project's root, such as `read secrets/*`, also hold in a worktree of the project
  and when you open a subfolder of it. A permission question can list a file under more than one name because of
  this. Search results leave out files your `read` rules refuse or ask about and say how many were left out.
- Rules apply to the file a path really leads to. A symlink in a repository, or another spelling of the same
  path, no longer gets around a rule that refuses or asks, and a symlink that leads out of the project counts as
  outside it: reading or writing through it asks like any other outside path.
- The agent can only rename or move its own session, and only move it to the project's root or one of the
  project's worktrees. A project cannot choose where worktrees are created, and no worktree is created inside or
  around the folders your own agents, commands, skills and settings load from.
- You can take back what you allowed a project. Settings → Permissions, Instructions, MCP servers and Advanced
  have Stop allowing, as Hooks has; it applies at once in open sessions, from the agent's next step: the rules
  stop allowing, the MCP servers stop, the downloads are no longer used. You can allow it again from the same row.
- Allowing something in Settings shows it as in effect right away; it used to disappear until Settings was
  reopened. Settings → Permissions also lists rules written for a single agent (`agents.<id>.permissions` and
  agent files) and names the agent, while they wait and once in effect.
- An instruction URL, a skill URL or a repository that cannot be downloaded stays listed with the reason
  ("404 Not Found", "timed out") and Try again; it used to vanish. A failed instruction URL is also tried again
  the next time the config loads. Instruction URLs larger than 1 MiB or that are not text are skipped with a
  reason.
- The approval question says when a project's downloads happen: when you open the project and when its config
  changes; repositories are updated at most once a day. It used to say "each time".
- The header's change count in a git project is the net difference, the same numbers as Review: editing one line
  three times counts once. Outside git it still adds up every edit.
- Discord status uses Anysola Code's own Discord application, so turning it on in Settings → Discord is enough;
  the application ID there is an optional override. It stays off until you turn it on.
- In Plan mode the agent tells you how to leave it the way the app names it (Build this plan, or another mode).
- Not changed, so you know: a project's config can still choose the model, replace an agent's prompt, and turn
  snapshots off; a child session follows its parent's current mode, not the mode it was started in; and a
  shell command is judged by your `shell` rules only, so `cat` reaches a file a `read` rule refuses unless a
  `shell` rule stops it.

## 1.1.0 (2026-11-18)

The terminal app is retired. Anysola Code is the desktop app for Windows and Linux.

- The `anysola` command line and its TUI are no longer built or released: no CLI archives, no install scripts and
  no new npm packages. 1.0.9 was its last version: an installed terminal app no longer gets answers from Anysola,
  only a note that it has been retired.
- If you used the terminal app, download the desktop app from www.anysola.com/code. Your sessions, sign-in and
  settings in `~/.anysola` carry over; then remove the old binary with `anysola uninstall` or
  `npm rm -g @anysola/code`.
- Not in the desktop app: `anysola run` (one prompt without a window, used in CI and by `Anysola/code-action`),
  the editor integration (`anysola acp`), importing a session file, and the import of 0.5.x data on first start.
- Messages that told you to run `anysola login` now say to sign in again.
- A project's MCP servers and formatters need your approval once, like its hooks. When a project's config defines
  them, the app asks before starting them, in one question per project file that lists its hooks, its MCP servers
  (with the headers and environment they would send) and its formatters. Until you allow it those servers are not
  started and those formatters do not run; your own servers and formatters are not affected, and a project file
  that redefines one of your servers says so and changes nothing until you allow it. A change to them asks again.
  Settings → MCP servers shows a project's servers that wait, with Allow.
- A project only tightens your rules until you say otherwise: the rules in its config, its agents and its agent
  files that would let the agent act without asking (and its policies that would turn off your own servers or
  providers) wait for your approval, in the same question; its rules that ask or deny apply at once. Settings →
  Permissions lists what waits, and no longer shows a project's unapproved rules as if they were in effect.
- A project's config makes the app download nothing until you allow it: instruction URLs and skill URLs are not
  fetched and git references are not cloned before that. Settings → Instructions lists what waits. A project's
  instructions can only name files inside the project; your own config may still name any file.
- A project starting sessions on an agent of its own is asked about with its rules.
- Opening a folder no longer reads a project file that is not a regular file or is larger than 1 MiB (config) or
  4 MiB (instructions, agents, commands, skills): a config linked to a device used to exhaust memory.
- A project's config can no longer choose the shell that runs commands: `shell` only works in your own config.
- The agent's edit and write tools also ask before changing your global `AGENTS.md` and the agent, command and
  skill folders in your config directory. `{file:…}` in a config reads at most 1 MiB, and only regular files from a project's config.
- Names and commands from a project are shown with invisible and direction-changing characters written out, and
  a header or environment value that takes one of your files or variables is shown as the project wrote it
  (`{file:…}`, `{env:…}`).
- Known limit: a project's rule that asks can still turn one of your denials into a question, because the last
  matching rule decides.

## 1.0.9 (2026-11-18)

Hooks: commands of your own that run around the agent's work, in the terminal, the desktop app and `anysola run`.

- Hooks. A hook is a command you configure under `hooks` in your config; it gets the moment as JSON on stdin and
  answers with its exit code and what it prints.
  - Five moments: before a tool runs (it can stop the call), after a tool runs (it can tell the agent what it
    found), when you send a message (it can stop it or add context), when the agent would finish (it can keep it
    working, for example until the tests pass) and when a session starts (it can add context).
  - The contract is Claude Code's, so hook scripts written for it run unchanged: the same `hooks` block, the same
    JSON on stdin and stdout, exit code 2 blocks. Tools are named both ways (`shell` is also `Bash`, `path` is also
    `file_path`).
  - Hooks only tighten. A hook can block, ask you, add context or keep a turn going; it cannot allow something your
    rules, your organization's policies or the session's mode would not.
  - They cover every route to a tool: calls from Code Mode programs and from subagents fire them too. They see
    the input the tool actually runs, with `file_path` resolved through symlinks, and a `PreToolUse` hook that
    cannot be run (its command or script is missing) blocks the call instead of letting it through.
  - A project's hooks run only after you allowed them, and again after they change. In the open session the
    version you allowed keeps running until you allow the change (refusing it does not switch the guard off), so
    the agent cannot drop a project's guard mid-session by editing the file. Hooks in your own config
    (`~/.anysola/config.json`) run without asking; a guard that must hold against the agent belongs there.
  - Desktop: Settings → Hooks adds, edits and removes your hooks with a form (three starters to begin from), shows
    a project's hooks with their state, and turns hooks off. The conversation asks about a project's hooks before
    your first message, shows what a hook did under the tool it ran for, and puts a message a hook stopped back in
    the message box.
  - Terminal: the same lines under each tool, a dialog for a project's hooks, `/hooks` to see and decide, and
    `anysola hooks` / `anysola hooks trust` / `anysola hooks deny` outside the TUI. `anysola run` skips hooks that
    are not allowed and says so once.
  - Not in this release: other moments (before a compaction, when a subagent finishes, …), hooks that are not
    commands, a hook allowing a call or rewriting its input, and reading `.claude/settings.json`.
- The agent's edit and write tools always ask before changing your config files, the files your hook commands
  run, the database or the desktop app's preferences, whatever the rules allow.
- A session forks as often as you like. The second fork of the same session failed with
  `UNIQUE constraint failed: message.id` and left an empty "(fork #2)" session behind; a fork that cannot be made
  now leaves nothing.

## 1.0.8 (2026-11-18)

Anysola Code Desktop for everyday use: nothing waits out of sight, nothing typed is lost, and nothing needs a
terminal.

- Nothing waits out of sight.
  - A permission, a question or a form that is waiting comes back when you reopen its session or the app catches
    up; it used to be gone while the turn waited for it forever.
  - MCP servers can ask with a form (elicitation): the conversation shows it with a control per field, and you
    submit, decline or dismiss it. Forms were declined without being shown.
  - A message sent while a turn runs shows as a chip above the message box: send it now (it joins the running
    turn), edit it, or remove it. After a stop or a failure, what was still waiting stays there with Send.
  - Commands the agent left running in the background are listed above the message box, each with Stop.
  - While it works, the conversation says what runs now and for how long; a finished turn says how long it took;
    Stop answers at once and leaves "Stopped." in the conversation. A long-running command shows as a row while
    it runs.
  - The footer shows how full the model's context is, with Compact when it fills.
  - Closing the window while a turn runs asks first. An update waits for the running turn, shows its download,
    and stops MCP servers and background commands before it restarts.
- Nothing typed is lost.
  - A draft, its images and its review comments belong to their session; its text survives a restart.
  - A message that could not be sent goes back into the message box.
  - Arrow up recalls what you sent; a long paste folds into a chip; Escape stops a turn on the second press.
- Errors say what to do. A full usage window shows when it starts over, with "Use a saved reset" and "See usage"
  at the message box; a lost connection says so and retries; an expired sign-in offers Sign in again; a model
  that is not available opens the model list. A database that cannot be opened has its own screen.
- Files and folders open in your editor: VS Code, Cursor, Windsurf, Zed, the JetBrains IDEs, Sublime Text and
  Notepad++ are found on their own; Settings › General picks one. Paths in the conversation, in Review and in
  links open there; each also has Reveal in folder and Copy path. With no editor, a file is shown in its folder.
- Programs are found when the app starts from the Start menu or a desktop launcher: the app reads your shell's
  PATH (Linux) or the registry's (Windows), so `npx`, `uvx`, `git` and `gh` are found as they are in a terminal.
- Git without a terminal.
  - Commit lists the changed files with checkboxes and names where the commit goes.
  - Conflicts are listed, each opens in the editor, and committing waits for them: Commit used to commit the
    conflict markers.
  - A missing name and email are asked for in place; Pull appears when the remote is ahead; a missing Git says so
    with a link.
  - Errors are sentences (no credentials, a rejected push, a signing failure, a locked index), and nothing waits
    forever on a prompt no one can see.
  - Open pull request works for GitLab, Bitbucket and Azure remotes, picks the branch's own remote, and opens
    the existing pull request when there is one.
- Finding your way.
  - The sign-in and home screens say what the app is; a new session offers starters and names its mode.
  - Shortcuts: Ctrl B folds the sidebar, Ctrl L goes to the message box, Ctrl . stops, Alt Up/Down moves between
    sessions, Ctrl Shift N starts a session in the same folder, Ctrl Shift D opens Review, Ctrl plus/minus zooms,
    and Ctrl / lists them all.
  - Icon buttons have tooltips; menus work from the keyboard and stay inside the window at any size.
  - The session's header has a menu (rename, open in the editor, open folder, copy path, export, delete);
    deleting a session can be undone for a few seconds.
  - The account menu links to help, feedback and what's new.
  - A reply can be copied; sent images show as thumbnails and open full size.
- MCP: signing in can be cancelled (the browser step no longer holds its port), a server that fails or needs a
  sign-in says so in the conversation, and a command that is not found says which program is missing.
- The terminal app answers an MCP form's choices with the option's value and supports forms with several
  choices.

## 1.0.7 (2026-11-17)

Settings for Anysola Code Desktop, and a conversation that starts folded.

- Desktop has a Settings screen: the row above your plan, or Ctrl+comma.
  - Account, Usage and Billing: your plan, the devices signed in to your account (revoke one from here), both usage
    windows with when they start over, banked resets you can use, and requests per day and per model.
  - General, Appearance, Conversation, Notifications and Keyboard: the default folder, the mode new sessions start
    in and the default model; a light theme and one that follows the system, text size, conversation width, wrapped
    code and reduced motion; which events notify you; the send key (Enter or Ctrl+Enter); every shortcut.
  - MCP servers: add a remote or local server, switch it off, sign in and out, remove it. Each shows its state, its
    tools and prompts, its error in full, and the file that defines it. The terminal's `anysola mcp add` is no
    longer the only way.
  - Skills, Commands and Instructions: what the agent picks up for the open folder, the file each one came from,
    and every place that is looked in. A switch turns Claude Code's skill folders off. Your personal instructions
    (`~/.anysola/AGENTS.md`) are edited here.
  - Permissions: the answers you saved with "Always allow", per project, with Remove.
  - Advanced: web search, the task list, subagent depth, automatic summarizing, snapshots and the shell, written
    to `~/.anysola/config.json` with its comments kept; a setting a project file overrides says so.
  - Discord: show what you are working on as your Discord status. Off by default; it needs a Discord application
    ID.
  - Data and About: where everything is kept, cache sizes with Clear, exporting a session, deleting all sessions,
    the log file, the version and links.
- In a desktop conversation, a file change and a command's output are one row that opens on a click; long code
  blocks and messages are cut with "Show all"; a permission or question card keeps its buttons on screen however
  long the diff is; Review opens every file only when six or fewer changed. Conversation settings change each of
  these.
- Desktop writes to `~/.anysola/log/anysola.log`, as the terminal does.
- A config file with a comment after its last entry's comma (`"a": 1, // note` before `}`) loads; it was refused
  as invalid JSON.
- `anysola mcp add` refuses a command whose program is empty.

## 1.0.6 (2026-11-17)

The first release of Anysola Code Desktop, and a scan of the paths only a real install runs, Windows above all.

- Anysola Code Desktop for Windows (installer) and Linux (AppImage, .deb) is on the release page: your sessions
  from the terminal, with the conversation, diffs, permission cards and review in a window. It shares its
  sessions and sign-in with the CLI and updates itself.
- On Windows, the output of PowerShell and cmd commands keeps Turkish and other non-ASCII letters; it used to
  come back as `�`. The terminal is left on the UTF-8 code page afterwards.
- `anysola update` no longer says "Updated" when the installer could not be downloaded, and when the latest
  version cannot be checked it stops instead of installing anyway (`--force` installs regardless).
- `anysola login` signs in again when the server rejects the stored token, instead of answering "Already signed
  in". `anysola login --force` keeps the old token until the new sign-in has worked.
- `anysola run` and editors over ACP say "Not signed in. Run `anysola login` or set ANYSOLA_TOKEN." or "Cannot
  reach …" where they said "No model is available for session …".
- Formatters installed in `node_modules/.bin` (prettier, biome, oxfmt) and formatter commands that start with
  `npx` run on Windows; they were silently skipped.
- The system proxy set in Windows or macOS settings is used.
- Alt+V pastes an image from the clipboard on Windows, where the terminal keeps Ctrl+V for itself, and a
  clipboard whose image cannot be read still pastes its text.
- An attached image the model cannot take no longer breaks the session for good: BMP is converted, and SVG,
  TIFF, HEIC, AVIF and ICO are refused before the message is stored.
- Two prompts sent to one session at the same moment no longer start two turns; the second waits for the first.
- Restarting after `/update` starts the new version on Linux. `$EDITOR` may have spaces in its path. Prerelease
  versions order by their numbers (`rc.10` after `rc.9`).

## 1.0.5 (2026-11-15)

- A turn could fail with "This model does not support assistant message prefill. The conversation must end with a
  user message." on current models (seen with Claude Haiku 5.5). A request now never ends on an assistant turn: the
  step-limit notice goes in with the tool results instead of as an assistant message, and any request whose history
  ends on the model's own reply gets a short user turn after it.

## 1.0.4 (2026-11-09)

The rest of the scan behind 1.0.3: the remaining hangs, races and rough edges.

- The edit tool refuses a file that is not valid UTF-8 instead of silently rewriting every odd byte as `�`, and
  edits and writes land through a temporary file and a rename, so an interrupted write cannot leave a truncated
  file behind.
- `anysola mcp auth` and the MCP sign-in no longer hang when the browser opens an idle connection to the callback
  port; the sign-in gives up after five minutes, and the OAuth metadata, registration and token requests time out.
- A local MCP server whose process died is started again when it is next needed (up to three times), instead of
  staying failed until you reconnect it by hand.
- Pasting from the clipboard, writing into the terminal pane and decoding a large image no longer block the UI; the
  first keys typed into `$EDITOR` are no longer swallowed by the TUI.
- The crash screen's Restart continues in the directory you had moved to with `/cd`, not the one you started in,
  and no longer reopens a session whose opening crashed. A second `/cd` while one is still changing is refused, and
  a prompt typed meanwhile waits instead of going to the old core.
- Keys typed in the moment a permission or question appears are not taken as its answer (300 ms grace, Esc still
  works), and a prompt can be answered from the keyboard while the terminal pane has focus.
- The mouse wheel scrolls what is under the pointer: the diff viewer, the terminal pane's scrollback, or the
  conversation. `G` jumps to the end of the diff. A drag no longer sticks after the window loses focus.
- Copying a selection with CJK or emoji text no longer inserts a space after each wide character, and clicks on
  rows with such text land on the right target.
- `/connect` finishing after you closed its dialog no longer closes whatever dialog you had opened instead, and an
  abandoned sign-in is not stored.
- An interrupted turn's undo waits until the turn has stopped. After a burst of events the session's busy state is
  re-checked, so a lost event cannot leave a tab spinning for good.
- Starting two `anysola` processes at the same moment on a fresh install no longer fails one of them on the schema
  migration; a huge single-line file no longer makes the read tool buffer it whole; a broken system certificate
  store no longer makes every command panic; an `oauth.callback_port` above 65535 is reported instead of wrapped.

## 1.0.3 (2026-11-09)

Fixes from a scan for anything that interrupts a session: hangs, loops, lost input and crashes.

- A model stream that was cut after the answer had started (a dropped connection, or an `error` event from an
  overloaded relay) was resumed again and again with no pause and no limit, dozens of requests a second, each one
  counted against your usage, until you pressed Esc twice. It is now resumed three times with a growing pause, then
  the turn fails with the error.
- Turkish and other multi-byte characters were sometimes corrupted into `��` when the network split them across two
  packets. This reached the text on screen, the saved conversation and, through the write and edit tools, your files.
- A stalled title request kept the session spinning with the answer already on screen, and the next prompt waited
  behind it. A turn now waits at most 5 s for its title, and never once interrupted.
- A connection that went silent (sleep, Wi-Fi change) left the turn on "thinking" until the kernel gave up; the stream
  now fails and retries after 120 s without a byte, and the connection carries keepalives. The pause before a retry
  and the retry itself stop on Esc.
- Starting offline for the first time: every prompt failed with "No model is available" for ten minutes, the typed
  text was gone, and the hidden prompt went out with the next one once the network was back. The model list is now
  fetched again from 15 s on and right when a prompt needs it, the prompt comes back into the composer, and nothing
  is sent unseen. The composer is also restored when a send fails, and through the budget gate with attachments.
- Answering the second question of a multi-question prompt could crash the UI when the first had more options.
- A permission or question asked by a subagent while you were on the home screen or another idle tab was never
  shown, and the turn waited forever. It is now shown in the nearest open session, or the root session's tab.
- The session list stayed at "Searching sessions…" and the shell output view froze until the next key while no
  session was busy.
- A panic in a background task disabled the terminal's raw mode under the running UI; the terminal is now restored
  only when the UI itself crashes, and every panic is written to the log with its location.
- Closing the terminal window or `kill` left the shells the agent had started (dev servers, test runs) running;
  the UI now shuts down the same way as on `ctrl+c`.
- The `@` file list is built off the UI thread (big trees and network disks froze the input), and background shell
  output reads only its last MiB instead of the whole file every second.
- A session could stay marked busy for the rest of the process after a turn crashed; a prompt refused because another
  process drives the session no longer goes out later from that process; a message write that raced another process
  is no longer lost silently.
- Importing 0.5.x data skips a row it cannot read instead of failing the whole import on every start.
- A formatter that hangs is killed after 60 s; MCP HTTP connections time out; a mermaid diagram the renderer cannot
  draw shows its source instead of a crash screen; a `Retry-After` header that is not a number no longer crashes.

## 1.0.2 (2026-11-08)

- The `/` and `@` popup in a session is drawn on top of the conversation again. Lines of the transcript no longer
  show through its rows, and clicks on it no longer reach what is behind it.

## 1.0.1 (2026-11-06)

- `/reset` spends a banked reset whenever you choose, after a confirm that shows how many you have, when the next
  one expires and which windows it clears. `/usage` offers the same with `r` (or a click) when your bank holds one.
- Anysola Code speaks as itself: `anysola --version` prints `Anysola Code <version>`, and requests, MCP servers and
  the model see Anysola Code instead of OpenCode. Version 1.0.1 needs the Anysola server to accept its new
  identity; 1.0.0 keeps working as before.
- New file and variable names: `anysola.json` / `anysola.jsonc` and a project `.anysola/` folder for settings,
  agents, commands, skills and themes, `ANYSOLA_*` environment variables, plans in `~/.anysola/plan` and scratch
  files in `<tmp>/anysola`. The old `opencode.json[c]`, `.opencode/`, `~/.config/opencode/` and `OPENCODE_*` names
  are still read, so nothing needs to change; where both exist, the new name wins. `anysola mcp add` writes to
  `anysola.json` unless the project already has one of the other files.

## 1.0.0 (2026-11-06)

Anysola Code is rewritten in Rust. Everything 0.5.1 does keeps working the same way; [PARITY.md](PARITY.md) lists
each feature and how it was checked.

- One small binary: about 10 MB instead of 196 MB, `anysola --version` in about 3 ms instead of 80 ms, and the TUI
  runs as one process with no background service.
- The first start imports your 0.5.x sessions, settings and sign-in. The old files are left as they were.
- Usage limits replace credits: a 5-hour and a weekly window shared with Anysola Chat, shown as percentages with
  their reset times in the status line, `/usage`, `anysola usage` and `anysola status`, with a warning at 80%. When
  a window is full, the TUI offers a banked reset and sends your prompt again; `anysola limits`, `anysola limits
  redeem` and `anysola run --use-banked-reset` do the same from the command line. `/budget` and `run --budget` now
  take a percent of the 5-hour window (budgets set in credits are not carried over), `/credits` is gone, and the
  model picker no longer shows a per-turn estimate.
- The mouse works across the TUI: tabs (click, middle-click or ✕ to close, `+` for a new session), dialogs, permission
  and question prompts, the dock, toasts and collapsible blocks. Clicking outside a dialog closes it.
- Inline image previews in the composer, in your messages and under tool results (turn them on in `/settings`), and
  the image viewer shows the image.
- Mermaid blocks are drawn as diagrams: flowcharts, sequence, state, gantt, timeline and git graphs.
- Windows: multi-line paste arrives as one paste, the clipboard keeps Turkish and other non-ASCII text, `npx` MCP
  servers and `$EDITOR` resolve through `PATHEXT`.
- npm: `@anysola/code` no longer needs its install script to run; the `anysola` command finds the platform binary
  on its own.
- Known gap: when the terminal switches between light and dark while Anysola Code is open, the theme follows on the
  next start.
- The 0.5.x TypeScript source moved to the `legacy-0.5` branch.

## 0.5.1

- The model picker says what each model is for ("Auto: Picks Haiku or Sonnet per turn", "Default", "Fastest")
  instead of repeating Anysola, and "~1 credit/turn" is singular.
- The theme setting's default reads `anysola`.
- Web search credits count toward the session total in the status line and the `anysola run` summary.
- An `install-test` workflow runs the install scripts on Linux, macOS and Windows after each release.

## 0.5.0

- New look: the ANYSOLA wordmark on the home screen and in the exit summary, the Anysola theme on the web's warm
  graphite palette with one sand accent, the directory's latest sessions under the composer, and the plan with its
  renewal date in the bottom bar.
- Web search runs through Anysola and your credits (each search asks for permission; `"websearch": false` turns it
  off). The third-party search providers are gone.
- Models: `Auto` picks Haiku for light turns and Sonnet for real work, and an `economy` variant runs a model at lower
  effort, when Anysola turns them on for your plan.
- `anysola run` ends with what it spent and the balance left. `/account` opens your account, `/feedback` opens a
  support ticket.
- CI: tokens made in Account > Apps > Code (`ANYSOLA_TOKEN`) and the `Anysola/code-action` GitHub Action for PR
  reviews and `/anysola` on issues.
- MCP sign-ins name Anysola Code, with Anysola's OAuth client metadata and callback page.
- Removed: `anysola pair`, `auth` (use `login`, `logout`, `status`; `mcp auth` for MCP servers), `api`, `mini`,
  `stats` (use `anysola usage`), `debug agents`, `debug heap-dump`; in the TUI `/pair`, `/stats`, the OpenCode
  Console limits and the release highlights. `anysola acp --login` signs in to Anysola.
- Plugins cannot add a model provider or point Anysola's models at another server.
- About 18 MB smaller.

## 0.4.0

- Install without npm: `curl -fsSL https://www.anysola.com/install | sh` (macOS, Linux) and
  `irm https://www.anysola.com/install.ps1 | iex` (Windows) install the binary from the GitHub release into
  `~/.anysola/bin`, checked against `SHA256SUMS`. `anysola update` and `anysola uninstall` know this install.

- Session budgets: `/budget <credits>`, `/budget off`, `/budget default <credits>` in the TUI and
  `anysola run --budget <credits>`; the status line shows `session spent/budget`, and a session over its budget
  asks before the next turn.
- The cost of the current turn and the session total now count streamed answers (they stayed at 0 before); the
  totals are kept across restarts.
- `/usage` and `anysola usage` show the cache hit rate.

## 0.3.2

- Links to the product page (Open docs, package homepage, web fetch user agent) use https://www.anysola.com/code.

## 0.3.1

- Fixes /usage, /credits and /devices crashing after an update; older background services are restarted.

## 0.3.0

- Status line shows credits remaining, the cost of the current turn and the session total.
- TUI commands `/credits`, `/usage` and `/devices`; CLI commands `anysola usage [--days 7|30]`, `anysola devices`
  and `anysola devices revoke <id>`.
- Banners for a low balance and for account events.
- Version check against the Anysola server, with a notice when a newer release exists, and `anysola update`.
- Traffic to upstream services disabled: models.dev, the web UI, remote tunnel and pairing are off.
- Anysola theme as the default.
- Exit summary when the TUI closes.

## 0.2.0

- Fork of OpenCode v2.2.5.
- Branding: the CLI, TUI and binary are Anysola Code / `anysola`.
- Anysola is the only provider.
- Built-in sign-in: `anysola login`, `anysola logout`, `anysola status` and a device sign-in on first run.
- Packaging as `@anysola/code` with per-platform packages; the self-updater is disabled and updates come from npm.
