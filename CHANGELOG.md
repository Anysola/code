# Changelog

Anysola Code releases, newest first. OpenCode's changelog up to 2.2.5 is in
[UPSTREAM-CHANGELOG.md](UPSTREAM-CHANGELOG.md).

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
