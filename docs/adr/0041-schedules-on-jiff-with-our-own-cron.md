# ADR-0041: Schedules on jiff, with our own cron evaluator

Status: accepted (Phase 10)

## Context

Schedules ([ADR-0034](0034-the-workflow-host-and-triggers.md)) and the workforce's week
([ADR-0039](0039-autonomy-is-decided-by-the-policy.md)) use `chrono`, `chrono-tz` and `croner`, all in
`zorix-workflows/src/schedule.rs`. `chrono` and `chrono-tz` are flagged unmaintained (RUSTSEC-2026-0543, -0555) and are
were accepted in `deny.toml` only until they were replaced. `croner` needs `chrono` types, so it cannot stay if `chrono` goes. The
public functions of `schedule.rs` use only `Timestamp` and strings, so the change can stay inside that module.

## Decision

- **Time zones and arithmetic move to `jiff`**, which is maintained and models gaps and folds explicitly.
- **Two rules for the clocks changing, one for each kind of schedule.**
  - *Fixed times of the day* (`Daily`) keep the rule the code had: a time that happens twice (the clocks go back) takes the first; one that does not exist (the clocks go
    forward) moves forward an hour at a time to the first hour that does, so 02:30 becomes 03:30.
  - *Cron* keeps what the library it replaces did, which the golden answers show: a schedule of fixed times (`30 2 * * *`) fires, for a time the clocks jump over, at the first instant
    after the jump (03:00), and once in an hour that happens twice (the first time); a schedule with a `*` or a step in its minute or hour (`0 * * * *`, `*/15 * * * *`, `*/30 1-3 * * *`) keeps
    its beat, so it does not fire for a skipped minute and fires every time through a repeated hour.
- **Cron expressions are evaluated by our own five-field evaluator** (`cron.rs`): minute, hour, day of month, month and day of week; lists, ranges and steps (`*/15`, `10-50/10`); names for
  months and days in any case; Sunday as 0 or 7 (`mon-sun` is the whole week); exactly five fields. When the day of month and the day of week are both restricted a day matches if either does, and a
  plain `*` leaves it to the other (a step is not a `*`). `a/n` without a range, a reversed range, an empty item and a value outside its field are refused with a sentence that says which.
  What the old library also accepted and was never documented or used is refused now, with a message: `L` (the last day), `W`, `#` (the first Monday) and `?`, and an empty item in a list. It is a change of what is accepted, and it is in the changelog.
- **The old library's answers were kept before it was removed**: the next 14 fires of 2,300 combinations of an expression, a zone and a starting moment (about 100 expressions, six zones, nine starting moments,
  including ten minutes before a clock change in New York, Lisbon and Sydney), in `crates/zorix-workflows/tests/fixtures/cron_golden.json`. The new evaluator reproduces all that the old one answered, and every
  difference but the refused forms had to be explained and resolved before the library went. A test also checks the evaluator against a search a minute at a time on random expressions, and a fuzz
  target keeps it from panicking on any input.
- `chrono`, `chrono-tz` and `croner` leave the workspace, and their two advisories leave `deny.toml`.

## Consequences

- One small module of our own to maintain, covered by fixtures and a fuzz target, in place of three libraries.
- Schedules fire at the same instants as before. A difference found by the fixtures is a bug in the new evaluator, or a
  documented correction, never a silent change.
