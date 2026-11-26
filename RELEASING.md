# Releasing Anysola Code

Releases go through `.github/workflows/release.yml` in `Anysola/code`. A tag `v<ver>` builds the desktop app for
Windows x64 (NSIS, per-user) and Linux x64 (AppImage, .deb) with `--config '{"version":"<ver>"}'`, signs them for
the updater and creates the GitHub release with four files:

| File | What reads it |
|---|---|
| `anysola-code-desktop-windows-x64-setup.exe` | www `/code` download link, the updater on Windows |
| `anysola-code-desktop-linux-x64.AppImage` | www `/code` download link, the updater on Linux |
| `anysola-code-desktop-linux-x64.deb` | www `/code` download link |
| `latest-desktop.json` | the updater: `version`, `pub_date`, `notes`, and per platform (`windows-x86_64`, `linux-x86_64`) the installer's `url` and `signature` |

Installed apps from 1.0.6 on depend on these names, on the manifest's shape and on the updater key: none of them
may change. The `.sig` files are not uploaded; their contents are in the manifest.

## Before the tag

1. `main` is green in `ci` for the commit to be tagged; every push tests Windows x64 and Linux x64.
2. Dry run: Actions → release → Run workflow. The `release` artifact holds exactly the four files above (the
   `assemble` job fails otherwise); check `latest-desktop.json` names both platforms.
3. `CHANGELOG.md` has the version's entry (the release notes point to it).
4. The app is tried end to end on Linux under Xvfb with the fake gateway, and on Windows when the change touches it.

## Release

1. `git tag v<ver> && git push origin v<ver>` (a `-` in the version makes a prerelease).
2. Actions → release: version → desktop (2) → assemble → github-release.
3. `gh release view v<ver> --repo Anysola/code`: title `Anysola Code <ver>`, the four files.
4. `curl -sL https://github.com/Anysola/code/releases/download/v<ver>/latest-desktop.json` names `<ver>` and both
   platforms.
5. Admin → Code → Release: set latest to `<ver>` (Admin only accepts a version with a GitHub Release). Installed
   apps ask `GET /v1/code/version` for the latest version and only then download that version's
   `latest-desktop.json`, so this step is what offers the update. Check
   `curl -s https://api.anysola.com/v1/code/version`.
6. Web deploy if the `/code` page or News mention the version.

## Updater key

The manifest and installers are signed with the updater key (`TAURI_SIGNING_PRIVATE_KEY` and its password,
repository secrets); `desktop/src-tauri/tauri.conf.json` holds the public half. Losing the key means installed apps
can no longer verify an update and have to be reinstalled by hand: it is backed up with the server's encrypted
file backups.

Installers are not code-signed yet: Windows shows SmartScreen's "unknown publisher" on first run. A .deb install
cannot replace itself; the app opens the release page instead. AppImage and Windows update in place.

## History

- 1.0.0 to 1.0.9 also released the terminal app: 12 CLI archives, `SHA256SUMS`, `install.sh`, `install.ps1` and the
  npm packages `@anysola/code` and `@anysola/code-<os>-<arch>`. From 1.1.0 none of these are built; the npm packages
  and the old releases stay as they are.
- The 0.5.x TypeScript line is the `legacy-0.5` branch; its tags and releases stay as they are.
