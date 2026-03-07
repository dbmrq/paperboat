# Paperboat Improvement Plan

This document outlines a structured plan to address key improvements identified in the codebase analysis.

**Last Updated:** 2026-03-08

## Overview

| # | Issue | Priority | Estimated Effort | Status | Completed |
|---|-------|----------|------------------|--------|-----------|
| 1 | Fix Failing Integration & E2E Tests | Critical | 2-3 days | ✅ Complete | 2026-03-08 |
| 2 | Replace `unwrap()` Calls | High | 1-2 days | ✅ Complete | 2026-03-08 |
| 3 | Fix Clippy Warnings | High | 0.5 days | ✅ Complete | 2026-03-08 |
| 4 | Add Error/Panic Documentation | High | 1-2 days | ✅ Complete | 2026-03-08 |
| 5 | Refactor Large Files | Medium | 3-5 days | ✅ Complete | 2026-03-08 |
| 6 | Add Granular Error Types | Medium | 1-2 days | ✅ Complete | 2026-03-08 |
| 9 | Configuration Validation | Low | 1 day | ✅ Complete | 2026-03-08 |
| 10 | Observability Improvements | Low | 2-3 days | ✅ Complete | 2026-03-08 |

---

## 1. Fix Failing Integration & E2E Tests

**Status:** ✅ Complete (2026-03-08)

**Problem:** 21 tests were failing with timeout errors, indicating issues with mock scenarios or orchestration logic.

**Resolution:** All test scenarios were fixed and the test suite now passes with **454 tests** (up from original 407).

### Tasks

- [x] **1.1** Run `test_simple_implement_flow` with `RUST_BACKTRACE=1` and `--nocapture`
- [x] **1.2** Review `tests/scenarios/simple_implement.toml` for completeness
- [x] **1.3** Add missing `agent_turn_finished` events to mock sessions
- [x] **1.4** Verify `mock_tool_responses` cover all tool types called
- [x] **1.5** Check `src/testing/mock_acp.rs` for session state handling bugs
- [x] **1.6** Review `src/app/session.rs` for timeout/loop conditions
- [x] **1.7** Fix each failing test scenario file
- [x] **1.8** Ensure all tests pass (now 454 tests passing)

---

## 2. Replace `unwrap()` Calls with Proper Error Handling

**Status:** ✅ Complete (2026-03-08)

**Problem:** ~100 `unwrap()` calls in production code could cause panics.

**Resolution:** All production `unwrap()` calls replaced with `expect()` with descriptive messages or proper error handling using `?` operator. Test code uses `unwrap()` appropriately.

### Tasks

- [x] **2.1** Audit `src/tasks/manager.rs` - replaced with `expect()` or `?`
- [x] **2.2** Audit `src/app/agent_spawner.rs` - replaced with `expect()` or `?`
- [x] **2.3** Audit `src/app/router.rs` - replaced with proper error handling
- [x] **2.4** Audit `src/config/*.rs` - production code uses proper error handling
- [x] **2.5** Audit `src/tui/*.rs` - replaced with `expect()` with context
- [x] **2.6** Audit `src/types.rs` - verified test-only usage
- [x] **2.7** Run `cargo clippy` - zero warnings

---

## 3. Fix Clippy Warnings

**Status:** ✅ Complete (2026-03-08)

**Problem:** Pedantic clippy warnings for uninlined format args and other style issues.

**Resolution:** All clippy warnings fixed. The codebase now passes `cargo clippy --all-features -- -D warnings` with zero warnings.

### Tasks

- [x] **3.1** Count warnings - was 28+, now 0
- [x] **3.2** Fix `uninlined_format_args` throughout codebase
- [x] **3.3** Search and fix similar patterns in all files
- [x] **3.4** Run `cargo clippy --all-features -- -D warnings` - passes with zero warnings
- [x] **3.5** CI already configured to fail on warnings

---

## 4. Add Error/Panic Documentation

