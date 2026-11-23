# ADR-0042: The quality harnesses

Status: accepted (Phase 10)

## Context

The [guidelines](../engineering-guidelines.md) ask for tests of failure, concurrency, persistence, scale, security and
the built app. ZORIX had about 1,100 tests, but no way to make a simulated model stream, fail with a typed error or produce
malformed output, no fail points in the store, no benchmarks, no fuzzing, no coverage report and no test of the built
desktop app. Each later audit (Phases 11 to 15) needs these tools, so they are built once.

## Decision

- **Simulated faults.** `zorix-sim` can stream partial deltas and cut the stream, raise every `ModelErrorKind` with a
  `retry_after`, return malformed turns (invalid tool-call JSON, an unknown tool, an empty answer, a wrong finish reason) and
  inject faults from a seed, so a failing run can be repeated.
- **A fault-injecting repository.** A wrapper over any `ExecutionRepository` fails the Nth call of a chosen method. A real full
  disk is made with SQLite's page limit, and a real crash with `kill -9` of the command line. After each, `recover()` must
  bring the run to a consistent state and no tool that is not repeatable may run twice.
- **Adversarial tests, one per untrusted input** (tool results, documents, past work, facts, MCP descriptions, webhook
  payloads, task text, answers to a judge, a child's hand-back): the text cannot unlock a tool, approve a call, widen a grant,
  speak as the user or close its wrapper.
- **Fuzzing** with `cargo-fuzz` in a `fuzz/` crate outside the workspace, for every parser of untrusted bytes or text. A seed
  corpus is committed, a weekly workflow runs each target for a fixed time, and a crash becomes a regression test.
- **Benchmarks** in a `zorix-bench` binary build a realistic data set through the real stores and time the paths people wait
  for. CI runs a tenth of the scale with generous thresholds so only a large regression fails; the numbers that matter are
  recorded from full runs together with the machine they ran on.
- **Coverage** is measured per crate with `cargo llvm-cov` and published. There is no target; the report shows where the audits
  should look first.
- **End-to-end tests drive the built app** through a WebDriver client written for the purpose (no dependency) on Linux and
  Windows, with a throwaway data folder and a screenshot attached to a failure. macOS has no WebDriver for its web view, so it
  gets a launch smoke test; the limit is written down, not hidden.
- **Installers are checked on clean runners** at each release: installed silently, started with the smoke flag, and removed.
  What CI cannot see (the prompts for unsigned builds) is a manual checklist in the distribution guide.

## Consequences

- More CI time: the fuzz, coverage and full-scale benchmark jobs run on a schedule or on demand, not on every push.
- A finding from any harness is fixed on its own `fix/` branch with a regression test, as the guidelines require.
- Flaky tests are not retried silently: a retry is limited and logged, and a test that stays flaky is reported.
