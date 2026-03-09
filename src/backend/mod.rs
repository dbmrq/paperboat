//! Backend abstraction layer for agent providers.
//!
//! This module provides the core abstraction for supporting multiple agent backends
//! (Auggie, Cursor, etc.) while maintaining a unified interface for the application.
//!
//! # Architecture: Backend × Transport = Client
//!
//! The backend system separates concerns into three layers:
//!
//! ```text
//! Backend (vendor)     × Transport (protocol)  = Client (implementation)
//! ─────────────────────────────────────────────────────────────────────
//! Auggie               × ACP                   = AuggieAcpClient ✓
//! Cursor               × ACP                   = CursorAcpClient (MCP broken)
//! Cursor               × CLI                   = CursorCliClient (MCP works) ✓
//! ```
//!
//! - **Backend**: Vendor-specific configuration (auth, models, MCP setup)
//! - **Transport**: Communication protocol (ACP vs CLI)
//! - **AgentType**: Permission control (what tools each agent type can use)
//!
//! # Core Types
//!
//! - [`BackendKind`] - An enum representing available backend providers
//! - [`BackendConfig`] - Configuration combining backend + optional transport
//! - [`Backend`] - Trait for vendor-specific configuration
//! - [`TransportKind`] - Available communication protocols (ACP, CLI)
//! - [`AgentTransport`] - Trait for communication implementations
//!
//! # Backend Selection
//!
//! Backends can be selected via (in priority order):
//! 1. CLI flag: `--backend cursor` or `--backend cursor:cli`
//! 2. Environment variable: `PAPERBOAT_BACKEND=cursor:cli`
//! 3. Project config: `.paperboat/config.toml` with `backend = "cursor:cli"`
//! 4. User config: `~/.paperboat/config.toml`
//! 5. Default: Auggie with ACP
//!
//! # Backend:Transport Syntax
//!
//! The `--backend` flag supports an optional transport suffix:
//!
//! ```text
//! --backend cursor        # Uses default transport (CLI for Cursor)
//! --backend cursor:cli    # Explicitly use CLI transport
//! --backend cursor:acp    # Explicitly use ACP transport
//! --backend auggie        # Uses default transport (ACP for Auggie)
//! --backend auggie:acp    # Explicitly use ACP transport (redundant but allowed)
//! ```
//!
//! # Default Transports
//!
//! | Backend | Default Transport | Reason |
//! |---------|-------------------|--------|
//! | Auggie  | ACP              | Only supported transport |
//! | Cursor  | CLI              | Better MCP tool support |
//!
//! # Usage
//!
//! ```ignore
//! use paperboat::backend::{BackendKind, BackendConfig, Backend, TransportKind};
//!
//! // Parse from string (e.g., from CLI or config)
//! let config = BackendConfig::parse("cursor:cli").unwrap();
//!
//! // Create the backend instance
//! let backend = config.kind.create();
//!
//! // Get the effective transport (explicit or default)
//! let transport = config.effective_transport();
//! assert_eq!(transport, TransportKind::Cli);
//! ```
//!
//! # Adding a New Backend
//!
//! To add a new backend (e.g., "newbackend"):
//!
//! 1. **Create the module**: Add `src/backend/newbackend/` with:
//!    - `mod.rs` - Main module with struct implementing [`Backend`]
//!    - `auth.rs` - Authentication checking
//!    - `cache.rs` - Cache directory setup
//!    - Transport implementations (e.g., `acp.rs`, `cli.rs`)
//!
//! 2. **Implement the [`Backend`] trait** for your new backend struct.
//!
//! 3. **Add the variant to [`BackendKind`]**:
//!    - Add `NewBackend` variant to the enum
//!    - Update `BackendKind::ALL`
//!    - Update `as_str()`, `from_str()`, `create()`, `default_transport()`,
//!      and `supported_transports()`
//!
//! 4. **Update the string parsing** in `FromStr for BackendKind`
//!
//! See [`auggie`] and [`cursor`] modules for reference implementations.