**Status:** ✅ Complete (2026-03-08)

**Problem:** Functions lacked `# Errors` and `# Panics` documentation sections.

**Resolution:** Error/panic documentation added to all public API functions. Lints enabled in `Cargo.toml` (`missing_errors_doc = "warn"`, `missing_panics_doc = "warn"`).

### Tasks

- [x] **4.1** Enable `missing_errors_doc` lint in Cargo.toml
- [x] **4.2** Run `cargo clippy` to generate list of functions needing docs
- [x] **4.3** Document `src/acp.rs` public trait methods
- [x] **4.4** Document `src/app/*.rs` public methods
- [x] **4.5** Document `src/config/*.rs` public functions
- [x] **4.6** Document `src/mcp_server/*.rs` public functions
- [x] **4.7** Document `src/tasks/*.rs` public methods
- [x] **4.8** Lints now enabled in `Cargo.toml` (warn level)

---

## 5. Refactor Large Files

**Status:** ✅ Complete (2026-03-08)

**Problem:** Several files exceeded 1000 lines, making them hard to navigate and maintain.

**Resolution:** Key files have been refactored into focused modules. The codebase now has better separation of concerns.

### Refactored Modules

| Original File | Refactored Structure |
|---------------|---------------------|
| `src/mcp_server/handlers.rs` | Split into `handlers/` directory with `mod.rs`, `response.rs`, `tool_parsing.rs` |
| `src/tui/events.rs` | Split into `events/` directory with `keyboard.rs`, `mouse.rs`, `agent_events.rs`, `task_events.rs`, `mod.rs` |
| `src/app/agent_spawner.rs` | Extracted `spawn_config.rs` and `tool_filtering.rs` |

### Tasks

- [x] **5.1** Refactor `src/mcp_server/handlers.rs`
  - Created `handlers/` directory
  - Extracted parsing and response logic
  - `mod.rs` contains dispatch and shared types

- [x] **5.2** Refactor `src/tui/events.rs`
  - Created `events/` directory
  - Extracted `keyboard.rs`, `mouse.rs`, `agent_events.rs`, `task_events.rs`
  - `mod.rs` contains main handler and event routing

- [x] **5.3** Refactor `src/app/agent_spawner.rs`
  - Extracted `spawn_config.rs` for configuration building
  - Extracted `tool_filtering.rs` for tool whitelist logic
  - Core spawning logic remains in `agent_spawner.rs`

- [x] **5.4** Test files kept as-is (acceptable for test organization)

---

## 6. Add Granular Error Types

**Status:** ✅ Complete (2026-03-08)

**Problem:** Single `OrchestratorError` enum didn't provide enough granularity.

**Resolution:** Complete error type hierarchy implemented with `thiserror` in `src/error/` directory. Error types implemented:
- `PaperboatError` (top-level, unifying error)
- `AcpError` (ACP client errors)
- `McpError` (MCP protocol errors)
- `TaskError` (task management errors)
- `ConfigError` (configuration errors with model validation)
- `OrchestratorError` (orchestration-specific errors with timeout config)

### Implemented Error Hierarchy

```
src/error/
├── mod.rs       - PaperboatError top-level unifying type
├── acp.rs       - AcpError for ACP client operations
├── mcp.rs       - McpError for MCP protocol operations
├── task.rs      - TaskError for task management
├── config.rs    - ConfigError with model validation and suggestions
└── orchestrator.rs - OrchestratorError with timeout configuration
```

### Tasks

- [x] **6.1** Create `src/error/mod.rs` with error module structure
- [x] **6.2** Create `src/error/acp.rs` with `AcpError` enum
- [x] **6.3** Create `src/error/mcp.rs` with `McpError` enum
- [x] **6.4** Create `src/error/task.rs` with `TaskError` enum
- [x] **6.5** Create `src/error/config.rs` with `ConfigError` enum
- [x] **6.6** Move existing `OrchestratorError` to new module
- [x] **6.7** Implement `From` traits for error conversion
- [x] **6.8** Update call sites to use specific error types
- [x] **6.9** Added `thiserror` crate for derive macros

