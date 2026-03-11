# Coverage Audit

Updated: 2026-03-11

This audit is based on the existing coverage artifacts already present in the repo:

- `coverage-summary.json` for overall and per-file metrics
- `coverage.txt` for the human-readable export
- `lcov.info` for file-level detail usable by coverage tooling

The goal of this document is to keep follow-on agents focused on the highest-value uncovered behavior, not on easy percentage wins.

## Baseline

Current project baseline from `coverage-summary.json`:

- Lines: `14137 / 20293` (`69.66%`)
- Regions: `22385 / 31810` (`70.37%`)
- Functions: `1684 / 2292` (`73.47%`)

Largest gaps by subsystem, ranked by uncovered lines:

| Subsystem | Coverage | Uncovered lines | Notes |
| --- | ---: | ---: | --- |
| `src/app/*` | 42.46% | 1622 | Core orchestration and agent lifecycle |
| `src/tui/widgets/*` | 30.56% | 843 | Many render-only widgets still untested |
| `src/backend/cursor/*` | 65.71% | 679 | Cursor-specific MCP and session setup paths |
| `src/tui/*` excluding widgets/events/layout | 76.37% | 557 | `tui/app.rs` is a large zero-coverage entrypoint |
| `src/main.rs` | 0.00% | 499 | Binary entrypoint; low ROI unless smoke-tested |
| `src/mcp_server/handlers/*` | 60.23% | 381 | Parsing and response formatting are under-covered |
| `src/self_improve/*` | 75.91% | 323 | Almost all remaining gap is in `runner.rs` |
| `src/backend/auggie/*` | 63.82% | 161 | Adapter and cache coverage still uneven |
| `src/ipc/*` | 63.59% | 67 | Some platform/socket edges still uncovered |

Largest individual file gaps by uncovered lines:

| File | Coverage | Uncovered lines |
| --- | ---: | ---: |
| `src/main.rs` | 0.00% | 499 |
| `src/tui/app.rs` | 0.00% | 316 |
| `src/self_improve/runner.rs` | 33.64% | 284 |
| `src/app/mod.rs` | 25.39% | 238 |
| `src/app/orchestrator.rs` | 47.09% | 218 |
| `src/backend/cursor/acp.rs` | 0.00% | 204 |
| `src/mcp_server/handlers/response.rs` | 0.00% | 200 |
| `src/tui/widgets/agent_output.rs` | 32.01% | 189 |
| `src/app/agent_handler.rs` | 0.00% | 185 |
| `src/app/agent_spawner.rs` | 28.23% | 178 |
| `src/acp.rs` | 0.00% | 165 |
| `src/backend/cursor/mcp_config.rs` | 15.53% | 136 |
| `src/mcp_server/handlers/tool_parsing.rs` | 16.88% | 133 |
| `src/app/socket.rs` | 4.03% | 119 |
| `src/app/agent_session_handler.rs` | 0.00% | 116 |

## Current Test Layout

Coverage work should build on the test infrastructure that already exists instead of inventing a second test stack.

- Inline `#[cfg(test)]` modules are common across the codebase for focused unit tests.
- Shared test infrastructure lives in `src/testing/*`.
- `src/testing/unit_tests.rs` exercises builders, mock types, and fixture parsing.
- `src/testing/integration_tests.rs` uses `TestHarness` to drive planner/orchestrator/implementer flows with scripted tool calls.
- `src/testing/e2e_tests.rs` drives the full `App::run()` path and validates session order, tool call order, retries, and completion behavior.
- `src/testing/harness.rs` is the key reusable primitive for high-value orchestration tests. It already wraps `App`, injects tool calls, records captured responses, and loads scenarios.
- Scenario coverage currently comes from seven TOML files in `tests/scenarios/`:
  - `simple_implement.toml`
  - `planning_only.toml`
  - `nested_decompose.toml`
  - `multi_implement.toml`
  - `error_recovery.toml`
  - `concurrent_agents.toml`
  - `planner_failure.toml`
