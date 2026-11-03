# Changelog

Anysola Code releases, newest first. OpenCode's changelog up to 2.2.5 is in
[UPSTREAM-CHANGELOG.md](UPSTREAM-CHANGELOG.md).

## Unreleased

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
