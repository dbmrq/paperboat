# Villalobos

An AI agent orchestrator that coordinates task completion by delegating work to specialized agents.

## Overview

Villalobos uses a hierarchical agent architecture to break down complex tasks into manageable subtasks:

```
┌─────────────┐
│ Orchestrator│ ← Coordinates overall task execution
└──────┬──────┘
       │
   ┌───┴───┐
   ▼       ▼
┌──────┐ ┌───────────┐
│Planner│ │Implementer│
└──────┘ └───────────┘
   │           │
   ▼           ▼
 Plan      Code/Tests
```

- **Orchestrator**: Routes tasks to planner or implementer agents, decides when to decompose vs. implement
- **Planner**: Breaks complex tasks into ordered, actionable subtasks
- **Implementer**: Executes individual tasks (writes code, tests, documentation)

## Features

- **ACP (Agent Control Protocol)**: JSON-RPC based protocol for spawning and communicating with agents
- **MCP Server**: Exposes tools (`decompose`, `implement`, `complete`) for orchestrator decision-making
- **Recursive Decomposition**: Complex tasks can be recursively broken down into subtasks
- **Unix Socket Communication**: Inter-process communication between orchestrator and MCP server
- **Daily Rotating Logs**: Automatic log rotation with configurable output directory

## Installation

```bash
cargo build --release
```

## Usage

### Default Mode (Orchestrator)

Run villalobos with a task description:

```bash
villalobos "Create a REST API with user authentication"
```

### MCP Server Mode

Run as an MCP server (typically spawned by the orchestrator):

```bash
VILLALOBOS_SOCKET=/path/to/socket.sock villalobos --mcp-server
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `VILLALOBOS_LOG_DIR` | Directory for log files | `logs` |
| `VILLALOBOS_SOCKET` | Unix socket path (required for MCP server mode) | — |
| `RUST_LOG` | Log level filter | `villalobos=debug,info` |

## MCP Tools

When running as an MCP server, villalobos exposes three tools:

| Tool | Description |
|------|-------------|
| `decompose` | Break a complex task into subtasks via the planner agent |
| `implement` | Delegate a task to an implementer agent for execution |
| `complete` | Signal task completion with success/failure status |

## Architecture

```
                    ┌─────────────────────────────────────────┐
                    │              Orchestrator               │
                    │  ┌─────────┐      ┌──────────────────┐  │
                    │  │ACP Client│◄────►│   MCP Server     │  │
                    │  └────┬────┘      │ (Unix Socket)    │  │
                    │       │           └──────────────────┘  │
                    └───────┼─────────────────────────────────┘
                            │ ACP (JSON-RPC)
           ┌────────────────┼────────────────┐
           ▼                ▼                ▼
    ┌─────────────┐  ┌─────────────┐  ┌─────────────┐
    │   Planner   │  │ Implementer │  │   Child     │
    │   Agent     │  │   Agent     │  │ Orchestrator│
    └─────────────┘  └─────────────┘  └─────────────┘
```

1. User provides a task to the orchestrator
2. Orchestrator spawns an MCP server with `decompose`, `implement`, `complete` tools
3. For complex tasks, orchestrator calls `decompose` → planner creates subtask plan
4. For simple tasks, orchestrator calls `implement` → implementer executes
5. Orchestrator calls `complete` when all work is done

## License

MIT