- TUI tests already exist for some state-heavy modules such as `src/tui/agent_tree_state.rs`, `src/tui/task_list_state.rs`, `src/tui/layout.rs`, and parts of `src/tui/widgets/agent_output.rs` and `src/tui/widgets/settings.rs`.

Practical implication: new coverage should usually be one of four styles already compatible with the repo:

- Unit tests
- Harness-backed scenario tests
- Narrow integration tests with tempdirs/mocks
- Rendering/state tests using ratatui test backends or direct state assertions

## Do Not Chase These

These files are already sufficiently covered for now. Only add tests here when behavior changes or a bug is found; do not farm them for percentage points.

| File | Coverage |
| --- | ---: |
| `src/backend/transport.rs` | 97.73% |
| `src/backend/cursor/permission.rs` | 100.00% |
| `src/config/loader.rs` | 90.40% |
| `src/config/writer.rs` | 91.18% |
| `src/models.rs` | 93.44% |
| `src/mcp_server/error.rs` | 100.00% |
| `src/mcp_server/handlers/tool_schemas.rs` | 100.00% |
| `src/self_improve/config.rs` | 99.34% |
| `src/self_improve/context_builder.rs` | 93.03% |
| `src/self_improve/detection.rs` | 99.57% |
| `src/testing/assertions.rs` | 92.62% |
| `src/testing/harness.rs` | 90.50% |
| `src/testing/mock_backend.rs` | 99.00% |
| `src/testing/mock_transport.rs` | 98.85% |
| `src/tui/agent_tree_state.rs` | 96.11% |
| `src/tui/events/mod.rs` | 99.80% |
| `src/tui/layout.rs` | 99.16% |
| `src/tui/task_list_state.rs` | 94.85% |
| `src/app/router.rs` | 96.04% |
| `src/app/tool_filtering.rs` | 98.59% |
| `src/agents/config.rs` | 100.00% |

Two special cases:

- `src/tasks/manager.rs` is only `88.35%`, but it is already substantially exercised and should not outrank the near-zero orchestration/UI gaps unless a regression points there.
- `src/backend/mod.rs` and `src/backend/trait.rs` are healthy enough that new tests should usually target the concrete backend adapters instead.

## Meaningful Improvement Criteria

Later agents should treat coverage as a lagging indicator. A test counts as a meaningful improvement only if it does at least one of the following:

- Covers a previously untested branch that changes task state, session lifecycle, retry behavior, backend configuration, tool parsing, or rendered UI state.
- Verifies an observable outcome, not just that code ran. Examples: task status transitions, captured tool responses, config file mutation, socket cleanup, selected backend changes, rendered text/scroll position.
- Introduces a new failure mode or edge case that is realistic for users or agent flows.
- Reuses the harness or existing mocking layers to pin a behavior the product relies on.
- Eliminates a blind spot in a large low-coverage file by covering a coherent behavior slice, not by touching incidental helper lines.

A test is probably useless if it:

- Only asserts constructors, getters, or serde round-trips unless wire-format compatibility is the actual risk.
- Adds another happy-path end-to-end test that duplicates an existing scenario without new assertions.
- Exists mainly to execute logging, tracing, or formatting lines.
- Uses broad smoke coverage to touch large files but does not pin user-visible behavior.
- Targets already-healthy files solely because they are easy to raise from 94% to 96%.

Rule of thumb for acceptance:

- For parser/formatter modules: add success and failure cases, plus at least one edge/deprecated path.
- For orchestration modules: add a scenario that proves a state transition, reconciliation rule, retry path, or tool-call sequence.
- For UI modules: assert specific render/state outcomes under focused interactions, not just that rendering does not panic.

## Prioritized Backlog

### P0: Core Orchestration in `src/app/*`

Why first:

- This is the single biggest uncovered area by raw lines.
- It contains the behavior users actually experience: task creation, orchestration, retries, agent spawning, session handling, and completion.

Highest-value files:

- `src/app/mod.rs`
- `src/app/orchestrator.rs`
- `src/app/agent_spawner.rs`
- `src/app/agent_handler.rs`
- `src/app/agent_session_handler.rs`
- `src/app/orchestrator_acp.rs`
- `src/app/socket.rs`
- `src/app/session.rs`
- `src/app/session_drain.rs`

Recommended test styles:

- Harness-backed scenario tests for planner/orchestrator/implementer flows
- Targeted unit tests for reconciliation, retry classification, and session-drain behavior
- Narrow integration tests around socket request/response forwarding using mocked channels

Best next behaviors to cover:

- Orchestrator completion rejection when tasks remain pending
- `skip_tasks` plus later completion reconciliation
- `wait_for_any` / `wait_for_all` behavior under mixed success/failure
- Agent spawn fallback and retry behavior when the first model/session creation fails
- Message routing and cleanup when sessions end or sockets close early
- Handling of CLI-specific socket paths versus shared ACP socket flow

Notes:

- Prefer extending `tests/scenarios/*.toml` and `TestHarness` before adding new ad hoc integration scaffolding.
- `src/app/router.rs` is already healthy; do not spend time there unless the new scenario depends on router behavior.

### P0: MCP Parsing and Response Text in `src/mcp_server/handlers/*`

Why first:

- These files are small-to-medium and currently under-covered, so the payoff per test is high.
- Bugs here directly affect agent/tool interoperability and error recovery quality.

Highest-value files:

- `src/mcp_server/handlers/tool_parsing.rs`
- `src/mcp_server/handlers/response.rs`

Recommended test styles:

- Table-driven unit tests

Best next behaviors to cover:

- Missing required arguments for every tool
- Deprecated fallback paths such as `task` vs `task_id`
- `spawn_agents` validation with mixed valid/invalid agent specs
- `skip_tasks` empty array rejection
- `build_response_text_with_state()` guidance when tasks are pending, parallelizable, blocked, or complete
- Failure responses that include actionable next steps

Notes:

- This subsystem is a good candidate for fast, deterministic coverage wins that still protect real behavior.

### P0: Cursor Backend Integration in `src/backend/cursor/*`

Why first:

- This is the third-largest subsystem gap.
- The weak spots are exactly where local environment mutation and session bootstrap happen.

Highest-value files:

- `src/backend/cursor/mcp_config.rs`
- `src/backend/cursor/mod.rs`
- `src/backend/cursor/acp.rs`
- `src/backend/cursor/cli.rs` for specific uncovered failure paths only

Recommended test styles:

- Tempdir-backed unit tests for config file reading/writing and server replacement
- Mocked process/transport tests for ACP and CLI session setup
- Narrow integration tests around server enable/disable behavior where feasible

Best next behaviors to cover:

- Creation of `~/.cursor/mcp.json` when missing
- Replacement of stale `paperboat-*` MCP entries while preserving unrelated servers
- Unique suffix handling in `enable_mcp_for_agent()`
- Error propagation when config write or `agent mcp enable` invocation fails
- Backend transport selection and environment setup branches in Cursor adapters

Notes:

- The current inline tests in `mcp_config.rs` only cover path and serialization shape; they do not cover the behavior that actually mutates config state.

### P1: TUI Widgets and Startup Loop

Why next:

- The widget layer has a large raw gap, but much of it is display logic; it matters, but not before core orchestration.
- Several files are near-zero because they are new or render-only, so targeted render/state tests can cover important behavior without over-investing.

Highest-value files:

- `src/tui/widgets/backend_selection.rs`
- `src/tui/widgets/agent_output.rs`
- `src/tui/widgets/task_detail.rs`
- `src/tui/widgets/help.rs`
- `src/tui/widgets/status_bar.rs`
- `src/tui/widgets/app_logs.rs`
- `src/tui/widgets/agent_tree.rs`
- `src/tui/app.rs`
- `src/tui/mod.rs`

