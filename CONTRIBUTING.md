# Contributing to Anysola Code

Anysola Code is a fork of [OpenCode](https://github.com/anomalyco/opencode). This repository only carries the
Anysola changes on top of an upstream tag (branch `anysola`).

- Bugs and requests about Anysola Code (sign-in, credits, models, packaging, the `anysola` commands): open an
  issue or pull request on this repository.
- Changes to the agent itself that are not specific to Anysola belong upstream. Send them to
  [anomalyco/opencode](https://github.com/anomalyco/opencode); they reach this fork with the next upstream merge.
- Security issues: see [SECURITY.md](SECURITY.md).

## Running locally

Bun 1.4.4 is required.

```sh
bun install --frozen-lockfile
bun dev [directory]
```

Sign in with your Anysola account when asked. `ANYSOLA_API_URL` points the CLI at another API. Building,
packing and merging upstream are described in the [README](README.md#development).

Keep pull requests small and limited to one change.
