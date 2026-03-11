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
use std::path::{Path, PathBuf};

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

fn read_or_create_mcp_config(config_path: &Path) -> Result<CursorMcpConfig> {
    if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    } else {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        Ok(CursorMcpConfig::default())
    }
}

fn write_mcp_config(config_path: &Path, config: &CursorMcpConfig) -> Result<()> {
    let content = serde_json::to_string_pretty(config).context("Failed to serialize MCP config")?;
    std::fs::write(config_path, content)
        .with_context(|| format!("Failed to write {}", config_path.display()))?;
    Ok(())
}

fn current_binary_path() -> Result<String> {
    Ok(std::env::current_exe()
        .context("Failed to get current executable path")?
        .to_string_lossy()
        .to_string())
}

fn build_paperboat_server_config(
    binary_path: &str,
    agent_type: &str,
    socket_path: &str,
) -> McpServerConfig {
    let mut env = HashMap::new();
    env.insert("PAPERBOAT_AGENT_TYPE".to_string(), agent_type.to_string());

    McpServerConfig {
        command: binary_path.to_string(),
        args: vec![
            "--mcp-server".to_string(),
            "--socket".to_string(),
            socket_path.to_string(),
        ],
        env,
    }
}

fn agent_server_name(agent_type: &str, unique_suffix: Option<&str>) -> String {
    unique_suffix.map_or_else(
        || mcp_server_name(agent_type),
        |suffix| format!("paperboat-{agent_type}-{suffix}"),
    )
}

fn remove_paperboat_servers(config: &mut CursorMcpConfig) {
    config
        .mcp_servers
        .retain(|name, _| !name.starts_with("paperboat-"));
}

fn configure_mcp_for_agent_at_path(
    config_path: &Path,
    binary_path: &str,
    agent_type: &str,
    socket_path: &str,
    unique_suffix: Option<&str>,
) -> Result<String> {
    let mut config = read_or_create_mcp_config(config_path)?;

    let server_name = agent_server_name(agent_type, unique_suffix);
    let mcp_config = build_paperboat_server_config(binary_path, agent_type, socket_path);

    // Keep other Paperboat entries intact so concurrently running agents retain
    // their own socket-backed MCP servers. Replacing the exact server name is
    // enough to refresh stale config for retries of the same session.
    config.mcp_servers.insert(server_name.clone(), mcp_config);

    write_mcp_config(config_path, &config)?;
    Ok(server_name)
}

