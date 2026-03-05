//! Configuration module for Villalobos
//!
//! This module provides configuration management functionality including
//! model resolution and configuration file handling.

pub mod resolver;

pub use resolver::{parse_model_string, resolve_model};

