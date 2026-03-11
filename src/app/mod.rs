//! Main orchestrator application module.
//!
//! This module contains the core [`App`] struct and orchestration logic, split into
//! focused sub-modules for maintainability:
//!
//! # Module Structure
//!
//! ## Core Types & Configuration
//! - [`types`]: Constants, type aliases, and utility functions
//! - [`router`]: Session routing for ACP messages to per-session channels
//!
//! ## Agent Lifecycle
//! - [`agent_spawner`]: Generic agent spawning with concurrent execution support
//! - [`agent_handler`]: Agent tool call handling and completion processing
//! - [`context_generator`]: Task context generation for implementer agents
//!
//! ## Agent Types
//! - [`planner`]: Planner agent spawning for task decomposition
//! - [`orchestrator`]: Orchestrator agent spawning and execution
//! - [`orchestrator_acp`]: ACP message handling for orchestrator
//! - [`decompose`]: Decomposition logic for subtasks
//!
//! ## Communication
//! - [`socket`]: IPC socket setup and MCP connection handling
//! - [`session`]: Session waiting and output collection
//! - [`session_drain`]: Message draining after session completion
//!
//! ## Entry Point
//! - [`run`]: Main execution entry point for the orchestrator

mod agent_handler;
mod agent_session_handler;
mod agent_spawner;
mod concurrent_spawner;
mod context_generator;
mod decompose;
mod orchestrator;
mod orchestrator_acp;
mod planner;
pub mod retry;
pub mod router;
mod run;
mod sequential_impl;
mod session;
mod session_drain;
mod socket;
mod spawn_config;
mod tool_filtering;
pub mod types;

pub use types::ToolMessage;

use crate::agents::{AgentRegistry, ORCHESTRATOR_CONFIG, PLANNER_CONFIG};
use crate::backend::transport::{AgentTransport, AgentType, TransportKind};
use crate::backend::{AgentCacheType, Backend, TransportConfig};
use crate::error::TimeoutConfig;
use crate::ipc::IpcAddress;
use crate::logging::{LogScope, RunLogManager};
use crate::models::ModelConfig;
use crate::tasks::TaskManager;
use anyhow::Result;
use router::SessionRouter;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// The main application struct that orchestrates multi-agent workflows.
pub struct App {
    /// The backend provider for agent communication (Auggie, Cursor, etc.)
    #[allow(dead_code)]
    backend: Box<dyn Backend>,
    /// Transport for the orchestrator agent (coordinates task execution)
    pub(crate) acp_orchestrator: Box<dyn AgentTransport>,
    /// Transport for the planner agent (decomposes tasks into subtasks)
    pub(crate) acp_planner: Box<dyn AgentTransport>,
    /// Transport for worker agents (implements tasks)
    pub(crate) acp_worker: Box<dyn AgentTransport>,
    pub(crate) socket_address: Option<IpcAddress>,
    /// Channel for receiving tool messages from socket handlers
    pub(crate) tool_rx: Option<mpsc::Receiver<ToolMessage>>,
    pub(crate) model_config: ModelConfig,
    /// Timeout configuration for orchestrator operations
    pub(crate) timeout_config: TimeoutConfig,
    /// The user's original goal/prompt
    pub(crate) original_goal: String,
    /// Handle to the socket listener task for cleanup
    socket_listener_task: Option<tokio::task::JoinHandle<()>>,
    /// Handle to the message routing task for cleanup
    router_task: Option<tokio::task::JoinHandle<()>>,
    /// Whether the message router is active (controls session message routing behavior)
    pub(crate) router_active: bool,
    /// Log manager for this run (kept for future use/API exposure)
    #[allow(dead_code)]
    log_manager: Arc<RunLogManager>,
    /// Current logging scope (changes during decompose/subtasks)
    pub(crate) current_scope: LogScope,
    /// Router for directing ACP messages to per-session channels
    pub(crate) session_router: Arc<RwLock<SessionRouter>>,
    /// Task manager for tracking structured plan execution
    pub(crate) task_manager: Arc<RwLock<TaskManager>>,
    /// Registry of built-in agent templates
    pub(crate) agent_registry: AgentRegistry,
    /// Channel for receiving model configuration updates from TUI (TUI feature only)
    #[cfg(feature = "tui")]
    config_update_rx: Option<tokio::sync::mpsc::Receiver<crate::tui::ModelConfigUpdate>>,
}

