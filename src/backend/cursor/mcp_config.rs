//! Cursor MCP configuration management.
//!
//! This module handles writing Paperboat's MCP server configuration to
//! Cursor's `~/.cursor/mcp.json` file so that Cursor agents can access
//! our orchestration tools.
//!
//! # Multi-Server Architecture
//!
//! We register multiple MCP servers, one per agent type:
//! - `paperboat-planner`: Exposes planner tools (set_goal, create_task, complete)
//! - `paperboat-orchestrator`: Exposes orchestrator tools (spawn_agents, decompose, etc.)
//! - `paperboat-implementer`: Exposes implementer tools (complete)
//!
//! Before spawning an agent, we enable the appropriate server and disable others.
//! This ensures each agent only sees the tools it should use.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Agent types for MCP server registration.
pub const AGENT_TYPES: &[&str] = &["planner", "orchestrator", "implementer"];

/// Get the MCP server name for a given agent type.
pub fn mcp_server_name(agent_type: &str) -> String {
    format!("paperboat-{agent_type}")
}

/// Get the path to Cursor's mcp.json config file.
pub fn cursor_mcp_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".cursor").join("mcp.json"))
}

/// Cursor's MCP configuration file format.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CursorMcpConfig {
    /// Map of MCP server name to configuration.
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Command to run the MCP server.
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Environment variables to set.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

/// Register all Paperboat MCP servers in Cursor's config.
///
/// This adds/updates entries for each agent type in `~/.cursor/mcp.json`:
/// - `paperboat-planner`: For planner agents
/// - `paperboat-orchestrator`: For orchestrator agents
/// - `paperboat-implementer`: For implementer agents
///
/// Each server is configured with the appropriate `PAPERBOAT_AGENT_TYPE` env var
/// so the MCP server exposes only the tools for that agent type.
///
/// Note: This function is currently unused as we use `enable_mcp_for_agent`
/// for just-in-time MCP registration per agent type. Keeping for potential
/// future use cases.
///
/// # Arguments
///
/// * `socket_path` - Path to the Unix socket for MCP communication
#[allow(dead_code)]
pub fn register_paperboat_mcp(socket_path: &str) -> Result<()> {
    let config_path = cursor_mcp_config_path()?;

    // Read existing config or create empty one
    let mut config = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        // Create the .cursor directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        CursorMcpConfig::default()
    };

    // Get path to paperboat binary
    let binary_path = std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string();

    // Register an MCP server for each agent type
    for agent_type in AGENT_TYPES {
        let mut env = HashMap::new();
        env.insert("PAPERBOAT_AGENT_TYPE".to_string(), (*agent_type).to_string());

        let mcp_config = McpServerConfig {
            command: binary_path.clone(),
            args: vec![
                "--mcp-server".to_string(),
                "--socket".to_string(),
                socket_path.to_string(),
            ],
            env,
        };

        config
            .mcp_servers
            .insert(mcp_server_name(agent_type), mcp_config);
    }

    // Write back the config
    let content =
        serde_json::to_string_pretty(&config).context("Failed to serialize MCP config")?;
    std::fs::write(&config_path, content)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;

    tracing::info!(
        "📝 Registered Paperboat MCP servers in {}",
        config_path.display()
    );

    Ok(())
}

/// Remove all Paperboat MCP servers from Cursor's config.
///
/// Call this when Paperboat exits to clean up.
pub fn unregister_paperboat_mcp() -> Result<()> {
    let config_path = cursor_mcp_config_path()?;

    if !config_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    let mut config: CursorMcpConfig = serde_json::from_str(&content).unwrap_or_default();

    // Remove all paperboat entries
    let mut removed = false;
    for agent_type in AGENT_TYPES {
        if config.mcp_servers.remove(&mcp_server_name(agent_type)).is_some() {
            removed = true;
        }
    }

    if removed {
        let content =
            serde_json::to_string_pretty(&config).context("Failed to serialize MCP config")?;
        std::fs::write(&config_path, content)
            .with_context(|| format!("Failed to write {}", config_path.display()))?;

        tracing::info!(
            "🗑️ Removed Paperboat MCP servers from {}",
            config_path.display()
        );
    }

    Ok(())
}

/// Configure MCP for a specific agent type, removing all other paperboat servers.
///
/// This ensures only ONE paperboat MCP server is registered at a time, with the
/// appropriate `PAPERBOAT_AGENT_TYPE` env var. This is necessary because Cursor
/// loads ALL enabled MCP servers, and we need each agent to see only its tools.
///
/// # Arguments
///
/// * `agent_type` - The agent type ("planner", "orchestrator", "implementer")
/// * `socket_path` - Path to the Unix socket for MCP communication
pub fn enable_mcp_for_agent(agent_type: &str, socket_path: &str) -> Result<()> {
    let config_path = cursor_mcp_config_path()?;

    // Read existing config or create empty one
    let mut config = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        CursorMcpConfig::default()
    };

    // Remove ALL paperboat MCP servers first
    for at in AGENT_TYPES {
        config.mcp_servers.remove(&mcp_server_name(at));
    }

    // Get path to paperboat binary
    let binary_path = std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string();

    // Register ONLY the MCP server for this agent type
    let server_name = mcp_server_name(agent_type);
    let mut env = HashMap::new();
    env.insert("PAPERBOAT_AGENT_TYPE".to_string(), agent_type.to_string());

    let mcp_config = McpServerConfig {
        command: binary_path,
        args: vec![
            "--mcp-server".to_string(),
            "--socket".to_string(),
            socket_path.to_string(),
        ],
        env,
    };

    config.mcp_servers.insert(server_name.clone(), mcp_config);

    // Write back the config
    let content =
        serde_json::to_string_pretty(&config).context("Failed to serialize MCP config")?;
    std::fs::write(&config_path, content)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;

    // Enable the server
    let output = std::process::Command::new("agent")
        .args(["mcp", "enable", &server_name])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run 'agent mcp enable': {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("Failed to enable {}: {}", server_name, stderr);
    }

    tracing::debug!("✅ Configured MCP for {} agent: {}", agent_type, server_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_mcp_config_path() {
        let path = cursor_mcp_config_path().unwrap();
        assert!(path.ends_with(".cursor/mcp.json"));
    }

    #[test]
    fn test_mcp_config_serialization() {
        let mut config = CursorMcpConfig::default();

        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "value".to_string());

        config.mcp_servers.insert(
            "test-server".to_string(),
            McpServerConfig {
                command: "/usr/bin/test".to_string(),
                args: vec!["--arg1".to_string()],
                env,
            },
        );

        let json = serde_json::to_string_pretty(&config).unwrap();
        assert!(json.contains("mcpServers"));
        assert!(json.contains("test-server"));

        // Deserialize back
        let parsed: CursorMcpConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.mcp_servers.contains_key("test-server"));
    }
}