pub mod auggie;
pub mod cursor;
mod r#trait;
pub mod transport;

pub use r#trait::{AgentCacheType, Backend, TransportConfig};
pub use transport::TransportKind;

use std::str::FromStr;

// ============================================================================
// Backend Configuration (backend + transport selection)
// ============================================================================

/// Parsed backend configuration from CLI string.
///
/// Represents a backend selection with an optional transport override.
/// Parsed from strings like "cursor", "cursor:cli", "cursor:acp", "auggie:acp".
///
/// # Examples
///
/// ```ignore
/// let config = BackendConfig::parse("cursor:cli")?;
/// assert_eq!(config.kind, BackendKind::Cursor);
/// assert_eq!(config.transport, Some(TransportKind::Cli));
///
/// let config = BackendConfig::parse("auggie")?;
/// assert_eq!(config.kind, BackendKind::Auggie);
/// assert_eq!(config.transport, None); // Uses default
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendConfig {
    /// The backend to use (Auggie, Cursor, etc.)
    pub kind: BackendKind,

    /// Optional transport override.
    ///
    /// If `None`, the backend's default transport is used:
    /// - Auggie: ACP (only option)
    /// - Cursor: CLI (default, better MCP support)
    pub transport: Option<TransportKind>,
}

impl BackendConfig {
    /// Create a new backend configuration with default transport.
    #[must_use]
    pub const fn new(kind: BackendKind) -> Self {
        Self {
            kind,
            transport: None,
        }
    }

    /// Create a new backend configuration with explicit transport.
    #[must_use]
    #[allow(dead_code)]
    pub const fn with_transport(kind: BackendKind, transport: TransportKind) -> Self {
        Self {
            kind,
            transport: Some(transport),
        }
    }

    /// Parse a backend string with optional transport suffix.
    ///
    /// Supported formats:
    /// - `"cursor"` - Cursor with default transport (CLI)
    /// - `"cursor:cli"` - Cursor with CLI transport
    /// - `"cursor:acp"` - Cursor with ACP transport
    /// - `"auggie"` - Auggie with default transport (ACP)
    /// - `"auggie:acp"` - Auggie with ACP transport (explicit but redundant)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The backend name is invalid
    /// - The transport name is invalid
    /// - The transport is not supported by the backend (e.g., "auggie:cli")
    pub fn parse(s: &str) -> Result<Self, ParseBackendConfigError> {
        let (backend_str, transport_str) = if let Some(idx) = s.find(':') {
            (&s[..idx], Some(&s[idx + 1..]))
        } else {
            (s, None)
        };

        // Parse backend kind
        let kind = BackendKind::from_str(backend_str).map_err(|e| ParseBackendConfigError {
            input: s.to_string(),
            message: e.to_string(),
        })?;

        // Parse transport if specified
        let transport = if let Some(transport_str) = transport_str {
            let transport =
                parse_transport_kind(transport_str).ok_or_else(|| ParseBackendConfigError {
                    input: s.to_string(),
                    message: format!(
                        "unknown transport '{}'. Valid options: acp, cli",
                        transport_str
                    ),
                })?;

            // Validate transport is supported by backend
            kind.validate_transport(transport)
                .map_err(|msg| ParseBackendConfigError {
                    input: s.to_string(),
                    message: msg,
                })?;

            Some(transport)
        } else {
            None
        };

        Ok(Self { kind, transport })
    }

    /// Get the effective transport for this configuration.
    ///
    /// Returns the explicit transport if set, otherwise the backend's default.
    #[must_use]
    pub fn effective_transport(&self) -> TransportKind {
        self.transport
            .unwrap_or_else(|| self.kind.default_transport())
    }
}

impl Default for BackendConfig {
    /// Returns the default configuration: Auggie backend with ACP transport.
    fn default() -> Self {
        Self::new(BackendKind::default())
    }
}

