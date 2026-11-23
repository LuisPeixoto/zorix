## Summary

<!-- What does this change and why? Link the issue it closes, e.g. "Closes #123". -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Documentation
- [ ] Refactor / chore / CI

## How was it tested?

<!-- Commands you ran, host OS, and screenshots for UI changes (dark and light theme). -->

## Checklist

- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [ ] `cargo test --workspace --exclude zorix-desktop` passes
- [ ] `npm run check` passes in `apps/desktop` (for UI changes)
- [ ] Tests added or updated, including failure and edge cases (empty, invalid, limits, interruptions)
- [ ] Security review written down when permissions, tools, agents, approvals or integrations change
- [ ] No secrets in code, logs, events or error messages
- [ ] UI changes: Light and Dark screenshots at more than one window size, and the design system followed
- [ ] Data and settings stay compatible, or the change is documented (with a migration for the database)
- [ ] Docs and `CHANGELOG.md` updated when behaviour changes
- [ ] The [engineering guidelines](../docs/engineering-guidelines.md#17-final-approval-criteria) criteria that apply are met
- [ ] Commits follow [Conventional Commits](https://www.conventionalcommits.org)
