# Releasing Anysola Code

Releases go through `.github/workflows/release.yml` in `Anysola/code`, the repository the install scripts download
from (`github.com/Anysola/code/releases`) and the www `/install` routes proxy (`releases/latest/download`).

## Before the tag

1. `main` is green in `ci` on all six platforms, `script/budget.sh` included.
2. Dry run: Actions → release → Run workflow (`dry_run` on). The `release` artifact holds the 15 assets of
   `spec/verbatim/cli/release.txt` (the pack step fails otherwise); `npm` holds the 13 tarballs.
3. Live gateway: `mid-conversation-output-config-2026-07-01` is still in the `allowedBetas` (added 2026-11-03).
4. `CHANGELOG.md` has the version's entry (the release notes point to it).

## Release

1. `git tag v<ver> && git push origin v<ver>` (a `-` in the version makes a prerelease and the npm `next` tag).
2. Actions → release: build (6) → pack → publish (npm, environment `npm`, secret `NPM_TOKEN`) → github-release.
3. `gh release view v<ver> --repo Anysola/code`: title `Anysola Code <ver>`, 15 assets, `SHA256SUMS` lists 14.
4. `npm view @anysola/code@<ver> optionalDependencies` lists the 12 platform packages.
5. Admin → Code → Release: set latest to `<ver>` (Admin only accepts a version with a GitHub Release). Check
   `curl -s https://api.anysola.com/v1/code/version`.
6. Web deploy if the `/code` page or News mention the version.
7. install-test runs on the `published` release event; for a re-check run it by hand with the version.

## Owner decisions for 1.0.0

- Repository: this tree has no remote yet. The 1.0 tag must be created in `Anysola/code` (the 0.5.x TypeScript line
  lives on its `anysola` branch there), otherwise the install scripts and `/install` do not see the release.
- Minimum version: keep `minimum` as it is (0.5.x keeps working, updates are offered), or raise it to `1.0.0` so every
  0.5.x start is gated until updated.
- `NOTICE` is shipped unchanged from 0.5.1 (it calls Anysola Code a fork of OpenCode); reword it for the rewrite or
  keep it.