impl App {
    /// Create a new App with a pre-created log manager and backend.
    ///
    /// Uses the backend's default transport kind.
    pub async fn with_log_manager(
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        backend: Box<dyn Backend>,
        transport_kind: TransportKind,
    ) -> Result<Self> {
        Self::with_log_manager_and_timeout(
            model_config,
            log_manager,
            TimeoutConfig::default(),
            backend,
            transport_kind,
        )
        .await
    }

    /// Create a new App with a pre-created log manager, custom timeout configuration, and backend.
    ///
    /// # Arguments
    ///
    /// * `model_config` - Configuration for model selection
    /// * `log_manager` - Log manager for this run
    /// * `timeout_config` - Timeout configuration for sessions and requests
    /// * `backend` - Backend provider (Auggie, Cursor, etc.)
    /// * `transport_kind` - Transport protocol to use (ACP, CLI)
    pub async fn with_log_manager_and_timeout(
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        timeout_config: TimeoutConfig,
        backend: Box<dyn Backend>,
        transport_kind: TransportKind,
    ) -> Result<Self> {
        // Check backend authentication first
        backend.check_auth()?;

        tracing::info!(
            "🔌 Creating App with {} backend using {} transport",
            backend.name(),
            transport_kind
        );

        // Set up IPC socket FIRST - we need this for MCP setup
        let (socket_address, tool_rx, listener_task) = socket::setup_socket().await?;
        let socket_address_str = socket_address.as_str();

        // Set up MCP server configuration for this backend
        // For Cursor, this writes to ~/.cursor/mcp.json and enables the MCP
        // This MUST happen before creating transports
        backend.setup_mcp(&socket_address_str).await?;

        // Set up orchestrator cache directory with removed tools via backend
        let orchestrator_cache = backend.setup_agent_cache(
            AgentCacheType::Orchestrator,
            ORCHESTRATOR_CONFIG.removed_auggie_tools,
        )?;

        // Set up planner cache directory with task management tools removed via backend
        let planner_cache = backend
            .setup_agent_cache(AgentCacheType::Planner, PLANNER_CONFIG.removed_auggie_tools)?;

        // Get current working directory for transport workspace
        let workspace =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        // Orchestrator uses a custom cache directory with editing tools removed
        let orchestrator_config = TransportConfig::new(&orchestrator_cache)
            .with_request_timeout(timeout_config.request_timeout);
        let mut acp_orchestrator = backend
            .create_transport(transport_kind, AgentType::Orchestrator, orchestrator_config)
            .await?;
        acp_orchestrator.initialize().await?;

        // Planner uses a custom cache directory with task management tools removed
        let planner_config = TransportConfig::new(&planner_cache)
            .with_request_timeout(timeout_config.request_timeout);
        let mut acp_planner = backend
            .create_transport(transport_kind, AgentType::Planner, planner_config)
            .await?;
        acp_planner.initialize().await?;

        // Workers use the default workspace with all tools available
        let worker_config =
            TransportConfig::new(&workspace).with_request_timeout(timeout_config.request_timeout);
        let mut acp_worker = backend
            .create_transport(transport_kind, AgentType::Implementer, worker_config)
            .await?;
        acp_worker.initialize().await?;

        let current_scope = log_manager.root_scope();

        // Initialize task manager with event sender for structured plan tracking
        let event_tx = log_manager.event_sender();
        let task_manager = Arc::new(RwLock::new(TaskManager::new(event_tx)));

        tracing::info!(
            "⏱️  Timeout config: session={}s, request={}s (backend: {})",
            timeout_config.session_timeout.as_secs(),
            timeout_config.request_timeout.as_secs(),
            backend.name()
        );

        Ok(Self {
            backend,
            acp_orchestrator,
            acp_planner,
            acp_worker,
            socket_address: Some(socket_address),
            tool_rx: Some(tool_rx),
            model_config,
            timeout_config,
            original_goal: String::new(),
            socket_listener_task: Some(listener_task),
            router_task: None,
            router_active: false,
            log_manager,
            current_scope,
            session_router: Arc::new(RwLock::new(SessionRouter::new())),
            task_manager,
            agent_registry: AgentRegistry::new(),
            #[cfg(feature = "tui")]
            config_update_rx: None,
        })
    }

