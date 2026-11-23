# Testing

The runtime is tested without any external service: models are simulated, and tests are deterministic. At the time of
writing there are **913** Rust tests plus **61** for the interface helpers.

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --exclude zorix-desktop
cd apps/desktop && npm run check && npm test && npm run build
```

## Kinds of tests

| Kind | Where | What it proves |
|---|---|---|
| Unit | next to the code | The lifecycle table, permission matching, the calculator parser, path rules, redaction, budgeting, retry backoff… |
| Workflows | `zorix-workflows/tests`, `tools/live-workflows.sh` | Expressions, templates and schedules (across daylight saving); the engine with a scripted launcher and a manual clock (joins, skips, budgets, time-outs, restarts); the host and a webhook over a real socket; and the command line end to end |
| Projects | `zorix-store/tests/projects.rs`, `zorix-projects/tests`, `lib/project.test.ts`, `tools/live-projects.sh` | Keys and numbers that are never reused, writes that race on separate connections, a task's run through a scripted launcher (a move starts it, a finish sends it to Review, a failure, a dependency, a stop that asks first), the manager's tools and what each grant reaches, the board's pure rules, and the command line end to end including `zorix serve` |
| Workforce | `zorix-runtime/src/policy.rs` and `tests/autonomy.rs`, `zorix-store/tests/org.rs`, `zorix-workforce/tests`, `lib/workforce.test.ts`, `tools/live-workforce.sh` | Every level against every kind of call (read, held by default, asks by itself, person-only, rule, the worker's list), the run recording a call that ran without asking and a proposal, a child never freer than its parent, the organisation's rules (loops, trees that shrink, departments), spend that counts each cost once, budgets as narrowings, a spent week starting at Ask, a manager's plan and report, a schedule that is off, runs once and records what it missed, `org.team` confined to the caller's reports, the chart's layout, and the command line end to end |
| Model faults | `zorix-sim` tests, `zorix-runtime/tests/model_faults.rs` | A model that fails like a real provider (every kind of error with its pause, a stream cut part-way, an empty, cut-off or refused answer, calls that make no sense, failures that come and go by seed) and how the loop copes: retries, what is shown live, and a tool that never runs twice |
| Faults in storage | `zorix-runtime/tests/faults.rs` (a `FaultyRepository` that breaks the Nth call of a method, before or after it is written), `zorix-store/tests/disk_full.rs`, `tests/concurrency.rs`, `zorix-cli/tests/cli.rs` (`kill -9`) | A commit that fails anywhere in a run and a restart that finishes it without repeating an unsafe tool; a file that cannot grow, told in words, with the work continuing when there is room; four connections writing at once without a lost update or a repeated event number; a real crash of the command line |
| Adversarial | `zorix-runtime/tests/adversarial.rs`, `zorix-agents/tests/delegation.rs`, `zorix-tools` and `zorix-agents` path tables, `zorix-mcp` tool tests, `zorix-integrations` address tables | Text that tries to give itself powers (a result that says the person approved, a child's answer that closes and opens wrappers, a delegation that invents fields to get more, a tool description of a million characters) and paths that try to leave a folder (`..`, drive letters, UNC, reserved names, `.ZORIX`, zip entries) |
| Untrusted text | `zorix-core/src/untrusted.rs` (with property tests), `zorix-store/tests/tampering.rs` | Text from outside cannot close or open a wrapper of ours in any spelling, and a call or approval edited on disk never runs on the answer given to the original |
| Contract | `zorix-runtime/src/testing.rs` | Every `ExecutionRepository` behaves the same. The in-memory and the SQLite repositories run the **same** cases |
| Integration | `zorix-runtime/tests` | The engine with a scripted model and small tools: lifecycle, tools, approvals, failures, recovery, observability |
| Durability | `zorix-store/tests/durability.rs` | What only a real file shows: reopening, concurrent writers, crash recovery |
| Acceptance | `zorix-store/tests/acceptance.rs` | The scenario of the Phase 1 brief, with the real tools and a database file |
| Program | `zorix-cli/tests/cli.rs` | The `zorix` binary as a real program, with a throwaway data folder, across separate processes |

Models are **simulated** by `zorix-sim`: a scripted model (exact turns, delays, hangs, failures of every kind with a `retry_after`, streams that
write piece by piece and can be cut, and responses with any finish reason), a failure-injecting wrapper (the first calls, a chosen error, or a
seed that fails the same calls every time) and a rule-based demo model.

## The required cases, and where each is proven

| # | Case | Tests |
|---|---|---|
| 1 | Creating an execution | `lifecycle::creating_an_execution_stores_it_without_running_it`, `bad_requests_are_refused_before_anything_is_stored` |
| 2 | Valid state transitions | `state::every_state_and_trigger_pair_behaves_as_documented`, `state::every_state_is_reachable_from_created`, `lifecycle::a_run_without_tools_follows_the_documented_lifecycle` |
| 3 | Invalid transitions rejected | `state::terminal_states_accept_nothing`, `lifecycle::steps_the_lifecycle_does_not_allow_are_refused`, `execution::apply_rejects_a_step_the_lifecycle_does_not_allow_and_changes_nothing` |
| 4 | Several iterations | `tools::one_turn_can_ask_for_several_tools_and_the_loop_runs_several_iterations` |
| 5 | Tool calls | `tools::a_tool_call_is_run_and_its_result_goes_back_to_the_model` |
| 6 | Parameter validation | `tools::invalid_arguments_never_reach_the_tool_and_the_model_is_told_why`, `tools::arguments_are_validated_against_the_schema` (runtime) |
| 7 | Unauthorised tools blocked | `tools::a_tool_without_a_grant_is_blocked_when_the_model_asks_for_it`, `tools::text_inside_a_tool_result_cannot_unlock_another_tool`, the `policy::` tests |
| 8 | Human approval | all of `approvals.rs`; `acceptance::the_scenario_from_the_brief` |
| 9 | Cancellation | `lifecycle::cancelling_*`, `approvals::cancelling_a_waiting_execution_closes_its_open_approval` |
| 10 | Failures | `failures.rs`: permanent and temporary model errors, timeouts, rejected answers; `tools::a_tool_that_fails_is_reported_and_the_model_can_go_on` |
| 11 | Limits | `failures::the_iteration_limit…`, `the_tool_call_limit…`, `the_token_limit…`, `the_time_limit…`, `waiting_for_a_person_does_not_use_up_the_time_limit`, `policy::limits_trip_one_by_one_and_name_which` |
| 12 | Persistence | the `contract::` tests on SQLite, `durability::everything_survives_closing_and_reopening_the_file` |
| 13 | Recovery | all of `recovery.rs`; `durability::after_a_crash_a_new_process_continues_from_what_is_on_disk` |
| 14 | Event publication | `lifecycle::events_are_numbered_in_order_and_a_subscriber_sees_the_same_ones_live`, the `events::` tests, `a_subscriber_that_lags_still_sees_everything` |
| 15 | Idempotency | `lifecycle::the_same_idempotency_key_returns_the_same_execution_and_runs_once`, the repository contract, `durability::the_idempotency_key_holds_across_connections`, `recovery::a_crash_during_a_tool_that_may_not_be_repeated_asks_a_person…` |

## Agents

| What | Where |
|---|---|
| The agent file round trip, the strict YAML subset (aliases, size, depth), agent versions and hashes | `zorix-agents` unit and `tests/runner.rs`, `zorix-store/tests/agents.rs` |
| Skill import limits (zip-slip, symlinks, sizes), hashes that do not depend on the platform, the three skill tools and the sandbox | `zorix-agents/tests/skills.rs`, `zorix-hub/tests/agents.rs` |
| The router never calls a model outside the allow-list; live text resets before a fallback | `zorix-providers/tests/router.rs`, `tests/live_text.rs` |
| A run records its agent; the live stream is per execution and per tenant | `zorix-runtime/tests/agent_runs.rs`, `tests/live_text.rs` |
| Agent memory is invisible to other agents and plain runs; a proposal still waits for a person | `zorix-memory/tests/agent_memory.rs` |
| Every check, the judge, refused approvals, dropped writes, comparison | `zorix-agents/tests/evals.rs` |
| The four starter agents parse, import, pin their skills and have valid cases | `zorix-agents/tests/starters.rs` |
| The commands as real programs; agents over MCP with only what the server allows | `zorix-cli/tests/agents.rs`, `tests/mcp_serve.rs` |
| The Markdown renderer cannot produce markup or unsafe links; the thread builder | `apps/desktop/src/lib/ui/markdown.test.ts`, `src/lib/thread.test.ts` |

## Orchestration

| What | Where |
|---|---|
| The new state and triggers, every pair, every state still reachable | `zorix-core` `state::` tests |
| Parallel results in order, no deadlock with one worker and four parents, failures reported, cascading cancel, a child cancelled alone, an approval below decided from the root, depth and descendant limits, a refused plan, the same call after a restart, recovery, roots listing | `zorix-runtime/tests/delegation.rs`, and `trees_are_queries` in the repository contract (both repositories) |
| Grants only shrink, the budget split, loops, archived agents, the parent's model list, the `<available_agents>` index, the `delegated` check, tree-wide approval refusal | `zorix-agents/tests/delegation.rs`, `tests/evals.rs` |
| A 0.5 database upgrades to schema 5; teams; children and roots queries | `zorix-store` schema and `tests/agents.rs` |
| Teams and trees as real programs, a sub-run's approval answered from the root, MCP with `team` | `zorix-cli/tests/orchestration.rs`, `tests/mcp_serve.rs` |
| The tree builder, totals and cost shares; the card rows of a delegation | `apps/desktop/src/lib/tree.test.ts`, `src/lib/delegation.test.ts` |

## Security cases

| Concern | Tests |
|---|---|
| Path escape | `workspace::paths_that_could_escape_are_refused_before_touching_the_disk`, `a_link_that_points_outside_is_not_followed` (Unix), CLI `file_tools_stay_inside_the_workspace…` |
| Tenant isolation | `lifecycle::another_tenant_cannot_see_or_touch_an_execution`, `contract::tenants_are_isolated`, notes per tenant |
| Forged approvals | `approvals::an_approval_applies_to_the_call_it_names_and_cannot_be_forged_or_repeated` |
| Secrets in logs | `tracing_output::spans_follow_the_genai_names_and_nothing_secret_reaches_the_logs`, `tools::secrets_in_arguments_are_kept_out_of_events…`, the `redact::` tests |
| Two programs on one folder | `paths::only_one_holder_at_a_time`, CLI `only_one_zorix_program_works_on_a_data_folder_at_a_time` |

## Conventions

- Tests that wait use a time limit, so a bug fails instead of hanging.
- Anything that spawns a program validates arguments **before** it touches devices or the data folder (a lesson from
  Windows CI runners).
- The `tracing` test lives in its own test binary: `tracing` caches interest per process, so a subscriber installed for one
  test must not share a process with others.
- A new check is proved by making it fail once.

## Cross-platform

The crates other than the desktop shell have no platform code and run in CI on Linux, macOS and Windows. The CLI runs its
work on a thread with a 16 MB stack (the default 1 MB main thread is small on Windows). Unix-only tests (symbolic links)
are marked `#[cfg(unix)]`. The desktop job type-checks and builds the interface and lints the shell on all three systems.

