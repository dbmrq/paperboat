# Backend Abstraction Implementation Plan

## Overview

This plan abstracts the ACP client layer in paperboat to support multiple agent backends (Auggie, Cursor, etc.) while maintaining backward compatibility.

### Goals
- Define a `Backend` trait for different agent providers
- Abstract authentication, model discovery, and cache management per backend
- Make backend selection configurable via env vars, config files, and CLI flags
- Keep existing `AcpClientTrait` as the unified interface
- Maintain full backward compatibility (default to Auggie)

### Success Criteria
- Existing functionality works unchanged with Auggie backend
- Adding a new backend requires only implementing the `Backend` trait
- Configuration supports env var (`PAPERBOAT_BACKEND`), config file, and CLI flag
- All existing tests pass without modification

---

## 1. Prerequisites

- No new external dependencies required
- No database migrations needed
- Configuration format changes are additive (backward compatible)

---

## 1.5 Backend Comparison: Auggie vs Cursor

| Aspect | Auggie | Cursor |
|--------|--------|--------|
| **Binary name** | `auggie` | `agent` |
| **ACP mode** | `auggie --acp` | `agent acp` |
| **Auth check file** | `~/.augment/session.json` | N/A (uses API key or login) |
| **Login command** | `auggie login` | `agent login` |
| **Auth env vars** | N/A | `CURSOR_API_KEY`, `CURSOR_AUTH_TOKEN` |
| **Config directory** | `~/.augment/` | `~/.cursor/` |
| **Model list command** | `auggie model list` | `/model` slash command (no CLI discovery?) |
| **Cache directory** | `~/.paperboat/augment-*` | `~/.paperboat/cursor-*` (proposed) |
| **Tool removal** | `settings.json` with `removedTools` | `cli-config.json` with `permissions.deny` |
| **ACP authenticate step** | Not required | `authenticate` with `methodId: "cursor_login"` |

### Key Protocol Differences

**Cursor requires an explicit `authenticate` call after `initialize`:**
```json
{"jsonrpc":"2.0","id":2,"method":"authenticate","params":{"methodId":"cursor_login"}}
```

**Cursor uses `permissions.allow`/`permissions.deny` in config:**
```json
{
  "version": 1,
  "permissions": {
    "allow": ["Shell(ls)", "Shell(echo)"],
    "deny": ["Shell(rm)"]
  }
}
```

This is different from Auggie's `removedTools` array in `settings.json`.

---

## 2. New Module Structure

```
src/
├── backend/                     # NEW: Backend abstraction layer
│   ├── mod.rs                   # Module exports, BackendKind enum
│   ├── trait.rs                 # Backend trait definition
│   ├── auggie/                  # Auggie backend implementation
│   │   ├── mod.rs               # Auggie backend struct
│   │   ├── auth.rs              # Authentication checking
│   │   ├── models.rs            # Model discovery (moved from src/models.rs)
│   │   └── cache.rs             # Cache directory management
│   └── cursor/                  # Cursor CLI backend
│       ├── mod.rs               # Cursor backend struct
│       ├── auth.rs              # API key / token authentication
│       ├── acp.rs               # CursorAcpClient with authenticate step
│       └── cache.rs             # Permissions-based config management
├── acp.rs                       # MODIFIED: Use Backend trait for spawning
├── app/
│   ├── mod.rs                   # MODIFIED: Use Backend for cache setup
│   └── types.rs                 # MODIFIED: Backend-agnostic cache paths
├── config/
│   └── loader.rs                # MODIFIED: Load backend config
├── error/
│   └── acp.rs                   # MODIFIED: Backend-agnostic errors
├── models.rs                    # MODIFIED: Thin wrapper, delegates to backend
└── main.rs                      # MODIFIED: Backend selection from CLI/env
```

---

## 3. Implementation Steps

### Step 1: Create Backend Trait (`src/backend/trait.rs`)

Define the core abstraction that all backends must implement.

