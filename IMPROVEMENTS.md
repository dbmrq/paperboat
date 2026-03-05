# Villalobos Improvement Roadmap

## Project Overview

Villalobos is an AI agent orchestrator written in Rust that coordinates task completion using a hierarchical agent architecture (Orchestrator → Planner/Implementer). It uses:
- **ACP (Agent Control Protocol)** - JSON-RPC for spawning/communicating with agents via `auggie`
- **MCP Server** - Exposes tools for decision-making (decompose, implement, complete)
- **Unix Socket Communication** - IPC between orchestrator and MCP server

---

## Most Important Improvements

### 1. Error Handling & Resilience (High Priority)

The current implementation lacks robust error recovery:
- **No timeout handling** - `wait_for_plan()` and `wait_for_session_complete()` can block indefinitely
- **No retry logic** for failed implementations
- **Silent failures** - If an implementer agent fails, the orchestrator isn't notified properly
- **No graceful shutdown** - Ctrl+C leaves orphan processes

### 2. Progress Tracking & Observability (High Priority)

- No persistent state between runs - if the process crashes, all progress is lost
- No way to **resume** a partially completed task
- Limited visibility into what each agent is doing (beyond logs)
- Consider adding a **progress file** or **checkpoint system**

### 3. Configuration & Flexibility (Medium Priority)

- Model selection is hardcoded (`Opus4.5` for orchestrator, `Sonnet4.5` for workers)
- No CLI flags to override models, log level, or other settings
- Consider adding a **config file** (`villalobos.toml`) or **CLI arguments**:
  ```
  villalobos --orchestrator-model opus4.5 --worker-model haiku4.5 "task"
  ```

### 4. Task Dependencies & Parallel Execution (Medium Priority)

- Currently executes subtasks **sequentially**
- No support for **parallel execution** of independent subtasks
- No way to express **dependencies** between subtasks
- The `Plan` type has `priority` but it's not used

### 5. Integration Testing ✅ COMPLETE

- ~~Unit tests are excellent (65 tests), but no **integration tests**~~
- ~~No tests for the full orchestrator flow~~
- ~~Consider adding mock-based tests for the ACP client~~

**Implemented:**
- Full mock data system in `src/testing/` with `MockAcpClient`, `TestHarness`, and scenario loading
- `AcpClientTrait` abstraction for dependency injection
- 6 complete test scenarios in `tests/scenarios/` covering all major flows
- 160+ tests including unit, integration, and E2E tests
- See `docs/MOCK_DATA_SYSTEM_ARCHITECTURE.md` for details

### 6. Prompt Engineering Improvements (Medium Priority)

The prompts are quite basic:
- **Orchestrator prompt** (8 lines) - could include examples of when to decompose vs implement
- **Planner prompt** (10 lines) - could include output format guidance, size limits
- **Implementer prompt** (7 lines) - could include context about the codebase, coding standards

### 7. Better CLI Experience (Low Priority)

- No `--help` flag
- No `--version` flag  
- Consider using `clap` for proper argument parsing
- Add `--dry-run` to see what would happen without executing

### 8. Cost & Token Management (Low Priority)

- No tracking of API costs/tokens used
- No budget limits
- No way to estimate cost before running

---

## Quick Wins (Easy Improvements)

1. **Add timeouts** to `wait_for_plan()` and `wait_for_session_complete()`
2. **Add `clap`** for CLI argument parsing with `--help`
3. **Use PlanEntry.priority** for task ordering
4. **Add a `--verbose` flag** for detailed output
5. **Track success/failure** of individual subtasks

---

## Implementation Priority

| Priority | Improvement | Effort | Impact |
|----------|-------------|--------|--------|
| 1 | Add timeouts to blocking waits | Low | High |
| 2 | Graceful shutdown with Ctrl+C | Low | High |
| 3 | CLI with clap (--help, --version) | Low | Medium |
| 4 | Track subtask success/failure | Medium | High |
| 5 | Progress checkpointing | Medium | High |
| 6 | Config file support | Medium | Medium |
| 7 | Parallel subtask execution | High | High |
| 8 | Integration tests | ~~Medium~~ | ~~Medium~~ | ✅ COMPLETE |
| 9 | Enhanced prompts | Low | Medium |
| 10 | Cost/token tracking | Medium | Low |

