# Windows Support (Completed)

Paperboat now supports Windows through a cross-platform IPC abstraction layer.

## Implementation Summary

The IPC abstraction provides a unified interface for bidirectional stream communication:

- **Unix (macOS, Linux)**: Uses Unix domain sockets (`tokio::net::UnixStream`)
- **Windows**: Uses named pipes (`tokio::net::windows::named_pipe`)

## Key Components

- `src/ipc/mod.rs` - Cross-platform module entry point with conditional compilation
- `src/ipc/address.rs` - Platform-agnostic `IpcAddress` type
- `src/ipc/stream.rs` - `IpcStream` and `IpcListener` abstractions
- `src/ipc/unix.rs` - Unix socket implementation
- `src/ipc/windows.rs` - Windows named pipe implementation

## Files Updated to Use IPC Abstraction

- `src/app/socket.rs` - Uses `IpcAddress`, `IpcListener`, `IpcStream`
- `src/mcp_server/socket.rs` - Uses `connect_with_retry` abstraction
- `src/self_improve/runner.rs` - Uses `IpcAddress`, `IpcListener`, `IpcStream`

## Architecture

The app spawns MCP server processes that need to communicate back to the parent:
- Each agent gets its own socket/pipe for isolation
- Socket/pipe addresses are passed to child processes via command line arguments
- Messages are `ToolRequest` and `ToolResponse` structs serialized as JSON

## Testing

- Unix tests pass (850 tests, including IPC round-trip test)
- Windows build is verified in CI
- Windows binary is included in release workflow