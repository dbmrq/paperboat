# Coverage Plan

Updated: 2026-03-11

This plan is the current-branch handoff for follow-on coverage work. It is intentionally derived from the artifacts already present in the repo instead of re-auditing from source:

- `coverage-summary.json`
- `coverage.txt`
- `lcov.info`
- `.paperboat/docs/coverage-audit.md`

The goal is to help later agents choose meaningful tests quickly, especially in branch-touched low-coverage areas, without wasting time on already-healthy modules or brittle smoke coverage.

## Confirmed Baseline

Confirmed from `coverage-summary.json`:

- Lines: `14137 / 20293` (`69.66%`)
- Regions: `22385 / 31810` (`70.37%`)
- Functions: `1684 / 2292` (`73.47%`)

Largest subsystem gaps by uncovered lines:

| Subsystem | Coverage | Uncovered lines | Why it matters |
| --- | ---: | ---: | --- |
| `src/app/*` | 42.46% | 1622 | Core orchestration, lifecycle, retries, socket/session behavior |
| `src/tui/widgets/*` | 30.56% | 843 | User-visible render/state behavior, several near-zero widgets |
| `src/backend/cursor/*` | 65.71% | 679 | Cursor bootstrap, MCP config mutation, transport/session setup |
| `src/tui/*` excluding widgets/events/layout | 76.37% | 557 | Startup loop and shell-level UI behavior |
| `src/main.rs` | 0.00% | 499 | Large zero-coverage entrypoint, but low ROI |
| `src/mcp_server/handlers/*` | 60.23% | 381 | Tool parsing and response guidance affect agent interoperability |
| `src/self_improve/*` | 75.91% | 323 | Most remaining gap is concentrated in `runner.rs` |

Largest file gaps still worth planning around:

| File | Coverage | Uncovered lines |
| --- | ---: | ---: |
| `src/self_improve/runner.rs` | 33.64% | 284 |
| `src/app/mod.rs` | 25.39% | 238 |
| `src/app/orchestrator.rs` | 47.09% | 218 |
| `src/backend/cursor/acp.rs` | 0.00% | 204 |
| `src/mcp_server/handlers/response.rs` | 0.00% | 200 |
| `src/app/agent_handler.rs` | 0.00% | 185 |
| `src/app/agent_spawner.rs` | 28.23% | 178 |
| `src/backend/cursor/mcp_config.rs` | 15.53% | 136 |
| `src/mcp_server/handlers/tool_parsing.rs` | 16.88% | 133 |
| `src/app/socket.rs` | 4.03% | 119 |
| `src/app/agent_session_handler.rs` | 0.00% | 116 |
| `src/tui/widgets/backend_selection.rs` | 2.88% | 101 |

## Meaningful Improvement Rules

Later agents should treat coverage as a lagging indicator. A test is a meaningful improvement only when it protects a real behavior slice.

Prefer tests that:

- Cover a previously untested branch that changes task state, session lifecycle, retry behavior, backend configuration, tool parsing, or rendered UI state.
- Assert an observable outcome, not merely execution. Good assertions include task status transitions, captured tool responses, config mutation, socket cleanup, selected backend changes, rendered text, scroll position, or popup visibility.
- Add a realistic failure mode or edge case.
- Reuse the existing harness, mock transports, mock backend, or tempdir setup already used in the repo.
- Remove a blind spot in a large low-coverage file by covering one coherent behavior slice.

Avoid tests that:

- Only assert constructors, getters, or serde round-trips unless compatibility is the actual risk.
- Duplicate an existing happy-path scenario with no new assertions.
- Exist mainly to execute logging, tracing, or formatting code.
- Use broad smoke coverage to touch big files without pinning user-visible behavior.
- Target already-healthy files purely because they are easy to move from the low 90s into the mid 90s.

Rule-of-thumb acceptance criteria:

- Parser/formatter modules: add success and failure cases, plus one realistic edge or deprecated path.
- Orchestration modules: prove a state transition, retry path, reconciliation rule, or tool-call sequence.
- UI modules: assert specific render/state outcomes under focused interactions, not just "does not panic".

## Preferred Test Styles Already In Use

Follow the project’s existing test stack instead of inventing a new one.

### 1. Unit tests

Use inline `#[cfg(test)]` modules or focused crate test modules for deterministic logic.

Best for:

- Parser validation
- Response text generation
- Retry classification
- Scroll and selection helpers
- Builder/mapping logic

Examples already in the repo:

- `src/testing/unit_tests.rs`
- Inline tests in files like `src/tui/widgets/agent_output.rs`

### 2. Harness-backed scenario tests

Use `TestHarness` plus `MockScenario` and the existing TOML scenarios when the behavior spans planner, orchestrator, implementer, tool interception, or final task state.

Best for:

- Planner -> orchestrator -> implementer flow
- Decomposition
- Retry/recovery patterns
- `skip_tasks` and completion reconciliation
- Tool-call capture and ordering

Existing primitives:

- `src/testing/harness.rs`
- `src/testing/integration_tests.rs`
- `src/testing/e2e_tests.rs`
- `tests/scenarios/simple_implement.toml`
- `tests/scenarios/planning_only.toml`
- `tests/scenarios/nested_decompose.toml`
- `tests/scenarios/multi_implement.toml`
- `tests/scenarios/error_recovery.toml`
- `tests/scenarios/concurrent_agents.toml`
- `tests/scenarios/planner_failure.toml`

Preferred pattern:

- Extend an existing scenario first.
- Add a new TOML scenario only when the behavior does not fit the current set cleanly.
- Assert specific task/tool/session outcomes, not just overall success.

### 3. Tempdir and mock-backed integration tests

Use `tempfile::tempdir()`, filesystem fixtures, and mock backend/transport layers for subsystems that mutate config or depend on local IO/process setup.

Best for:

- Cursor MCP config creation and replacement
- Cache hit/miss behavior
- Self-improvement run directory handling
- Config loading/writing branches

Examples already in the repo:

- `src/backend/cursor/mcp_config.rs`
- `src/config/loader.rs`
- `src/config/writer.rs`
- `src/self_improve/context_builder.rs`
- `src/testing/mock_backend.rs`

Preferred pattern:

- Use tempdirs for isolated config state.
- Preserve unrelated config entries and assert exact mutated output.
- Mock process/bootstrap layers instead of trying to spawn real external tools.

### 4. `ratatui` render and state tests

Use render-focused assertions for widgets and direct state assertions for navigation/scroll logic.

Best for:

- Popup visibility and selection wraparound
- Scroll clamping and auto-scroll behavior
- Rendered text and indicator changes
- Focus-sensitive border/title behavior

Examples already in the repo:

- `src/tui/layout.rs`
- `src/tui/agent_tree_state.rs`
- `src/tui/task_list_state.rs`
- Inline tests in `src/tui/widgets/agent_output.rs`

Preferred pattern:

- Use `ratatui` test backends or equivalent focused render harnesses.
- Assert targeted text, selection index, scroll offset, and visibility flags.
- Avoid brittle full-screen snapshots when a smaller state assertion is enough.

## Branch-Touched High-Value Targets

The current branch already modifies several low-coverage areas. Later agents should prefer adding tests adjacent to those changes before going hunting elsewhere.

### P0: MCP parsing and response text

Files:

- `src/mcp_server/handlers/response.rs`
- `src/mcp_server/handlers/tool_parsing.rs`

Why first:

- Fastest risk-reduction per test.
- Directly affects agent/tool interoperability.
- `response.rs` is currently at `0.00%`.

Best next behaviors:

- Pending-task guidance in `build_response_text_with_state()`
- Blocked vs parallelizable vs complete messaging
- Actionable error text for failure responses
- Required argument validation for every tool
- Deprecated fallback paths such as `task` vs `task_id`
- `skip_tasks` empty-array rejection
- Mixed valid/invalid `spawn_agents` payloads