```rust
use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use crate::acp::AcpClientTrait;
use crate::models::AvailableModel;

/// Trait defining a backend provider for agent communication.
#[async_trait::async_trait]
pub trait Backend: Send + Sync {
    /// Human-readable name of the backend (e.g., "auggie", "claude-code")
    fn name(&self) -> &'static str;
    
    /// Check if the backend is authenticated and ready to use.
    /// Returns an error with a helpful message if not authenticated.
    fn check_auth(&self) -> Result<()>;
    
    /// Discover available models from this backend.
    async fn discover_models(&self) -> Result<Vec<AvailableModel>>;
    
    /// Create an ACP client for the given cache directory and timeout.
    async fn create_client(
        &self,
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Box<dyn AcpClientTrait + Send>>;
    
    /// Set up a cache directory with agent-specific settings.
    /// Returns the path to the created cache directory.
    fn setup_agent_cache(
        &self,
        agent_type: AgentCacheType,
        removed_tools: &[&str],
    ) -> Result<PathBuf>;
    
    /// Get the login command hint for authentication errors.
    /// Example: "auggie login" or "agent login"
    fn login_hint(&self) -> &'static str;

    /// Get a full authentication error message for this backend.
    /// Includes the login hint and any alternative auth methods.
    fn auth_error_message(&self) -> String {
        format!(
            "Please run '{}' first to authenticate, then try again.",
            self.login_hint()
        )
    }
}

/// Type of agent cache to set up.
#[derive(Debug, Clone, Copy)]
pub enum AgentCacheType {
    Orchestrator,
    Planner,
    Worker,
}
```

**Files to create:** `src/backend/trait.rs`

**Key details:**
- Uses `async_trait` for async methods
- Returns `Box<dyn AcpClientTrait>` for polymorphism
- `AgentCacheType` enum for cache directory types

---

### Step 2: Create Backend Module Root (`src/backend/mod.rs`)

Define the `BackendKind` enum and module exports.

```rust
pub mod trait;
pub mod auggie;
// pub mod claude_code;  // Uncomment when implementing

pub use self::trait::{Backend, AgentCacheType};

use anyhow::{anyhow, Result};
use std::str::FromStr;

/// Available backend types.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackendKind {
    /// Augment CLI (auggie) - default backend
    #[default]
    Auggie,
    /// Cursor CLI
    Cursor,
}

impl BackendKind {
    /// Get the backend instance for this kind.
    pub fn create(&self) -> Box<dyn Backend> {
        match self {
            Self::Auggie => Box::new(auggie::AuggieBackend::new()),
            Self::Cursor => Box::new(cursor::CursorBackend::new()),
        }
    }
}

impl FromStr for BackendKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "auggie" | "augment" => Ok(Self::Auggie),
            "cursor" => Ok(Self::Cursor),
            _ => Err(anyhow!("Unknown backend: {}. Available: auggie, cursor", s)),
        }
    }
}
```

**Files to create:** `src/backend/mod.rs`

---

### Step 3: Implement Auggie Backend (`src/backend/auggie/mod.rs`)

Move existing Auggie-specific logic into a dedicated module.

```rust
mod auth;
mod cache;
mod models;

use crate::acp::{AcpClient, AcpClientTrait};
use crate::backend::{AgentCacheType, Backend};
use crate::models::AvailableModel;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;

pub struct AuggieBackend;

impl AuggieBackend {
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Backend for AuggieBackend {
    fn name(&self) -> &'static str {
        "auggie"
    }

    fn check_auth(&self) -> Result<()> {
        auth::check_auggie_auth()
    }

    async fn discover_models(&self) -> Result<Vec<AvailableModel>> {
        models::discover_auggie_models().await
    }

    async fn create_client(
        &self,
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Box<dyn AcpClientTrait + Send>> {
        let client = AcpClient::spawn_with_timeout(cache_dir, request_timeout).await?;
        Ok(Box::new(client))
    }

    fn setup_agent_cache(
        &self,
        agent_type: AgentCacheType,
        removed_tools: &[&str],
    ) -> Result<PathBuf> {
        cache::setup_auggie_cache(agent_type, removed_tools)
    }

    fn login_hint(&self) -> &'static str {
        "auggie login"
    }
}
```

