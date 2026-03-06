//! Main orchestrator application module.
//!
//! This module contains the core App struct and orchestration logic, split into
//! focused sub-modules for maintainability:
//!
//! - `types`: Constants, type aliases, and utility functions
//! - `socket`: Unix socket setup and MCP connection handling
//! - `planner`: Planner agent spawning
//! - `agent_spawner`: Generic agent spawning with concurrent execution support
//! - `decompose`: Decomposition logic for subtasks
//! - `orchestrator`: Orchestrator agent spawning and execution
//! - `orchestrator_acp`: ACP message handling for orchestrator
//! - `session`: Session waiting and output collection
//! - `session_drain`: Message draining after session completion
//! - `router`: Session routing for ACP messages

mod agent_spawner;
mod decompose;
mod orchestrator;
mod orchestrator_acp;
mod planner;
pub mod router;
mod run;
mod session;
mod session_drain;
mod socket;
pub mod types;

pub use types::ToolMessage;

use crate::acp::{AcpClient, AcpClientTrait};
use crate::agents::{AgentRegistry, ORCHESTRATOR_CONFIG, PLANNER_CONFIG};
use crate::error::TimeoutConfig;
use crate::logging::{LogScope, RunLogManager};
use crate::models::ModelConfig;
use crate::tasks::TaskManager;
use anyhow::{Context, Result};
use router::SessionRouter;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use types::{ORCHESTRATOR_CACHE_DIR, PLANNER_CACHE_DIR};

/// The main application struct that orchestrates multi-agent workflows.
pub struct App {
    pub(crate) acp_orchestrator: Box<dyn AcpClientTrait + Send>,
    pub(crate) acp_planner: Box<dyn AcpClientTrait + Send>,
    pub(crate) acp_worker: Box<dyn AcpClientTrait + Send>,
    pub(crate) socket_path: Option<PathBuf>,
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
}

impl App {
    /// Set up the orchestrator cache directory with the required configuration.
    /// This ensures the orchestrator agent has editing tools removed.
    fn setup_orchestrator_cache() -> Result<String> {
        // First, check if auggie is authenticated
        let main_augment_dir = shellexpand::tilde("~/.augment").to_string();
        let main_session = std::path::Path::new(&main_augment_dir).join("session.json");

        if !main_session.exists() {
            anyhow::bail!(
                "Augment CLI is not authenticated.\n\n\
                Please run 'auggie login' first to authenticate, then try again."
            );
        }

        let cache_dir = shellexpand::tilde(ORCHESTRATOR_CACHE_DIR).to_string();
        let cache_path = std::path::Path::new(&cache_dir);

        // Create directory if it doesn't exist
        if !cache_path.exists() {
            std::fs::create_dir_all(cache_path)
                .context("Failed to create orchestrator cache directory")?;
            tracing::info!("Created orchestrator cache directory: {}", cache_dir);
        }

        // Copy session.json from main augment directory for authentication
        let orchestrator_session = cache_path.join("session.json");

        if !orchestrator_session.exists() {
            std::fs::copy(&main_session, &orchestrator_session)
                .context("Failed to copy session.json to orchestrator cache")?;
            tracing::info!("Copied session.json to orchestrator cache");
        }

        // Always write settings.json to ensure removedTools is current
        // Use centralized config from agents::config
        let settings = json!({
            "removedTools": ORCHESTRATOR_CONFIG.removed_auggie_tools
        });
        let settings_path = cache_path.join("settings.json");
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
            .context("Failed to write orchestrator settings.json")?;
        tracing::debug!(
            "Wrote orchestrator settings.json with {} removed tools",
            ORCHESTRATOR_CONFIG.removed_auggie_tools.len()
        );

        Ok(cache_dir)
    }

    /// Set up the planner cache directory with the required configuration.
    /// This ensures the planner agent has file editing and process execution tools removed.
    fn setup_planner_cache() -> Result<String> {
        // First, check if auggie is authenticated
        let main_augment_dir = shellexpand::tilde("~/.augment").to_string();
        let main_session = std::path::Path::new(&main_augment_dir).join("session.json");

        if !main_session.exists() {
            anyhow::bail!(
                "Augment CLI is not authenticated.\n\n\
                Please run 'auggie login' first to authenticate, then try again."
            );
        }

        let cache_dir = shellexpand::tilde(PLANNER_CACHE_DIR).to_string();
        let cache_path = std::path::Path::new(&cache_dir);

        // Create directory if it doesn't exist
        if !cache_path.exists() {
            std::fs::create_dir_all(cache_path)
                .context("Failed to create planner cache directory")?;
            tracing::info!("Created planner cache directory: {}", cache_dir);
        }

        // Copy session.json from main augment directory for authentication
        let planner_session = cache_path.join("session.json");

        if !planner_session.exists() {
            std::fs::copy(&main_session, &planner_session)
                .context("Failed to copy session.json to planner cache")?;
            tracing::info!("Copied session.json to planner cache");
        }

        // Always write settings.json to ensure removedTools is current
        // Use centralized config from agents::config
        let settings = json!({
            "removedTools": PLANNER_CONFIG.removed_auggie_tools
        });
        let settings_path = cache_path.join("settings.json");
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
            .context("Failed to write planner settings.json")?;
        tracing::debug!(
            "Wrote planner settings.json with {} removed tools",
            PLANNER_CONFIG.removed_auggie_tools.len()
        );

        Ok(cache_dir)
    }