Preferred style:

- Table-driven unit tests

### P0: Core orchestration in `src/app/*`

Files:

- `src/app/mod.rs`
- `src/app/orchestrator.rs`
- `src/app/agent_spawner.rs`
- `src/app/agent_handler.rs`
- `src/app/agent_session_handler.rs`
- `src/app/orchestrator_acp.rs`
- `src/app/socket.rs`
- `src/app/session.rs`
- `src/app/session_drain.rs`

Why first:

- Biggest raw gap in the repo.
- Many of these files are already modified on this branch, so adjacent tests will protect fresh behavior.

Best next behaviors:

- Orchestrator completion rejection when tasks remain pending
- `skip_tasks` followed by successful reconciliation and completion
- `wait_for_any` / `wait_for_all` under mixed success/failure
- Retry or fallback behavior when model/session creation fails
- Early socket close and cleanup behavior
- Message routing when a session ends before all updates arrive
- Session-drain behavior when notifications and completion race

Preferred style:

- Harness-backed scenario tests first
- Narrow unit tests for reconciliation/session-drain helpers
- Mock-channel integration tests for socket/session forwarding edges

### P0: Cursor backend setup and config mutation

Files:

- `src/backend/cursor/mcp_config.rs`
- `src/backend/cursor/mod.rs`
- `src/backend/cursor/acp.rs`
- `src/backend/cursor/cli.rs` for uncovered failure branches only

Why first:

- High leverage and already branch-touched.
- These paths mutate local config and start backend sessions, which are user-visible failure points.

Best next behaviors:

- Create `~/.cursor/mcp.json` when missing
- Replace stale `paperboat-*` entries while preserving unrelated servers
- Unique suffix behavior in MCP enablement
- Write failure propagation
- CLI or ACP bootstrap failure propagation
- Backend transport selection branches in adapter setup

Preferred style:

- Tempdir-backed integration tests
- Mocked process/transport tests

### P1: TUI render/state behavior

Files:

- `src/tui/widgets/backend_selection.rs`
- `src/tui/widgets/agent_output.rs`
- `src/tui/widgets/task_detail.rs`
- `src/tui/widgets/help.rs`
- `src/tui/widgets/status_bar.rs`
- `src/tui/widgets/app_logs.rs`
- `src/tui/widgets/agent_tree.rs`
- `src/tui/app.rs`

Why next:

- Several files are new or near-zero.
- The branch already introduces `backend_selection`, which is especially cheap to cover with focused render/state tests.

Best next behaviors:

- Backend selection wraparound and confirmation hiding the popup
- `selected_backend()` behavior after navigation
- Popup visibility rules for zero/one/many backends
- Agent output auto-scroll on new content
- Scroll clamp when content shrinks or viewport changes
- Waiting-state text for running/completed/failed agent output
- Overlay precedence for help/settings/backend-selection UI

Preferred style:

- `ratatui` render/state tests
- Direct state assertions for navigation helpers

### P1: Self-improvement runner

File:

- `src/self_improve/runner.rs`

Why next:

- One file accounts for most of the subsystem’s remaining gap.
- Already branch-touched.

Best next behaviors:

- Skip paths for disabled config, failed primary run, or non-paperboat repo
- Prompt generation and task creation
- Completion signal handling
- Failure isolation so self-improvement errors do not flip the main result
- Retry/error handling during ACP startup and session execution

Preferred style:

- Tempdir-backed integration tests
- Mock ACP/socket tests
- Helper-focused unit tests where available

## Keep De-Prioritized

These files should stay below the targets above unless behavior changes there or a real regression points to them.

### Already healthy enough