Recommended test styles:

- Rendering/state tests with ratatui `TestBackend`
- Direct state-machine tests for navigation and selection behavior

Best next behaviors to cover:

- Backend selection wraparound, confirmation, and hidden/visible rules
- Agent output auto-scroll behavior, scrollbar clamping, and waiting-state display
- Overlay rendering for help/settings/backend-selection interactions
- Startup loop behavior when multiple backends arrive before initial config
- Focus changes and keyboard/mouse event handling at the TUI shell level

Notes:

- `src/tui/widgets/agent_output.rs` already has format-oriented tests, but the big gap is still in render behavior, waiting animation paths, and scrolling logic.
- Favor assertions on rendered content/state over brittle full-screen snapshots.

### P1: Self-Improvement Runner

Why next:

- `src/self_improve/runner.rs` alone accounts for most of the subsystem gap.
- This code coordinates sockets, ACP sessions, prompt construction, and failure isolation after successful runs.

Highest-value file:

- `src/self_improve/runner.rs`

Recommended test styles:

- Tempdir-backed integration tests
- Mock ACP/socket tests
- Unit tests for gating and prompt construction helpers

Best next behaviors to cover:

- Skip conditions: disabled feature, failed main run, non-paperboat repo
- Prompt/task generation for self-improvement sessions
- Completion signal handling from the socket listener
- Failure isolation so self-improvement errors do not affect the main run result
- Retry/error behavior during ACP startup and session execution

Notes:

- `config.rs`, `detection.rs`, and `context_builder.rs` are already well covered, so later work should target the runner only.

### P2: Secondary Adapter and IPC Gaps

Why later:

- These areas matter, but they are either smaller or more expensive per test than the P0/P1 groups above.

Files:

- `src/backend/auggie/acp.rs`
- `src/backend/auggie/cache.rs`
- `src/backend/cursor/cache.rs` when cache behavior changes
- `src/ipc/unix.rs`
- `src/ipc/stream.rs`
- `src/metrics.rs`

Recommended test styles:

- Unit tests with tempdirs and mock IO
- Platform-scoped integration tests where needed

Best next behaviors to cover:

- Cache hit/miss and stale data behavior
- IPC read/write edge cases and malformed message handling
- Metrics toggles or aggregation behavior only if metrics become product-relevant

### P3: Low-ROI Entrypoints and Process Wiring

These files are low coverage but should not be attacked first:

- `src/main.rs`
- `src/acp.rs`
- `src/mcp_server/socket.rs`
- `src/tui/app.rs` pieces that require brittle process-level terminal setup

Reason:

- They are heavy on process setup, binary wiring, or OS interaction.
- High coverage here usually requires expensive smoke tests.
- The same user-facing protection is often better obtained by covering the underlying subsystems directly.

Only prioritize them when:

- There is a known bug in entrypoint wiring
- A CLI flag or startup path changed
- A subprocess smoke test is specifically requested

## Recommended Execution Order For Follow-On Agents

1. Add table-driven tests for `src/mcp_server/handlers/tool_parsing.rs` and `src/mcp_server/handlers/response.rs`.
2. Extend harness-backed scenarios to cover missing `src/app/*` orchestration behaviors, especially reconciliation, retries, and session/socket cleanup.
3. Add tempdir-backed tests for `src/backend/cursor/mcp_config.rs` and related Cursor bootstrap paths.
4. Add focused TUI render/state tests for `backend_selection`, `agent_output`, and one or two other high-value widgets.
5. Fill in `src/self_improve/runner.rs` with skip-path and completion-path tests.

That order should maximize risk reduction and coverage gain without wasting effort on already-healthy modules or brittle binary smoke tests.
