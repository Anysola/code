# Anysola Code

[![npm](https://img.shields.io/npm/v/@anysola/code)](https://www.npmjs.com/package/@anysola/code)

A terminal coding agent for Anysola accounts, based on [OpenCode](https://github.com/anomalyco/opencode) v2.2.5.

Product page: https://anysola.com/code. Account: https://account.anysola.com.

## Install

Requires Node 20 or newer. Runs on Linux, macOS and Windows (x64 and arm64).

```sh
npm i -g @anysola/code
```

## First run

```sh
anysola
```

The first run opens a device sign-in at https://account.anysola.com/device. After you approve it, the TUI starts.

## Commands

| Command | What it does |
|---|---|
| `anysola` | Start the TUI in the current directory |
| `anysola login` | Sign in with the device flow |
| `anysola logout` | Sign out |
| `anysola status` | Show the sign-in state |
| `anysola usage [--days 7\|30]` | Show credit usage for the last 7 or 30 days |
| `anysola devices` | List signed-in devices |
| `anysola devices revoke <id>` | Sign out a device |
| `anysola update` | Install the latest release from npm |
| `anysola --version` | Print the Anysola Code and OpenCode versions |

In the TUI: `/credits` (balance), `/usage` (recent usage), `/devices` (signed-in devices).

## Credits and billing

Usage is billed to the credits of your Anysola subscription (Pro or Team); free accounts get a daily quota. The
status line shows the credits remaining, the cost of the current turn and the total for the session.

## Models

The model list comes from the Anysola server. Sonnet 5.5 is the default, Haiku 5.5 is the small model, and Opus
5.5 can be selected in the model picker.

## Differences from OpenCode

- Anysola is the only provider. Other providers and models.dev are not loaded.
- Sign-in is built in (`anysola login`, or the first run) and uses your Anysola account.
- No telemetry, no session sharing, no web UI, no remote tunnel.
- Self-update is disabled. Updates come from npm (`anysola update` or `npm i -g @anysola/code@latest`).

## Based on OpenCode

Anysola Code is a fork of [OpenCode](https://github.com/anomalyco/opencode), used under the MIT License. The
`anysola` branch is upstream tag `v2.2.5` plus a series of Anysola commits. The upstream copyright and permission
notice are in [LICENSE](LICENSE) and must be kept with every copy; [NOTICE](NOTICE) describes the fork. Anysola
Code is not affiliated with or endorsed by the OpenCode project. OpenCode's own release notes up to 2.2.5 are in
[UPSTREAM-CHANGELOG.md](UPSTREAM-CHANGELOG.md).

## Development

Bun 1.4.4 is required (the root `packageManager`).

```sh
bun install --frozen-lockfile
bun dev [directory]          # run the CLI and TUI from source
```

`ANYSOLA_API_URL` (default `https://api.anysola.com`) points the device flow, config and provider at another API.

Build the platform binaries and the npm packages:

```sh
cd packages/cli
export ANYSOLA_VERSION=0.3.0 OPENCODE_CHANNEL=latest
export OPENCODE_VERSION="$(node -p "require('./package.json').version")"
bun run build --skip-web-ui                              # all targets; OPENCODE_TARGET=linux-x64 for one
rm -f dist/SHA256SUMS && bun run pack                    # tarballs in dist/npm (ANYSOLA_NPM_DIR to change)
dist/code-linux-x64/bin/anysola --version
```

### Merging an upstream tag

```sh
git fetch upstream --tags
git checkout anysola
git merge vX.Y.Z             # or: git rebase --onto vX.Y.Z v2.2.5 anysola
```

Conflicts usually land in the files the Anysola commits touch:

- `packages/core/src/plugin/internal.ts`: keep upstream's new plugins, leave `ModelsDevPlugin`,
  `...ProviderPlugins` and `WellKnownPlugin` out and `AnysolaPlugin` in.
- `packages/core/src/provider.ts`, `integration.ts`, `config.ts`: re-apply the single `Anysola.*` line if the
  surrounding code moved.
- `packages/cli/script/build.ts`, `pack.ts`: keep `binary = "anysola"`, the `@anysola/` names, `ANYSOLA_VERSION`,
  the LICENSE/NOTICE copy and the `opencode/` User-Agent (the Anysola gateway allowlists it).
- `packages/tui/src/logo.ts`, `component/logo.tsx`: keep ours.
- Docs and `.github` files deleted on this branch: keep them deleted (`git rm`).
- If upstream changed the provider or credential APIs, compare `plugin/provider/anysola.ts` with upstream's
  `plugin/provider/opencode.ts`, which uses the same calls.

Then sweep for new branding strings, typecheck and rebuild:

```sh
git grep -n -E 'OpenCode|opencode\.ai|`opencode [a-z]' -- 'packages/tui/src' 'packages/cli/src' \
  ':!*test*' | grep -v -E 'OpenCode(Client|Event|\.make)|@opencode/'
(cd packages/core && ../../node_modules/.bin/tsc -p tsconfig.json --noEmit)
```

Kept as upstream on purpose: config and data paths (`~/.config/opencode`), `OPENCODE_*` environment variables,
`@opencode/*` workspace package names and plugin APIs. `packages/cli/package.json` `version` is the upstream
OpenCode version and changes only with a merge.

## Release

The Anysola Code version is not stored in any `package.json`; it comes from the tag.

1. Add the release to [CHANGELOG.md](CHANGELOG.md) and push `anysola`.
2. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. `.github/workflows/release.yml` builds every platform, smoke-tests linux-x64, signs the macOS binaries ad hoc,
   packs, and publishes `@anysola/code` and the platform packages to npm with the `NPM_TOKEN` secret. A version
   with a `-` suffix goes to the `next` dist-tag.

Running the workflow by hand (`workflow_dispatch`, `dry_run` on by default) builds and packs without publishing.

## License

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