impl std::fmt::Display for BackendConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(transport) = self.transport {
            write!(f, "{}:{}", self.kind, transport)
        } else {
            write!(f, "{}", self.kind)
        }
    }
}

/// Error type for parsing backend configuration from string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseBackendConfigError {
    /// The invalid input string
    pub input: String,
    /// Description of what went wrong
    pub message: String,
}

impl std::fmt::Display for ParseBackendConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseBackendConfigError {}

/// Parse transport kind from string.
fn parse_transport_kind(s: &str) -> Option<TransportKind> {
    match s.to_lowercase().as_str() {
        "acp" => Some(TransportKind::Acp),
        "cli" => Some(TransportKind::Cli),
        _ => None,
    }
}

// ============================================================================
// Backend Kind
// ============================================================================

/// Available backend providers for agent communication.
///
/// This enum represents the different agent backends that paperboat can use.
/// Each variant corresponds to a specific implementation of the [`Backend`] trait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendKind {
    /// Augment's Auggie CLI (default).
    ///
    /// Uses the `auggie` command-line tool for ACP communication.
    /// Authenticate with `auggie login`.
    ///
    /// **Supported transports:** ACP only
    #[default]
    Auggie,

    /// Cursor's agent CLI.
    ///
    /// Uses Cursor's agent infrastructure for agent communication.
    /// Authenticate with `cursor login` or set `CURSOR_API_KEY`.
    ///
    /// **Supported transports:** CLI (default, better MCP), ACP
    Cursor,
}

impl BackendKind {
    /// All available backend kinds.
    #[allow(dead_code)]
    pub const ALL: &'static [BackendKind] = &[BackendKind::Auggie, BackendKind::Cursor];

    /// Returns the string identifier for this backend kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Auggie => "auggie",
            Self::Cursor => "cursor",
        }
    }

    /// Create a backend instance for this kind.
    ///
    /// Returns a boxed trait object implementing [`Backend`].
    #[must_use]
    pub fn create(&self) -> Box<dyn Backend> {
        match self {
            Self::Auggie => Box::new(auggie::AuggieBackend::new()),
            Self::Cursor => Box::new(cursor::CursorBackend::new()),
        }
    }

    /// Returns the default transport for this backend.
    ///
    /// - Auggie: ACP (the only supported transport)
    /// - Cursor: CLI (better MCP support)
    #[must_use]
    pub const fn default_transport(&self) -> TransportKind {
        match self {
            Self::Auggie => TransportKind::Acp,
            Self::Cursor => TransportKind::Cli,
        }
    }

    /// Returns the list of transports supported by this backend.
    #[must_use]
    pub const fn supported_transports(&self) -> &'static [TransportKind] {
        match self {
            Self::Auggie => &[TransportKind::Acp],
            Self::Cursor => &[TransportKind::Cli, TransportKind::Acp],
        }
    }

    /// Check if a transport is supported by this backend.
    #[must_use]
    pub fn supports_transport(&self, transport: TransportKind) -> bool {
        self.supported_transports().contains(&transport)
    }

    /// Validate that a transport is supported by this backend.
    ///
    /// Returns `Ok(())` if supported, or an error message if not.
    pub fn validate_transport(&self, transport: TransportKind) -> Result<(), String> {
        if self.supports_transport(transport) {
            Ok(())
        } else {
            let supported_list = self
                .supported_transports()
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            Err(format!(
                "transport '{}' is not supported by {} backend. Supported: {}",
                transport.as_str(),
                self.as_str(),
                supported_list
            ))
        }
    }

    /// Parse a backend string with optional transport suffix.
    ///
    /// This is a convenience method that delegates to [`BackendConfig::parse`].
    ///
    /// # Returns
    ///
    /// Tuple of (backend_kind, optional_transport)
    #[allow(dead_code)]
    pub fn parse_with_transport(
        s: &str,
    ) -> Result<(BackendKind, Option<TransportKind>), ParseBackendConfigError> {
        let config = BackendConfig::parse(s)?;
        Ok((config.kind, config.transport))
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Error type for parsing backend kind from string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseBackendKindError {
    /// The invalid input string
    pub input: String,
}

impl std::fmt::Display for ParseBackendKindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown backend '{}'. Valid options: auggie, augment, cursor\n\n\
            Optionally specify transport: cursor:cli, cursor:acp, auggie:acp",
            self.input
        )
    }
}

