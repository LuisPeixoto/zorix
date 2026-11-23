# Contributing to ZORIX

Thank you for helping! Bug reports, documentation, tests and code are all welcome. By participating you agree to follow
the [Code of Conduct](CODE_OF_CONDUCT.md). For anything bigger than a small fix, please open an issue first.

Every change follows the [engineering guidelines](docs/engineering-guidelines.md), and [AGENTS.md](AGENTS.md) summarises
them for day-to-day work.

## Development setup

You need [Rust](https://rustup.rs) (stable, 1.90 or newer), [Node.js](https://nodejs.org) 22 or newer and, on Linux,
the [Tauri system dependencies](https://tauri.app/start/prerequisites/#linux).

```bash
git clone https://github.com/LuisPeixoto/zorix.git
cd zorix

# Runtime crates and command line
cargo test --workspace --exclude zorix-desktop
cargo run -p zorix-cli -- run "What is 17*23?"

# Desktop app
cd apps/desktop
npm install
npm run tauri dev
```

Set `ZORIX_HOME` to a throwaway folder when you try things, so your own data is never touched.

## Before you open a pull request

Run all of these from the repository root, chained so a failure stops the chain:

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --exclude zorix-desktop
```

For interface changes also run, in `apps/desktop`, `npm run check && npm test && npm run build`, and from the root `node tools/design-tokens.mjs --check && node tools/check-design.mjs && node tools/check-docs.mjs`.

## Design system contract

The app follows the design system in [`design/`](design/README.md) strictly ([ADR-0032](docs/adr/0032-design-system-v3-is-the-contract.md), [how to follow it](docs/design-system.md#the-contract)):

1. Open the screen's image in `design/screens/` and the README of each component it uses before you build.
2. Use tokens and the kit only. `tools/check-design.mjs` rejects colour literals, arbitrary values for colour, type, radius, shadow and duration, and slow transitions; an exception needs a reason in its allowlist.
3. A screen or component the design system lacks is added there first, then re-vendored into `design/`.
4. An element that needs data ZORIX does not have yet is left out and listed in the [roadmap](docs/roadmap.md), never mocked.
5. Done means real-app Light and Dark screenshots beside the design system's, a design critique and an accessibility review with their findings fixed, and any remaining difference written under [Known differences](docs/design-system.md#known-differences).

## Conventions

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org): `type(scope): imperative
  summary`. Keep commits small: one purpose each.
- One branch per piece of work: `feat/…`, `fix/…`, `docs/…`, `test/…`, `ci/…`, `chore/…`.
- Code, comments and docs are in English. Comments explain *why*. Public items have `///` docs.
- No `unwrap` or `expect` in library code (fine in tests). Errors are small enums with human sentences.
- The crates must build and test on macOS, Windows and Linux. Keep platform-specific code isolated.
- Docs and code ship together: update the guide and `CHANGELOG.md` when behaviour changes.

## Definition of done

A change is done when the criteria of [section 17 of the guidelines](docs/engineering-guidelines.md#17-final-approval-criteria)
that apply are met: it meets the request and the architecture, has tests (including failures and edge cases) that pass,
builds and lints clean, has been looked at in the real app when it is visual, keeps data and settings compatible,
handles errors, has its documentation updated, and was reviewed by its author before it was proposed. A change to
permissions, tools, agents or integrations also carries a security review.