## Dependency and code audits

CI runs `cargo deny check` (known vulnerabilities, licences, banned and unknown sources; the policy and every accepted
advisory with its reason are in `deny.toml`), `npm audit --omit=dev --audit-level=high` for the interface, and CodeQL for
Rust and TypeScript on every push, pull request and weekly. The advisory databases are fetched fresh, so a new advisory can
turn an unchanged tree red; fix it on its own `fix/…` branch with the smallest update that clears it. Run
`cargo deny check` locally before a release.

## Live tests

The adapters are tested against local mock servers, so the normal suite needs no network and no keys. Tests that talk to
real providers are `#[ignore]`d and run with `tools/live-tests.sh`:

- **Ollama** (`ollama serve` and `ollama pull qwen3:1.7b`): discovery, a plain answer, a tool call and a whole execution with
  a real tool loop. A real `/api/show` answer is kept as a fixture.
- **Anthropic, OpenAI, Gemini**: the same, when a key is stored (`zorix provider set-key <name>`); without one the test is
  skipped. They cost a few cents. Models come from `ZORIX_LIVE_<NAME>_MODEL`.

- **Embeddings and memory** (`tools/live-tests.sh embeddings`; `ollama pull all-minilm`): stable dimensions and similar text
  scoring higher; this repository's `docs/` indexed, with differently worded questions finding the right file and an unrelated
  question finding nothing; and `qwen3:1.7b` answering from an indexed runbook, checked against the recorded context. OpenAI and
  Gemini embeddings run when a key is stored.

