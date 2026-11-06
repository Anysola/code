# Changelog

Anysola Code releases, newest first. OpenCode's changelog up to 2.2.5 is in
[UPSTREAM-CHANGELOG.md](https://github.com/Anysola/code/blob/legacy-0.5/UPSTREAM-CHANGELOG.md) on the `legacy-0.5`
branch.

## 1.0.0 (unreleased)

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