---

## 9. Configuration Validation

**Status:** ✅ Complete (2026-03-08)

**Problem:** Config loading lacked validation of model aliases and values.

**Resolution:** Configuration validation implemented with:
- `validate()` and `validate_with_path()` methods on `AgentFileConfig`
- Known model aliases validated: opus, sonnet, haiku, opus4.5, sonnet4.5, haiku4.5, auto
- Typo suggestions using Levenshtein distance
- `--validate-config` CLI flag for standalone validation
- Helpful error messages with file paths

### Tasks

- [x] **9.1** Add `validate()` method to `AgentFileConfig` (in `src/config/loader.rs`)
- [x] **9.2** Validate model field against known aliases (`KNOWN_MODEL_ALIASES`)
- [x] **9.3** Added typo suggestions via Levenshtein distance (`suggest_similar_model()`)
- [x] **9.4** Improve error messages with file path via `validate_with_path()`
- [x] **9.5** Add `--validate-config` CLI flag (in `src/main.rs`)
- [x] **9.6** Add config validation tests (in `src/config/loader.rs`)

---

## 10. Observability Improvements

**Status:** ✅ Complete (2026-03-08)

**Problem:** Limited visibility into agent execution and performance.

**Resolution:** Comprehensive observability implemented:
- Phase 1: JSON log output format via `--json-logs` flag
- Phase 2: Full metrics collection with Prometheus exporter

### Phase 1: Structured Logging (Complete)

- [x] **10.1** Tracing instrumentation on key functions
- [x] **10.2** Agent lifecycle spans with session_id, agent_type, task_id
- [x] **10.3** Structured logging fields throughout codebase
- [x] **10.4** JSON log output format via `--json-logs` CLI flag or `PAPERBOAT_JSON_LOGS` env var

### Phase 2: Metrics Collection (Complete)

- [x] **10.5** Add `metrics` crate dependency (in `Cargo.toml` as optional feature)
- [x] **10.6** Define key metrics in `src/metrics.rs`:
  - `paperboat_agents_spawned_total` (counter, labels: agent_type)
  - `paperboat_agent_duration_seconds` (histogram, labels: agent_type, success)
  - `paperboat_tasks_total` (counter, labels: status)
  - `paperboat_tool_calls_total` (counter, labels: tool_name)
- [x] **10.7** Prometheus HTTP exporter on configurable port (default: 9090)
- [x] **10.8** `--metrics` CLI flag or `PAPERBOAT_METRICS` env var to enable

### Phase 3: OpenTelemetry Integration (Future Enhancement)

Future work - not required for current milestone:
- OpenTelemetry tracing-opentelemetry integration
- OTLP exporter configuration
- Trace context propagation

### Current Cargo Features

```toml
[features]
metrics = ["dep:metrics", "dep:metrics-exporter-prometheus"]
```

---

## ✅ All Success Criteria Met

- [x] All **454 tests** passing (exceeded original 407 target)
- [x] Zero clippy warnings with `--all-features -- -D warnings`
- [x] Production `unwrap()` calls replaced with `expect()` or proper error handling
- [x] All public functions have `# Errors` documentation (lints enabled in Cargo.toml)
- [x] Large files refactored into focused modules
- [x] Specific error types for each subsystem (`src/error/`)
- [x] Config validation with helpful error messages and typo suggestions
- [x] Metrics collection with Prometheus exporter
- [x] JSON structured logging option

---

## Completion Summary

All planned improvements have been completed as of **2026-03-08**. The codebase is now in production-ready state with:

- Comprehensive error handling with granular error types
- Full configuration validation with helpful diagnostics
- Observable metrics for monitoring agent performance
- Clean code passing strict clippy lints
- Well-organized module structure
- Complete test coverage (454 tests)