    /// Create a new App with mock ACP clients for testing.
    ///
    /// This constructor allows injection of mock ACP clients, enabling deterministic
    /// testing without requiring live agent processes.
    #[cfg(any(test, feature = "testing"))]
    #[allow(dead_code)]
    pub fn with_mock_transports(
        backend: Box<dyn Backend>,
        orchestrator: Box<dyn AgentTransport>,
        planner: Box<dyn AgentTransport>,
        worker: Box<dyn AgentTransport>,
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
    ) -> Self {
        let current_scope = log_manager.root_scope();

        Self {
            backend,
            acp_orchestrator: orchestrator,
            acp_planner: planner,
            acp_worker: worker,
            // Use a placeholder IPC address for tests (not actually connected)
            socket_address: Some(IpcAddress::from_string(
                "/tmp/paperboat-test-placeholder.sock",
            )),
            tool_rx: None,
            model_config,
            timeout_config: TimeoutConfig::default(),
            original_goal: String::new(),
            socket_listener_task: None,
            router_task: None,
            router_active: false,
            log_manager,
            current_scope,
            session_router: Arc::new(RwLock::new(SessionRouter::new())),
            task_manager: Arc::new(RwLock::new(TaskManager::default())),
            agent_registry: AgentRegistry::new(),
            #[cfg(feature = "tui")]
            config_update_rx: None,
        }
    }

    /// Create a new App with mock transports and an injected tool channel for testing.
    ///
    /// This constructor enables full test control over tool call handling by injecting
    /// the `tool_rx` channel directly, bypassing IPC socket setup.
    #[cfg(any(test, feature = "testing"))]
    pub fn with_mock_transports_and_tool_rx(
        backend: Box<dyn Backend>,
        orchestrator: Box<dyn AgentTransport>,
        planner: Box<dyn AgentTransport>,
        worker: Box<dyn AgentTransport>,
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        tool_rx: mpsc::Receiver<ToolMessage>,
    ) -> Self {
        let current_scope = log_manager.root_scope();

        Self {
            backend,
            acp_orchestrator: orchestrator,
            acp_planner: planner,
            acp_worker: worker,
            // Use a placeholder IPC address for tests - not used since tool_rx is injected
            socket_address: Some(IpcAddress::from_string(
                "/tmp/paperboat-test-placeholder.sock",
            )),
            tool_rx: Some(tool_rx),
            model_config,
            timeout_config: TimeoutConfig::default(),
            original_goal: String::new(),
            socket_listener_task: None,
            router_task: None,
            router_active: false,
            log_manager,
            current_scope,
            session_router: Arc::new(RwLock::new(SessionRouter::new())),
            task_manager: Arc::new(RwLock::new(TaskManager::default())),
            agent_registry: AgentRegistry::new(),
            #[cfg(feature = "tui")]
            config_update_rx: None,
        }
    }

    /// Set up IPC socket for MCP server communication
    #[allow(dead_code)]
    async fn setup_socket(&mut self) -> Result<IpcAddress> {
        let (socket_address, tool_rx, listener_task) = socket::setup_socket().await?;

        self.socket_address = Some(socket_address.clone());
        self.tool_rx = Some(tool_rx);
        self.socket_listener_task = Some(listener_task);

        // Start the message routing task for worker sessions
        self.start_worker_router();

        Ok(socket_address)
    }

