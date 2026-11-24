# Testing

The runtime is tested without any external service: models are simulated, and tests are deterministic. At the time of
writing there are **1,172** Rust tests (and 11 that need a real service and run only when asked) plus **154** for the interface.

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --exclude zorix-desktop
cd apps/desktop && npm run check && npm test && npm run build
```

## Kinds of tests

| Kind | Where | What it proves |
|---|---|---|
| Unit | next to the code | The lifecycle table, permission matching, the calculator parser, path rules, redaction, budgeting, retry backoff… |
| Schedules | `zorix-workflows/src/cron.rs` and `schedule.rs` tests, `tests/cron_golden.rs` | The cron grammar and what is refused, a day of month and a day of week together, the clocks going forward and back, leap days, a search a minute at a time on random expressions, and 2,300 answers kept from the library that was replaced (the new evaluator gives every one of them, except the forms it now refuses on purpose) |
| Workflows | `zorix-workflows/tests`, `tools/live-workflows.sh` | Expressions, templates and schedules (across daylight saving); the engine with a scripted launcher and a manual clock (joins, skips, budgets, time-outs, restarts); the host and a webhook over a real socket; and the command line end to end |
| Projects | `zorix-store/tests/projects.rs`, `zorix-projects/tests`, `lib/project.test.ts`, `tools/live-projects.sh` | Keys and numbers that are never reused, writes that race on separate connections, a task's run through a scripted launcher (a move starts it, a finish sends it to Review, a failure, a dependency, a stop that asks first), the manager's tools and what each grant reaches, the board's pure rules, and the command line end to end including `zorix serve` |
| Workforce | `zorix-runtime/src/policy.rs` and `tests/autonomy.rs`, `zorix-store/tests/org.rs`, `zorix-workforce/tests`, `lib/workforce.test.ts`, `tools/live-workforce.sh` | Every level against every kind of call (read, held by default, asks by itself, person-only, rule, the worker's list), the run recording a call that ran without asking and a proposal, a child never freer than its parent, the organisation's rules (loops, trees that shrink, departments), spend that counts each cost once, budgets as narrowings, a spent week starting at Ask, a manager's plan and report, a schedule that is off, runs once and records what it missed, `org.team` confined to the caller's reports, the chart's layout, and the command line end to end |
| Model faults | `zorix-sim` tests, `zorix-runtime/tests/model_faults.rs` | A model that fails like a real provider (every kind of error with its pause, a stream cut part-way, an empty, cut-off or refused answer, calls that make no sense, failures that come and go by seed) and how the loop copes: retries, what is shown live, and a tool that never runs twice |
| Faults in storage | `zorix-runtime/tests/faults.rs` (a `FaultyRepository` that breaks the Nth call of a method, before or after it is written), `zorix-store/tests/disk_full.rs`, `tests/concurrency.rs`, `zorix-cli/tests/cli.rs` (`kill -9`) | A commit that fails anywhere in a run and a restart that finishes it without repeating an unsafe tool; a file that cannot grow, told in words, with the work continuing when there is room; four connections writing at once without a lost update or a repeated event number; a real crash of the command line |
| Adversarial | `zorix-runtime/tests/adversarial.rs`, `zorix-agents/tests/delegation.rs`, `zorix-tools` and `zorix-agents` path tables, `zorix-mcp` tool tests, `zorix-integrations` address tables | Text that tries to give itself powers (a result that says the person approved, a child's answer that closes and opens wrappers, a delegation that invents fields to get more, a tool description of a million characters) and paths that try to leave a folder (`..`, drive letters, UNC, reserved names, `.ZORIX`, zip entries) |
| Coverage | the **Coverage** CI job, `tools/coverage-summary.mjs` | What the tests run, per crate, and which files nothing tests |
| End-to-end | `tools/e2e/` (a WebDriver client of our own, the scenarios, the smoke check), the **End-to-end** workflow | The built app on a throwaway data folder: every part of it opens, a run with the demo model finishes, a change waits at an approval and carries on after Approve and after Refuse, a project is created, the workforce shows you. Linux drives it through tauri-driver; macOS and Windows only start the app and wait for it to say it has loaded (`ZORIX_SMOKE`; the reasons are under the limits below) |
| Interface text | `lib/i18n/*.test.ts`, `tools/check-text.mjs` | Messages are filled in, plurals follow the language, a key that does not exist is a type error, no message is dead, the pseudo-locale touches every message, formats are right in English, German and Portuguese, and the lint fails on a sentence in a component (proved with a probe file) |
| Benchmarks | `crates/zorix-bench`, the **Benchmarks** CI job | The paths people wait for, at 10,000 runs, a million events and 50,000 pieces of documents, with limits that fail a large regression |
| Fuzzing | `fuzz/` (14 targets, a seed corpus each), `.github/workflows/fuzz.yml` | Every reader of text or bytes from outside (the agent file and its YAML, `SKILL.md`, server-sent events cut anywhere, MCP messages and other clients' configs, workflow files, conditions, templates, cron expressions and zones, documents, access patterns, file paths, the data wrappers) takes any input without a panic or a hang, and keeps the property it promises |
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

## Coverage

```bash
cargo install cargo-llvm-cov            # once, and: rustup component add llvm-tools-preview
cargo llvm-cov --workspace --exclude zorix-desktop --exclude zorix-bench --json --summary-only --output-path cov.json
node tools/coverage-summary.mjs cov.json            # a table per crate (add --markdown for a Markdown table)
```

The **Coverage** CI job does the same on every change, writes the table into the job's summary and keeps `lcov.info` and `coverage.json` as an artifact. It is a report and not a gate: there is no number to reach, and
the table says where an audit ([roadmap](roadmap.md#how-a-quality-phase-works)) should look first. What the tests cover on 2026-11-23, at 0.9.0 plus the Phase 10 work (lines, with functions and regions beside them):

| Crate | Lines | Functions | Regions |
|---|---|---|---|
| `zorix-agents` | 92.9 % (1824/1963) | 88.8 % | 90.5 % |
| `zorix-cli` | 51.0 % (2862/5608) | 47.2 % | 49.0 % |
| `zorix-core` | 94.8 % (3654/3856) | 94.9 % | 95.3 % |
| `zorix-hub` | 85.5 % (655/766) | 81.4 % | 80.4 % |
| `zorix-integrations` | 82.5 % (2354/2854) | 72.9 % | 79.7 % |
| `zorix-mcp` | 84.4 % (2757/3268) | 81.6 % | 82.5 % |
| `zorix-memory` | 91.5 % (2242/2451) | 88.8 % | 91.5 % |
| `zorix-projects` | 79.6 % (1083/1361) | 73.6 % | 76.8 % |
| `zorix-providers` | 92.2 % (3375/3662) | 87.7 % | 92.3 % |
| `zorix-runtime` | 94.2 % (3112/3304) | 93.1 % | 94.4 % |
| `zorix-sandbox` | 83.7 % (319/381) | 83.0 % | 81.8 % |
| `zorix-sim` | 97.9 % (749/765) | 96.3 % | 96.9 % |
| `zorix-store` | 97.2 % (3984/4098) | 90.8 % | 88.1 % |
| `zorix-tools` | 95.0 % (805/847) | 84.0 % | 94.2 % |
| `zorix-workflows` | 87.6 % (2715/3101) | 83.1 % | 84.2 % |
| `zorix-workforce` | 91.1 % (1111/1219) | 81.4 % | 89.3 % |
| **all** | **85.1 %** (33601/39504) | **81.6 %** | **82.7 %** |

What the numbers say, so the audits start from them:

- **The command line is the least covered (51 %).** Its project, workflow and workforce commands (`zorix-cli/src/projects.rs`, `workflows.rs`, `workforce.rs`) show 0 %: they are exercised end to end by `tools/live-projects.sh`, `live-workflows.sh` and `live-workforce.sh`, which
  run the real program, but not by `cargo test`, so a change to them is not caught by the gates. Phase 14 brings them into the test suite. `mcp_serve.rs` is at 21 % for the same reason.
- **`postgres_db.rs` (30 %)** is covered only by the **Postgres tools** job, which runs against a real server and is not part of this measurement.
- **Below 80 %** among the code that people's data goes through: `zorix-projects/src/rules.rs` (73 %), `zorix-workflows/src/service.rs` (75 %), `zorix-providers/src/secrets.rs` (79 %, the keychain paths that need a real keychain), `zorix-hub/src/lib.rs` (72 %).
- This measurement was made without the Python module of the sandbox tests, so `zorix-sandbox` (84 %) is lower here than in the CI job, which downloads it.
- The core of the product is well covered: `zorix-core` 95 %, `zorix-runtime` 94 %, `zorix-store` 97 %, `zorix-sim` 98 %.

Coverage counts lines that ran, not whether the test would notice a wrong answer; it finds what nothing tests, and says nothing about the rest.

## End-to-end tests

```bash
cd apps/desktop && npx tauri build --debug --no-bundle     # build the app once
node tools/e2e/smoke.mjs target/debug/zorix-desktop        # any system: starts it and waits for "ZORIX ready"
cargo install tauri-driver --locked                        # Linux only, then:
node tools/e2e/run.mjs target/debug/zorix-desktop          # drives it through tauri-driver (needs a display; xvfb-run on a server)
```

`tools/e2e/webdriver.mjs` is a small client for the W3C WebDriver protocol (just `fetch`), `scenarios.mjs` holds the flows, and `run.mjs` gives each scenario a fresh data folder and a fresh app. A failing scenario leaves a screenshot and the
page's source in `e2e-artifacts/`, which the **End-to-end** workflow uploads. Scenarios use what a person sees (a button's text, a page's title), so they break when the interface's words change: that is on purpose, and the fix is one line.

**The limits.** Only Linux drives the flows. WebKit's web view on macOS has no WebDriver, so macOS only checks that the built app starts and loads its interface (`ZORIX_SMOKE`, the `app_ready` command). On Windows the app starts and loads, but `msedgedriver` never finds the web view's debugging port on the runner, so Windows has the same launch check until [#27](https://github.com/LuisPeixoto/zorix/issues/27) is solved. The interface code is the same on all three.

## Benchmarks

`zorix-bench` builds a data folder through the real stores and times what a person waits for. It is a program, not a test:

```bash
cargo run --release -p zorix-bench -- --scale 1 --json bench.json    # the full size; --scale 0.1 is a tenth
cargo run --release -p zorix-bench -- --scale 0.1 --check crates/zorix-bench/limits.json
```

The full size is 10,000 runs with 1,000,000 events, 200 agents (the most a tenant keeps), 5,000 facts (the most a tenant keeps) and 50,000 pieces of documents with a vector each. A tenth of it runs in CI
(**Benchmarks**), where every measurement is compared with a limit in `limits.json` that is many times what a healthy run takes, so that only a large regression fails; a measurement with no limit fails too, so a new one is not forgotten.

Full-size numbers, in milliseconds (the median of 15 runs, release build, an Intel 3.4 GHz Mac with an SSD; the data folder is 345 MB):

| What a person waits for | Before the list indexes | Now |
|---|---|---|
| The newest page of runs (100) | 0.3 | 0.3 |
| Runs of one agent | 9 | 0.4 |
| Runs waiting for approval | 18 | 0.4 |
| Runs of one conversation | 17 | 0.1 |
| Runs since a week ago | 5 | 5 |
| Replaying all 10,000 events of a run (in pages of 1,000) | 10 | 9 |
| The Inbox: runs waiting for a person, and the approvals of fifty of them | 0.3 and 1 | 0.4 and 1 |
| Keyword search over 5,000 facts | 2 | 2 |
| Keyword search over 50,000 pieces of documents | 67 | 66 |
| Search by meaning over 50,000 pieces (every vector is read) | 190 | 188 |
| Opening the store again | 0.5 | 0.5 |

The first full-size run showed three lists that read and sorted every match (an agent's runs, the runs in one state, a conversation's); schema 10 gives each an index that ends in the order of the list, and a test checks the planner uses it.

Search by meaning grows with the number of vectors, since each is compared with the query; at the product's limit of 200,000 pieces it would be about four times as long. An on-disk index for large collections is planned
([roadmap](roadmap.md), Phase 11).

## Fuzzing

`fuzz/` is a separate crate (not in the workspace: it needs nightly Rust and `cargo install cargo-fuzz`) with one target for each reader of untrusted text or bytes. A target does more than "no panic": the server-sent-events reader must give the same
events however its input is cut, a workflow file that parses must write out and read back the same, a cron expression's next fire must be after the moment asked from, an accepted path must never climb or name the
reserved folder, and the data wrapper must always have one opening, one closing and nothing live inside.

```bash
cd fuzz
cargo +nightly fuzz run sse            # until you stop it; add  -- -max_total_time=60  to stop by itself
cargo +nightly fuzz list
```

A small seed corpus for each target is committed in `fuzz/corpus/`; what a run finds goes to a folder of your own (`cargo +nightly fuzz run sse /tmp/work fuzz/corpus/sse`), not into the repository. The **Fuzz** workflow runs every target for two minutes
each week and on demand, and a crash is uploaded as an artifact. A crash is fixed on its own `fix/` branch, and the input that found it becomes a test in the crate that owns the code. A fuzz build with the address sanitizer
compiles every dependency again and needs several gigabytes: set `CARGO_TARGET_DIR` to a disk with room.

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
