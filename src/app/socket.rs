//! IPC socket handling for MCP server communication.
//!
//! This module provides cross-platform IPC communication between the main app
//! and MCP server processes using the IPC abstraction layer.

use super::types::{ToolMessage, ToolReceiver, ToolSender};
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::ipc::{IpcAddress, IpcListener, IpcStream};
use crate::mcp_server::{ToolRequest, ToolResponse};

/// Handle for a per-agent socket, including cleanup resources.
pub struct AgentSocketHandle {
    /// The IPC address for this agent's socket
    pub socket_address: IpcAddress,
    /// Receiver for tool messages from this agent
    pub tool_rx: ToolReceiver,
    /// Handle to the listener task (for cleanup)
    listener_task: JoinHandle<()>,
}

impl AgentSocketHandle {
    /// Clean up the socket, removing the endpoint and aborting the listener task.
    pub fn cleanup(self) {
        // Abort the listener task
        self.listener_task.abort();

        // Clean up the socket endpoint (removes file on Unix, no-op on Windows)
        self.socket_address.cleanup();
    }
}

impl Drop for AgentSocketHandle {
    fn drop(&mut self) {
        // Log when this handle is dropped - this helps debug socket lifecycle issues
        tracing::warn!(
            "⚠️ AgentSocketHandle DROPPED: {} - listener will be aborted!",
            self.socket_address
        );

        // Abort the listener task on drop
        self.listener_task.abort();

        // Clean up the socket endpoint
        self.socket_address.cleanup();
    }
}

/// Set up an IPC socket for a single agent.
///
/// Creates a unique socket address for the agent and starts a listener task.
/// Returns an `AgentSocketHandle` that can be used to receive tool messages
/// and clean up resources when done.
///
/// # Arguments
///
/// * `agent_id` - A unique identifier for the agent (used in socket address).
pub async fn setup_agent_socket(agent_id: &str) -> Result<AgentSocketHandle> {
    // Generate a unique IPC address for this agent
    // The IpcAddress::generate handles platform-specific addressing and path length limits
    let socket_address = IpcAddress::generate(agent_id);

    let listener = IpcListener::bind(&socket_address).await.with_context(|| {
        format!("Failed to bind agent socket at {socket_address} (agent_id={agent_id})")
    })?;

    // Verify the socket endpoint exists before proceeding (on Unix)
    // This ensures the socket is ready for MCP server connections
    if !socket_address.exists() {
        anyhow::bail!(
            "Socket was not created at {socket_address} after bind (agent_id={agent_id})"
        );
    }

    tracing::debug!(
        "Agent socket listening at: {} (agent_id={})",
        socket_address,
        agent_id
    );

    // Create channel for tool messages
    let (tool_tx, tool_rx) = mpsc::channel::<ToolMessage>(100);

    let socket_addr_str = socket_address.as_str().to_string();
    let listener_task = tokio::spawn(async move {
        tracing::debug!("🔌 Socket listener task started for {}", socket_addr_str);
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    tracing::debug!("🔌 Socket accepted connection on {}", socket_addr_str);
                    let tool_tx = tool_tx.clone();
                    let socket_addr = socket_addr_str.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_mcp_connection(stream, tool_tx).await {
                            tracing::error!("🔌 MCP connection error on {}: {}", socket_addr, e);
                        } else {
                            tracing::debug!(
                                "🔌 MCP connection completed successfully on {}",
                                socket_addr
                            );
                        }
                    });
                }
                Err(e) => {
                    // This error is expected when the listener is dropped during cleanup
                    tracing::debug!("🔌 Socket listener stopped for {}: {}", socket_addr_str, e);
                    break;
                }
            }
        }
        tracing::debug!("🔌 Socket listener task exiting for {}", socket_addr_str);
    });

    // Yield to let the listener task start running
    // This helps prevent race conditions where the MCP server tries to connect
    // before the listener is ready to accept connections
    tokio::task::yield_now().await;

    Ok(AgentSocketHandle {
        socket_address,
        tool_rx,
        listener_task,
    })
}

/// Set up an IPC socket for MCP server communication.
///
/// Returns the socket address and tool receiver channel.
pub async fn setup_socket() -> Result<(IpcAddress, ToolReceiver, JoinHandle<()>)> {
    // Generate a unique IPC address
    // IpcAddress::generate handles platform-specific addressing and path length limits
    let socket_address = IpcAddress::generate("main");

    let listener = IpcListener::bind(&socket_address)
        .await
        .context("Failed to bind IPC socket")?;

    tracing::info!("IPC socket listening at: {}", socket_address);

    // Spawn task to accept connections and forward tool requests
    let (tool_tx, tool_rx) = mpsc::channel::<ToolMessage>(100);

    let listener_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    let tool_tx = tool_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_mcp_connection(stream, tool_tx).await {
                            tracing::error!("MCP connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    // This error is expected when the listener is dropped during shutdown
                    tracing::debug!("Socket listener stopped: {}", e);
                    break;
                }
            }
        }
    });

    Ok((socket_address, tool_rx, listener_task))
}

/// Clean up an IPC socket endpoint and abort the listener task.
pub fn cleanup_socket(socket_address: &IpcAddress, listener_task: Option<JoinHandle<()>>) {
    // Abort the socket listener task
    if let Some(task) = listener_task {
        task.abort();
    }

    // Clean up the socket endpoint (removes file on Unix, no-op on Windows)
    socket_address.cleanup();
}

/// Handle an MCP connection from the MCP server.
///
/// Each connection represents a single tool call. The connection stays open
/// until the tool call completes, allowing the response to be sent back.
pub async fn handle_mcp_connection(stream: IpcStream, tool_tx: ToolSender) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    // Read the tool request (one line of JSON)
    if reader.read_line(&mut line).await? == 0 {
        tracing::debug!("🔌 Socket connection closed (0 bytes read)");
        return Ok(()); // Connection closed
    }

    let request: ToolRequest =
        serde_json::from_str(&line).context("Failed to parse tool request")?;

    tracing::debug!(
        "🔌 Socket received tool call: {:?}, request_id={}",
        request.tool_call.tool_type(),
        request.request_id
    );

    // Create a oneshot channel for the response
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();

    // Send the request to the app for processing
    let tool_type = format!("{:?}", request.tool_call.tool_type());
    if let Err(e) = tool_tx
        .send(ToolMessage::Request {
            request,
            response_tx,
        })
        .await
    {
        tracing::error!(
            "🔌 Failed to send {} to app: {} - receiver may be dropped!",
            tool_type,
            e
        );
        return Err(anyhow::anyhow!(
            "Failed to send tool request to app: {} - is tool_rx receiver alive?",
            e
        ));
    }
    tracing::debug!(
        "🔌 Tool request {} sent to app, waiting for response...",
        tool_type
    );

    // Wait for the response
    let response: ToolResponse = match response_rx.await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!(
                "🔌 Failed to receive response for {}: {} - response_tx was dropped!",
                tool_type,
                e
            );
            return Err(anyhow::anyhow!(
                "Failed to receive response from app: {} - did the handler drop response_tx?",
                e
            ));
        }
    };

    tracing::debug!(
        "🔌 Got response for {}: success={}",
        tool_type,
        response.success
    );

    // Send the response back to the MCP server
    let response_json = serde_json::to_string(&response)?;
    writer.write_all(response_json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    tracing::debug!("🔌 Response for {} sent to MCP server", tool_type);

    Ok(())
}
