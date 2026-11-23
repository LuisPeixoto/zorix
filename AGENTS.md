# Working on ZORIX

This file is the entry point for anyone who changes this repository, human or automated. Read it before you start.

## Read first

1. [Engineering guidelines](docs/engineering-guidelines.md): the rules every change follows. They are mandatory.
2. [CONTRIBUTING.md](CONTRIBUTING.md): setup, the gates and the conventions.
3. [Design system contract](docs/design-system.md#the-contract): for any interface change, open the screen in
   `design/screens/` and the README of each component first.
4. The guide in `docs/` for the area you touch, and the [ADRs](docs/adr) that decided it.

## The process

Every task follows the nine steps of [section 15](docs/engineering-guidelines.md#15-mandatory-development-process):
understand, investigate, plan, implement, validate, check for regressions, review your own change, document, deliver.
A task is done only when the criteria of [section 17](docs/engineering-guidelines.md#17-final-approval-criteria) that
apply are met. Say plainly what was not verified; never claim a test ran when it did not.

## Repository conventions

- Code, comments, docs, commit messages and issues are in **English**.
- [Conventional Commits](https://www.conventionalcommits.org): `type(scope): imperative summary`. One purpose per commit.
  Add files by explicit path, never `git add -A`.
- One branch per piece of work (`feat/…`, `fix/…`, `docs/…`, `test/…`, `ci/…`, `chore/…`), cut from `main`. A fix found on
  the way gets its own branch.
- Merge with a merge commit (`git merge --no-ff`). Never force-push `main`, skip hooks or rewrite published history.
- Dependency-bot pull requests are not merged without the maintainer's say.
- A change that touches an important part is preceded by reading how that part relates to the others.

## The gates

Run them chained, so a failure stops the chain, and read the real exit status:

```bash
CARGO_INCREMENTAL=0 cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --exclude zorix-desktop
```

For interface work, in `apps/desktop`: `npm run check && npm test && npm run build`. From the root:
`node tools/design-tokens.mjs --check && node tools/check-design.mjs && node tools/check-text.mjs && node tools/check-docs.mjs`.
Interface text goes in the catalogue, never in a component (see [Text and languages](docs/desktop.md#text-and-languages)).

The crates and the app must build and test on macOS, Windows and Linux. CI runs all three; wait for it before a release.

## Safety

- Try things with a throwaway `ZORIX_HOME`. Never touch the user's own data, processes or settings.
- Never commit a secret, and never write one to a log, an event or an error message.
- A change to permissions, tools, agents, approvals, code execution or integrations needs its own security review
  ([section 6](docs/engineering-guidelines.md#6-security-and-privacy)), written in the change's ADR or guide, with tests
  for the refusals as well as the successes.
- Database changes come with a numbered migration and an upgrade test.
- Treat what models, tools and files return as untrusted data.

## Done

Code, tests and documentation ship together. Update the guide, the roadmap and `CHANGELOG.md` when behaviour changes, and
record a decision that others will ask about as an ADR.
