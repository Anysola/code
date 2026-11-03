# Changelog

Anysola Code releases, newest first. OpenCode's changelog up to 2.2.5 is in
[UPSTREAM-CHANGELOG.md](UPSTREAM-CHANGELOG.md).

## Unreleased

- `anysola run` ends with what the run spent and the balance left.
- MCP sign-ins name Anysola Code: OAuth client metadata from www.anysola.com and an Anysola callback page.
- Daily upstream tracking: a workflow tries each new OpenCode v2 release and opens a PR or a conflict issue.
- The home screen draws the ANYSOLA wordmark (both A's are the brand symbol) with "Code" under it; narrow terminals
  show the A alone. The exit summary prints the same mark.
- The Anysola theme follows the web's Grafit palette: warm graphite surfaces (page #110f0d, panels #1a1815, the
  composer one step lighter), warm greys for text, one sand accent for the Build agent and selection, quieter syntax
  and diff colours. The light variant uses the same warm greys.
- Home: the latest three sessions of the directory under the composer (click to open; `ctrl+x l` lists all), and the
  plan with its renewal date in the bottom bar ("Team · renews 23 Nov").
- Prompt examples are Anysola's own ("Explain this project", "Fix the failing tests", "Review my last commit").
- The event banner uses the info colour without a decorative marker.
- The composer no longer repeats "Anysola" after the model name; Anysola is the only provider.
- `/credits` writes the plan capitalised and the renewal as a date ("23 Nov"); the status line drops "this turn 0"
  after a session is reopened.
- Dialog group headings are grey; the sand accent is kept for the selected row.
- `anysola mini` ends with the A symbol beside the resume command.

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
