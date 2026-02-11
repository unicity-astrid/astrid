//! Gateway runtime orchestrator.

use crate::config::GatewayConfig;
use crate::error::{GatewayError, GatewayResult};
use crate::health::{HealthStatus, run_health_checks};
use crate::manager::AgentManager;
use crate::router::MessageRouter;
use crate::secrets::Secrets;
use crate::state::PersistedState;
use astralis_runtime::SubAgentPool;

use astralis_events::{AstralisEvent, EventBus, EventMetadata};
use chrono::{DateTime, Utc};
use notify::{Event, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, error, info, warn};

/// Gateway runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    /// Runtime is initializing.
    Initializing,
    /// Runtime is starting up.
    Starting,
    /// Runtime is running.
    Running,
    /// Runtime is shutting down.
    ShuttingDown,
    /// Runtime has stopped.
    Stopped,
}

/// Main gateway runtime.
pub struct GatewayRuntime {
    /// Configuration.
    config: Arc<RwLock<GatewayConfig>>,

    /// Path to the config file (for hot-reload).
    config_path: Option<PathBuf>,

    /// Secrets store.
    secrets: Secrets,

    /// Agent manager.
    agents: Arc<RwLock<AgentManager>>,

    /// Message router.
    router: Arc<RwLock<MessageRouter>>,

    /// Subagent pool.
    subagents: Arc<SubAgentPool>,

    /// Persisted state.
    state: Arc<RwLock<PersistedState>>,

    /// Event bus.
    event_bus: Arc<EventBus>,

    /// Current runtime state.
    runtime_state: Arc<RwLock<RuntimeState>>,

    /// When the runtime started.
    started_at: Option<DateTime<Utc>>,

    /// Shutdown signal sender.
    shutdown_tx: broadcast::Sender<()>,

    /// State file path.
    state_path: PathBuf,

    /// Config reload channel.
    reload_tx: mpsc::Sender<()>,
    reload_rx: Option<mpsc::Receiver<()>>,
}

impl GatewayRuntime {
    /// Create a new gateway runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration is invalid or resources cannot be initialized.
    pub fn new(config: GatewayConfig) -> GatewayResult<Self> {
        Self::with_config_path(config, None)
    }

