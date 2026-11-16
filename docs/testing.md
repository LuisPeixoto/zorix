# Testing

The runtime is tested without any external service: models are simulated, and tests are deterministic. At the time of
writing there are ** Rust tests** plus **7** for the interface helpers.

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --exclude zorix-desktop
cd apps/desktop && npm run check && npm test && npm run build
```

## Kinds of tests

| Kind | Where | What it proves |
|---|---|---|
| Unit | next to the code | The lifecycle table, permission matching, the calculator parser, path rules, redaction, budgeting, retry backoff… |
| Contract | `zorix-runtime/src/testing.rs` | Every `ExecutionRepository` behaves the same. The in-memory and the SQLite repositories run the **same** cases |
| Integration | `zorix-runtime/tests` | The engine with a scripted model and small tools: lifecycle, tools, approvals, failures, recovery, observability |
| Durability | `zorix-store/tests/durability.rs` | What only a real file shows: reopening, concurrent writers, crash recovery |
| Acceptance | `zorix-store/tests/acceptance.rs` | The scenario of the Phase 1 brief, with the real tools and a database file |
| Program | `zorix-cli/tests/cli.rs` | The `zorix` binary as a real program, with a throwaway data folder, across separate processes |

Models are **simulated** by `zorix-sim`: a scripted model (exact turns, delays, failures, hangs), a failure-injecting
wrapper, and a rule-based demo model.

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

## Live tests

The adapters are tested against local mock servers, so the normal suite needs no network and no keys. Tests that talk to
real providers are `#[ignore]`d and run with `tools/live-tests.sh`:

- **Ollama** (`ollama serve` and `ollama pull qwen3:1.7b`): discovery, a plain answer, a tool call and a whole execution with
  a real tool loop. A real `/api/show` answer is kept as a fixture.
- **Anthropic, OpenAI, Gemini**: the same, when a key is stored (`zorix provider set-key <name>`); without one the test is
  skipped. They cost a few cents. Models come from `ZORIX_LIVE_<NAME>_MODEL`.

The tests print lengths, tool names and token counts, never a key or an answer.
