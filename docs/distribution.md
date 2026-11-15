# Distribution and releases

ZORIX is one code base for macOS, Windows and Linux. The desktop app is built with Tauri; the command line is a plain Rust binary.

## What a release contains

| Platform | Desktop app | Command line |
|---|---|---|
| macOS (Apple silicon, Intel) | `.dmg` | `.tar.gz` |
| Windows (x64) | `.msi` and `.exe` installers | `.zip` |
| Linux | `.AppImage`, `.deb`, `.rpm` | `.tar.gz` (x64 musl, arm64) |

Every release also has a `SHA256SUMS` file.

## The pipeline

`.github/workflows/ci.yml` runs on every push and pull request:

| Job | Systems | Does |
|---|---|---|
| Rust | Ubuntu, macOS, Windows | Format check, clippy with warnings as errors, all tests |
| Desktop app | Ubuntu 22.04, macOS, Windows | Tauri crate and npm versions match, type-check, interface tests, build the interface, clippy on the shell |

`.github/workflows/release.yml` runs when a tag `vX.Y.Z` is pushed: it builds the installers and the command line
archives on each system, uploads them to a **draft** GitHub release, and publishes the checksums. A person reads the draft
and publishes it.

Cutting a release:

1. Bump the version in the workspace `Cargo.toml`, `apps/desktop/package.json` (and its lock) and `tauri.conf.json`.
2. Move the changelog's *Unreleased* section under the new version.
3. Wait for CI to be green **on all three systems**.
4. Tag (`git tag -a vX.Y.Z`), push the tag, wait for the release workflow.
5. Download one artifact per platform and start it; then publish the draft.

## Signing and notarization

**The first releases are not signed.** Expect a warning on first launch:

- **macOS** — Gatekeeper says the app cannot be checked. Right-click the app, choose *Open*, and confirm once.
- **Windows** — SmartScreen shows "unknown publisher". Choose *More info*, then *Run anyway*.
- **Linux** — no signature is needed for AppImage, deb or rpm, but verify the checksum.

The workflow is ready for signing and does it when these repository secrets exist, and says in the log when they do not:

| Platform | Secrets |
|---|---|
| macOS | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` (signing and notarization) |
| Windows | `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` (a base64 `.pfx`) |

Check a download against `SHA256SUMS`:

```bash
shasum -a 256 -c SHA256SUMS --ignore-missing      # macOS and Linux
```

## Updates

Automatic updates are **not** part of this phase. They need signed releases and an update key; they are planned with the
signing work. Until then, install a newer release over the old one. Data lives outside the application, in the data folder
([persistence](persistence.md#where-the-data-lives)), so it survives updates and uninstalling the app. A database written
by a newer version is never opened by an older one.

## Uninstalling

Remove the application with the platform's usual method. To also remove data, delete the data folder
(`zorix data-dir` prints it).

## Cross-platform rules

- Platform-specific code is kept out of the runtime crates, so they build and test on every system.
- File tools accept only relative, forward-slash paths and refuse Windows reserved names, so behaviour is the same everywhere.
- The data folder comes from one function and can be moved with `ZORIX_HOME`.
- The command line runs its work on a 16 MB stack thread.
