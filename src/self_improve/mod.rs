//! Self-improvement feature for Paperboat.
//!
//! This module provides functionality for paperboat to analyze its own runs
//! and make incremental improvements when running in its own repository.
//!
//! # Overview
//!
//! The self-improvement feature allows paperboat to:
//! - Detect when it's running in its own repository
//! - Analyze completed run logs for patterns and issues
//! - Make small, safe code improvements
//!
//! # Usage
//!
//! The main entry point is [`maybe_run_self_improvement`], which should be called
//! from `main.rs` after a successful run completes:
//!
//! ```ignore
//! // After app.shutdown().await?;
//! if result.success {
//!     if let Err(e) = self_improve::maybe_run_self_improvement(&run_dir, &result, &task_manager).await {
//!         tracing::warn!("Self-improvement failed (non-fatal): {}", e);
//!     }
//! }
//! ```
//!
//! # Components
//!
//! - [`detection`]: Repository detection utilities
//! - [`config`]: Configuration loading for self-improvement feature
//! - [`context_builder`]: Builds rich context for self-improvement agent
//! - [`runner`]: Agent spawning and execution

pub mod config;
pub mod context_builder;
pub mod detection;
pub mod runner;

// Re-export main types for convenience
pub use config::is_self_improvement_enabled;
pub use runner::maybe_run_self_improvement;