    /// Create a new App with a pre-created log manager.
    pub async fn with_log_manager(
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
    ) -> Result<Self> {
        Self::with_log_manager_and_timeout(model_config, log_manager, TimeoutConfig::default())
            .await
    }

    /// Create a new App with a pre-created log manager and custom timeout configuration.
    pub async fn with_log_manager_and_timeout(
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        timeout_config: TimeoutConfig,
    ) -> Result<Self> {
        // Set up orchestrator cache directory with removed tools
        let orchestrator_cache = Self::setup_orchestrator_cache()?;

        // Set up planner cache directory with task management tools removed
        let planner_cache = Self::setup_planner_cache()?;

        // Orchestrator uses a custom cache directory with editing tools removed
        let mut acp_orchestrator = AcpClient::spawn_with_timeout(
            Some(&orchestrator_cache),
            timeout_config.request_timeout,
        )
        .await?;
        acp_orchestrator.initialize().await?;

        // Planner uses a custom cache directory with task management tools removed
        let mut acp_planner = AcpClient::spawn_with_timeout(
            Some(&planner_cache),
            timeout_config.request_timeout,
        )
        .await?;
        acp_planner.initialize().await?;

        // Workers use the default cache directory with all tools available
        let mut acp_worker =
            AcpClient::spawn_with_timeout(None, timeout_config.request_timeout).await?;
        acp_worker.initialize().await?;

        let current_scope = log_manager.root_scope();

        // Initialize task manager with event sender for structured plan tracking
        let event_tx = log_manager.event_sender();
        let task_manager = Arc::new(RwLock::new(TaskManager::new(event_tx)));

        tracing::info!(
            "⏱️  Timeout config: session={}s, request={}s",
            timeout_config.session_timeout.as_secs(),
            timeout_config.request_timeout.as_secs()
        );

        Ok(Self {
            acp_orchestrator: Box::new(acp_orchestrator),
            acp_planner: Box::new(acp_planner),
            acp_worker: Box::new(acp_worker),
            socket_path: None,
            tool_rx: None,
            model_config,
            timeout_config,
            original_goal: String::new(),
            socket_listener_task: None,
            router_task: None,
            router_active: false,
            log_manager,
            current_scope,
            session_router: Arc::new(RwLock::new(SessionRouter::new())),
            task_manager,
            agent_registry: AgentRegistry::new(),
        })
    }

    /// Create a new App with mock ACP clients for testing.
    ///
    /// This constructor allows injection of mock ACP clients, enabling deterministic
    /// testing without requiring live agent processes.
    #[cfg(any(test, feature = "testing"))]
    #[allow(dead_code)]
    pub fn with_mock_clients(
        orchestrator: Box<dyn AcpClientTrait + Send>,
        planner: Box<dyn AcpClientTrait + Send>,
        worker: Box<dyn AcpClientTrait + Send>,
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
    ) -> Self {
        let current_scope = log_manager.root_scope();

        Self {
            acp_orchestrator: orchestrator,
            acp_planner: planner,
            acp_worker: worker,
            socket_path: None,
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
        }
    }

    /// Create a new App with mock ACP clients and an injected tool channel for testing.
    ///
    /// This constructor enables full test control over tool call handling by injecting
    /// the `tool_rx` channel directly, bypassing Unix socket setup.
    #[cfg(any(test, feature = "testing"))]
    pub fn with_mock_clients_and_tool_rx(
        orchestrator: Box<dyn AcpClientTrait + Send>,
        planner: Box<dyn AcpClientTrait + Send>,
        worker: Box<dyn AcpClientTrait + Send>,
        model_config: ModelConfig,
        log_manager: Arc<RunLogManager>,
        tool_rx: mpsc::Receiver<ToolMessage>,
    ) -> Self {
        let current_scope = log_manager.root_scope();

        Self {
            acp_orchestrator: orchestrator,
            acp_planner: planner,
            acp_worker: worker,
            socket_path: None,
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
        }
    }

    /// Set up Unix socket for MCP server communication
    async fn setup_socket(&mut self) -> Result<PathBuf> {
        let (socket_path, tool_rx, listener_task) = socket::setup_socket().await?;

        self.socket_path = Some(socket_path.clone());
        self.tool_rx = Some(tool_rx);
        self.socket_listener_task = Some(listener_task);

        // Start the message routing task for worker sessions
        self.start_worker_router();

        Ok(socket_path)
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
            tracing::warn!("Worker notification receiver already taken, skipping router setup");
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
            tracing::warn!(
                "Planner notification receiver already taken, skipping planner router setup"
            );
        }

        // Only mark router as active if at least one router was started
        // Mock clients don't have notification receivers, so tests use direct mode
        self.router_active = any_router_started;
    }

    /// Clean up the socket file and listener task
    fn cleanup_socket(&mut self, socket_path: &PathBuf) {
        let listener_task = self.socket_listener_task.take();
        socket::cleanup_socket(socket_path, listener_task);
    }

    /// Gracefully shutdown the application and all child processes.
    ///
    /// This should be called before the App is dropped to ensure clean termination
    /// of all agent processes and background tasks.
    pub async fn shutdown(&mut self) -> Result<()> {
        tracing::info!("🛑 Shutting down application...");

        // Clean up socket if it exists
        if let Some(socket_path) = self.socket_path.take() {
            self.cleanup_socket(&socket_path);
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
}