fn run_agent_mcp_enable(server_name: &str) -> Result<()> {
    let output = std::process::Command::new("agent")
        .args(["mcp", "enable", server_name])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run 'agent mcp enable': {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("Failed to enable {}: {}", server_name, stderr);
    }

    Ok(())
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
/// * `socket_path` - IPC address for MCP communication
#[allow(dead_code)]
pub fn register_paperboat_mcp(socket_path: &str) -> Result<()> {
    let config_path = cursor_mcp_config_path()?;
    let binary_path = current_binary_path()?;
    let mut config = read_or_create_mcp_config(&config_path)?;

    // Register an MCP server for each agent type
    for agent_type in AGENT_TYPES {
        config.mcp_servers.insert(
            mcp_server_name(agent_type),
            build_paperboat_server_config(&binary_path, agent_type, socket_path),
        );
    }

    write_mcp_config(&config_path, &config)?;

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

    let mut config = read_or_create_mcp_config(&config_path)?;
    let original_len = config.mcp_servers.len();
    remove_paperboat_servers(&mut config);
    let removed = config.mcp_servers.len() != original_len;

    if removed {
        write_mcp_config(&config_path, &config)?;

        tracing::info!(
            "🗑️ Removed Paperboat MCP servers from {}",
            config_path.display()
        );
    }

    Ok(())
}

/// Configure MCP for a specific agent type.
///
/// Each session can register its own uniquely named Paperboat MCP server. We
/// intentionally preserve other Paperboat entries here so concurrently running
/// agents do not lose the socket mapping for their already-approved MCP tool.
/// Cleanup still removes all Paperboat entries when Paperboat exits.
///
/// # Arguments
///
/// * `agent_type` - The agent type ("planner", "orchestrator", "implementer")
/// * `socket_path` - IPC address for MCP communication
/// * `unique_suffix` - Optional unique suffix to prevent MCP server caching.
///   When provided, creates a server name like `paperboat-implementer-abc123`.
///   This forces Cursor to start a new MCP server process instead of reusing
///   a cached one from a previous agent.
pub fn enable_mcp_for_agent(
    agent_type: &str,
    socket_path: &str,
    unique_suffix: Option<&str>,
) -> Result<()> {
    let config_path = cursor_mcp_config_path()?;
    let binary_path = current_binary_path()?;
    let server_name = configure_mcp_for_agent_at_path(
        &config_path,
        &binary_path,
        agent_type,
        socket_path,
        unique_suffix,
    )?;
    run_agent_mcp_enable(&server_name)?;

    tracing::debug!(
        "✅ Configured MCP for {} agent: {}",
        agent_type,
        server_name
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use tempfile::tempdir;

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => env::set_var(self.key, value),
                None => env::remove_var(self.key),
            }
        }
    }

    #[cfg(unix)]
    fn prepend_path(dir: &Path) -> EnvGuard {
        let mut path_entries = vec![dir.display().to_string()];
        if let Ok(existing) = env::var("PATH") {
            path_entries.push(existing);
        }
        EnvGuard::set("PATH", &path_entries.join(":"))
    }

    #[cfg(unix)]
    fn write_agent_stub(dir: &Path, log_path: &Path) {
        let script_path = dir.join("agent");
        std::fs::write(
            &script_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit 0\n",
                log_path.display()
            ),
        )
        .unwrap();

        let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script_path, permissions).unwrap();
    }

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

    #[test]
    #[serial]
    #[cfg_attr(windows, ignore)] // Flaky on Windows due to env var and file isolation
    fn test_register_paperboat_mcp_creates_missing_config() {
        let temp = tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());

        register_paperboat_mcp("/tmp/paperboat.sock").unwrap();

        let config_path = temp.path().join(".cursor/mcp.json");
        assert!(config_path.exists());

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();

        assert_eq!(config.mcp_servers.len(), AGENT_TYPES.len());
        for agent_type in AGENT_TYPES {
            let server = config
                .mcp_servers
                .get(&mcp_server_name(agent_type))
                .unwrap();
            assert_eq!(
                server.env.get("PAPERBOAT_AGENT_TYPE"),
                Some(&agent_type.to_string())
            );
            assert_eq!(
                server.args,
                vec![
                    "--mcp-server".to_string(),
                    "--socket".to_string(),
                    "/tmp/paperboat.sock".to_string()
                ]
            );
        }
    }

    #[test]
    #[cfg(unix)]
    #[serial]
    fn test_enable_mcp_for_agent_creates_missing_config_and_uses_unique_suffix() {
        let temp = tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let log_path = temp.path().join("agent.log");
        write_agent_stub(temp.path(), &log_path);
        let _path = prepend_path(temp.path());

        enable_mcp_for_agent("implementer", "/tmp/paperboat.sock", Some("session123")).unwrap();

        let config_path = temp.path().join(".cursor/mcp.json");
        assert!(config_path.exists());

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);

        let server_name = "paperboat-implementer-session123";
        let server = config.mcp_servers.get(server_name).unwrap();
        assert_eq!(
            server.command,
            std::env::current_exe().unwrap().to_string_lossy()
        );
        assert_eq!(
            server.args,
            vec![
                "--mcp-server".to_string(),
                "--socket".to_string(),
                "/tmp/paperboat.sock".to_string()
            ]
        );
        assert_eq!(
            server.env.get("PAPERBOAT_AGENT_TYPE"),
            Some(&"implementer".to_string())
        );

        assert_eq!(
            std::fs::read_to_string(&log_path).unwrap().trim(),
            "mcp enable paperboat-implementer-session123"
        );
    }

    #[test]
    fn test_configure_mcp_preserves_other_paperboat_entries_for_concurrent_agents() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        std::fs::write(
            &config_path,
            serde_json::json!({
                "mcpServers": {
                    "github": {
                        "command": "gh",
                        "args": ["mcp", "serve"]
                    },
                    "paperboat-implementer-old123": {
                        "command": "/old/paperboat",
                        "args": ["--mcp-server", "--socket", "/tmp/old.sock"],
                        "env": {
                            "PAPERBOAT_AGENT_TYPE": "implementer"
                        }
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let server_name = configure_mcp_for_agent_at_path(
            &config_path,
            "/new/paperboat",
            "implementer",
            "/tmp/new.sock",
            Some("unique123"),
        )
        .unwrap();

        assert_eq!(server_name, "paperboat-implementer-unique123");

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();

        assert!(config.mcp_servers.contains_key("github"));
        assert!(
            config
                .mcp_servers
                .contains_key("paperboat-implementer-old123"),
            "existing concurrent Paperboat entry should be preserved"
        );
        assert_eq!(config.mcp_servers.len(), 3);

        let server = config.mcp_servers.get(&server_name).unwrap();
        assert_eq!(server.command, "/new/paperboat");
        assert_eq!(
            server.args,
            vec![
                "--mcp-server".to_string(),
                "--socket".to_string(),
                "/tmp/new.sock".to_string()
            ]
        );
        assert_eq!(
            server.env.get("PAPERBOAT_AGENT_TYPE"),
            Some(&"implementer".to_string())
        );
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false)] // Test cleanup needs to restore permissions
    fn test_configure_mcp_propagates_write_failures() {
        let temp = tempdir().unwrap();
        let cursor_dir = temp.path().join(".cursor");
        std::fs::create_dir_all(&cursor_dir).unwrap();
        let config_path = cursor_dir.join("mcp.json");
        std::fs::write(&config_path, r#"{"mcpServers":{}}"#).unwrap();
        let mut permissions = std::fs::metadata(&config_path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&config_path, permissions.clone()).unwrap();

        let result = configure_mcp_for_agent_at_path(
            &config_path,
            "/paperboat",
            "planner",
            "/tmp/planner.sock",
            None,
        );

        let mut restore = permissions;
        restore.set_readonly(false);
        std::fs::set_permissions(&config_path, restore).unwrap();

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to write"));
    }

    #[test]
    fn test_configure_mcp_propagates_bootstrap_failures() {
        let temp = tempdir().unwrap();
        let cursor_path = temp.path().join(".cursor");
        std::fs::write(&cursor_path, "not a directory").unwrap();

        let result = configure_mcp_for_agent_at_path(
            &cursor_path.join("mcp.json"),
            "/paperboat",
            "planner",
            "/tmp/planner.sock",
            None,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to create"));
    }

    #[test]
    #[serial]
    fn test_enable_mcp_for_agent_propagates_agent_invocation_failures() {
        let temp = tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let _path = EnvGuard::set("PATH", temp.path().to_str().unwrap());

        let result = enable_mcp_for_agent("implementer", "/tmp/worker.sock", Some("session123"));

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to run 'agent mcp enable'"));
    }

    // ========================================================================
    // Additional tempdir-backed integration tests for expanded coverage
    // ========================================================================

    #[test]
    #[serial]
    fn test_unregister_paperboat_mcp_when_config_missing_returns_ok() {
        let temp = tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        // No .cursor/mcp.json file exists

        let result = unregister_paperboat_mcp();

        assert!(result.is_ok());
        // Verify the file was not created
        assert!(!temp.path().join(".cursor/mcp.json").exists());
    }

    #[test]
    #[serial]
    #[cfg_attr(windows, ignore)] // Flaky on Windows due to env var and file isolation
    fn test_unregister_paperboat_mcp_removes_entries_and_preserves_unrelated() {
        let temp = tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let config_path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        std::fs::write(
            &config_path,
            serde_json::json!({
                "mcpServers": {
                    "github": {
                        "command": "gh",
                        "args": ["mcp", "serve"]
                    },
                    "paperboat-planner": {
                        "command": "/old/paperboat",
                        "args": ["--mcp-server"]
                    },
                    "paperboat-implementer-session456": {
                        "command": "/old/paperboat",
                        "args": ["--mcp-server"]
                    },
                    "another-tool": {
                        "command": "/usr/bin/tool",
                        "args": []
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        unregister_paperboat_mcp().unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();

        // Verify paperboat entries removed
        assert!(!config
            .mcp_servers
            .keys()
            .any(|k| k.starts_with("paperboat-")));
        // Verify unrelated entries preserved
        assert!(config.mcp_servers.contains_key("github"));
        assert!(config.mcp_servers.contains_key("another-tool"));
        assert_eq!(config.mcp_servers.len(), 2);
    }

    #[test]
    #[serial]
    fn test_unregister_paperboat_mcp_no_changes_when_no_paperboat_entries() {
        let temp = tempdir().unwrap();
        let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let config_path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        let original_content = serde_json::json!({
            "mcpServers": {
                "github": {
                    "command": "gh",
                    "args": ["mcp", "serve"]
                }
            }
        })
        .to_string();
        std::fs::write(&config_path, &original_content).unwrap();

        unregister_paperboat_mcp().unwrap();

        let content = std::fs::read_to_string(&config_path).unwrap();
        // Verify file wasn't rewritten (content should match original structure)
        let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert!(config.mcp_servers.contains_key("github"));
    }

    #[test]
    fn test_configure_mcp_for_agent_without_unique_suffix() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, r#"{"mcpServers":{}}"#).unwrap();

        let server_name = configure_mcp_for_agent_at_path(
            &config_path,
            "/test/paperboat",
            "orchestrator",
            "/tmp/orchestrator.sock",
            None, // No unique suffix
        )
        .unwrap();

        // Without suffix, should be just "paperboat-orchestrator"
        assert_eq!(server_name, "paperboat-orchestrator");

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();
        assert!(config.mcp_servers.contains_key("paperboat-orchestrator"));
    }

    #[test]
    fn test_configure_mcp_replaces_same_server_name_entry() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        // Pre-populate with an old entry for the same server name
        std::fs::write(
            &config_path,
            serde_json::json!({
                "mcpServers": {
                    "paperboat-planner-xyz": {
                        "command": "/old/path",
                        "args": ["--old-socket", "/old.sock"]
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let server_name = configure_mcp_for_agent_at_path(
            &config_path,
            "/new/paperboat",
            "planner",
            "/tmp/new.sock",
            Some("xyz"), // Same suffix as existing entry
        )
        .unwrap();

        assert_eq!(server_name, "paperboat-planner-xyz");

        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();

        // Should still have only one entry (replaced)
        assert_eq!(config.mcp_servers.len(), 1);
        let server = config.mcp_servers.get("paperboat-planner-xyz").unwrap();
        assert_eq!(server.command, "/new/paperboat");
        assert_eq!(
            server.args,
            vec!["--mcp-server", "--socket", "/tmp/new.sock"]
        );
    }

    #[test]
    fn test_read_or_create_mcp_config_invalid_json_returns_default() {
        let temp = tempdir().unwrap();
        let config_path = temp.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();

        // Write invalid JSON
        std::fs::write(&config_path, "{ not valid json }").unwrap();

        let config = read_or_create_mcp_config(&config_path).unwrap();

        // Should return default (empty mcpServers)
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_mcp_server_name_for_all_agent_types() {
        assert_eq!(mcp_server_name("planner"), "paperboat-planner");
        assert_eq!(mcp_server_name("orchestrator"), "paperboat-orchestrator");
        assert_eq!(mcp_server_name("implementer"), "paperboat-implementer");
    }

    #[test]
    fn test_agent_server_name_with_and_without_suffix() {
        assert_eq!(agent_server_name("planner", None), "paperboat-planner");
        assert_eq!(
            agent_server_name("planner", Some("abc123")),
            "paperboat-planner-abc123"
        );
        assert_eq!(
            agent_server_name("implementer", Some("session-1")),
            "paperboat-implementer-session-1"
        );
    }

    #[test]
    fn test_remove_paperboat_servers_removes_all_variants() {
        let mut config = CursorMcpConfig::default();
        config.mcp_servers.insert(
            "paperboat-planner".to_string(),
            McpServerConfig {
                command: "/test".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
        );
        config.mcp_servers.insert(
            "paperboat-implementer-session123".to_string(),
            McpServerConfig {
                command: "/test".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
        );
        config.mcp_servers.insert(
            "github".to_string(),
            McpServerConfig {
                command: "gh".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
        );

        remove_paperboat_servers(&mut config);

        assert_eq!(config.mcp_servers.len(), 1);
        assert!(config.mcp_servers.contains_key("github"));
    }

    #[test]
    fn test_build_paperboat_server_config_structure() {
        let config = build_paperboat_server_config(
            "/usr/local/bin/paperboat",
            "orchestrator",
            "/var/run/paperboat.sock",
        );

        assert_eq!(config.command, "/usr/local/bin/paperboat");
        assert_eq!(
            config.args,
            vec![
                "--mcp-server".to_string(),
                "--socket".to_string(),
                "/var/run/paperboat.sock".to_string()
            ]
        );
        assert_eq!(
            config.env.get("PAPERBOAT_AGENT_TYPE"),
            Some(&"orchestrator".to_string())
        );
    }

    #[test]
    #[cfg(unix)]
    #[serial]
    fn test_enable_mcp_for_agent_with_different_agent_types() {
        for agent_type in AGENT_TYPES {
            let temp = tempdir().unwrap();
            let _home = EnvGuard::set("HOME", temp.path().to_str().unwrap());
            let log_path = temp.path().join("agent.log");
            write_agent_stub(temp.path(), &log_path);
            let _path = prepend_path(temp.path());

            let suffix = format!("test-{}", agent_type);
            enable_mcp_for_agent(agent_type, "/tmp/test.sock", Some(&suffix)).unwrap();

            let config_path = temp.path().join(".cursor/mcp.json");
            let content = std::fs::read_to_string(&config_path).unwrap();
            let config: CursorMcpConfig = serde_json::from_str(&content).unwrap();

            let expected_server_name = format!("paperboat-{}-{}", agent_type, suffix);
            assert!(
                config.mcp_servers.contains_key(&expected_server_name),
                "Expected server {} for agent type {}",
                expected_server_name,
                agent_type
            );

            let server = config.mcp_servers.get(&expected_server_name).unwrap();
            assert_eq!(
                server.env.get("PAPERBOAT_AGENT_TYPE"),
                Some(&agent_type.to_string())
            );

            // Verify agent mcp enable was called
            let log_content = std::fs::read_to_string(&log_path).unwrap();
            assert!(
                log_content.contains(&format!("mcp enable {}", expected_server_name)),
                "Expected agent mcp enable call for {}",
                expected_server_name
            );
        }
    }

    #[test]
    fn test_configure_mcp_creates_nested_cursor_directory() {
        let temp = tempdir().unwrap();
        // Don't create .cursor directory - let the function do it
        let config_path = temp.path().join(".cursor/mcp.json");

        let server_name = configure_mcp_for_agent_at_path(
            &config_path,
            "/paperboat",
            "implementer",
            "/tmp/test.sock",
            Some("auto-create"),
        )
        .unwrap();

        assert_eq!(server_name, "paperboat-implementer-auto-create");
        assert!(config_path.exists());
        assert!(config_path.parent().unwrap().exists());
    }
}