**Files to create:**
- `src/backend/auggie/mod.rs`
- `src/backend/auggie/auth.rs` (extract from `src/app/mod.rs`)
- `src/backend/auggie/cache.rs` (extract from `src/app/mod.rs`)
- `src/backend/auggie/models.rs` (extract from `src/models.rs`)

---

### Step 4: Extract Auth Logic (`src/backend/auggie/auth.rs`)

Move authentication checking from `App::setup_orchestrator_cache()`.

```rust
use anyhow::{bail, Result};
use std::path::Path;

/// Check if Auggie CLI is authenticated.
pub fn check_auggie_auth() -> Result<()> {
    let main_augment_dir = shellexpand::tilde("~/.augment").to_string();
    let main_session = Path::new(&main_augment_dir).join("session.json");

    if !main_session.exists() {
        bail!(
            "Augment CLI is not authenticated.\n\n\
            Please run 'auggie login' first to authenticate, then try again."
        );
    }

    Ok(())
}

/// Get the path to the main Augment session file.
pub fn session_file_path() -> std::path::PathBuf {
    let main_augment_dir = shellexpand::tilde("~/.augment").to_string();
    Path::new(&main_augment_dir).join("session.json")
}
```

**Files to create:** `src/backend/auggie/auth.rs`

---

### Step 5: Extract Cache Logic (`src/backend/auggie/cache.rs`)

Move cache directory setup from `App::setup_orchestrator_cache()` and `App::setup_planner_cache()`.

```rust
use crate::backend::AgentCacheType;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

use super::auth;

/// Base path for paperboat's Auggie cache directories.
const CACHE_BASE: &str = "~/.paperboat";

/// Set up an Auggie cache directory for a specific agent type.
pub fn setup_auggie_cache(
    agent_type: AgentCacheType,
    removed_tools: &[&str],
) -> Result<PathBuf> {
    // Check auth first
    auth::check_auggie_auth()?;

    let cache_name = match agent_type {
        AgentCacheType::Orchestrator => "augment-orchestrator",
        AgentCacheType::Planner => "augment-planner",
        AgentCacheType::Worker => return Ok(PathBuf::new()), // Workers use default
    };

    let cache_dir = shellexpand::tilde(&format!("{}/{}", CACHE_BASE, cache_name)).to_string();
    let cache_path = Path::new(&cache_dir);

    // Create directory if needed
    if !cache_path.exists() {
        std::fs::create_dir_all(cache_path)
            .with_context(|| format!("Failed to create cache directory: {}", cache_dir))?;
    }

    // Copy session.json from main augment directory
    let session_dest = cache_path.join("session.json");
    if !session_dest.exists() {
        std::fs::copy(auth::session_file_path(), &session_dest)
            .context("Failed to copy session.json to cache")?;
    }

    // Write settings.json with removed tools
    let settings = json!({ "removedTools": removed_tools });
    let settings_path = cache_path.join("settings.json");
    std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
        .context("Failed to write settings.json")?;

    Ok(cache_path.to_path_buf())
}
```

**Files to create:** `src/backend/auggie/cache.rs`

---

### Step 6: Extract Model Discovery (`src/backend/auggie/models.rs`)

Move model discovery from `src/models.rs`.

```rust
use crate::models::{parse_model_list, AvailableModel};
use anyhow::{anyhow, Result};
use tokio::process::Command;

/// Discover available models by running `auggie model list`.
pub async fn discover_auggie_models() -> Result<Vec<AvailableModel>> {
    let output = Command::new("auggie")
        .args(["model", "list"])
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run 'auggie model list': {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "auggie model list failed with status {}: {}",
            output.status,
            stderr
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_model_list(&stdout)
}
```

**Files to create:** `src/backend/auggie/models.rs`