    /// Start the background message routing tasks for agent sessions.
    ///
    /// This starts router tasks that continuously read from ACP clients' notification channels
    /// and route messages to per-session channels via the `session_router`.
    fn start_worker_router(&mut self) {
        let mut any_router_started = false;

        // Start router for worker client
        if let Some(notification_rx) = self.acp_worker.take_notification_rx() {
            let session_router = Arc::clone(&self.session_router);
            let router_task = tokio::spawn(async move {
                let mut rx = notification_rx;
                while let Some(msg) = rx.recv().await {
                    let routed = {
                        let router = session_router.read().await;
                        router.route(msg.clone())
                    };
                    if !routed {
                        if let Some(session_id) = router::SessionRouter::extract_session_id(&msg) {
                            tracing::trace!(
                                "Message for unregistered session {}: {:?}",
                                session_id,
                                msg.get("method")
                            );
                        }
                    }
                }
                tracing::debug!("Worker notification channel closed, router task exiting");
            });
            self.router_task = Some(router_task);
            any_router_started = true;
            tracing::debug!("Started worker message router task");
        } else {
            // This is expected when using CLI transport - the CLI handles messages differently
            tracing::debug!("Worker notification receiver already taken, skipping router setup");
        }

        // Start router for planner client
        if let Some(notification_rx) = self.acp_planner.take_notification_rx() {
            let session_router = Arc::clone(&self.session_router);
            tokio::spawn(async move {
                let mut rx = notification_rx;
                while let Some(msg) = rx.recv().await {
                    let routed = {
                        let router = session_router.read().await;
                        router.route(msg.clone())
                    };
                    if !routed {
                        if let Some(session_id) = router::SessionRouter::extract_session_id(&msg) {
                            tracing::trace!(
                                "Planner message for unregistered session {}: {:?}",
                                session_id,
                                msg.get("method")
                            );
                        }
                    }
                }
                tracing::debug!("Planner notification channel closed, router task exiting");
            });
            any_router_started = true;
            tracing::debug!("Started planner message router task");
        } else {
            // This is expected when using CLI transport - the CLI handles messages differently
            tracing::debug!(
                "Planner notification receiver already taken, skipping planner router setup"
            );
        }

        // Only mark router as active if at least one router was started
        // Mock clients don't have notification receivers, so tests use direct mode
        self.router_active = any_router_started;
    }

    /// Clean up the socket endpoint and listener task
    fn cleanup_socket(&mut self, socket_address: &IpcAddress) {
        let listener_task = self.socket_listener_task.take();
        socket::cleanup_socket(socket_address, listener_task);
    }

    /// Returns a clone of the current model configuration.
    ///
    /// This can be used to pass the configuration to the TUI for display.
    #[must_use]
    #[allow(dead_code)] // Public API for external consumers
    pub const fn model_config(&self) -> &ModelConfig {
        &self.model_config
    }

    /// Sets the channel for receiving model configuration updates from the TUI.
    ///
    /// This should be called after App is created to connect it to the TUI's
    /// configuration update channel. The App will poll this channel during
    /// execution and apply any updates received.
    #[cfg(feature = "tui")]
    pub fn set_config_update_channel(
        &mut self,
        rx: tokio::sync::mpsc::Receiver<crate::tui::ModelConfigUpdate>,
    ) {
        self.config_update_rx = Some(rx);
    }

    /// Applies a model configuration update from the TUI.
    ///
    /// This method applies partial updates to the model configuration.
    /// Only fields that are `Some` in the update will be changed.
    /// Changes take effect for newly spawned agents.
    /// The changes are also persisted to TOML config files.
    #[cfg(feature = "tui")]
    pub fn apply_model_config_update(&mut self, update: &crate::tui::ModelConfigUpdate) {
        use crate::config::save_agent_config;
        use crate::models::ModelFallbackChain;

        if let Some(tier) = update.orchestrator_model {
            tracing::info!("📝 Model config updated: orchestrator -> {}", tier);
            self.model_config.orchestrator_model = ModelFallbackChain::single(tier);
            if let Err(e) = save_agent_config("orchestrator", tier) {
                tracing::warn!("Failed to persist orchestrator config: {}", e);
            }
        }
        if let Some(tier) = update.planner_model {
            tracing::info!("📝 Model config updated: planner -> {}", tier);
            self.model_config.planner_model = ModelFallbackChain::single(tier);
            if let Err(e) = save_agent_config("planner", tier) {
                tracing::warn!("Failed to persist planner config: {}", e);
            }
        }
        if let Some(tier) = update.implementer_model {
            tracing::info!("📝 Model config updated: implementer -> {}", tier);
            self.model_config.implementer_model = ModelFallbackChain::single(tier);
            if let Err(e) = save_agent_config("implementer", tier) {
                tracing::warn!("Failed to persist implementer config: {}", e);
            }
        }
    }

