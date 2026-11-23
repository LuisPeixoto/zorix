# ADR-0035: People steps and unattended actions

Status: accepted (Phase 7)

## Context

Approvals today exist only for a tool call inside a model loop (a `call_id` and a `call_hash`). A workflow must also stop for a person
without a model: "On-call approves", "Ask the owner which environment". It also runs actions with nobody present, which the tool
policy would stop at the first write.

## Decision

- A **workflow wait** is a record (`workflow_waits`) that is not tied to a tool call. Kinds: *approval*, *question*, *timer*. It has an
  optional due time and, for a person, a **fingerprint**: the SHA-256 of exactly what the person was shown (the fields the step chose
  to show, and the step's definition hash). The decision carries that fingerprint, and a mismatch is refused, as with tool approvals.
- **Approval step:** who decides is *a person* (there are no accounts yet, so the person using the app or the command line), what they
  are shown (summary, chosen fields, cost), a time-out, and *then* **refuse** or **continue**. A reminder in Slack is offered only when
  a Slack connection exists. An approval can be answered once.
- **Question step:** free text with an optional default and a time-out. The answer enters the run as data.
- Waits appear in the **Inbox** and in the workflow run view, with the same card as every other approval, and are answered with
  `zorix workflow answer`. An agent step's own tool approvals still rise to its root run, as in ADR-0030; the step shows *waiting
  for a person*.
- **Actions ask first by default.** An action step has `ask: always` (the default) or `never`. With `always`, the step makes a wait
  bound to the call's `call_hash`, like any tool call. `never` needs a person's explicit confirmation **when publishing**: the
  version records who confirmed, when, and the hash of each confirmed step. It is refused for destructive tools (`db.execute`,
  `workspace.write`, `code.run_in_workspace`) and for tools a person must approve (`memory.remember`). Resource grants (a channel, a
  host) apply as they do for agents. Editing a confirmed step drops its confirmation and needs a new one at the next publish.
- **Test runs** ask before every action.
- An action interrupted mid-flight (the app stopped) is not repeated: it becomes a wait that asks what to do, the same rule as an
  unknown tool outcome in a run.

## Consequences

- Publishing is the moment a person accepts unattended actions, in one place, with a record.
- "On-call rotation" as the decider needs people and accounts; it is deferred to Phase 16 and listed as a known difference.
- The Inbox gains a third kind of item next to tool approvals and server reviews.