**Note:** `parse_model_list` stays in `src/models.rs` as a shared utility (it's format-agnostic).

---

### Step 6.5: Implement Cursor Backend (`src/backend/cursor/`)

The Cursor backend requires several key differences from Auggie:

#### `src/backend/cursor/mod.rs`

```rust
mod auth;
mod acp;
mod cache;

use crate::acp::AcpClientTrait;
use crate::backend::{AgentCacheType, Backend};
use crate::models::AvailableModel;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;

pub struct CursorBackend;

impl CursorBackend {
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Backend for CursorBackend {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn check_auth(&self) -> Result<()> {
        auth::check_cursor_auth()
    }

    async fn discover_models(&self) -> Result<Vec<AvailableModel>> {
        // Cursor doesn't have `agent model list` - use hardcoded defaults
        // or parse from config. For now, return supported models.
        Ok(vec![
            AvailableModel { name: "claude-sonnet".to_string(), is_default: true },
            AvailableModel { name: "gpt-4o".to_string(), is_default: false },
            AvailableModel { name: "claude-opus".to_string(), is_default: false },
        ])
    }

    async fn create_client(
        &self,
        cache_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Box<dyn AcpClientTrait + Send>> {
        // Use CursorAcpClient which handles the authenticate step
        let client = acp::CursorAcpClient::spawn_with_timeout(cache_dir, request_timeout).await?;
        Ok(Box::new(client))
    }

    fn setup_agent_cache(
        &self,
        agent_type: AgentCacheType,
        denied_tools: &[&str],
    ) -> Result<PathBuf> {
        cache::setup_cursor_cache(agent_type, denied_tools)
    }

    fn login_hint(&self) -> &'static str {
        "agent login"
    }

    fn auth_error_message(&self) -> String {
        "Cursor CLI is not authenticated.\n\n\
        Please run 'agent login' first to authenticate, then try again.\n\
        Alternatively, set CURSOR_API_KEY or CURSOR_AUTH_TOKEN environment variable.".to_string()
    }
}
```

#### `src/backend/cursor/auth.rs`

```rust
use anyhow::{bail, Result};
use std::env;
use std::path::Path;

/// Check if Cursor CLI is authenticated.
///
/// Authentication can be via:
/// 1. `CURSOR_API_KEY` env var
/// 2. `CURSOR_AUTH_TOKEN` env var
/// 3. Interactive login (`agent login` - stores credentials in ~/.cursor/)
pub fn check_cursor_auth() -> Result<()> {
    // Check for API key or auth token in environment
    if env::var("CURSOR_API_KEY").is_ok() || env::var("CURSOR_AUTH_TOKEN").is_ok() {
        return Ok(());
    }

    // Check for existing cursor config directory
    // `agent login` stores auth in ~/.cursor/ (exact file TBD - may need to test)
    let cursor_dir = shellexpand::tilde("~/.cursor").to_string();
    let cursor_path = Path::new(&cursor_dir);

    // Check if the config directory exists with some auth indicator
    // TODO: Determine exact file `agent login` creates (credentials.json? session.json?)
    // For now, check if config exists as a proxy for "user has set up cursor"
    let config_file = cursor_path.join("cli-config.json");

    if !cursor_path.exists() {
        bail!(
            "Cursor CLI is not configured.\n\n\
            Please run 'agent login' first to authenticate, then try again.\n\
            Alternatively, set CURSOR_API_KEY environment variable."
        );
    }

    // Config dir exists - assume auth will be handled by `authenticate` JSON-RPC call
    // The ACP client will send `authenticate` with `methodId: cursor_login` after initialize
    Ok(())
}
```

**Note:** Unlike Auggie's clear `session.json` file, Cursor's auth storage location isn't explicitly documented. The `authenticate` JSON-RPC call handles the actual auth flow. We do a basic sanity check here and let the ACP client handle the rest.

#### `src/backend/cursor/acp.rs`

This is the key difference - Cursor requires an `authenticate` call after `initialize`:

```rust
use crate::acp::AcpClientTrait;
use crate::error::AcpError;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub struct CursorAcpClient {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    request_id: i64,
    request_timeout: Duration,
}

impl CursorAcpClient {
    pub async fn spawn_with_timeout(
        config_dir: Option<&str>,
        request_timeout: Duration,
    ) -> Result<Self> {
        let mut cmd = Command::new("agent");
        cmd.arg("acp");

        // Cursor uses --config-dir for per-agent configuration
        if let Some(dir) = config_dir {
            cmd.arg("--config-dir").arg(dir);
        }

        cmd.stdin(std::process::Stdio::piped())
           .stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());

        Ok(Self {
            child,
            stdin,
            stdout,
            request_id: 0,
            request_timeout,
        })
    }

    /// Send authenticate request (Cursor-specific).
    /// Must be called after initialize().
    async fn authenticate(&mut self) -> Result<Value, AcpError> {
        self.send_request("authenticate", json!({
            "methodId": "cursor_login"
        })).await
    }
}

#[async_trait]
impl AcpClientTrait for CursorAcpClient {
    async fn initialize(&mut self) -> Result<Value, AcpError> {
        // Standard initialize
        let result = self.send_request("initialize", json!({
            "capabilities": {},
            "clientInfo": {
                "name": "paperboat",
                "version": env!("CARGO_PKG_VERSION")
            }
        })).await?;

        // Cursor requires authenticate after initialize
        self.authenticate().await?;

        Ok(result)
    }

    // ... implement other AcpClientTrait methods similar to AcpClient ...
}
```

#### `src/backend/cursor/cache.rs`

Cursor uses `cli-config.json` with `permissions.deny` instead of `settings.json`:

```rust
use crate::backend::AgentCacheType;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

const CACHE_BASE: &str = "~/.paperboat";

pub fn setup_cursor_cache(
    agent_type: AgentCacheType,
    denied_tools: &[&str],
) -> Result<PathBuf> {
    let cache_name = match agent_type {
        AgentCacheType::Orchestrator => "cursor-orchestrator",
        AgentCacheType::Planner => "cursor-planner",
        AgentCacheType::Worker => return Ok(PathBuf::new()),
    };

    let cache_dir = shellexpand::tilde(&format!("{}/{}", CACHE_BASE, cache_name)).to_string();
    let cache_path = Path::new(&cache_dir);

    if !cache_path.exists() {
        std::fs::create_dir_all(cache_path)
            .with_context(|| format!("Failed to create cache directory: {}", cache_dir))?;
    }

    // Write cli-config.json with permissions (Cursor format)
    let config = json!({
        "version": 1,
        "permissions": {
            "allow": [],
            "deny": denied_tools
        }
    });

    let config_path = cache_path.join("cli-config.json");
    std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)
        .context("Failed to write cli-config.json")?;

    Ok(cache_path.to_path_buf())
}
```

**Files to create:**
- `src/backend/cursor/mod.rs`
- `src/backend/cursor/auth.rs`
- `src/backend/cursor/acp.rs`
- `src/backend/cursor/cache.rs`

---

### Step 7: Update `src/models.rs`

Make `discover_models` backend-agnostic by delegating to the backend.

**Changes:**
1. Make `parse_model_list` public (used by auggie backend)
2. Keep `discover_models` but deprecate it (calls auggie backend for compatibility)
3. Add `discover_models_with_backend` that takes a backend reference

```rust
// In src/models.rs

/// Parses the output of model list commands into AvailableModel structs.
/// This format is shared across backends that use similar CLI output.
pub fn parse_model_list(output: &str) -> Result<Vec<AvailableModel>> {
    // ... existing implementation unchanged
}

/// Discovers available models using the default backend (Auggie).
///
/// DEPRECATED: Use `backend.discover_models()` directly for new code.
pub async fn discover_models() -> Result<Vec<AvailableModel>> {
    crate::backend::auggie::models::discover_auggie_models().await
}
```

**Files to modify:** `src/models.rs`

---

### Step 8: Update `src/app/mod.rs`

Refactor `App` to use the backend abstraction.

**Changes:**
1. Add `backend: Box<dyn Backend>` field to `App`
2. Replace `setup_orchestrator_cache` and `setup_planner_cache` with backend calls
3. Update constructors to accept/create backend

```rust
// In App struct
pub struct App {
    // ... existing fields ...
    /// The backend used for agent communication
    backend: Box<dyn Backend>,
}

impl App {
    pub async fn with_log_manager_and_timeout(
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        timeout_config: TimeoutConfig,
        backend: Box<dyn Backend>,  // NEW parameter
    ) -> Result<Self> {
        // Check backend authentication first
        backend.check_auth()?;

        // Set up cache directories via backend
        let orchestrator_cache = backend.setup_agent_cache(
            AgentCacheType::Orchestrator,
            ORCHESTRATOR_CONFIG.removed_auggie_tools,
        )?;

        let planner_cache = backend.setup_agent_cache(
            AgentCacheType::Planner,
            PLANNER_CONFIG.removed_auggie_tools,
        )?;

        // Create ACP clients via backend
        let mut acp_orchestrator = backend.create_client(
            Some(orchestrator_cache.to_str().unwrap()),
            timeout_config.request_timeout,
        ).await?;
        acp_orchestrator.initialize().await?;

        // ... similar for planner and worker ...

        Ok(Self {
            // ... existing fields ...
            backend,
        })
    }
}
```

**Files to modify:** `src/app/mod.rs`

---

### Step 9: Update `src/app/types.rs`

Make cache paths backend-agnostic (move constants to backend module).

**Changes:**
1. Remove hardcoded `ORCHESTRATOR_CACHE_DIR` and `PLANNER_CACHE_DIR`
2. Add comment pointing to backend module for cache configuration

```rust
// Remove these constants (they move to src/backend/auggie/cache.rs):
// pub const ORCHESTRATOR_CACHE_DIR: &str = "~/.paperboat/augment-orchestrator";
// pub const PLANNER_CACHE_DIR: &str = "~/.paperboat/augment-planner";

// These paths are now managed by the Backend trait implementation.
// See src/backend/auggie/cache.rs for Auggie-specific paths.
```

**Files to modify:** `src/app/types.rs`

---

### Step 10: Update `src/error/acp.rs`

Make error messages backend-agnostic.

**Changes:**
1. Remove hardcoded "auggie" references
2. Add backend name to error messages where relevant

```rust
// Before:
#[error("Is auggie installed and in your PATH?")]
InstallAuggie,

#[error("Try running 'auggie login' first to authenticate")]
Authenticate,

// After:
#[error("Is the agent CLI installed and in your PATH? ({0})")]
InstallCli(String),  // Backend name for context

#[error("{0}")]
AuthRequired(String),  // Backend-specific auth message

#[error("{0}")]
Custom(String),  // For other backend-specific errors
```

**Usage in App:**
```rust
// When auth fails, use the backend's error message
if let Err(e) = backend.check_auth() {
    return Err(AcpError::AuthRequired(backend.auth_error_message()).into());
}
```

**Files to modify:** `src/error/acp.rs`

---

### Step 11: Add Configuration Support (`src/config/loader.rs`)

Add backend configuration loading.

**Changes:**
1. Add `backend` field to `AgentFileConfig` or create separate `BackendConfig`
2. Support `PAPERBOAT_BACKEND` environment variable
3. Support `backend = "auggie"` in config files

```rust
// In src/config/loader.rs

/// Configuration for the backend provider
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BackendConfig {
    /// Backend to use (e.g., "auggie", "claude-code")
    pub backend: Option<String>,
}

/// Load backend configuration from environment and config files.
pub fn load_backend_kind() -> BackendKind {
    // Priority: env var > project config > user config > default
    if let Ok(backend_str) = std::env::var("PAPERBOAT_BACKEND") {
        if let Ok(kind) = BackendKind::from_str(&backend_str) {
            return kind;
        }
        tracing::warn!("Invalid PAPERBOAT_BACKEND '{}', using default", backend_str);
    }

    // Try loading from config files (similar to agent config loading)
    // ... implementation details ...

    BackendKind::default() // Auggie
}
```

**Files to modify:** `src/config/loader.rs`

---

### Step 12: Update `src/main.rs`

Add CLI flag and use backend abstraction.

**Changes:**
1. Add `--backend <name>` CLI argument
2. Load backend from config/env/CLI
3. Pass backend to `App::with_log_manager_and_timeout`

```rust
// In parse_args:
struct CliArgs {
    // ... existing fields ...
    /// Backend to use (auggie, cursor)
    backend: Option<String>,
}

// In main():
// Load backend configuration
let backend_kind = cli_args.backend
    .map(|s| BackendKind::from_str(&s))
    .transpose()?
    .unwrap_or_else(load_backend_kind);

let backend = backend_kind.create();

// Discover models using the backend
let available_models = backend.discover_models().await?;

// Create app with backend
let mut app = App::with_log_manager_and_timeout(
    model_config,
    log_manager,
    TimeoutConfig::default(),
    backend,
).await?;
```

**Files to modify:** `src/main.rs`

---

## 4. File Changes Summary

### Files to Create
| File | Purpose |
|------|---------|
| `src/backend/mod.rs` | Module root, `BackendKind` enum |
| `src/backend/trait.rs` | `Backend` trait definition |
| `src/backend/auggie/mod.rs` | Auggie backend implementation |
| `src/backend/auggie/auth.rs` | Auggie authentication checking |
| `src/backend/auggie/cache.rs` | Auggie cache directory management |
| `src/backend/auggie/models.rs` | Auggie model discovery |
| `src/backend/cursor/mod.rs` | Cursor backend implementation |
| `src/backend/cursor/auth.rs` | Cursor API key/token authentication |
| `src/backend/cursor/acp.rs` | CursorAcpClient with authenticate step |
| `src/backend/cursor/cache.rs` | Cursor permissions-based config |

### Files to Modify
| File | Changes |
|------|---------|
| `src/main.rs` | Add `--backend` flag, use backend abstraction |
| `src/app/mod.rs` | Add backend field, delegate cache setup |
| `src/app/types.rs` | Remove hardcoded cache paths |
| `src/models.rs` | Make `parse_model_list` public, deprecate `discover_models` |
| `src/error/acp.rs` | Remove "auggie" references |
| `src/config/loader.rs` | Add backend config loading |

### Files Unchanged
- `src/acp.rs` - `AcpClient` stays Auggie-specific (moved to auggie backend)
- `src/testing/mock_acp.rs` - Mock remains backend-agnostic

---

## 5. Testing Strategy

### Unit Tests

1. **Backend trait tests** (`src/backend/mod.rs`)
   - Test `BackendKind::from_str` parsing
   - Test default backend is Auggie

2. **Auggie backend tests** (`src/backend/auggie/mod.rs`)
   - Test auth checking with mock filesystem
   - Test cache directory creation
   - Test model list parsing (existing tests in `src/models.rs`)

3. **Configuration tests** (`src/config/loader.rs`)
   - Test env var `PAPERBOAT_BACKEND` loading
   - Test config file backend setting
   - Test priority: CLI > env > config > default

### Integration Tests

1. **Backend selection tests**
   - Verify correct backend is used based on config
   - Verify error messages mention correct backend

2. **Backward compatibility tests**
   - Run existing E2E tests without any backend config
   - Verify default Auggie behavior unchanged

### Manual Testing Steps

1. Run without any config → should use Auggie
2. Set `PAPERBOAT_BACKEND=auggie` → should work
3. Set `PAPERBOAT_BACKEND=cursor` → should use Cursor CLI
4. Set `PAPERBOAT_BACKEND=invalid` → should warn and use default
5. Pass `--backend auggie` → should work
6. Pass `--backend cursor` → should work
7. Test Cursor with `CURSOR_API_KEY` env var
8. Test Cursor with interactive login flow

---

## 6. Rollback Plan

If issues arise:

1. **Config rollback**: Remove `backend` field from config files
2. **Code rollback**:
   - Revert `src/app/mod.rs` to use direct `AcpClient` calls
   - Revert `src/main.rs` to remove backend logic
   - Keep `src/backend/` module for future use (no harm)
3. **No data migration needed** - this is code-only

---

## 7. Estimated Effort

| Component | Effort | Complexity |
|-----------|--------|------------|
| Backend trait and module structure | 2 hours | Low |
| Auggie backend implementation | 2 hours | Low |
| Extract and refactor existing code | 3 hours | Medium |
| Update App and main.rs | 2 hours | Medium |
| Configuration support | 1 hour | Low |
| Testing | 2 hours | Medium |
| **Total** | **~12 hours** | **Medium** |

---

## 8. Future Extensions

Once this abstraction is in place, adding new backends requires:

1. Create `src/backend/new_backend/mod.rs`
2. Implement the `Backend` trait
3. Add variant to `BackendKind` enum
4. Update `BackendKind::from_str` to recognize the new backend

No changes needed to:
- `AcpClientTrait` (remains the unified interface)
- `App` logic (works with any backend)
- Existing tests (backend-agnostic)

### Claude Code Backend (Future)

```rust
// src/backend/claude_code/mod.rs
pub struct ClaudeCodeBackend;

#[async_trait]
impl Backend for ClaudeCodeBackend {
    fn name(&self) -> &'static str { "claude-code" }

    fn check_auth(&self) -> Result<()> {
        // Check for ~/.claude/session or similar
    }

    async fn discover_models(&self) -> Result<Vec<AvailableModel>> {
        // Run `claude model list` or equivalent
    }

    async fn create_client(&self, ...) -> Result<Box<dyn AcpClientTrait + Send>> {
        // Spawn claude-code CLI with ACP mode
    }

    fn login_hint(&self) -> &'static str {
        "claude login"
    }
}
```

---

## 9. Open Questions

1. **Should `AcpClient` be made generic?**
   - Current plan: Keep separate `AcpClient` and `CursorAcpClient`, backends create them
   - Alternative: Make a single generic `AcpClient<T: CommandBuilder>`

2. **Cache directory strategy?**
   - Plan: Share `~/.paperboat/` with backend-prefixed subdirs (`augment-*`, `cursor-*`)
   - This allows multiple backends to coexist

3. **Model ID compatibility?**
   - Different backends have different model names (e.g., `claude-sonnet` vs `claude-3-5-sonnet`)
   - May need a model alias mapping layer in the future

4. **Cursor model discovery?**
   - Cursor doesn't seem to have `agent model list` CLI command
   - Options: hardcode known models, or parse from config

5. **Tool name mapping?**
   - Auggie uses `removedTools` with specific tool names
   - Cursor uses `permissions.deny` with glob patterns like `Shell(*)`
   - Need to translate between them

---

## Summary

This plan provides a clean abstraction layer for supporting multiple agent backends while:
- Maintaining full backward compatibility with Auggie (default)
- Adding Cursor CLI as a first-class alternative
- Keeping the existing `AcpClientTrait` as the unified interface
- Handling Cursor's unique requirements (authenticate step, permissions config)
- Enabling easy addition of more backends in the future
- Following the existing codebase patterns and conventions

### Key Cursor-Specific Items

1. **Binary**: `agent` instead of `auggie`
2. **ACP command**: `agent acp` instead of `auggie --acp`
3. **Authentication**: Requires explicit `authenticate` JSON-RPC call after `initialize`
4. **Auth methods**: `CURSOR_API_KEY`, `CURSOR_AUTH_TOKEN`, or interactive `agent login`
5. **Config file**: `cli-config.json` with `permissions.allow/deny` instead of `settings.json` with `removedTools`

