# ADR-0041: Schedules on jiff, with our own cron evaluator

Status: accepted (Phase 10)

## Context

Schedules ([ADR-0034](0034-the-workflow-host-and-triggers.md)) and the workforce's week
([ADR-0039](0039-autonomy-is-decided-by-the-policy.md)) use `chrono`, `chrono-tz` and `croner`, all in
`zorix-workflows/src/schedule.rs`. `chrono` and `chrono-tz` are flagged unmaintained (RUSTSEC-2026-0543, -0555) and are
accepted in `deny.toml` only until they are replaced. `croner` needs `chrono` types, so it cannot stay if `chrono` goes. The
public functions of `schedule.rs` use only `Timestamp` and strings, so the change can stay inside that module.

## Decision

- **Time zones and arithmetic move to `jiff`**, which is maintained and models gaps and folds explicitly. The rule stays the one the
  code has today: a wall-clock time that happens twice (the clocks go back) takes the first; one that does not exist (the clocks go
  forward) moves forward an hour at a time to the first hour that does.
- **Cron expressions are evaluated by our own five-field evaluator** (`cron.rs`): minute, hour, day of month, month and day of
  week, with lists, ranges and steps, and names for months and days. An expression must have exactly five fields, as today.
  Anything else croner accepted that ZORIX never documented (such as `@daily` names, `L`, `#` or `?`) is looked at against the
  fixtures below: it is kept only if it is documented or used, and otherwise refused with a clear message and a note in the
  changelog. The rule for a day that matches both the day of month and the day of week follows what the fixtures show.
- **Before `croner` is removed, its answers become fixtures**: the next 50 fire times of about 200 expressions across about ten
  zones, including daylight-saving changes. The new evaluator must reproduce them exactly, and a fuzz target keeps it from
  panicking on any input.
- `chrono`, `chrono-tz` and `croner` leave the workspace, and their two advisories leave `deny.toml`.

## Consequences

- One small module of our own to maintain, covered by fixtures and a fuzz target, in place of three libraries.
- Schedules fire at the same instants as before. A difference found by the fixtures is a bug in the new evaluator, or a
  documented correction, never a silent change.