impl std::error::Error for ParseBackendKindError {}

impl FromStr for BackendKind {
    type Err = ParseBackendKindError;

    /// Parse backend kind from string.
    ///
    /// This parses only the backend name, not the transport suffix.
    /// For parsing with transport, use [`BackendConfig::parse`] or
    /// [`BackendKind::parse_with_transport`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auggie" | "augment" => Ok(Self::Auggie),
            "cursor" => Ok(Self::Cursor),
            _ => Err(ParseBackendKindError {
                input: s.to_string(),
            }),
        }
    }
}

// ============================================================================
// Backend Availability Detection
// ============================================================================

/// Check which backends are available by attempting to list models.
///
/// This function runs the model list command for each backend and returns
/// which ones successfully respond. This is useful for auto-detecting
/// which backend to use when the user hasn't specified one.
///
/// # Returns
///
/// A vector of available backend kinds (those that successfully list models).
///
/// # Example
///
/// ```ignore
/// let available = discover_available_backends().await;
/// match available.len() {
///     0 => panic!("No backends available"),
///     1 => use_backend(available[0]),
///     _ => prompt_user_to_select(available),
/// }
/// ```
pub async fn discover_available_backends() -> Vec<BackendKind> {
    use tokio::process::Command;

    let mut available = Vec::new();

    // Check Auggie availability by running `auggie model list`
    let auggie_result = Command::new("auggie")
        .args(["model", "list"])
        .output()
        .await;

    if let Ok(output) = auggie_result {
        if output.status.success() {
            tracing::debug!("Auggie backend is available");
            available.push(BackendKind::Auggie);
        } else {
            tracing::debug!(
                "Auggie backend not available: command failed with status {}",
                output.status
            );
        }
    } else {
        tracing::debug!("Auggie backend not available: auggie command not found");
    }

    // Check Cursor availability by running `cursor-agent --list-models`
    let cursor_result = Command::new("cursor-agent")
        .arg("--list-models")
        .output()
        .await;

    if let Ok(output) = cursor_result {
        if output.status.success() {
            tracing::debug!("Cursor backend is available");
            available.push(BackendKind::Cursor);
        } else {
            tracing::debug!(
                "Cursor backend not available: command failed with status {}",
                output.status
            );
        }
    } else {
        tracing::debug!("Cursor backend not available: cursor-agent command not found");
    }

    available
}