The tests print lengths, tool names and token counts, never a key or an answer.

### Integrations

The MCP client is tested against `zorix-fake-mcp`, a server in the repository that can play a well-behaved server of either
era or a hostile one (changes definitions, floods, never answers, asks for sampling, sends oversized messages, crashes), and
against mock HTTP servers for transports and OAuth (wiremock). The native tools are tested against mock APIs that check exact
requests, that credentials never appear in URLs or logs, and the SSRF cases (loopback, `169.254.169.254`, a name that resolves to
a private address, a redirect to an ungranted host). SQLite is tested on real files; Postgres runs against a real server in the
CI job `Postgres tools` (the tests skip when `ZORIX_TEST_POSTGRES` is not set). The sandbox is tested with an infinite loop, a
memory bomb, reading `/etc/passwd`, a network attempt and an output flood; the Python tests run where `ZORIX_PYTHON_WASM` points at
the module (CI fetches it on Linux). `zorix mcp serve` is driven by ZORIX's own client over a real process. The CLI has process
tests for `mcp`, `connect` and `sandbox`, using `zorix mcp serve` as the server under test.

`tools/live-tests.sh integrations` runs the real command line against real servers: DeepWiki and Cloudflare docs, the
`server-everything` and `server-filesystem` packages through `npx`, a public GitHub read with no token, and Python in the
sandbox. It uses no account of yours.

