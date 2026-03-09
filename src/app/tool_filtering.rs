//! Tool filtering and MCP server configuration logic for agent spawning.
//!
//! This module handles determining which tools should be removed for each agent type
//! based on whitelists and MCP server configuration.

use serde_json::json;
use std::path::Path;

/// All available backend tools that can be whitelisted/removed.
///
/// These are the standard tools available across backends (Auggie, Cursor, etc.).
pub const ALL_BACKEND_TOOLS: &[&str] = &[
    "str-replace-editor",
    "save-file",
    "remove-files",
    "launch-process",
    "kill-process",
    "read-process",
    "write-process",
    "list-processes",
    "web-search",
    "web-fetch",
];

/// Computes the list of tools to remove based on an optional whitelist.
///
/// If `allowed_tools` is provided, returns tools NOT in the whitelist.
/// If `allowed_tools` is `None`, returns an empty vec (all tools enabled).
pub fn compute_removed_tools(allowed_tools: Option<&Vec<String>>) -> Vec<String> {
    match allowed_tools {
        Some(whitelist) => ALL_BACKEND_TOOLS
            .iter()
            .filter(|t| !whitelist.contains(&(*t).to_string()))
            .map(std::string::ToString::to_string)
            .collect(),
        None => vec![],
    }
}

/// Builds the MCP server environment variables for an agent.
///
/// Always includes `PAPERBOAT_AGENT_TYPE`. Optionally includes `PAPERBOAT_REMOVED_TOOLS`
/// if there are tools to remove.
pub fn build_mcp_env_vars(role: &str, removed_tools: &[String]) -> Vec<serde_json::Value> {
    let mut env_vars = vec![json!({
        "name": "PAPERBOAT_AGENT_TYPE",
        "value": role
    })];

    if !removed_tools.is_empty() {
        env_vars.push(json!({
            "name": "PAPERBOAT_REMOVED_TOOLS",
            "value": removed_tools.join(",")
        }));
    }

    env_vars
}

/// Builds the MCP server configuration for an agent.
///
/// Creates a JSON configuration with the binary path, socket path, and environment variables.
/// The name includes the role to help prevent server caching issues.
pub fn build_mcp_server_config(
    binary_path: &Path,
    socket_path: &str,
    role: &str,
    removed_tools: &[String],
    unique_suffix: Option<&str>,
) -> serde_json::Value {
    let env_vars = build_mcp_env_vars(role, removed_tools);

    let name = if let Some(suffix) = unique_suffix {
        format!("paperboat-{role}-{suffix}")
    } else {
        format!("paperboat-{role}")
    };

    json!({
        "name": name,
        "command": binary_path.to_string_lossy(),
        "args": ["--mcp-server", "--socket", socket_path],
        "env": env_vars
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_removed_tools_with_none() {
        let result = compute_removed_tools(None);
        assert!(result.is_empty(), "No whitelist means no removed tools");
    }

    #[test]
    fn test_compute_removed_tools_with_whitelist() {
        let whitelist = vec!["str-replace-editor".to_string(), "save-file".to_string()];
        let result = compute_removed_tools(Some(&whitelist));

        // Should contain all tools except the whitelisted ones
        assert!(!result.contains(&"str-replace-editor".to_string()));
        assert!(!result.contains(&"save-file".to_string()));
        assert!(result.contains(&"remove-files".to_string()));
        assert!(result.contains(&"launch-process".to_string()));
    }

    #[test]
    fn test_compute_removed_tools_empty_whitelist() {
        let whitelist = Vec::new();
        let result = compute_removed_tools(Some(&whitelist));

        // All tools should be removed
        assert_eq!(result.len(), ALL_BACKEND_TOOLS.len());
    }

    #[test]
    fn test_build_mcp_env_vars_no_removed_tools() {
        let result = build_mcp_env_vars("implementer", &[]);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "PAPERBOAT_AGENT_TYPE");
        assert_eq!(result[0]["value"], "implementer");
    }

    #[test]
    fn test_build_mcp_env_vars_with_removed_tools() {
        let removed = vec!["web-search".to_string(), "web-fetch".to_string()];
        let result = build_mcp_env_vars("explorer", &removed);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["name"], "PAPERBOAT_AGENT_TYPE");
        assert_eq!(result[0]["value"], "explorer");
        assert_eq!(result[1]["name"], "PAPERBOAT_REMOVED_TOOLS");
        assert_eq!(result[1]["value"], "web-search,web-fetch");
    }
}
