//! Nested logging system for agent hierarchy.
//!
//! This module provides a hierarchical logging system that mirrors the agent hierarchy:
//! Planner → Orchestrator → Implementer/nested decomposition.
//!
//! # Structure
//!
//! Each app run creates its own timestamped folder:
//! ```text
//! logs/
//! └── 2026-03-05_14-32-15_abc123/
//!     ├── planner.log
//!     ├── orchestrator.log
//!     ├── implementer-001.log
//!     └── subtask-001/
//!         ├── planner.log
//!         ├── orchestrator.log
//!         └── implementer-001.log
//! ```
//!
//! # Components
//!
//! - [`RunLogManager`] - Manages the run directory and provides factory methods
//! - [`LogScope`] - Represents a logging context at a specific hierarchy level
//! - [`AgentWriter`] - Writes to individual log files with streaming support
//! - [`LogEvent`] - Events broadcast for UI streaming and observation

mod manager;
mod scope;
mod stream;
mod writer;

pub use manager::RunLogManager;
pub use scope::LogScope;
pub use stream::LogEvent;
pub use writer::{AgentType, AgentWriter};

