# Anysola Code: architecture

Anysola Code is a desktop app (Tauri 2) for Windows x64 and Linux x64. The agent is a Rust library; the app runs it
in its own process and draws the window with React. 1.0.0 to 1.0.9 also shipped a terminal app (`anysola`, CLI and
TUI) on the same library; it was retired in 1.1.0 and its code is in the history up to `v1.0.9`.

## Process model

One process. The agent loop, tools, storage and the window's backend run on one tokio runtime; the window is a
WebView that talks to it through Tauri commands and one event channel. Child processes exist only for work that is
inherently external: the shell commands the shell tool runs, stdio MCP servers, formatters, hooks, git and the
editor the user opens files in.

Sessions live in `~/.anysola/anysola.db` (SQLite, WAL). A process that drives a session marks it busy, so a second
app (or a 1.0.x terminal app still installed) cannot run the same session at the same time.

## Layout

```
crates/            the agent (one Cargo workspace)
  core             config, storage, gateway client, agent loop, tools, permissions, hooks, compaction,
                   snapshots, instructions, skills, commands, agents, Code Mode, event bus
  mcp              MCP client (stdio, streamable HTTP, SSE) and OAuth
  testkit          fake Anysola gateway (scripted SSE), temp homes; a dev-dependency of core and mcp
desktop/           the app (its own Cargo workspace and npm package)
  src/             the window: React 19, HeroUI, Tailwind; bridge.ts is the contract with the Rust side
  src-tauri/       Tauri shell and commands on top of core and mcp (account, sessions, git, settings,
                   MCP servers, hooks, updates)
spec/              0.5.1 parity inventories of core behaviour, with the captures the tests compare against
                   (spec/verbatim)
```

Core exposes one API (`anysola_core::api::Core`): open or continue a session, send a prompt, abort, answer a
permission, question or form, and a broadcast stream of events (message and part updates, permission requests,
status, usage). Nothing in core knows about the window. The app keeps one `Core` per open folder.

`desktop/src-tauri` is a separate Cargo workspace so `crates/*` builds and tests without WebKitGTK and keeps its
own lockfile; it depends on `crates/core` and `crates/mcp` by path.

## Stack

| Need | Choice | Note |
|---|---|---|
| App shell | Tauri 2 | WebView2 on Windows, WebKitGTK on Linux; updater, dialog, notification, single-instance, window-state plugins |
| Window | React 19 + HeroUI 3 + Tailwind 4, Vite | |
| Async | tokio | |
| Code Mode | rquickjs (QuickJS) | the `execute` tool |
| HTTP | reqwest + rustls | no OpenSSL; same TLS on every platform |
| SSE | hand-written parser | Anthropic stream plus the `anysola_usage` event |
| Shell parsing for permissions | port of 0.5.1's scanner | same accept/reject behaviour |
| Storage | rusqlite, bundled SQLite | identical engine on every platform |
| Search | ripgrep's crates (grep-searcher, grep-regex, ignore, globset) | no bundled `rg`; results sorted by path |
| Diff | port of jsdiff (`tool/edit/diff.rs`) | edit metadata patches equal 0.5.1's byte for byte |
| HTML to Markdown | port of 0.5.1's converter | webfetch output matches the captured 0.5.1 output |
| Images | image (png, jpeg, webp, gif) | resized before sending |

## Contracts that do not change

- The `/v1/code/*` requests and headers the gateway accepts (`spec/protocol.md`); model calls send the user agent
  `anysola-code/<ver>/desktop`.
- `/v1/code/version` (Admin's latest and minimum version) gates updates.
- The release's asset names, `latest-desktop.json`'s shape and the updater key (`RELEASING.md`): installed apps
  from 1.0.6 on find updates through them.
- The `~/.anysola` layout and the session database, which 1.0.x terminal installs share.

## Testing

- Unit tests next to the code, in core, mcp and the app's Rust side.
- Integration tests in `crates/core/tests` and `crates/mcp/tests` drive core in-process against `testkit`'s fake
  gateway; `requests.rs` replays the 0.5.1 request captures of `spec/verbatim` and compares body and headers.
- The window has no unit tests; `npm run build` type-checks it. The app is tried end to end under Xvfb with a fake
  gateway before a release.
- CI (`.github/workflows/ci.yml`): fmt, clippy and tests of both workspaces and the window's build, on Windows x64
  for branch pushes and on Windows x64 and Linux x64 for tags and manual runs.