    /// Create a new gateway runtime with a config file path for hot-reload.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration is invalid or resources cannot be initialized.
    pub fn with_config_path(
        config: GatewayConfig,
        config_path: Option<PathBuf>,
    ) -> GatewayResult<Self> {
        // Load secrets if configured
        let secrets = if let Some(ref path) = config.gateway.secrets_file {
            Secrets::load(path)?
        } else {
            Secrets::new()
        };

        // Create subagent pool
        let subagents = Arc::new(SubAgentPool::new(
            config.defaults.subagents.max_concurrent,
            config.defaults.subagents.max_depth,
        ));

        // Load or create state
        let state_path = PathBuf::from(&config.gateway.state_dir).join("gateway.json");
        let state = PersistedState::load_or_default(&state_path);

        let (shutdown_tx, _) = broadcast::channel(1);
        let (reload_tx, reload_rx) = mpsc::channel(1);

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
            secrets,
            agents: Arc::new(RwLock::new(AgentManager::new())),
            router: Arc::new(RwLock::new(MessageRouter::new())),
            subagents,
            state: Arc::new(RwLock::new(state)),
            event_bus: Arc::new(EventBus::new()),
            runtime_state: Arc::new(RwLock::new(RuntimeState::Initializing)),
            started_at: None,
            shutdown_tx,
            state_path,
            reload_tx,
            reload_rx: Some(reload_rx),
        })
    }

    /// Start the gateway runtime.
    ///
    /// This performs initialization but does not block. Use `run()` for blocking operation.
    ///
    /// # Errors
    ///
    /// Returns an error if startup fails.
    pub async fn start(&mut self) -> GatewayResult<()> {
        *self.runtime_state.write().await = RuntimeState::Starting;
        info!("Starting gateway runtime");

        self.started_at = Some(Utc::now());

        // Start config file watcher if hot-reload is enabled and we have a config path
        let config = self.config.read().await;
        let hot_reload_enabled = config.gateway.hot_reload;
        drop(config);

        if hot_reload_enabled && let Some(ref config_path) = self.config_path {
            self.start_config_watcher(config_path.clone());
        }

        // Start auto-start agents
        let config = self.config.read().await;
        let auto_start = config.auto_start_agents();
        for agent_name in auto_start {
            if let Some(agent_config) = config.agent_config(agent_name) {
                match self.agents.write().await.start(agent_config.clone()).await {
                    Ok(id) => {
                        info!(agent = %agent_name, id = %id, "Started agent");

                        // Register channel bindings
                        let mut router = self.router.write().await;
                        for channel in &agent_config.channels {
                            let binding = crate::router::ChannelBinding::new(
                                id.to_string(),
                                &channel.channel_type,
                            );
                            router.register(binding);
                        }
                    },
                    Err(e) => {
                        error!(agent = %agent_name, error = %e, "Failed to start agent");
                    },
                }
            }
        }
        drop(config);

        *self.runtime_state.write().await = RuntimeState::Running;
        info!("Gateway runtime started");

        self.event_bus.publish(AstralisEvent::GatewayStarted {
            metadata: EventMetadata::new("gateway"),
            version: env!("CARGO_PKG_VERSION").to_string(),
        });

        Ok(())
    }

    /// Start the config file watcher for hot-reload.
    fn start_config_watcher(&self, config_path: PathBuf) {
        let reload_tx = self.reload_tx.clone();

        // Create the watcher in a blocking task since notify uses sync APIs
        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();

            let Ok(mut watcher): Result<notify::RecommendedWatcher, _> = Watcher::new(
                tx,
                notify::Config::default().with_poll_interval(Duration::from_secs(2)),
            ) else {
                error!(path = %config_path.display(), "Failed to create config watcher");
                return;
            };

            if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
                error!(error = %e, path = %config_path.display(), "Failed to watch config file");
                return;
            }

            info!(path = %config_path.display(), "Started config file watcher");

            for event in rx {
                match event {
                    Ok(Event {
                        kind: notify::EventKind::Modify(_),
                        ..
                    }) => {
                        debug!("Config file modified, triggering reload");
                        if let Err(e) = reload_tx.blocking_send(()) {
                            error!(error = %e, "Failed to send reload signal");
                            break;
                        }
                    },
                    Ok(_) => {}, // Ignore other events
                    Err(e) => {
                        error!(error = %e, "Config watcher error");
                    },
                }
            }
        });
    }

    /// Reload configuration from the config file.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed.
    pub async fn reload_config(&self) -> GatewayResult<()> {
        let Some(ref config_path) = self.config_path else {
            return Err(GatewayError::Config("no config path set".into()));
        };

        info!(path = %config_path.display(), "Reloading configuration");

        let new_config = GatewayConfig::load(config_path)?;

        // Update config
        let mut config = self.config.write().await;
        *config = new_config;

        info!("Configuration reloaded successfully");

        self.event_bus.publish(AstralisEvent::ConfigReloaded {
            metadata: EventMetadata::new("gateway"),
        });

        Ok(())
    }

    /// Run the gateway (blocking).
    ///
    /// This blocks until shutdown is signaled.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime fails.
    pub async fn run(&mut self) -> GatewayResult<()> {
        self.start().await?;

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // Get intervals from config
        let config = self.config.read().await;
        let health_interval = Duration::from_secs(config.gateway.health_interval_secs);
        let save_interval = Duration::from_secs(config.sessions.save_interval_secs);
        drop(config);

        let mut health_timer = tokio::time::interval(health_interval);
        let mut save_timer = tokio::time::interval(save_interval);

        // Take the reload receiver (we own it now)
        let mut reload_rx = self.reload_rx.take();

        loop {
            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!("Received shutdown signal");
                    break;
                }
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown command");
                    break;
                }
                Some(()) = async { reload_rx.as_mut()?.recv().await } => {
                    if let Err(e) = self.reload_config().await {
                        error!(error = %e, "Failed to reload configuration");
                    }
                }
                _ = health_timer.tick() => {
                    let status = self.health().await;
                    let checks_performed = u32::try_from(status.checks.len()).unwrap_or(u32::MAX);
                    let checks_failed = u32::try_from(status.unhealthy_checks().len()).unwrap_or(u32::MAX);
                    if !status.is_healthy() {
                        warn!(state = %status.state, "Health check failed");
                        for check in status.unhealthy_checks() {
                            warn!(component = %check.component, message = ?check.message, "Unhealthy component");
                        }
                    }
                    self.event_bus.publish(AstralisEvent::HealthCheckCompleted {
                        metadata: EventMetadata::new("gateway"),
                        healthy: status.is_healthy(),
                        checks_performed,
                        checks_failed,
                    });
                }
                _ = save_timer.tick() => {
                    if let Err(e) = self.save_state().await {
                        error!(error = %e, "Failed to save state");
                    }
                }
            }
        }

        self.shutdown().await
    }

    /// Shutdown the gateway.
    ///
    /// # Errors
    ///
    /// Returns an error if shutdown fails.
    pub async fn shutdown(&mut self) -> GatewayResult<()> {
        *self.runtime_state.write().await = RuntimeState::ShuttingDown;
        info!("Shutting down gateway runtime");

        self.event_bus.publish(AstralisEvent::GatewayShutdown {
            metadata: EventMetadata::new("gateway"),
            reason: Some("shutdown requested".to_string()),
        });

        // Drain subagents gracefully, then force-cancel remaining
        let drain_timeout = {
            let config = self.config.read().await;
            Duration::from_secs(config.gateway.shutdown_timeout_secs.min(10))
        };
        if !self
            .subagents
            .wait_for_completion_timeout(drain_timeout)
            .await
        {
            warn!("Subagent drain timeout, cancelling remaining");
            self.subagents.cancel_all().await;
        }

        // Stop all agents
        self.agents.write().await.stop_all().await;

        // Save state
        self.save_state().await?;

        *self.runtime_state.write().await = RuntimeState::Stopped;
        info!("Gateway runtime stopped");

        Ok(())
    }

    /// Signal shutdown from another task.
    pub fn signal_shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    /// Get current health status.
    pub async fn health(&self) -> HealthStatus {
        let uptime = self
            .started_at
            .map(|s| (Utc::now() - s).to_std().unwrap_or_default())
            .unwrap_or_default();

        let agents = self.agents.read().await;
        let state = self.state.read().await;

        run_health_checks(
            agents.count(),
            0, // MCP server count - would come from MCP client
            state.pending_approvals.len(),
            true, // Audit healthy - would check actual audit log
            uptime,
            env!("CARGO_PKG_VERSION"),
        )
        .await
    }

    /// Get current runtime state.
    pub async fn state(&self) -> RuntimeState {
        *self.runtime_state.read().await
    }

    /// Get the event bus.
    #[must_use]
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }

    /// Get the agent manager.
    #[must_use]
    pub fn agents(&self) -> &Arc<RwLock<AgentManager>> {
        &self.agents
    }

    /// Get the message router.
    #[must_use]
    pub fn router(&self) -> &Arc<RwLock<MessageRouter>> {
        &self.router
    }

    /// Get the subagent pool.
    #[must_use]
    pub fn subagents(&self) -> &Arc<SubAgentPool> {
        &self.subagents
    }

    /// Get the secrets store.
    #[must_use]
    pub fn secrets(&self) -> &Secrets {
        &self.secrets
    }

    /// Get the configuration.
    #[must_use]
    pub fn config(&self) -> &Arc<RwLock<GatewayConfig>> {
        &self.config
    }

    /// Save state to disk.
    async fn save_state(&self) -> GatewayResult<()> {
        let mut state = self.state.write().await;
        state.save(&self.state_path)?;
        Ok(())
    }

    /// Expand a string with secrets and environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if expansion fails.
    pub fn expand(&self, input: &str) -> GatewayResult<String> {
        self.secrets.expand(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_runtime_creation() {
        let config = GatewayConfig::default();
        let runtime = GatewayRuntime::new(config);
        assert!(runtime.is_ok());
    }

    #[tokio::test]
    async fn test_runtime_state() {
        let config = GatewayConfig::default();
        let runtime = GatewayRuntime::new(config).unwrap();

        assert_eq!(runtime.state().await, RuntimeState::Initializing);
    }

    #[tokio::test]
    async fn test_runtime_health() {
        let config = GatewayConfig::default();
        let runtime = GatewayRuntime::new(config).unwrap();

        let health = runtime.health().await;
        // May be degraded with no agents
        assert!(matches!(
            health.state,
            crate::health::HealthState::Healthy | crate::health::HealthState::Degraded
        ));
    }

    #[tokio::test]
    async fn test_runtime_start_shutdown() {
        let config = GatewayConfig::default();
        let mut runtime = GatewayRuntime::new(config).unwrap();

        runtime.start().await.unwrap();
        assert_eq!(runtime.state().await, RuntimeState::Running);

        runtime.shutdown().await.unwrap();
        assert_eq!(runtime.state().await, RuntimeState::Stopped);
    }

    #[tokio::test]
    async fn test_runtime_config_reload() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("gateway.toml");

        // Create initial config
        std::fs::write(
            &config_path,
            r#"
            [gateway]
            hot_reload = true
            health_interval_secs = 30
            "#,
        )
        .unwrap();

        let config = GatewayConfig::load(&config_path).unwrap();
        let runtime = GatewayRuntime::with_config_path(config, Some(config_path.clone())).unwrap();

        // Check initial value
        assert_eq!(runtime.config.read().await.gateway.health_interval_secs, 30);

        // Update config file
        std::fs::write(
            &config_path,
            r#"
            [gateway]
            hot_reload = true
            health_interval_secs = 60
            "#,
        )
        .unwrap();

        // Reload config
        runtime.reload_config().await.unwrap();

        // Check updated value
        assert_eq!(runtime.config.read().await.gateway.health_interval_secs, 60);
    }
}
