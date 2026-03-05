//! Configuration module for Villalobos
//!
//! This module provides configuration management functionality including
//! model resolution and configuration file handling.

pub mod loader;
pub mod resolver;

pub use loader::{build_model_config, load_agent_configs, LoadedAgentConfigs};
pub use resolver::{parse_model_string, resolve_model};