- `src/backend/transport.rs` (`97.73%`)
- `src/backend/cursor/permission.rs` (`100.00%`)
- `src/config/loader.rs` (`90.40%`)
- `src/config/writer.rs` (`91.18%`)
- `src/models.rs` (`93.44%`)
- `src/mcp_server/error.rs` (`100.00%`)
- `src/mcp_server/handlers/tool_schemas.rs` (`100.00%`)
- `src/self_improve/config.rs` (`99.34%`)
- `src/self_improve/context_builder.rs` (`93.03%`)
- `src/self_improve/detection.rs` (`99.57%`)
- `src/testing/assertions.rs` (`92.62%`)
- `src/testing/harness.rs` (`90.50%`)
- `src/testing/mock_backend.rs` (`99.00%`)
- `src/testing/mock_transport.rs` (`98.85%`)
- `src/tui/agent_tree_state.rs` (`96.11%`)
- `src/tui/events/mod.rs` (`99.80%`)
- `src/tui/layout.rs` (`99.16%`)
- `src/tui/task_list_state.rs` (`94.85%`)
- `src/app/router.rs` (`96.04%`)
- `src/app/tool_filtering.rs` (`98.59%`)
- `src/agents/config.rs` (`100.00%`)

### Special cases that are not first-wave targets

- `src/tasks/manager.rs` (`88.35%`): important, but already substantially exercised and should not outrank the near-zero orchestration, parser, backend, and widget gaps.
- `src/backend/mod.rs` and `src/backend/trait.rs`: healthy enough that new tests should usually hit concrete adapters instead.

### Low-ROI process wiring and entrypoints

- `src/main.rs`
- `src/acp.rs`
- `src/mcp_server/socket.rs`
- Process-heavy portions of `src/tui/app.rs`

Reason:

- These are dominated by process setup, binary wiring, or OS interaction.
- Coverage here usually requires expensive smoke tests.
- The same user-facing protection is usually better achieved by testing underlying subsystems directly.

Only move these up if:

- A CLI/startup path changed
- A known regression exists in entrypoint wiring
- A subprocess smoke test was explicitly requested

## Verification Commands

Use the existing tooling and keep command shapes consistent across agents.

### Targeted test runs while developing

Run only the tests relevant to the behavior you are changing:

```bash
cargo test testing::unit_tests -- --nocapture
cargo test testing::integration_tests -- --nocapture
cargo test testing::e2e_tests -- --nocapture
```

For narrower iteration, filter by test name substring:

```bash
cargo test skip_tasks -- --nocapture
cargo test backend_selection -- --nocapture
cargo test agent_output -- --nocapture
cargo test response -- --nocapture
cargo test tool_parsing -- --nocapture
```

### Full regression before handoff

```bash
cargo test
```

### Coverage artifact regeneration

The repo CI already uses this LCOV command in `.github/workflows/coverage.yml`:

```bash
cargo llvm-cov --all-features --lcov --output-path lcov.info
```

To refresh all branch-local artifacts in the same style as the current audit:

```bash
cargo llvm-cov --all-features --json --summary-only --output-path coverage-summary.json
cargo llvm-cov --all-features --text --output-path coverage.txt
cargo llvm-cov --all-features --lcov --output-path lcov.info
```

### Fast sanity checks after coverage refresh

- Confirm the totals in `coverage-summary.json` still parse and reflect the expected delta.
- Spot-check the touched files in `coverage.txt`.
- Use `lcov.info` only when a tool or downstream report needs file-level export data.

## Recommended Execution Order

1. Add table-driven tests for `src/mcp_server/handlers/tool_parsing.rs` and `src/mcp_server/handlers/response.rs`.
2. Extend harness-backed scenarios around `src/app/*`, especially retries, reconciliation, session end, and socket cleanup.
3. Add tempdir-backed Cursor backend tests for `src/backend/cursor/mcp_config.rs` and related bootstrap paths.
4. Add focused `ratatui` tests for `src/tui/widgets/backend_selection.rs` and `src/tui/widgets/agent_output.rs`.
5. Fill remaining meaningful gaps in `src/self_improve/runner.rs`.

That order should maximize risk reduction while staying aligned with the parts of the codebase that are both low-coverage and active on the current branch.
