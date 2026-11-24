# Audits

Phases 11 to 15 each take one area of the product through the same steps (see the [roadmap](../roadmap.md#how-a-quality-phase-works)). This
folder holds one file per audit, named `phase-NN-area.md`, written from the template below, plus what Phase 10 checked.

## Template

```markdown
# Audit: <area> (Phase NN)

Date · the commit audited · who audited.

## Scope
What was looked at, and what was left out and why.

## Findings
One row per finding. A security finding is fixed first; every finding is an issue in the phase's milestone.

| # | Section of the guidelines | Finding | Severity | Issue | Fixed in |
|---|---|---|---|---|---|

## Checked, nothing found
The sections of the guidelines (2 to 14) that were checked and gave no finding, with how: the test that proves it, the command that was run.

## Not verified
What could not be checked here, and why. Never leave this out: an empty list means everything was checked.
```

Rules for an audit:

- A finding states the failing scenario in concrete terms (the input, the state, the wrong result). "Could be better" is not a finding.
- A fix is its own branch with a regression test that failed before it; the row names the branch.
- The audit is closed only when every row has an issue, a fix or a stated reason it stays open, and the area's documentation was read against the code.

## Phase 10: documentation drift

Every claim that something does not exist yet, or is for a later phase, was checked against the code. Found and corrected:

| Where | Said | Is |
|---|---|---|
| [persistence](../persistence.md#limits) | Secrets are never stored, there are none | Keys and tokens live in the system keychain or in the environment variable named; they are never in the database |
| [tools](../tools.md) | No built-in tool runs code, a sandbox is planned | `code.run` runs JavaScript and Python in a WebAssembly sandbox ([sandbox](../sandbox.md)); no tool runs a command on the host |
| [desktop](../desktop.md) | The full interface belongs to later phases | The agents, workflows, projects, workforce, memory, documents and settings screens exist |
| [getting started](../getting-started.md) | 247 tests | More than 1,100 Rust tests and 150 for the interface |
| [testing](../testing.md) | 913 Rust tests and 61 for the interface | The counts at the time of this audit |
| [guidelines](../engineering-guidelines.md#known-gaps) | Gaps #1, #2 and #3 open | Closed or reduced to what is tracked in [#27](https://github.com/LuisPeixoto/zorix/issues/27) |

Checked and still true: *accounts and *Share* wait for Phase 16*; *automatic updates are not part of the releases* ([distribution](../distribution.md#updates)); *retention policies are not implemented*
([persistence](../persistence.md#limits)); the workflow actions that are not built ([workflows](../workflows.md)); *teams have no versions* ([teams](../teams.md)).