/// Prompt the user to select a backend interactively.
///
/// Displays available backends and waits for user input.
/// Returns `None` if stdin is not a terminal or if there's an error reading input.
///
/// # Arguments
///
/// * `available` - List of available backends to choose from
///
/// # Returns
///
/// The selected `BackendKind`, or `None` if selection failed.
pub fn prompt_backend_selection(available: &[BackendKind]) -> Option<BackendKind> {
    use std::io::{self, BufRead, Write};

    // Don't prompt if stdin is not a terminal
    if !std::io::IsTerminal::is_terminal(&io::stdin()) {
        return None;
    }

    println!("\n🔍 Multiple AI backends are available:\n");

    for (i, kind) in available.iter().enumerate() {
        let description = match kind {
            BackendKind::Auggie => "Augment CLI (auggie)",
            BackendKind::Cursor => "Cursor Agent CLI",
        };
        println!("  {}. {} - {}", i + 1, kind.as_str(), description);
    }

    println!();
    print!("Select a backend [1-{}]: ", available.len());
    io::stdout().flush().ok()?;

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line).ok()?;

    let selection: usize = line.trim().parse().ok()?;
    if selection >= 1 && selection <= available.len() {
        let selected = available[selection - 1];
        println!("✓ Selected: {}\n", selected.as_str());
        Some(selected)
    } else {
        eprintln!("Invalid selection. Using default: {}", available[0].as_str());
        Some(available[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_kind_default_is_auggie() {
        assert_eq!(BackendKind::default(), BackendKind::Auggie);
    }

    #[test]
    fn test_backend_kind_as_str() {
        assert_eq!(BackendKind::Auggie.as_str(), "auggie");
        assert_eq!(BackendKind::Cursor.as_str(), "cursor");
    }

    #[test]
    fn test_backend_kind_display() {
        assert_eq!(format!("{}", BackendKind::Auggie), "auggie");
        assert_eq!(format!("{}", BackendKind::Cursor), "cursor");
    }

    #[test]
    fn test_backend_kind_from_str_auggie() {
        assert_eq!(
            "auggie".parse::<BackendKind>().unwrap(),
            BackendKind::Auggie
        );
        assert_eq!(
            "Auggie".parse::<BackendKind>().unwrap(),
            BackendKind::Auggie
        );
        assert_eq!(
            "AUGGIE".parse::<BackendKind>().unwrap(),
            BackendKind::Auggie
        );
    }

    #[test]
    fn test_backend_kind_from_str_augment() {
        // "augment" should also map to Auggie
        assert_eq!(
            "augment".parse::<BackendKind>().unwrap(),
            BackendKind::Auggie
        );
        assert_eq!(
            "Augment".parse::<BackendKind>().unwrap(),
            BackendKind::Auggie
        );
        assert_eq!(
            "AUGMENT".parse::<BackendKind>().unwrap(),
            BackendKind::Auggie
        );
    }

    #[test]
    fn test_backend_kind_from_str_cursor() {
        assert_eq!(
            "cursor".parse::<BackendKind>().unwrap(),
            BackendKind::Cursor
        );
        assert_eq!(
            "Cursor".parse::<BackendKind>().unwrap(),
            BackendKind::Cursor
        );
        assert_eq!(
            "CURSOR".parse::<BackendKind>().unwrap(),
            BackendKind::Cursor
        );
    }

    #[test]
    fn test_backend_kind_from_str_invalid() {
        let err = "unknown".parse::<BackendKind>().unwrap_err();
        assert_eq!(err.input, "unknown");
        assert!(err.to_string().contains("unknown backend"));
        assert!(err.to_string().contains("auggie"));
        assert!(err.to_string().contains("cursor"));
    }

    #[test]
    fn test_backend_kind_all() {
        assert_eq!(BackendKind::ALL.len(), 2);
        assert!(BackendKind::ALL.contains(&BackendKind::Auggie));
        assert!(BackendKind::ALL.contains(&BackendKind::Cursor));
    }

    #[test]
    fn test_agent_cache_type_as_str() {
        use super::AgentCacheType;
        assert_eq!(AgentCacheType::Orchestrator.as_str(), "orchestrator");
        assert_eq!(AgentCacheType::Planner.as_str(), "planner");
        assert_eq!(AgentCacheType::Worker.as_str(), "worker");
    }

    #[test]
    fn test_agent_cache_type_display() {
        use super::AgentCacheType;
        assert_eq!(format!("{}", AgentCacheType::Orchestrator), "orchestrator");
        assert_eq!(format!("{}", AgentCacheType::Planner), "planner");
        assert_eq!(format!("{}", AgentCacheType::Worker), "worker");
    }

    // ========================================================================
    // Backend Instance Creation Tests
    // ========================================================================

    #[test]
    fn test_backend_kind_create_auggie() {
        let backend = BackendKind::Auggie.create();
        assert_eq!(backend.name(), "auggie");
        assert_eq!(backend.login_hint(), "auggie login");
    }

    #[test]
    fn test_backend_kind_create_cursor() {
        let backend = BackendKind::Cursor.create();
        assert_eq!(backend.name(), "cursor");
        assert_eq!(backend.login_hint(), "agent login");
    }

    #[test]
    fn test_all_backends_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        // Test that the trait object returned by create() is Send + Sync
        let auggie_backend = BackendKind::Auggie.create();
        let cursor_backend = BackendKind::Cursor.create();

        // If these compile, the backends are Send + Sync
        fn takes_backend(_: Box<dyn Backend>) {}
        takes_backend(auggie_backend);
        takes_backend(cursor_backend);

        // Also test the concrete types
        assert_send_sync::<auggie::AuggieBackend>();
        assert_send_sync::<cursor::CursorBackend>();
    }

    #[test]
    fn test_all_backends_have_login_hints() {
        for kind in BackendKind::ALL {
            let backend = kind.create();
            let hint = backend.login_hint();
            assert!(
                !hint.is_empty(),
                "Backend {} should have a login hint",
                kind
            );
        }
    }

    #[test]
    fn test_all_backends_have_names() {
        for kind in BackendKind::ALL {
            let backend = kind.create();
            let name = backend.name();
            assert!(!name.is_empty(), "Backend {} should have a name", kind);
            // Name should match the kind's as_str()
            assert_eq!(name, kind.as_str());
        }
    }

    #[test]
    fn test_all_backends_have_auth_error_messages() {
        for kind in BackendKind::ALL {
            let backend = kind.create();
            let msg = backend.auth_error_message();
            assert!(
                !msg.is_empty(),
                "Backend {} should have an auth error message",
                kind
            );
            // Auth error message should mention how to authenticate
            assert!(
                msg.contains(backend.login_hint()) || msg.contains("API_KEY"),
                "Auth error message for {} should mention how to authenticate: {}",
                kind,
                msg
            );
        }
    }

    // ========================================================================
    // Backend Transport Tests
    // ========================================================================

    #[test]
    fn test_backend_kind_default_transport() {
        assert_eq!(BackendKind::Auggie.default_transport(), TransportKind::Acp);
        assert_eq!(BackendKind::Cursor.default_transport(), TransportKind::Cli);
    }

    #[test]
    fn test_backend_kind_supported_transports() {
        // Auggie only supports ACP
        let auggie_transports = BackendKind::Auggie.supported_transports();
        assert_eq!(auggie_transports.len(), 1);
        assert!(auggie_transports.contains(&TransportKind::Acp));

        // Cursor supports both CLI and ACP
        let cursor_transports = BackendKind::Cursor.supported_transports();
        assert_eq!(cursor_transports.len(), 2);
        assert!(cursor_transports.contains(&TransportKind::Cli));
        assert!(cursor_transports.contains(&TransportKind::Acp));
    }

    #[test]
    fn test_backend_kind_supports_transport() {
        // Auggie
        assert!(BackendKind::Auggie.supports_transport(TransportKind::Acp));
        assert!(!BackendKind::Auggie.supports_transport(TransportKind::Cli));

        // Cursor
        assert!(BackendKind::Cursor.supports_transport(TransportKind::Acp));
        assert!(BackendKind::Cursor.supports_transport(TransportKind::Cli));
    }

    #[test]
    fn test_backend_kind_validate_transport() {
        // Valid combinations
        assert!(BackendKind::Auggie
            .validate_transport(TransportKind::Acp)
            .is_ok());
        assert!(BackendKind::Cursor
            .validate_transport(TransportKind::Acp)
            .is_ok());
        assert!(BackendKind::Cursor
            .validate_transport(TransportKind::Cli)
            .is_ok());

        // Invalid: Auggie doesn't support CLI
        let err = BackendKind::Auggie
            .validate_transport(TransportKind::Cli)
            .unwrap_err();
        assert!(err.contains("not supported"));
        assert!(err.contains("auggie"));
        assert!(err.contains("acp"));
    }

    // ========================================================================
    // BackendConfig Parsing Tests
    // ========================================================================

    #[test]
    fn test_backend_config_parse_simple() {
        // Simple backend names (no transport suffix)
        let config = BackendConfig::parse("cursor").unwrap();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, None);

        let config = BackendConfig::parse("auggie").unwrap();
        assert_eq!(config.kind, BackendKind::Auggie);
        assert_eq!(config.transport, None);

        let config = BackendConfig::parse("augment").unwrap();
        assert_eq!(config.kind, BackendKind::Auggie);
        assert_eq!(config.transport, None);
    }

    #[test]
    fn test_backend_config_parse_with_transport() {
        // Cursor with CLI (explicit default)
        let config = BackendConfig::parse("cursor:cli").unwrap();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, Some(TransportKind::Cli));

        // Cursor with ACP
        let config = BackendConfig::parse("cursor:acp").unwrap();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, Some(TransportKind::Acp));

        // Auggie with ACP (explicit but redundant)
        let config = BackendConfig::parse("auggie:acp").unwrap();
        assert_eq!(config.kind, BackendKind::Auggie);
        assert_eq!(config.transport, Some(TransportKind::Acp));
    }

    #[test]
    fn test_backend_config_parse_case_insensitive() {
        let config = BackendConfig::parse("CURSOR:CLI").unwrap();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, Some(TransportKind::Cli));

        let config = BackendConfig::parse("Cursor:Acp").unwrap();
        assert_eq!(config.kind, BackendKind::Cursor);
        assert_eq!(config.transport, Some(TransportKind::Acp));
    }

    #[test]
    fn test_backend_config_parse_invalid_backend() {
        let err = BackendConfig::parse("unknown").unwrap_err();
        assert!(err.message.contains("unknown backend"));
    }

    #[test]
    fn test_backend_config_parse_invalid_transport() {
        let err = BackendConfig::parse("cursor:invalid").unwrap_err();
        assert!(err.message.contains("unknown transport"));
        assert!(err.input == "cursor:invalid");
    }

    #[test]
    fn test_backend_config_parse_unsupported_transport() {
        // Auggie doesn't support CLI
        let err = BackendConfig::parse("auggie:cli").unwrap_err();
        assert!(err.message.contains("not supported"));
        assert!(err.input == "auggie:cli");
    }

    #[test]
    fn test_backend_config_effective_transport() {
        // Explicit transport
        let config = BackendConfig::parse("cursor:acp").unwrap();
        assert_eq!(config.effective_transport(), TransportKind::Acp);

        // Default transport for Cursor (CLI)
        let config = BackendConfig::parse("cursor").unwrap();
        assert_eq!(config.effective_transport(), TransportKind::Cli);

        // Default transport for Auggie (ACP)
        let config = BackendConfig::parse("auggie").unwrap();
        assert_eq!(config.effective_transport(), TransportKind::Acp);
    }

    #[test]
    fn test_backend_config_display() {
        let config = BackendConfig::new(BackendKind::Cursor);
        assert_eq!(format!("{}", config), "cursor");

        let config = BackendConfig::with_transport(BackendKind::Cursor, TransportKind::Cli);
        assert_eq!(format!("{}", config), "cursor:cli");

        let config = BackendConfig::with_transport(BackendKind::Auggie, TransportKind::Acp);
        assert_eq!(format!("{}", config), "auggie:acp");
    }

    #[test]
    fn test_backend_kind_parse_with_transport() {
        // Using the convenience method on BackendKind
        let (kind, transport) = BackendKind::parse_with_transport("cursor:cli").unwrap();
        assert_eq!(kind, BackendKind::Cursor);
        assert_eq!(transport, Some(TransportKind::Cli));

        let (kind, transport) = BackendKind::parse_with_transport("auggie").unwrap();
        assert_eq!(kind, BackendKind::Auggie);
        assert_eq!(transport, None);
    }
}
