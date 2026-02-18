//! Agent runtime - the main orchestration component.
//!
//! Coordinates LLM, MCP, capabilities, and audit systems.

use astrid_approval::{SecurityInterceptor, SecurityPolicy};
use astrid_audit::AuditLog;
use astrid_core::{Frontend, SessionId};
use astrid_crypto::KeyPair;
use astrid_hooks::result::HookContext;
use astrid_hooks::{HookEvent, HookManager};
use astrid_llm::LlmProvider;
use astrid_mcp::McpClient;
use astrid_plugins::PluginRegistry;
use astrid_storage::KvStore;
use astrid_tools::{SparkConfig, ToolContext, ToolRegistry};
use astrid_workspace::WorkspaceBoundary;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info};

use crate::context::ContextManager;
use crate::error::RuntimeResult;
use crate::session::AgentSession;
use crate::store::SessionStore;
use crate::subagent::SubAgentPool;
use crate::subagent_executor::SubAgentExecutor;

mod config;
mod execution;
mod security;
mod tool_execution;
mod workspace;

#[cfg(test)]
mod tests;

pub use config::RuntimeConfig;

/// The main agent runtime.
pub struct AgentRuntime<P: LlmProvider> {
    /// LLM provider.
    pub(super) llm: Arc<P>,
    /// MCP client.
    pub(super) mcp: McpClient,
    /// Audit log.
    pub(super) audit: Arc<AuditLog>,
    /// Session store.
    pub(super) sessions: SessionStore,
    /// Runtime signing key.
    pub(super) crypto: Arc<KeyPair>,
    /// Configuration.
    pub(super) config: RuntimeConfig,
    /// Context manager.
    pub(super) context: ContextManager,
    /// Pre-compiled workspace boundary checker.
    pub(super) boundary: WorkspaceBoundary,
    /// Hook manager for user-defined extension points.
    pub(super) hooks: Arc<HookManager>,
    /// Built-in tool registry.
    pub(super) tool_registry: ToolRegistry,
    /// Shared current working directory (persists across turns).
    pub(super) shared_cwd: Arc<tokio::sync::RwLock<std::path::PathBuf>>,
    /// Security policy (shared across sessions).
    pub(super) security_policy: SecurityPolicy,
    /// Sub-agent pool (shared across turns).
    pub(super) subagent_pool: Arc<SubAgentPool>,
    /// Plugin registry (shared with the gateway).
    pub(super) plugin_registry: Option<Arc<tokio::sync::RwLock<PluginRegistry>>>,
    /// Per-plugin KV stores that persist across tool calls.
    /// Keyed by `{session_id}:{server}` to isolate sessions from each other.
    /// Call [`cleanup_plugin_kv_stores`](Self::cleanup_plugin_kv_stores) when a
    /// session ends to prevent unbounded growth.
    pub(super) plugin_kv_stores:
        std::sync::Mutex<std::collections::HashMap<String, Arc<dyn KvStore>>>,
    /// Weak self-reference for spawner injection (set via `set_self_arc`).
    pub(super) self_arc: tokio::sync::RwLock<Option<std::sync::Weak<Self>>>,
}

