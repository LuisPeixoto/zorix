# Contributing to ZORIX

Thank you for helping! Bug reports, documentation, tests and code are all welcome. By participating you agree to follow
the [Code of Conduct](CODE_OF_CONDUCT.md). For anything bigger than a small fix, please open an issue first.

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

For interface changes also run `npm run check && npm run build` in `apps/desktop`.

## Conventions

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org): `type(scope): imperative
  summary`. Keep commits small: one purpose each.
- One branch per piece of work: `feat/…`, `fix/…`, `docs/…`, `test/…`, `ci/…`, `chore/…`.
- Code, comments and docs are in English. Comments explain *why*. Public items have `///` docs.
- No `unwrap` or `expect` in library code (fine in tests). Errors are small enums with human sentences.
- The crates must build and test on macOS, Windows and Linux. Keep platform-specific code isolated.
- Docs and code ship together: update the guide and `CHANGELOG.md` when behaviour changes.
