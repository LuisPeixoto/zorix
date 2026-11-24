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

`.github/workflows/installers.yml` checks the installers themselves. On a fresh runner of each system it builds the installers (unsigned, as
a release is), installs each one silently, starts the **installed** app with `ZORIX_SMOKE=1` (it says it has loaded and closes; see
[testing](testing.md#end-to-end-tests)), uninstalls, and checks the program is gone:

| System | What it does |
|---|---|
| Linux | `apt install` of the `.deb`, start `/usr/bin/zorix-desktop` under a virtual display, `apt remove` and check no file of the package is left; start the `.AppImage` (it unpacks itself, the runner has no FUSE); read the `.rpm`'s file list and metadata (it cannot be installed on Ubuntu) |
| macOS | mount the `.dmg`, copy `ZORIX.app` to `/Applications`, start it, remove it |
| Windows | `msiexec /qn` install of the `.msi`, start, `msiexec /x`, check the program is gone; the same with the NSIS `-setup.exe /S` and its `uninstall.exe /S` |

Run it from the Actions tab (*Installers*, *Run workflow*) on `main` before tagging; it also runs on a tag. It is a launch check: it cannot see
what only a person sees (the checklist below).

Cutting a release:

1. Bump the version in the workspace `Cargo.toml`, `apps/desktop/package.json` (and its lock) and `tauri.conf.json`.
2. Move the changelog's *Unreleased* section under the new version.
3. Wait for CI to be green **on all three systems**.
4. Run the *Installers* workflow on `main` and wait for it to be green on all three systems.
5. Tag (`git tag -a vX.Y.Z`), push the tag, wait for the release workflow.
6. Download one artifact per platform and start it; then publish the draft.

**Checklist per release, for what CI cannot see** (an unsigned build, a person at a screen):

- [ ] macOS: the downloaded `.dmg` opens, and the first launch needs right-click, *Open* (Gatekeeper), as the docs say.
- [ ] Windows: the `.msi` and the setup `.exe` show SmartScreen's "unknown publisher" and can continue with *More info*, *Run anyway*.
- [ ] Linux: the `.AppImage` starts on a desktop session; the `.rpm` installs on a Fedora-family system.
- [ ] After installing over the previous release, the data of the previous release is there (a run, a setting).
- [ ] After uninstalling, the data folder (`zorix data-dir`) is still there, as the docs promise.
- [ ] `shasum -a 256 -c SHA256SUMS --ignore-missing` passes for what was downloaded.
- [ ] The command line of the system you are on prints the right `--version`.


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
