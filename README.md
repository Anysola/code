# Anysola Code

[![npm](https://img.shields.io/npm/v/@anysola/code)](https://www.npmjs.com/package/@anysola/code)

A terminal coding agent for Anysola accounts, based on [OpenCode](https://github.com/anomalyco/opencode) v2.2.5.

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
specific version: `sh -s -- --version 0.4.0` on macOS and Linux, `$env:ANYSOLA_VERSION = "0.4.0"` before the
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
| `anysola run --budget <credits> "<message>"` | Run once; stop when the session has spent that many credits |
| `anysola update` | Install the latest release from npm |
| `anysola --version` | Print the Anysola Code and OpenCode versions |

In the TUI: `/credits` (balance), `/usage` (recent usage and cache hit rate), `/devices` (signed-in devices),
`/budget` (session budget, below).

## Credits and billing

Usage is billed to the credits of your Anysola subscription (Pro or Team); free accounts get a daily quota. The
status line shows the credits remaining, the cost of the current turn and the total for the session.

### Session budget

`/budget 50` caps the session at 50 credits and the status line shows `session 12/50`. Once the session has spent
its budget, the next prompt asks first: continue once, add the same amount again, remove the budget, or stop. A
turn already running is never cut. `/budget` shows the budget, `/budget off` removes it, and `/budget default 50`
gives every new session a 50 credit budget (`/budget default off` to stop). The budget and the session's total
survive restarts, so `anysola -s <id>` resumes against the same budget.

`anysola run --budget 50` sets the same budget for a run. A run cannot ask, so it stops as the next model request
starts after the budget is reached, exits with code 1 and prints how to continue.

## Models

The model list comes from the Anysola server. Sonnet 5.5 is the default, Haiku 5.5 is the small model, and Opus
5.5 can be selected in the model picker.

## Differences from OpenCode

- Anysola is the only provider. Other providers and models.dev are not loaded, and plugins cannot add a provider or
  move Anysola's endpoint.
- Web search goes through the Anysola gateway (`POST /v1/code/search`, billed to your credits) instead of Exa,
  Firecrawl, Parallel, Tavily or TinyFish. `"websearch": false` in the config turns it off.
- Removed commands: `pair`, `auth` (use `anysola login`, `logout`, `status` and `anysola mcp auth`), `api`, `mini`,
  `stats` (use `anysola usage`), `debug agents`, `debug heap-dump`; in the TUI `/pair` (`/web`), `/stats`, the
  OpenCode Console `/usage` and limits, and release highlights. No built-in OpenCode docs or issue report skills.
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
                                                         # --skip-web-ui is required: packages/app is deleted
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
  `...ProviderPlugins`, `WellKnownPlugin` and `SkillPlugin` out, `AnysolaPlugin` and `AnysolaAccountPlugin` in, and
  `AnysolaProviderLockPlugin` as the last `post` entry; `guarded` lists the Anysola plugins instead of
  `OpencodePlugin`. New upstream provider plugins stay unregistered; new upstream web search providers too.
- `packages/core/src/provider.ts`, `integration.ts`, `config.ts`: re-apply the single `Anysola.*` line if the
  surrounding code moved.
- `packages/cli/script/build.ts`, `pack.ts`: keep `binary = "anysola"`, the `@anysola/` names, `ANYSOLA_VERSION`,
  the LICENSE/NOTICE copy and the `opencode/` User-Agent (the Anysola gateway allowlists it).
- `packages/tui/src/logo.ts`, `component/logo.tsx`: keep ours.
- Docs and `.github` files deleted on this branch: keep them deleted (`git rm`).
- Everything listed under "Deleted on this branch" below: keep it deleted (`git rm`; the upstream workflow does this
  for modify/delete conflicts). A new upstream file inside a deleted package directory goes too.
- `packages/cli/src/commands/commands.ts` and `src/index.ts`: a new upstream command needs a decision; the removed
  ones stay out. `packages/cli/src/services/updater.ts` keeps only `method`, `removal` and the Anysola update path.
- `packages/cli/script/build.ts`: keep `modelsSnapshotPlugin("{}")` and no release highlights define.
- `bun.lock` and the root `package.json` conflict on most merges now that this branch dropped about 1,100
  packages: take upstream's changes that concern kept packages, keep the deleted workspaces and patches out, then
  regenerate the lockfile with `bun install` (never merge `bun.lock` by hand) and check `bun install --frozen-lockfile`.
- If upstream changed the provider or credential APIs, compare `plugin/provider/anysola.ts` with upstream's
  `plugin/provider/opencode.ts`, which uses the same calls.

#### Deleted on this branch

Keep these deleted when merging; none of them is part of the CLI build or the release:

- Repo: `infra/`, `sst.config.ts`, every `sst-env.d.ts`, `salesforce/`, `nix/`, `flake.nix`, `flake.lock`,
  `sdks/`, `perf/`, `specs/`, `screenshot-uk.png`, `services/`, `.github/release/`, `.github/nix-hashes/`,
  `.github/actions/`, `.github/workflows/check.yml`, `check-test.yml`, `check-nix.yml`, `check-bot-scope.yml`,
  `script/sdk-docs.ts`, `script/ast-grep/gui-plugins/`, `.opencode/skills/app-e2e-tests`, `component-tests`, and the
  patches of packages no longer installed.
- Packages: `app`, `desktop`, `web`, `console/*`, `stats/*`, `session-ui`, `gui-plugins`, `patchmark`, `stories`,
  `posts`, `function`, `plugins/figma`, `sdk`, `identity`, `effect-drizzle-sqlite`.
- CLI (`packages/cli`): `bin/opencode2.cjs`, `Dockerfile`, `vite.node.config.ts`, `script/build-node.ts`,
  `script/node-assets.ts`, `src/node/`, `src/mini.ts`, `src/mini-host.ts`, handlers `api`, `stats`, `mini`, `pair`,
  `auth/{account,export,import,list,logout,switch}`, `debug/{agents,heap-dump}`, and their tests.
- TUI (`packages/tui`): `src/mini/*` except `tool.ts`, `tool.public.ts`, `types.ts`, `splash.ts`, `mono.ts`,
  `theme.ts`; `feature-plugins/system/stats*.ts(x)`, `feature-plugins/sidebar/limit.tsx`,
  `component/dialog-pair.tsx`, and their tests.
- Core (`packages/core`): `src/plugin/provider.ts`, every `src/plugin/provider/*` except `anysola*.ts`,
  `dynamic.ts` and `sdk-factory.ts` (with `claude-agent/`), `src/github-copilot/`, `src/modal/`,
  `src/plugin/models-dev.ts`, `src/plugin/skill.ts` and `src/plugin/skill/`, `src/plugin/websearch/*` except
  `anysola.ts` and `index.ts`, and their tests.

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
   with a `-` suffix goes to the `next` dist-tag. Then it creates the GitHub release the install scripts read
   (archives, `SHA256SUMS`, `install.sh`, `install.ps1`; a `-` version is a prerelease); www.anysola.com/install
   serves the latest release's scripts.

Running the workflow by hand (`workflow_dispatch`, `dry_run` on by default) builds and packs without publishing.

## License

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
