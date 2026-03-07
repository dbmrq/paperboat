//! Configuration module for Paperboat.
//!
//! This module provides configuration management for agent model assignments.
//! Configuration is stored in TOML files and supports a two-tier hierarchy:
//!
//! - **User-level** (`~/.paperboat/agents/`): Personal defaults
//! - **Project-level** (`.paperboat/agents/`): Project-specific overrides
//!
//! # Submodules
//!
//! - [`loader`]: Loads and merges configuration from TOML files
//! - [`resolver`]: Resolves model aliases (e.g., "opus") to concrete model IDs
//! - [`writer`]: Persists configuration changes to disk
//!
//! # Example
//!
//! ```ignore
//! use paperboat::config::{load_agent_configs, build_model_config, save_agent_config};
//!
//! // Load configuration at startup
//! let loaded = load_agent_configs()?;
//! let model_config = build_model_config(&loaded, &available_models)?;
//!
//! // Save changes from the settings UI
//! save_agent_config("orchestrator", &ModelId::Opus4_5)?;
//! ```

pub mod loader;
pub mod resolver;
pub mod writer;

pub use loader::{build_model_config, load_agent_configs};
#[cfg_attr(not(feature = "tui"), allow(unused_imports))]
pub use writer::save_agent_config;
