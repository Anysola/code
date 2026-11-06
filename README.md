# Anysola Code

[![npm](https://img.shields.io/npm/v/@anysola/code)](https://www.npmjs.com/package/@anysola/code)

A terminal coding agent for Anysola accounts, written in Rust.

Product page: https://www.anysola.com/code. Account: https://account.anysola.com.

## Install

macOS and Linux (x64 and arm64, glibc or musl):

```sh
curl -fsSL https://www.anysola.com/install | sh
```

Windows (x64 and arm64), in PowerShell:

```powershell
irm https://www.anysola.com/install.ps1 | iex
```

The scripts download the binary from this repository's GitHub release, check its SHA-256 against `SHA256SUMS`,
install it to `~/.anysola/bin` (`%USERPROFILE%\.anysola\bin` on Windows) and add that directory to `PATH`. A
specific version: `sh -s -- --version 1.0.0` on macOS and Linux, `$env:ANYSOLA_VERSION = "1.0.0"` before the
command on Windows. `anysola update` reruns the script; `anysola uninstall` removes the `PATH` entry.

With npm (Node 20 or newer):

```sh
npm i -g @anysola/code
```

## First run

```sh
anysola
```

The first run opens a device sign-in at https://account.anysola.com/device. After you approve it, the TUI starts.

Coming from 0.5.x: the first start imports your sessions, settings and sign-in from the 0.5.x data
(`opencode.db` and the config directory). The old files are left as they were.

## Commands

| Command | What it does |
|---|---|
| `anysola` | Start the TUI in the current directory |
| `anysola login` | Sign in with the device flow |
| `anysola logout` | Sign out |
| `anysola status` | Show your plan and usage limits |
| `anysola usage [--days 7\|30]` | Show your usage limits and requests for the last 7 or 30 days |
| `anysola limits` | Show the 5-hour and weekly windows and your banked resets |
| `anysola limits redeem` | Spend a banked reset |
| `anysola devices` | List signed-in devices |
| `anysola devices revoke <id>` | Sign out a device |
| `anysola run --budget <percent> "<message>"` | Run once; stop when the session has used that percent of the 5-hour window |
| `anysola run --use-banked-reset "<message>"` | Run once; spend a banked reset if a usage window is full |
| `anysola models` | List the models your plan can use |
| `anysola session list\|export\|import\|delete` | Manage stored sessions |
| `anysola mcp list\|add\|auth\|logout` | Manage MCP servers |
| `anysola acp` | Run as an Agent Client Protocol server for editors |
| `anysola update` | Install the latest release the way this copy was installed (install script or npm) |
| `anysola uninstall` | Remove the binary and its `PATH` entry; your data and config stay |

In the TUI: `/usage` (usage limits, banked resets, recent requests and cache hit rate), `/reset` (spend a banked
reset), `/devices` (signed-in devices), `/budget` (session budget, below), `/account` (opens account.anysola.com),
`/feedback` (opens a support ticket).

## Configuration

Settings are read from `~/.anysola/config.json`, then from `anysola.json` or `anysola.jsonc` in the project and its
parent folders, then from a project's `.anysola/` folder, which also holds `agent/`, `command/`, `skills/` and
`themes/`. `~/.anysola/` holds the global `AGENTS.md`, agents, commands, skills and themes the same way. Environment
settings are `ANYSOLA_*` (`ANYSOLA_CONFIG`, `ANYSOLA_CONFIG_CONTENT`, `ANYSOLA_LOG_LEVEL`, …).

For compatibility, the 0.5.x names are still read: `opencode.json[c]`, `.opencode/`, `~/.config/opencode/` and each
`OPENCODE_*` variable. Where both names exist, the `anysola` one wins.

## Usage limits

Anysola Code and Anysola Chat share two usage windows per account, like Claude and Codex: a **5-hour window** that
starts with your first request and lasts five hours, and a **weekly window** that works the same way over seven
days. How much of each your plan includes is set per plan; you see percentages and reset times, never money. The
status line shows the 5-hour window (`5h 42%`), adds the weekly one when it is the fuller of the two or past 80%
(`5h 30% · week 85%`), and hovering it shows when the 5-hour window resets. At 80% of either window a warning
appears once; `/usage` and `anysola usage` show both windows with their reset times.

When a window is full you can wait for it to reset or spend a **banked reset**. Banked resets are rights Anysola puts
in your bank; each has an expiry date and you hold at most 3 at a time. When a request is refused, the TUI asks
"Use a banked reset?" if you have one, and on "Use banked reset" clears the window and sends your prompt again.
To spend one on your own, use `/reset`, or `r` in `/usage`; a reset clears the 5-hour window, or both windows when
it was granted for both. `anysola limits` lists your bank, `anysola limits redeem` spends the reset that expires first, and
`anysola run --use-banked-reset` spends one on its own when a run would otherwise stop.

### Session budget

A budget is a share of the 5-hour window. `/budget 20` lets the session use 20% of the window and the status line
shows `session 12%/20%`. Once the session has used its budget, the next prompt asks first: continue once, add the
same amount again, remove the budget, or stop. A turn already running is never cut. `/budget` shows the budget,
`/budget off` removes it, and `/budget default 20` gives every new session a 20% budget (`/budget default off` to
stop). The budget and the session's use survive restarts, so `anysola -s <id>` resumes against the same budget.
Budgets set in credits by 0.5.x are not carried over.

`anysola run --budget 20` sets the same budget for a run. A run cannot ask, so it stops as the next model request
starts after the budget is reached, exits with code 1 and prints how to continue.

## Models

The model list comes from the Anysola server and depends on your plan. Sonnet 5.5 is the default and Haiku 5.5 the
small model. When Anysola turns them on, `Auto` uses Haiku for short questions in a small session and Sonnet for
everything else, and the `economy` variant runs a model at lower effort. Each request is billed at the model that
answered it.

## CI

Make a token in [Account > Apps > Code](https://account.anysola.com) ("New CI token") and store it as the GitHub
secret `ANYSOLA_TOKEN`. Anysola Code reads it from the environment, so `anysola run --budget 100 "…"` works in any
CI. For GitHub, [`Anysola/code-action`](https://github.com/Anysola/code-action) reviews pull requests and answers
`/anysola` on issues (opening a PR when it changes files).

## History

1.0 is a rewrite in Rust of 0.5.x, which was a fork of [OpenCode](https://github.com/anomalyco/opencode) 2.2.5 in
TypeScript. The 0.5.x source is kept on the [`legacy-0.5`](https://github.com/Anysola/code/tree/legacy-0.5) branch.
On linux-arm64 the binary went from 196 MB to about 10 MB, `--version` from 80 ms to about 3 ms, and the idle TUI
from three processes using about 340 MB to one process. [ARCHITECTURE.md](ARCHITECTURE.md) has the numbers and the
shape of the code; [PARITY.md](PARITY.md) is the feature-for-feature checklist against 0.5.1.

The system prompts, tool descriptions and other texts the model sees come from OpenCode and are used under the MIT
License; see [NOTICE](NOTICE). Anysola Code is not affiliated with or endorsed by the OpenCode project.

## Development

Rust 1.99.0 (`RUST_VERSION` in `.github/workflows/ci.yml`).

```sh
cargo build
cargo test --workspace
cargo run -p anysola -- [directory]
```

`ANYSOLA_API_URL` (default `https://api.anysola.com`) points the device flow, config and provider at another API.
`testing/` has a fake gateway (`fake-gateway.mjs`) and scripts that drive the TUI in a pty for screenshots.

## Release

See [RELEASING.md](RELEASING.md). The version comes from the tag: `git tag vX.Y.Z && git push origin vX.Y.Z` runs
`.github/workflows/release.yml`, which builds the six platforms, publishes `@anysola/code` and its platform packages
to npm (a `-` version goes to the `next` dist-tag) and creates the GitHub release the install scripts read.

## License

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
