# Anysola Code (Rust)

A from-scratch Rust rewrite of Anysola Code 0.5.1 (an OpenCode 2.2.5 fork in TypeScript/Bun). The target is the
same product, feature for feature: what a 0.5.1 user sees and uses keeps working, and existing session history is
imported. `PARITY.md` is the checklist that defines "done"; this file is the shape of the code and its budgets.

## Why

Measured on linux-arm64, 0.5.1:

| | 0.5.1 (TS/Bun) | Rust budget (CI fails above) |
|---|---|---|
| Binary | 196 MB (88 MB tar.gz), ~99 MB of it the Bun runtime | 25 MB stripped, 10 MB archive |
| `anysola --version` | 80 ms, 80 MB RSS | 15 ms, 10 MB RSS |
| Idle TUI | 3 processes, ~340 MB RSS | 1 process, 40 MB RSS |
| TUI first frame | not measured | 150 ms |

A dependency probe with the whole stack below (tokio, reqwest/rustls, ratatui, rusqlite bundled, syntect with
default syntaxes, ripgrep crates, tree-sitter-bash, image, rmcp, htmd, nucleo) builds to 10.7 MB stripped, 4.6 MB
gzipped, starts in under 10 ms at 4 MB RSS on linux-arm64. The budgets leave room for our own code on top of that.

Budgets are checked by `script/budget.sh` in CI on every push (size of the linux-x64 release binary and archive,
`--version` time and RSS, idle TUI RSS against the fake gateway). A dependency that spends a large share of the
size budget needs a line in this file saying why.

## Process model

One process. The TUI, the agent loop, tools and storage run in the same address space on a tokio runtime. Child
processes exist only for work that is inherently external: the shell commands the bash tool runs, stdio MCP
servers, language servers, formatters.

0.5.1 runs a background `serve --service` (one or two per TUI, sometimes outliving it). The Rust version has no
service. Several `anysola` processes may run at once (two terminals, a TUI and `anysola run`); they coordinate
through SQLite in WAL mode, and a session that is being driven by one process is marked busy for the others.

## Crates

```
crates/
  anysola        bin: argument parsing, subcommands, wiring
  core           config, storage, provider, agent loop, tools, permissions, compaction, snapshots,
                 instructions, skills, commands, agents, event bus
  tui            ratatui + crossterm front end, talks to core through its in-process API
  mcp            MCP client (stdio, streamable HTTP, SSE) and OAuth
  acp            Agent Client Protocol server for editors (`anysola acp`)
  lsp            language server client: detection, download, diagnostics
  migrate        import of 0.5.x data (opencode.db, config, token, KV)
  testkit        fake Anysola gateway (scripted SSE), temp homes, helpers for integration tests
```

Core exposes one API that the TUI, `run`, and ACP all use: open/continue a session, send a prompt, abort, answer a
permission or question, and a broadcast stream of events (message and part updates, permission requests, status,
credits). Nothing in core knows about the terminal.

## Stack

| Need | Choice | Note |
|---|---|---|
| Async | tokio | |
| Code Mode | rquickjs (QuickJS) | the `execute` tool |
| HTTP | reqwest + rustls | no OpenSSL; same TLS on every platform |
| SSE | hand-written parser | Anthropic stream plus the `anysola_credits` event |
| CLI | clap (derive) | |
| TUI | ratatui + crossterm | Windows console included |
| Markdown | pulldown-cmark | |
| Highlighting | syntect (fancy-regex) + two-face extra syntaxes | covers the 39 languages 0.5.1 highlighted without per-language C grammars; built in, nothing downloaded |
| Shell parsing for permissions | port of 0.5.1's hand-written scanner | same accept/reject behaviour |
| Storage | rusqlite, bundled SQLite | identical engine on all platforms; reads the 0.5.x database for import |
| Search | ripgrep's crates (grep-searcher, grep-regex, ignore, globset) | no bundled `rg`; results sorted by path |
| Diff | similar | |
| Fuzzy matching | nucleo-matcher | |
| HTML to Markdown | port of 0.5.1's converter | webfetch output must match the captured 0.5.1 output |
| Images | image (png, jpeg, webp, gif only) | resize before sending, as photon did |
| MCP | rmcp | |
| ACP | agent-client-protocol | |

## Contracts that do not change

The gateway, Account, Admin, install scripts and the GitHub Action stay as they are. The Rust binary keeps:

- the `/v1/code/*` requests and headers the gateway accepts (see `spec/protocol.md`),
- `anysola run` flags, output and exit codes that `Anysola/code-action` relies on,
- release archive names, `SHA256SUMS`, the `install` / `install.ps1` flow and `~/.anysola/bin`,
- the npm layout (`@anysola/code` and `@anysola/code-<os>-<arch>`, bin `anysola`),
- `/v1/code/version` semantics for update notices and the minimum version.

## Testing

- Unit tests next to the code.
- Integration tests run the real binary against `testkit`'s fake gateway, which replays scripted SSE streams
  (text, thinking, tool use, errors, `anysola_credits`).
- TUI tests render to ratatui's test backend and compare text snapshots; a pty-driven smoke test renders real
  screens the way `testing/tui-*.sh` did for 0.5.1.
- A CI matrix (Linux x64/arm64, macOS arm64/x64, Windows x64/arm64) builds, runs the tests and a smoke run on
  every push. Publishing stays a tag-only job.

## Release

Linux builds are static musl, so the `-musl` and glibc archive names can carry the same bytes; six real
builds (linux x64/arm64, macOS x64/arm64, Windows x64/arm64) fill the 12 asset names the install scripts know.

0.5.x stays on the TypeScript line until the Rust version covers `PARITY.md`. Rust ships once, as 1.0.0, through
the same channels (GitHub Release archives, install scripts, npm, Admin Release).