### Orchestration

`tools/live-tests.sh orchestration` runs the real command line on a throwaway data folder and always checks, with a scripted model: the research desk installs; a lead asks two members at once and
the tree, the list of roots and the cost roll-up are right; delegation nests three levels and an agent outside the lead's access is refused with a reason; a sub-run's approval is answered from the
run you started; cancelling the root cancels the sub-run; and a tool call made by a sub-run counts against the parent's limit. The scripted model is shared by a whole run, so each script lists the turns of the
lead and its sub-runs in the order they are used. When Ollama **already** runs here (this script never starts it) the research desk answers with a real local model; the result is shown, not asserted.

### Agents

`tools/live-tests.sh agents` runs the real command line on a throwaway data folder (your data and keychain are not touched). It always checks, with a
scripted model, that an agent loads a skill and **runs its JavaScript script in the sandbox**, that a run records its agent and version, that a model outside
an agent's list is refused with a reason, that a template installs, answers, exports with its skill and imports somewhere else, and that an evaluation runs
and passes. When Ollama runs here and `ZORIX_LIVE_OLLAMA_MODEL` (default `qwen3:1.7b`) is pulled, the `writer` template answers with the real model and its
smoke cases are tried; the result is shown and not asserted, because it depends on the model. No cloud provider is used.