    /// Polls for and applies any pending model configuration updates from the TUI.
    ///
    /// This method is non-blocking - it returns immediately if no updates are available.
    /// It should be called periodically in the main event loop.
    #[cfg(feature = "tui")]
    #[allow(dead_code)] // Public API for alternative polling patterns
    pub fn poll_config_updates(&mut self) {
        // Collect all pending updates first to avoid borrow issues
        let updates: Vec<_> = if let Some(ref mut rx) = self.config_update_rx {
            let mut collected = Vec::new();
            while let Ok(update) = rx.try_recv() {
                collected.push(update);
            }
            collected
        } else {
            return;
        };

        // Now apply all collected updates
        for update in &updates {
            self.apply_model_config_update(update);
        }
    }

    /// Get a reference to the task manager.
    ///
    /// This is primarily used by the self-improvement phase to access the final
    /// task state after a run completes.
    pub const fn task_manager(&self) -> &Arc<RwLock<TaskManager>> {
        &self.task_manager
    }

    /// Gracefully shutdown the application and all child processes.
    ///
    /// This should be called before the App is dropped to ensure clean termination
    /// of all agent processes and background tasks.
    pub async fn shutdown(&mut self) -> Result<()> {
        tracing::info!("🛑 Shutting down application...");

        // Clean up socket if it exists
        if let Some(socket_address) = self.socket_address.take() {
            self.cleanup_socket(&socket_address);
        }

        // Abort the router task if running
        if let Some(router_task) = self.router_task.take() {
            router_task.abort();
            tracing::debug!("Aborted message router task");
        }

        // Shutdown all ACP clients
        // Run them concurrently since they're independent
        let (orchestrator_result, planner_result, worker_result) = tokio::join!(
            self.acp_orchestrator.shutdown(),
            self.acp_planner.shutdown(),
            self.acp_worker.shutdown()
        );

        if let Err(e) = orchestrator_result {
            tracing::warn!("Error shutting down orchestrator ACP client: {}", e);
        }
        if let Err(e) = planner_result {
            tracing::warn!("Error shutting down planner ACP client: {}", e);
        }
        if let Err(e) = worker_result {
            tracing::warn!("Error shutting down worker ACP client: {}", e);
        }

        tracing::info!("✅ Shutdown complete");
        Ok(())
    }

    /// Build a combined summary from agent summaries, appending notes from `TaskManager` if present.
    ///
    /// This is a helper method that consolidates the repeated pattern of joining summaries
    /// and appending agent notes from the task manager.
    #[allow(dead_code)]
    pub(crate) async fn build_summary_with_notes(&self, summaries: Vec<String>) -> String {
        self.build_summary_with_notes_and_suggested_tasks(summaries, vec![])
            .await
    }

    /// Build a combined summary from agent summaries, appending notes and suggested tasks.
    ///
    /// If any tasks were suggested by agents (via `add_tasks` in their `complete()` call),
    /// they are included in the summary so the orchestrator knows to address them.
    #[allow(clippy::items_after_statements)] // use statement close to usage
    pub(crate) async fn build_summary_with_notes_and_suggested_tasks(
        &self,
        summaries: Vec<String>,
        suggested_task_ids: Vec<String>,
    ) -> String {
        let mut combined = summaries.join("\n");
        let tm = self.task_manager.read().await;

        // Append notes from agents
        if let Some(notes_section) = tm.format_notes() {
            combined.push_str("\n\n");
            combined.push_str(&notes_section);
        }

        // Append suggested tasks if any
        if !suggested_task_ids.is_empty() {
            combined.push_str("\n\n## New Tasks Suggested by Agents\n\n");
            combined.push_str("The following tasks were suggested by completed agents and have been added to your task list. ");
            combined.push_str("You MUST address them (execute with spawn_agents or skip with skip_tasks) before calling complete():\n\n");

            use std::fmt::Write;
            for task_id in &suggested_task_ids {
                if let Some(task) = tm.get(task_id) {
                    let _ = writeln!(
                        combined,
                        "- **[{task_id}] {}**: {}",
                        task.name, task.description
                    );
                } else {
                    let _ = writeln!(combined, "- **[{task_id}]** (task details not found)");
                }
            }
        }

        combined
    }
}
