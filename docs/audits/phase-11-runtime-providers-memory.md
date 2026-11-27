# Audit: runtime, providers and memory (Phase 11)

2026-11-27 · commit `5edaca3` (v0.10.0) · audited by reading the code of Phases 1 to 3 against
[sections 2 to 14](../engineering-guidelines.md) of the guidelines, with the questions written in the
[template](README.md). Status: **open**. Rows are added as the fixes uncover more, and the audit closes at the end of the phase.

## Scope

Looked at:

- **Runtime** (`zorix-runtime`, `zorix-core`, `zorix-store`): leases, crash recovery, idempotency of tool calls, the
  context builder, retries, limits and cost accounting, redaction on every path that stores text, migrations.
- **Providers** (`zorix-providers`): the five adapters, the router, error classification and fallback, timeouts.
- **Memory** (`zorix-memory`, the memory part of `zorix-store`): scopes, recall, the folder indexer, search, embeddings.

Left out: the tools, integrations and sandbox (Phase 12), agents, teams and orchestration (Phase 13), workflows,
projects and the workforce (Phase 14), and the interface (Phase 15). Where this audit touched them (past work comes from
every kind of run) it says so.

## Findings

A security finding is fixed first. Every finding is an issue in the milestone
[v0.11.0](https://github.com/LuisPeixoto/zorix/milestone/2).

| # | Section | Finding | Severity | Issue | Fixed in |
|---|---|---|---|---|---|
| F1 | 6 | Past work has no agent column: the past work of agent A in a project is recalled by agent B and by plain runs of that project | High | [#37](https://github.com/LuisPeixoto/zorix/issues/37) | `fix/past-work-isolation` |
| F2 | 6 | Past work stores tool arguments through the text-only redaction, so `{"password":"…"}` is kept and recalled | High | [#38](https://github.com/LuisPeixoto/zorix/issues/38) | `fix/redact-past-work-arguments` |
| F3 | 6 | Text redaction misses `password=` pairs, PEM keys, Google, Stripe and GitLab keys; the project, organisation and report event tables are not redacted | Medium | [#39](https://github.com/LuisPeixoto/zorix/issues/39) | `fix/redaction-patterns` |
| F5 | 6 | Re-indexing a file leaves its old text in past runs' snapshots (chunk ids can be reused, which stops mattering once every deletion scrubs: no stale reference is left for a new piece to inherit) | Medium | [#40](https://github.com/LuisPeixoto/zorix/issues/40) | `fix/scrub-reindexed-chunks` |
| F6 | 8, 13 | A lost lease only logs; the run keeps calling models and tools beside the engine that took it over | Medium | [#41](https://github.com/LuisPeixoto/zorix/issues/41) | `fix/lease-fencing` |
| F7 | 8 | The 60 s model timeout bounds the whole call and cuts long streamed answers that the HTTP layer allows | Medium | [#42](https://github.com/LuisPeixoto/zorix/issues/42) | `fix/model-timeout` |
| F8 | 7 | The indexer computes the file hash and never compares it | Low | [#43](https://github.com/LuisPeixoto/zorix/issues/43) | `fix/indexer-skip-by-hash` |
| F9 | 7 | Counting by loading every row, a full-tenant count per file written, one query per past-work hit, and a vector scan that holds the store's only connection | Medium | [#44](https://github.com/LuisPeixoto/zorix/issues/44) | |
| F11 | 8, 13 | A cost limit never trips for a model with no known price, and the screens show $0.00 | Medium | [#45](https://github.com/LuisPeixoto/zorix/issues/45) | |
| F12 | 2 | No test covers project scope on the vector path over documents | Low | [#46](https://github.com/LuisPeixoto/zorix/issues/46) | `test/vector-project-scope` (no bug: the behaviour was right, now it is proved) |
| F10 | 11 | About 170 validation sentences of these areas are text, not codes | Low | [#26](https://github.com/LuisPeixoto/zorix/issues/26) | |

Kept as designed, and written down so nobody has to rediscover it:

- `checkpoints`, `tool_calls` and `approvals` keep exact arguments and the transcript, because resuming a run and binding
  an approval to its call hash need them. They are not redacted in storage; they are redacted wherever they are published.
  Retention (this phase) bounds how long they live.
- A PDF that takes too long is left running on its own thread (a thread cannot be killed). The indexer allows one at a time,
  so at most one stray thread exists.
- Memory facts that look like secrets are refused rather than redacted, so a person sees why.

## Checked, nothing found

| Section | What | How |
|---|---|---|
| 2, 8 | A tool never runs twice after a crash | `zorix-runtime/tests/{recovery,faults,model_faults}.rs` (including a storage failure at every commit and seeded model faults), `zorix-cli/tests/cli.rs` (`kill -9`) |
| 8 | A non-idempotent call in `Started` becomes an approval that says the outcome is unknown | `ask_about_unknown_outcome` and its test in `recovery.rs` |
| 6 | An approval is bound to its call hash | `Engine::recover` and the policy; covered by the Phase 10 regression tests |
| 6 | Tenant isolation of facts, documents and vectors | `scope_sql` binds the tenant on every query; `zorix-store/tests/memory.rs` |
| 8, 13 | Provider failure and fallback | `zorix-providers/tests/router.rs` (fallback, cool-down, allow-list, explicit model never falls back) and the wiremock suites of each adapter |
| 10 | Migrations apply in order and refuse a newer database | `schema.rs` upgrade tests from every earlier version |

## Not verified

- The full pass over the interface of these areas (Phase 15 audits every screen).
- Behaviour with a real Anthropic, OpenAI or Gemini key: those adapters are checked against mock servers only, as in
  Phases 2 and 3.
- Windows and Linux for the new work, until CI runs it.