impl<P: LlmProvider + 'static> AgentRuntime<P> {
    /// Create a new runtime.
    #[must_use]
    pub fn new(
        llm: P,
        mcp: McpClient,
        audit: AuditLog,
        sessions: SessionStore,
        crypto: KeyPair,
        config: RuntimeConfig,
    ) -> Self {
        let context =
            ContextManager::new(config.max_context_tokens).keep_recent(config.keep_recent_count);
        let boundary = WorkspaceBoundary::new(config.workspace.clone());

        let tool_registry = ToolRegistry::with_defaults();
        let shared_cwd = Arc::new(tokio::sync::RwLock::new(config.workspace.root.clone()));
        let subagent_pool = Arc::new(SubAgentPool::new(
            config.max_concurrent_subagents,
            config.max_subagent_depth,
        ));

        info!(
            workspace_root = %config.workspace.root.display(),
            workspace_mode = ?config.workspace.mode,
            max_concurrent_subagents = config.max_concurrent_subagents,
            max_subagent_depth = config.max_subagent_depth,
            "Workspace boundary initialized"
        );

        Self {
            llm: Arc::new(llm),
            mcp,
            audit: Arc::new(audit),
            sessions,
            crypto: Arc::new(crypto),
            config,
            context,
            boundary,
            hooks: Arc::new(HookManager::new()),
            tool_registry,
            shared_cwd,
            security_policy: SecurityPolicy::default(),
            subagent_pool,
            plugin_registry: None,
            plugin_kv_stores: std::sync::Mutex::new(std::collections::HashMap::new()),
            self_arc: tokio::sync::RwLock::new(None),
        }
    }

    /// Set the plugin registry for plugin tool integration.
    #[must_use]
    pub fn with_plugin_registry(
        mut self,
        registry: Arc<tokio::sync::RwLock<PluginRegistry>>,
    ) -> Self {
        self.plugin_registry = Some(registry);
        self
    }

    /// Create a new runtime wrapped in `Arc` with the self-reference pre-set.
    ///
    /// Uses `Arc::new_cyclic` to avoid the two-step `new()` + `set_self_arc()` pattern.
    /// Accepts an optional `HookManager` since `with_hooks()` can't be chained after
    /// Arc wrapping. Accepts an optional `PluginRegistry` for plugin tool integration.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new_arc(
        llm: P,
        mcp: McpClient,
        audit: AuditLog,
        sessions: SessionStore,
        crypto: KeyPair,
        config: RuntimeConfig,
        hooks: Option<HookManager>,
        plugin_registry: Option<Arc<tokio::sync::RwLock<PluginRegistry>>>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak| {
            let mut runtime = Self::new(llm, mcp, audit, sessions, crypto, config);
            if let Some(hook_manager) = hooks {
                runtime.hooks = Arc::new(hook_manager);
            }
            runtime.plugin_registry = plugin_registry;
            // Pre-set the self-reference (no async needed — field is initialized directly).
            runtime.self_arc = tokio::sync::RwLock::new(Some(weak.clone()));
            runtime
        })
    }

    /// Create a new session.
    ///
    /// Uses `build_system_prompt()` to dynamically assemble a workspace-aware
    /// prompt with tool guidelines and project instructions. If the user has
    /// explicitly set a custom `system_prompt` in config, that takes priority.
    ///
    /// An optional `workspace_override` can be supplied to use a different
    /// workspace root than the one in the runtime config (e.g. the CLI
    /// client's actual working directory).
    #[must_use]
    pub fn create_session(&self, workspace_override: Option<&Path>) -> AgentSession {
        let workspace_root = workspace_override.unwrap_or(&self.config.workspace.root);

        // Build the base system prompt WITHOUT spark identity.
        // Spark is layered on each loop iteration for hot-reload support.
        let system_prompt = if self.config.system_prompt.is_empty() {
            astrid_tools::build_system_prompt(workspace_root, None)
        } else {
            self.config.system_prompt.clone()
        };

        let session = AgentSession::new(self.crypto.key_id(), system_prompt);
        info!(session_id = %session.id, "Created new session");
        session
    }

    /// Save a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the session cannot be serialized or written to disk.
    pub fn save_session(&self, session: &AgentSession) -> RuntimeResult<()> {
        self.sessions.save(session)
    }

    /// Load a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the session file cannot be read or deserialized.
    pub fn load_session(&self, id: &SessionId) -> RuntimeResult<Option<AgentSession>> {
        self.sessions.load(id)
    }

    /// List sessions.
    ///
    /// # Errors
    ///
    /// Returns an error if the session directory cannot be read or session files cannot be parsed.
    pub fn list_sessions(&self) -> RuntimeResult<Vec<crate::store::SessionSummary>> {
        self.sessions.list_with_metadata()
    }

    /// Get runtime configuration.
    #[must_use]
    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Get the audit log.
    #[must_use]
    pub fn audit(&self) -> &Arc<AuditLog> {
        &self.audit
    }

    /// Get the MCP client.
    #[must_use]
    pub fn mcp(&self) -> &McpClient {
        &self.mcp
    }

    /// Get the runtime key ID.
    #[must_use]
    pub fn key_id(&self) -> [u8; 8] {
        self.crypto.key_id()
    }

    /// Get the workspace boundary.
    #[must_use]
    pub fn boundary(&self) -> &WorkspaceBoundary {
        &self.boundary
    }

    /// Remove plugin KV stores for a session that has ended.
    ///
    /// Should be called when a session is finished to prevent unbounded growth
    /// of the `plugin_kv_stores` map in long-running processes.
    pub fn cleanup_plugin_kv_stores(&self, session_id: &SessionId) {
        let prefix = format!("{session_id}:");
        // SAFETY: no .await while lock is held — HashMap::retain is synchronous.
        let mut stores = self
            .plugin_kv_stores
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        stores.retain(|key, _| !key.starts_with(&prefix));
    }

    /// Set a custom security policy.
    #[must_use]
    pub fn with_security_policy(mut self, policy: SecurityPolicy) -> Self {
        self.security_policy = policy;
        self
    }

    /// Set a pre-configured hook manager.
    #[must_use]
    pub fn with_hooks(mut self, hooks: HookManager) -> Self {
        self.hooks = Arc::new(hooks);
        self
    }

    /// Get the hook manager.
    #[must_use]
    pub fn hooks(&self) -> &Arc<HookManager> {
        &self.hooks
    }

    /// Get the sub-agent pool.
    #[must_use]
    pub fn subagent_pool(&self) -> &Arc<SubAgentPool> {
        &self.subagent_pool
    }

    /// Store a weak self-reference for sub-agent spawner injection.
    ///
    /// **Important**: Callers must wrap the runtime in `Arc` and call this method
    /// for sub-agent support to work. Without it, the `task` tool will return
    /// "not available in this context".
    ///
    /// ```ignore
    /// let runtime = Arc::new(AgentRuntime::new(/* ... */));
    /// runtime.set_self_arc(&runtime).await;
    /// ```
    ///
    // TODO: Consider migrating to `Arc::new_cyclic` to eliminate the two-step
    // initialization pattern and make the self-reference setup infallible.
    pub async fn set_self_arc(self: &Arc<Self>) {
        *self.self_arc.write().await = Some(Arc::downgrade(self));
    }

    /// Read the effective spark identity (hot-reload support).
    ///
    /// Priority: `spark.toml` (living document) > `[spark]` in config (static seed).
    /// Returns `None` when no spark is configured or both sources are empty.
    pub(super) fn read_effective_spark(&self) -> Option<SparkConfig> {
        // 1. Try spark.toml (living document, takes priority)
        if let Some(ref path) = self.config.spark_file {
            match SparkConfig::load_from_file(path) {
                Some(spark) if !spark.is_empty() => return Some(spark),
                None if path.exists() => {
                    tracing::warn!(
                        path = %path.display(),
                        "spark.toml exists but failed to parse; falling back to config seed"
                    );
                },
                Some(_) | None => { /* empty or missing, fall through to seed */ },
            }
        }
        // 2. Fall back to [spark] from config (static seed)
        self.config
            .spark_seed
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
    }

    /// Inject a `SubAgentExecutor` into the per-turn `ToolContext`.
    ///
    /// Does nothing if `set_self_arc` was never called (graceful degradation).
    pub(super) async fn inject_subagent_spawner<F: Frontend + 'static>(
        &self,
        tool_ctx: &ToolContext,
        session: &AgentSession,
        frontend: &Arc<F>,
        parent_subagent_id: Option<crate::subagent::SubAgentId>,
    ) {
        let self_arc = {
            let guard = self.self_arc.read().await;
            guard.as_ref().and_then(std::sync::Weak::upgrade)
        };

        if let Some(runtime_arc) = self_arc {
            // Read callsign from effective spark for sub-agent identity inheritance.
            let parent_callsign = self.read_effective_spark().and_then(|s| {
                if s.callsign.is_empty() {
                    None
                } else {
                    Some(s.callsign)
                }
            });

            let executor = SubAgentExecutor::new(
                runtime_arc,
                Arc::clone(&self.subagent_pool),
                Arc::clone(frontend),
                session.user_id,
                parent_subagent_id,
                session.id.clone(),
                Arc::clone(&session.allowance_store),
                Arc::clone(&session.capabilities),
                Arc::clone(&session.budget_tracker),
                self.config.default_subagent_timeout,
                parent_callsign,
            );
            tool_ctx
                .set_subagent_spawner(Some(Arc::new(executor)))
                .await;
        } else {
            debug!("No self_arc set — sub-agent spawning disabled for this turn");
        }
    }

    /// Convert a `[u8; 8]` user ID to a UUID by zero-padding to 16 bytes.
    pub(super) fn user_uuid(user_id: [u8; 8]) -> uuid::Uuid {
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes[..8].copy_from_slice(&user_id);
        uuid::Uuid::from_bytes(uuid_bytes)
    }

    /// Build a hook context with session info.
    #[allow(clippy::unused_self)]
    pub(super) fn build_hook_context(
        &self,
        session: &AgentSession,
        event: HookEvent,
    ) -> HookContext {
        HookContext::new(event)
            .with_session(session.id.0)
            .with_user(Self::user_uuid(session.user_id))
    }

    /// Build a `SecurityInterceptor` for the given session.
    ///
    /// Cheap to create — just Arc clones of shared state.
    /// Uses the session's per-session budget tracker so budget persists across restarts.
    pub(super) fn build_interceptor(&self, session: &AgentSession) -> SecurityInterceptor {
        SecurityInterceptor::new(
            Arc::clone(&session.capabilities),
            Arc::clone(&session.approval_manager),
            self.security_policy.clone(),
            Arc::clone(&session.budget_tracker),
            Arc::clone(&self.audit),
            Arc::clone(&self.crypto),
            session.id.clone(),
            Arc::clone(&session.allowance_store),
            Some(self.config.workspace.root.clone()),
            session.workspace_budget_tracker.clone(),
        )
    }
}

// ---------------------------------------------------------------------------
// Cost tracking helpers
// ---------------------------------------------------------------------------

/// Hardcoded Claude model rates (USD per 1K tokens).
/// These will be configurable via TOML config in Step 3.
const INPUT_RATE_PER_1K: f64 = 0.003; // $3 per million input tokens
const OUTPUT_RATE_PER_1K: f64 = 0.015; // $15 per million output tokens

/// Convert token counts to estimated USD cost.
#[allow(clippy::cast_precision_loss)]
pub(super) fn tokens_to_usd(input_tokens: usize, output_tokens: usize) -> f64 {
    let input_cost = (input_tokens as f64 / 1000.0) * INPUT_RATE_PER_1K;
    let output_cost = (output_tokens as f64 / 1000.0) * OUTPUT_RATE_PER_1K;
    input_cost + output_cost
}
