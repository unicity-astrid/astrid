//! Agent runtime - the main orchestration component.
//!
//! Coordinates LLM, MCP, capabilities, and audit systems.

use astrid_approval::manager::ApprovalHandler;
use astrid_approval::request::{
    ApprovalDecision as InternalApprovalDecision, ApprovalRequest as InternalApprovalRequest,
    ApprovalResponse as InternalApprovalResponse,
};
use astrid_approval::{SecurityInterceptor, SecurityPolicy, SensitiveAction};
use astrid_audit::{AuditAction, AuditLog, AuditOutcome, AuthorizationProof};
use astrid_capabilities::AuditEntryId;
use astrid_core::{
    ApprovalDecision, ApprovalOption, ApprovalRequest, Frontend, RiskLevel, SessionId,
};
use astrid_crypto::KeyPair;
use astrid_hooks::result::HookContext;
use astrid_hooks::{HookEvent, HookManager};
use astrid_llm::{LlmProvider, LlmToolDefinition, Message, StreamEvent, ToolCall, ToolCallResult};
use astrid_mcp::McpClient;
use astrid_plugins::PluginRegistry;
use astrid_storage::{KvStore, MemoryKvStore, ScopedKvStore};
use astrid_tools::{ToolContext, ToolRegistry, truncate_output};
use astrid_workspace::{
    EscapeDecision, EscapeRequest, PathCheck, WorkspaceBoundary, WorkspaceConfig,
};
use async_trait::async_trait;
use futures::StreamExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::context::ContextManager;
use crate::error::{RuntimeError, RuntimeResult};
use crate::session::AgentSession;
use crate::store::SessionStore;
use crate::subagent::SubAgentPool;
use crate::subagent_executor::{DEFAULT_SUBAGENT_TIMEOUT, SubAgentExecutor};

/// Default maximum context tokens (100k).
const DEFAULT_MAX_CONTEXT_TOKENS: usize = 100_000;
/// Default number of recent messages to keep when summarizing.
const DEFAULT_KEEP_RECENT_COUNT: usize = 10;

/// Default maximum concurrent sub-agents.
const DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 4;
/// Default maximum sub-agent nesting depth.
const DEFAULT_MAX_SUBAGENT_DEPTH: usize = 3;

/// Configuration for the agent runtime.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Maximum context tokens.
    pub max_context_tokens: usize,
    /// System prompt.
    pub system_prompt: String,
    /// Whether to auto-summarize on context overflow.
    pub auto_summarize: bool,
    /// Number of recent messages to keep when summarizing.
    pub keep_recent_count: usize,
    /// Workspace configuration for operational boundaries.
    pub workspace: WorkspaceConfig,
    /// Maximum concurrent sub-agents.
    pub max_concurrent_subagents: usize,
    /// Maximum sub-agent nesting depth.
    pub max_subagent_depth: usize,
    /// Default sub-agent timeout.
    pub default_subagent_timeout: std::time::Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            max_context_tokens: DEFAULT_MAX_CONTEXT_TOKENS,
            system_prompt: String::new(),
            auto_summarize: true,
            keep_recent_count: DEFAULT_KEEP_RECENT_COUNT,
            workspace: WorkspaceConfig::new(workspace_root),
            max_concurrent_subagents: DEFAULT_MAX_CONCURRENT_SUBAGENTS,
            max_subagent_depth: DEFAULT_MAX_SUBAGENT_DEPTH,
            default_subagent_timeout: DEFAULT_SUBAGENT_TIMEOUT,
        }
    }
}

/// The main agent runtime.
pub struct AgentRuntime<P: LlmProvider> {
    /// LLM provider.
    llm: Arc<P>,
    /// MCP client.
    mcp: McpClient,
    /// Audit log.
    audit: Arc<AuditLog>,
    /// Session store.
    sessions: SessionStore,
    /// Runtime signing key.
    crypto: Arc<KeyPair>,
    /// Configuration.
    config: RuntimeConfig,
    /// Context manager.
    context: ContextManager,
    /// Pre-compiled workspace boundary checker.
    boundary: WorkspaceBoundary,
    /// Hook manager for user-defined extension points.
    hooks: Arc<HookManager>,
    /// Built-in tool registry.
    tool_registry: ToolRegistry,
    /// Shared current working directory (persists across turns).
    shared_cwd: Arc<tokio::sync::RwLock<PathBuf>>,
    /// Security policy (shared across sessions).
    security_policy: SecurityPolicy,
    /// Sub-agent pool (shared across turns).
    subagent_pool: Arc<SubAgentPool>,
    /// Plugin registry (shared with the gateway).
    plugin_registry: Option<Arc<tokio::sync::RwLock<PluginRegistry>>>,
    /// Per-plugin KV stores that persist across tool calls.
    /// Keyed by `{session_id}:{server}` to isolate sessions from each other.
    plugin_kv_stores: std::sync::Mutex<std::collections::HashMap<String, Arc<dyn KvStore>>>,
    /// Weak self-reference for spawner injection (set via `set_self_arc`).
    self_arc: tokio::sync::RwLock<Option<std::sync::Weak<Self>>>,
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

        let system_prompt = if self.config.system_prompt.is_empty() {
            astrid_tools::build_system_prompt(workspace_root)
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

    /// Run a single turn with streaming output.
    ///
    /// The `frontend` parameter is wrapped in `Arc` so it can be registered as an
    /// approval handler for the duration of the turn.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The LLM provider fails to generate a response
    /// - An MCP tool call fails
    /// - An approval request fails
    /// - Session persistence fails
    #[allow(clippy::too_many_lines)]
    pub async fn run_turn_streaming<F: Frontend + 'static>(
        &self,
        session: &mut AgentSession,
        input: &str,
        frontend: Arc<F>,
    ) -> RuntimeResult<()> {
        // Register the frontend as the approval handler for this turn.
        let handler: Arc<dyn ApprovalHandler> = Arc::new(FrontendApprovalHandler {
            frontend: Arc::clone(&frontend),
        });
        session.approval_manager.register_handler(handler).await;

        // Add user message
        session.add_message(Message::user(input));

        // Fire UserPrompt hook
        {
            let ctx = self
                .build_hook_context(session, HookEvent::UserPrompt)
                .with_data("input", serde_json::json!(input));
            let result = self.hooks.trigger_simple(HookEvent::UserPrompt, ctx).await;
            if let astrid_hooks::HookResult::Block { reason } = result {
                return Err(RuntimeError::ApprovalDenied { reason });
            }
            if let astrid_hooks::HookResult::ContinueWith { modifications } = &result {
                debug!(?modifications, "UserPrompt hook modified context");
            }
        }

        // Log session activity
        {
            let _ = self.audit.append(
                session.id.clone(),
                AuditAction::LlmRequest {
                    model: self.llm.model().to_string(),
                    input_tokens: session.token_count,
                    output_tokens: 0,
                },
                AuthorizationProof::System {
                    reason: "user input".to_string(),
                },
                AuditOutcome::success(),
            );
        }

        // Check context limit and summarize if needed
        if self.config.auto_summarize && self.context.needs_summarization(session) {
            frontend.show_status("Summarizing context...");
            let result = self.context.summarize(session, self.llm.as_ref()).await?;

            // Log summarization
            {
                let _ = self.audit.append(
                    session.id.clone(),
                    AuditAction::ContextSummarized {
                        evicted_count: result.messages_evicted,
                        tokens_freed: result.tokens_freed,
                    },
                    AuthorizationProof::System {
                        reason: "context overflow".to_string(),
                    },
                    AuditOutcome::success(),
                );
            }
        }

        // Create per-turn ToolContext (shares cwd, owns its own spawner slot)
        let tool_ctx = ToolContext::with_shared_cwd(
            self.config.workspace.root.clone(),
            Arc::clone(&self.shared_cwd),
        );

        // Inject sub-agent spawner (if self_arc is available)
        self.inject_subagent_spawner(&tool_ctx, session, &frontend, None)
            .await;

        // Run the agentic loop (tool_ctx is dropped at turn end — no cleanup needed)
        let loop_result = self.run_loop(session, &*frontend, &tool_ctx).await;

        loop_result?;

        self.sessions.save(session)?;
        Ok(())
    }

    /// Run a single turn for a sub-agent session.
    ///
    /// Like `run_turn_streaming` but without hooks, summarization, or session persistence.
    /// The session is ephemeral and owned by the caller (`SubAgentExecutor`).
    ///
    /// `parent_subagent_id` is the pool handle ID of this sub-agent, passed so that
    /// nested sub-agents (if the sub-agent calls `task` tool) can declare their parent.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM or tool execution fails.
    pub async fn run_subagent_turn<F: Frontend + 'static>(
        &self,
        session: &mut AgentSession,
        prompt: &str,
        frontend: Arc<F>,
        parent_subagent_id: Option<crate::subagent::SubAgentId>,
    ) -> RuntimeResult<()> {
        // Register the frontend as the approval handler for this turn.
        let handler: Arc<dyn ApprovalHandler> = Arc::new(FrontendApprovalHandler {
            frontend: Arc::clone(&frontend),
        });
        session.approval_manager.register_handler(handler).await;

        // Add user message
        session.add_message(Message::user(prompt));

        // Log sub-agent LLM request
        {
            let _ = self.audit.append(
                session.id.clone(),
                AuditAction::LlmRequest {
                    model: self.llm.model().to_string(),
                    input_tokens: session.token_count,
                    output_tokens: 0,
                },
                AuthorizationProof::System {
                    reason: "sub-agent prompt".to_string(),
                },
                AuditOutcome::success(),
            );
        }

        // Create per-turn ToolContext (shares cwd, owns its own spawner slot)
        let tool_ctx = ToolContext::with_shared_cwd(
            self.config.workspace.root.clone(),
            Arc::clone(&self.shared_cwd),
        );

        // Inject sub-agent spawner for nested sub-agents
        self.inject_subagent_spawner(&tool_ctx, session, &frontend, parent_subagent_id)
            .await;

        // Run the agentic loop (no hooks, no summarize, no save)
        // tool_ctx is dropped at turn end — no cleanup needed
        self.run_loop(session, &*frontend, &tool_ctx).await
    }

    /// The inner agentic loop: stream LLM → collect tool calls → execute → repeat.
    ///
    /// Shared by `run_turn_streaming` and `run_subagent_turn`.
    #[allow(clippy::too_many_lines)]
    async fn run_loop<F: Frontend>(
        &self,
        session: &mut AgentSession,
        frontend: &F,
        tool_ctx: &ToolContext,
    ) -> RuntimeResult<()> {
        loop {
            // Get tools: built-in + MCP
            let mut llm_tools: Vec<LlmToolDefinition> = self.tool_registry.all_definitions();

            let mcp_tools = self.mcp.list_tools().await?;
            llm_tools.extend(mcp_tools.iter().map(|t| {
                LlmToolDefinition::new(format!("{}:{}", &t.server, &t.name))
                    .with_description(t.description.clone().unwrap_or_default())
                    .with_schema(t.input_schema.clone())
            }));

            // Plugin tools (snapshot under a brief read lock).
            if let Some(ref registry) = self.plugin_registry {
                let registry = registry.read().await;
                llm_tools.extend(registry.all_tool_definitions().into_iter().map(|td| {
                    LlmToolDefinition::new(td.name)
                        .with_description(td.description)
                        .with_schema(td.input_schema)
                }));
            }

            // Stream from LLM
            let mut stream = self
                .llm
                .stream(&session.messages, &llm_tools, &session.system_prompt)
                .await?;

            let mut response_text = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut current_tool_args = String::new();

            while let Some(event) = stream.next().await {
                match event? {
                    StreamEvent::TextDelta(text) => {
                        frontend.show_status(&text);
                        response_text.push_str(&text);
                    },
                    StreamEvent::ToolCallStart { id, name } => {
                        tool_calls.push(ToolCall::new(id, name));
                        current_tool_args.clear();
                    },
                    StreamEvent::ToolCallDelta { id: _, args_delta } => {
                        current_tool_args.push_str(&args_delta);
                    },
                    StreamEvent::ToolCallEnd { id } => {
                        // Parse and set arguments for the completed tool call
                        if let Some(call) = tool_calls.iter_mut().find(|c| c.id == id)
                            && let Ok(args) = serde_json::from_str(&current_tool_args)
                        {
                            call.arguments = args;
                        }
                        current_tool_args.clear();
                    },
                    StreamEvent::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        debug!(input = input_tokens, output = output_tokens, "Token usage");
                        // Track cost in the session budget tracker
                        let cost = tokens_to_usd(input_tokens, output_tokens);
                        session.budget_tracker.record_cost(cost);
                        // Track cost in the workspace cumulative budget tracker
                        if let Some(ref ws_budget) = session.workspace_budget_tracker {
                            ws_budget.record_cost(cost);
                        }
                    },
                    StreamEvent::ReasoningDelta(_) => {
                        // Reasoning tokens are informational; not included in final output.
                    },
                    StreamEvent::Done => break,
                    StreamEvent::Error(e) => {
                        error!(error = %e, "Stream error");
                        return Err(RuntimeError::LlmError(
                            astrid_llm::LlmError::StreamingError(e),
                        ));
                    },
                }
            }

            // If we have tool calls, execute them
            if !tool_calls.is_empty() {
                // Add assistant message with tool calls
                session.add_message(Message::assistant_with_tools(tool_calls.clone()));

                // Execute each tool call
                for call in &tool_calls {
                    frontend.tool_started(&call.id, &call.name, &call.arguments);
                    let result = self
                        .execute_tool_call(session, call, frontend, tool_ctx)
                        .await?;
                    frontend.tool_completed(&call.id, &result.content, result.is_error);
                    session.add_message(Message::tool_result(result));
                    session.metadata.tool_call_count =
                        session.metadata.tool_call_count.saturating_add(1);
                }

                // Continue the loop for next LLM turn
                continue;
            }

            // If we have text and no tool calls, we're done
            if !response_text.is_empty() {
                session.add_message(Message::assistant(&response_text));
                return Ok(());
            }

            // Empty response, done
            break;
        }

        Ok(())
    }

    /// Execute a tool call with security checks via the `SecurityInterceptor`.
    #[allow(clippy::too_many_lines)]
    async fn execute_tool_call<F: Frontend>(
        &self,
        session: &mut AgentSession,
        call: &ToolCall,
        frontend: &F,
        tool_ctx: &ToolContext,
    ) -> RuntimeResult<ToolCallResult> {
        // Check for built-in tool first (no colon in name)
        if ToolRegistry::is_builtin(&call.name) {
            return self
                .execute_builtin_tool(session, call, frontend, tool_ctx)
                .await;
        }

        // Check for plugin tool (plugin:{plugin_id}:{tool_name})
        if PluginRegistry::is_plugin_tool(&call.name) {
            return self.execute_plugin_tool(session, call, frontend).await;
        }

        let (server, tool) = call.parse_name().ok_or_else(|| {
            RuntimeError::McpError(astrid_mcp::McpError::ToolNotFound {
                server: "unknown".to_string(),
                tool: call.name.clone(),
            })
        })?;

        // Check workspace boundaries before MCP authorization
        if let Err(tool_error) = self
            .check_workspace_boundaries(session, call, server, tool, frontend)
            .await
        {
            return Ok(tool_error);
        }

        // Fire PreToolCall hook
        {
            let ctx = self
                .build_hook_context(session, HookEvent::PreToolCall)
                .with_data("tool_name", serde_json::json!(tool))
                .with_data("server_name", serde_json::json!(server))
                .with_data("arguments", call.arguments.clone());
            let result = self.hooks.trigger_simple(HookEvent::PreToolCall, ctx).await;
            if let astrid_hooks::HookResult::Block { reason } = result {
                return Ok(ToolCallResult::error(&call.id, reason));
            }
            if let astrid_hooks::HookResult::ContinueWith { modifications } = &result {
                debug!(?modifications, "PreToolCall hook modified context");
            }
        }

        // Classify the MCP tool call as a SensitiveAction
        let action = classify_tool_call(server, tool, &call.arguments);

        // Run through the SecurityInterceptor (5-step check)
        let interceptor = self.build_interceptor(session);
        let tool_result = match interceptor
            .intercept(&action, &format!("MCP tool call to {server}:{tool}"), None)
            .await
        {
            Ok(intercept_result) => {
                // Surface budget warning to user
                if let Some(warning) = &intercept_result.budget_warning {
                    frontend.show_status(&format!(
                        "Budget warning: ${:.2}/${:.2} spent ({:.0}%)",
                        warning.current_spend, warning.session_max, warning.percent_used
                    ));
                }
                // Authorized — execute via MCP client directly
                let result = self
                    .mcp
                    .call_tool(server, tool, call.arguments.clone())
                    .await?;
                ToolCallResult::success(&call.id, result.text_content())
            },
            Err(e) => ToolCallResult::error(&call.id, e.to_string()),
        };

        // Fire PostToolCall or ToolError hook (informational, never blocks)
        {
            let hook_event = if tool_result.is_error {
                HookEvent::ToolError
            } else {
                HookEvent::PostToolCall
            };
            let ctx = self
                .build_hook_context(session, hook_event)
                .with_data("tool_name", serde_json::json!(tool))
                .with_data("server_name", serde_json::json!(server))
                .with_data("is_error", serde_json::json!(tool_result.is_error));
            let _ = self.hooks.trigger_simple(hook_event, ctx).await;
        }

        Ok(tool_result)
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

    /// Inject a `SubAgentExecutor` into the per-turn `ToolContext`.
    ///
    /// Does nothing if `set_self_arc` was never called (graceful degradation).
    async fn inject_subagent_spawner<F: Frontend + 'static>(
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
            );
            tool_ctx
                .set_subagent_spawner(Some(Arc::new(executor)))
                .await;
        } else {
            debug!("No self_arc set — sub-agent spawning disabled for this turn");
        }
    }

    /// Convert a `[u8; 8]` user ID to a UUID by zero-padding to 16 bytes.
    fn user_uuid(user_id: [u8; 8]) -> uuid::Uuid {
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes[..8].copy_from_slice(&user_id);
        uuid::Uuid::from_bytes(uuid_bytes)
    }

    /// Build a hook context with session info.
    #[allow(clippy::unused_self)]
    fn build_hook_context(&self, session: &AgentSession, event: HookEvent) -> HookContext {
        HookContext::new(event)
            .with_session(session.id.0)
            .with_user(Self::user_uuid(session.user_id))
    }

    /// Build a `SecurityInterceptor` for the given session.
    ///
    /// Cheap to create — just Arc clones of shared state.
    /// Uses the session's per-session budget tracker so budget persists across restarts.
    fn build_interceptor(&self, session: &AgentSession) -> SecurityInterceptor {
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

    /// Execute a built-in tool with workspace boundary checks, interceptor, and hooks.
    async fn execute_builtin_tool<F: Frontend>(
        &self,
        session: &mut AgentSession,
        call: &ToolCall,
        frontend: &F,
        tool_ctx: &ToolContext,
    ) -> RuntimeResult<ToolCallResult> {
        let tool_name = &call.name;

        let Some(tool) = self.tool_registry.get(tool_name) else {
            return Ok(ToolCallResult::error(
                &call.id,
                format!("Unknown built-in tool: {tool_name}"),
            ));
        };

        // Check workspace boundaries (built-in tools use the same path extraction)
        if let Err(tool_error) = self
            .check_workspace_boundaries(session, call, "builtin", tool_name, frontend)
            .await
        {
            return Ok(tool_error);
        }

        // Fire PreToolCall hook
        {
            let ctx = self
                .build_hook_context(session, HookEvent::PreToolCall)
                .with_data("tool_name", serde_json::json!(tool_name))
                .with_data("server_name", serde_json::json!("builtin"))
                .with_data("arguments", call.arguments.clone());
            let result = self.hooks.trigger_simple(HookEvent::PreToolCall, ctx).await;
            if let astrid_hooks::HookResult::Block { reason } = result {
                return Ok(ToolCallResult::error(&call.id, reason));
            }
        }

        // Classify and intercept — all tools go through the SecurityInterceptor
        let action = classify_builtin_tool_call(tool_name, &call.arguments);
        let interceptor = self.build_interceptor(session);
        match interceptor
            .intercept(&action, &format!("Built-in tool: {tool_name}"), None)
            .await
        {
            Ok(intercept_result) => {
                // Surface budget warning to user
                if let Some(warning) = &intercept_result.budget_warning {
                    frontend.show_status(&format!(
                        "Budget warning: ${:.2}/${:.2} spent ({:.0}%)",
                        warning.current_spend, warning.session_max, warning.percent_used
                    ));
                }
            },
            Err(e) => return Ok(ToolCallResult::error(&call.id, e.to_string())),
        }

        // Execute the built-in tool
        let tool_result = match tool.execute(call.arguments.clone(), tool_ctx).await {
            Ok(output) => {
                let output = truncate_output(output);
                ToolCallResult::success(&call.id, output)
            },
            Err(e) => ToolCallResult::error(&call.id, e.to_string()),
        };

        // Fire PostToolCall or ToolError hook
        {
            let hook_event = if tool_result.is_error {
                HookEvent::ToolError
            } else {
                HookEvent::PostToolCall
            };
            let ctx = self
                .build_hook_context(session, hook_event)
                .with_data("tool_name", serde_json::json!(tool_name))
                .with_data("server_name", serde_json::json!("builtin"))
                .with_data("is_error", serde_json::json!(tool_result.is_error));
            let _ = self.hooks.trigger_simple(hook_event, ctx).await;
        }

        Ok(tool_result)
    }

    /// Execute a plugin tool with security checks, interceptor, and hooks.
    ///
    /// Plugin tool names follow the format `plugin:{plugin_id}:{tool_name}`.
    /// The qualified name is used as-is for `PluginRegistry::find_tool()`.
    #[allow(clippy::too_many_lines)]
    async fn execute_plugin_tool<F: Frontend>(
        &self,
        session: &mut AgentSession,
        call: &ToolCall,
        frontend: &F,
    ) -> RuntimeResult<ToolCallResult> {
        let Some(ref registry_lock) = self.plugin_registry else {
            return Ok(ToolCallResult::error(
                &call.id,
                "Plugin tools are not available (no plugin registry configured)",
            ));
        };

        // Parse the qualified name into plugin ID and tool name.
        // Format: "plugin:{plugin_id}:{tool_name}" where tool_name may contain colons.
        // Uses strip_prefix + split_once (left-to-right) to correctly handle tool names
        // with colons (e.g. "plugin:foo:name:with:colons" → id="foo", tool="name:with:colons").
        let (plugin_id_str, tool) = match call.name.strip_prefix("plugin:") {
            Some(rest) => match rest.split_once(':') {
                Some((id, tool_name)) => (id, tool_name),
                None => {
                    return Ok(ToolCallResult::error(
                        &call.id,
                        format!(
                            "Malformed plugin tool name (missing tool segment): {}",
                            call.name
                        ),
                    ));
                },
            },
            None => {
                return Ok(ToolCallResult::error(
                    &call.id,
                    format!(
                        "Malformed plugin tool name (missing plugin: prefix): {}",
                        call.name
                    ),
                ));
            },
        };
        // Server-like prefix used for hooks, interceptor, and audit metadata.
        let server = format!("plugin:{plugin_id_str}");

        // Check workspace boundaries
        if let Err(tool_error) = self
            .check_workspace_boundaries(session, call, &server, tool, frontend)
            .await
        {
            return Ok(tool_error);
        }

        // Fire PreToolCall hook
        {
            let ctx = self
                .build_hook_context(session, HookEvent::PreToolCall)
                .with_data("tool_name", serde_json::json!(tool))
                .with_data("server_name", serde_json::json!(server))
                .with_data("arguments", call.arguments.clone());
            let result = self.hooks.trigger_simple(HookEvent::PreToolCall, ctx).await;
            if let astrid_hooks::HookResult::Block { reason } = result {
                return Ok(ToolCallResult::error(&call.id, reason));
            }
            if let astrid_hooks::HookResult::ContinueWith { modifications } = &result {
                debug!(?modifications, "PreToolCall hook modified context");
            }
        }

        // Classify the plugin tool call as a SensitiveAction
        let action = classify_tool_call(&server, tool, &call.arguments);

        // Run through the SecurityInterceptor (same 5-step check as MCP tools).
        // Capture the intercept proof alongside the tool result for accurate auditing.
        let interceptor = self.build_interceptor(session);
        let (tool_result, auth_proof) = match interceptor
            .intercept(&action, &format!("Plugin tool call to {}", call.name), None)
            .await
        {
            Ok(intercept_result) => {
                // Surface budget warning to user
                if let Some(warning) = &intercept_result.budget_warning {
                    frontend.show_status(&format!(
                        "Budget warning: ${:.2}/${:.2} spent ({:.0}%)",
                        warning.current_spend, warning.session_max, warning.percent_used
                    ));
                }

                let proof = intercept_proof_to_auth_proof(
                    &intercept_result.proof,
                    session.user_id,
                    &call.name,
                );

                // Look up the tool under a brief read lock, clone the Arc handle
                // and extract plugin config, then drop the lock before executing.
                // This avoids blocking write-lock callers (load/unload/hot-reload)
                // during potentially slow tool calls.
                let (plugin_tool, plugin_config) = {
                    let registry = registry_lock.read().await;
                    match registry.find_tool(&call.name) {
                        Some((plugin, tool_arc)) => {
                            (Some(tool_arc), plugin.manifest().config.clone())
                        },
                        None => (None, std::collections::HashMap::new()),
                    }
                    // Read lock dropped here.
                };

                let result = match plugin_tool {
                    Some(plugin_tool) => {
                        // Get or create a persistent KV store for this plugin+session.
                        // Keyed by "{session_id}:{server}" so different sessions are
                        // isolated from each other (prevents cross-session data leaks).
                        // MCP plugins ignore the KV context (call peer.call_tool()
                        // directly), but WASM plugins can use it for cross-call state.
                        let plugin_kv = {
                            let kv_key = format!("{}:{server}", session.id);
                            let mut stores = self
                                .plugin_kv_stores
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            Arc::clone(
                                stores
                                    .entry(kv_key)
                                    .or_insert_with(|| Arc::new(MemoryKvStore::new())),
                            )
                        };
                        let scoped_kv =
                            match ScopedKvStore::new(plugin_kv, format!("plugin-tool:{server}")) {
                                Ok(kv) => kv,
                                Err(e) => {
                                    return Ok(ToolCallResult::error(
                                        &call.id,
                                        format!("Internal error creating plugin KV scope: {e}"),
                                    ));
                                },
                            };

                        // Build PluginId from the already-parsed plugin_id_str.
                        let plugin_id = astrid_plugins::PluginId::new(plugin_id_str)
                            .unwrap_or_else(|e| {
                                warn!(
                                    plugin_id_str,
                                    error = %e,
                                    "Failed to parse plugin ID from tool name, using fallback"
                                );
                                astrid_plugins::PluginId::new("unknown").unwrap()
                            });

                        let user_uuid = Self::user_uuid(session.user_id);

                        let tool_ctx = astrid_plugins::PluginToolContext::new(
                            plugin_id,
                            self.config.workspace.root.clone(),
                            scoped_kv,
                        )
                        .with_config(plugin_config)
                        .with_session(session.id.clone())
                        .with_user(user_uuid);

                        match plugin_tool.execute(call.arguments.clone(), &tool_ctx).await {
                            Ok(output) => {
                                let output = astrid_tools::truncate_output(output);
                                ToolCallResult::success(&call.id, output)
                            },
                            Err(e) => {
                                let msg = astrid_tools::truncate_output(e.to_string());
                                ToolCallResult::error(&call.id, msg)
                            },
                        }
                    },
                    None => ToolCallResult::error(
                        &call.id,
                        format!(
                            "Plugin tool not found: {} (plugin may have been unloaded)",
                            call.name
                        ),
                    ),
                };
                (result, proof)
            },
            Err(e) => (
                ToolCallResult::error(&call.id, e.to_string()),
                AuthorizationProof::Denied {
                    reason: e.to_string(),
                },
            ),
        };

        // Audit the plugin tool call
        {
            let outcome = if tool_result.is_error {
                AuditOutcome::failure(&tool_result.content)
            } else {
                AuditOutcome::success()
            };
            let args_hash = astrid_crypto::ContentHash::hash(call.arguments.to_string().as_bytes());
            if let Err(e) = self.audit.append(
                session.id.clone(),
                AuditAction::PluginToolCall {
                    plugin_id: plugin_id_str.to_string(),
                    tool: tool.to_string(),
                    args_hash,
                },
                auth_proof,
                outcome,
            ) {
                warn!(
                    error = %e,
                    tool_name = %call.name,
                    "Failed to audit plugin tool call"
                );
            }
        }

        // Fire PostToolCall or ToolError hook
        {
            let hook_event = if tool_result.is_error {
                HookEvent::ToolError
            } else {
                HookEvent::PostToolCall
            };
            let ctx = self
                .build_hook_context(session, hook_event)
                .with_data("tool_name", serde_json::json!(tool))
                .with_data("server_name", serde_json::json!(server))
                .with_data("is_error", serde_json::json!(tool_result.is_error));
            let _ = self.hooks.trigger_simple(hook_event, ctx).await;
        }

        Ok(tool_result)
    }

    /// Check workspace boundaries for a tool call's file path arguments.
    ///
    /// Returns `Ok(())` if all paths are allowed, or a tool error result if blocked/denied.
    #[allow(clippy::too_many_lines)]
    async fn check_workspace_boundaries<F: Frontend>(
        &self,
        session: &mut AgentSession,
        call: &ToolCall,
        server: &str,
        tool: &str,
        frontend: &F,
    ) -> Result<(), ToolCallResult> {
        let paths = extract_paths_from_args(&call.arguments);
        if paths.is_empty() {
            return Ok(());
        }

        for path in &paths {
            // Check escape handler first (already approved paths)
            if session.escape_handler.is_allowed(path) {
                debug!(path = %path.display(), "Path already approved by escape handler");
                continue;
            }

            let check = self.boundary.check(path);
            match check {
                PathCheck::Allowed | PathCheck::AutoAllowed => {},
                PathCheck::NeverAllowed => {
                    warn!(
                        path = %path.display(),
                        tool = %format!("{server}:{tool}"),
                        "Access to protected path blocked"
                    );

                    // Audit the blocked access
                    {
                        let _ = self.audit.append(
                            session.id.clone(),
                            AuditAction::ApprovalDenied {
                                action: format!("{server}:{tool} -> {}", path.display()),
                                reason: Some("protected system path".to_string()),
                            },
                            AuthorizationProof::System {
                                reason: "workspace boundary: never-allowed path".to_string(),
                            },
                            AuditOutcome::failure("protected path"),
                        );
                    }

                    return Err(ToolCallResult::error(
                        &call.id,
                        format!(
                            "Access to {} is blocked — this is a protected system path",
                            path.display()
                        ),
                    ));
                },
                PathCheck::RequiresApproval => {
                    let escape_request = EscapeRequest::new(
                        path.clone(),
                        infer_operation(tool),
                        format!(
                            "Tool {server}:{tool} wants to access {} outside the workspace",
                            path.display()
                        ),
                    )
                    .with_tool(tool)
                    .with_server(server);

                    // Bridge to frontend approval
                    let approval_request = ApprovalRequest::new(
                        format!("workspace-escape:{server}:{tool}"),
                        format!(
                            "Allow {} {} outside workspace?\n  Path: {}",
                            tool,
                            escape_request.operation,
                            path.display()
                        ),
                    )
                    .with_risk_level(risk_level_for_operation(escape_request.operation))
                    .with_resource(path.display().to_string());

                    let decision =
                        frontend
                            .request_approval(approval_request)
                            .await
                            .map_err(|_| {
                                ToolCallResult::error(
                                    &call.id,
                                    "Failed to request workspace escape approval",
                                )
                            })?;

                    // Convert ApprovalDecision to EscapeDecision
                    let escape_decision = match decision.decision {
                        ApprovalOption::AllowOnce => EscapeDecision::AllowOnce,
                        ApprovalOption::AllowSession | ApprovalOption::AllowWorkspace => {
                            EscapeDecision::AllowSession
                        },
                        ApprovalOption::AllowAlways => EscapeDecision::AllowAlways,
                        ApprovalOption::Deny => EscapeDecision::Deny,
                    };

                    // Record the decision in the escape handler
                    session
                        .escape_handler
                        .process_decision(&escape_request, escape_decision);

                    // Audit the decision
                    if escape_decision.is_allowed() {
                        let _ = self.audit.append(
                            session.id.clone(),
                            AuditAction::ApprovalGranted {
                                action: format!("{server}:{tool}"),
                                resource: Some(path.display().to_string()),
                                scope: match decision.decision {
                                    ApprovalOption::AllowSession => {
                                        astrid_audit::ApprovalScope::Session
                                    },
                                    ApprovalOption::AllowWorkspace => {
                                        astrid_audit::ApprovalScope::Workspace
                                    },
                                    ApprovalOption::AllowAlways => {
                                        astrid_audit::ApprovalScope::Always
                                    },
                                    ApprovalOption::AllowOnce | ApprovalOption::Deny => {
                                        astrid_audit::ApprovalScope::Once
                                    },
                                },
                            },
                            AuthorizationProof::UserApproval {
                                user_id: session.user_id,
                                approval_entry_id: AuditEntryId::new(),
                            },
                            AuditOutcome::success(),
                        );
                    } else {
                        let _ = self.audit.append(
                            session.id.clone(),
                            AuditAction::ApprovalDenied {
                                action: format!("{server}:{tool} -> {}", path.display()),
                                reason: Some(
                                    decision
                                        .reason
                                        .clone()
                                        .unwrap_or_else(|| "user denied".to_string()),
                                ),
                            },
                            AuthorizationProof::UserApproval {
                                user_id: session.user_id,
                                approval_entry_id: AuditEntryId::new(),
                            },
                            AuditOutcome::failure("user denied workspace escape"),
                        );
                    }

                    if !escape_decision.is_allowed() {
                        return Err(ToolCallResult::error(
                            &call.id,
                            decision.reason.unwrap_or_else(|| {
                                format!("Access to {} denied — outside workspace", path.display())
                            }),
                        ));
                    }

                    info!(
                        path = %path.display(),
                        decision = ?escape_decision,
                        "Workspace escape approved"
                    );
                },
            }
        }

        Ok(())
    }
}

/// Extract file paths from tool call JSON arguments.
///
/// Scans for common path-like keys and string values that look like file paths.
fn extract_paths_from_args(args: &serde_json::Value) -> Vec<PathBuf> {
    /// Keys commonly used for file path arguments in MCP tools.
    const PATH_KEYS: &[&str] = &[
        "path",
        "file",
        "file_path",
        "filepath",
        "filename",
        "directory",
        "dir",
        "target",
        "source",
        "destination",
        "src",
        "dst",
        "input",
        "output",
        "uri",
        "url",
        "cwd",
        "working_directory",
    ];

    let mut paths = Vec::new();

    if let Some(obj) = args.as_object() {
        for (key, value) in obj {
            let key_lower = key.to_lowercase();
            if let Some(s) = value.as_str()
                && PATH_KEYS.contains(&key_lower.as_str())
                && let Some(path) = try_extract_path(s)
            {
                paths.push(path);
            }
        }
    }

    paths
}

/// Try to interpret a string value as a file path.
fn try_extract_path(value: &str) -> Option<PathBuf> {
    // Handle file:// URIs
    if let Some(stripped) = value.strip_prefix("file://") {
        return Some(PathBuf::from(stripped));
    }

    // Skip non-file URIs
    if value.contains("://") {
        return None;
    }

    // Check if it looks like an absolute or relative file path
    if value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
    {
        return Some(PathBuf::from(value));
    }

    None
}

/// Infer the operation type from a tool name.
fn infer_operation(tool: &str) -> astrid_workspace::escape::EscapeOperation {
    use astrid_workspace::escape::EscapeOperation;
    let tool_lower = tool.to_lowercase();

    if tool_lower.contains("read") || tool_lower.contains("get") || tool_lower.contains("cat") {
        EscapeOperation::Read
    } else if tool_lower.contains("write")
        || tool_lower.contains("set")
        || tool_lower.contains("put")
        || tool_lower.contains("edit")
        || tool_lower.contains("update")
    {
        EscapeOperation::Write
    } else if tool_lower.contains("create")
        || tool_lower.contains("mkdir")
        || tool_lower.contains("touch")
        || tool_lower.contains("new")
    {
        EscapeOperation::Create
    } else if tool_lower.contains("delete")
        || tool_lower.contains("remove")
        || tool_lower.contains("rm")
    {
        EscapeOperation::Delete
    } else if tool_lower.contains("exec")
        || tool_lower.contains("run")
        || tool_lower.contains("launch")
    {
        EscapeOperation::Execute
    } else if tool_lower.contains("list") || tool_lower.contains("ls") || tool_lower.contains("dir")
    {
        EscapeOperation::List
    } else {
        // Default to Read for unknown operations (least destructive assumption)
        EscapeOperation::Read
    }
}

/// Determine risk level based on the escape operation.
fn risk_level_for_operation(operation: astrid_workspace::escape::EscapeOperation) -> RiskLevel {
    use astrid_workspace::escape::EscapeOperation;
    match operation {
        EscapeOperation::Read | EscapeOperation::List => RiskLevel::Medium,
        EscapeOperation::Write | EscapeOperation::Create => RiskLevel::High,
        EscapeOperation::Delete | EscapeOperation::Execute => RiskLevel::Critical,
    }
}

/// Classify a tool call into a [`SensitiveAction`] for structured approval.
fn classify_tool_call(server: &str, tool: &str, args: &serde_json::Value) -> SensitiveAction {
    let tool_lower = tool.to_lowercase();

    // File delete/remove operations
    if (tool_lower.contains("delete") || tool_lower.contains("remove"))
        && let Some(path) = args
            .get("path")
            .or_else(|| args.get("file"))
            .and_then(|v| v.as_str())
    {
        return SensitiveAction::FileDelete {
            path: path.to_string(),
        };
    }

    // Command execution
    if tool_lower.contains("exec") || tool_lower.contains("run") || tool_lower.contains("bash") {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or(tool)
            .to_string();
        let cmd_args = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        return SensitiveAction::ExecuteCommand {
            command,
            args: cmd_args,
        };
    }

    // File write outside workspace (detected by path args starting with / and outside cwd)
    if tool_lower.contains("write")
        && let Some(path) = args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
        && path.starts_with('/')
    {
        return SensitiveAction::FileWriteOutsideSandbox {
            path: path.to_string(),
        };
    }

    // Default: generic MCP tool call
    SensitiveAction::McpToolCall {
        server: server.to_string(),
        tool: tool.to_string(),
    }
}

/// Convert an [`InterceptProof`] to an [`AuthorizationProof`] for audit logging.
///
/// Maps the interceptor's authorization decision to the audit trail's proof
/// format, preserving the actual authorization mechanism (policy, user approval,
/// capability, or allowance) rather than using a generic `System` proof.
fn intercept_proof_to_auth_proof(
    proof: &astrid_approval::InterceptProof,
    user_id: [u8; 8],
    context: &str,
) -> AuthorizationProof {
    use astrid_approval::InterceptProof;
    match proof {
        InterceptProof::PolicyAllowed => AuthorizationProof::NotRequired {
            reason: format!("policy auto-approved: {context}"),
        },
        InterceptProof::UserApproval { approval_audit_id }
        | InterceptProof::CapabilityCreated {
            approval_audit_id, ..
        } => AuthorizationProof::UserApproval {
            user_id,
            approval_entry_id: approval_audit_id.clone(),
        },
        InterceptProof::SessionApproval { allowance_id } => AuthorizationProof::NotRequired {
            reason: format!("session-scoped allowance {allowance_id}: {context}"),
        },
        InterceptProof::WorkspaceApproval { allowance_id } => AuthorizationProof::NotRequired {
            reason: format!("workspace-scoped allowance {allowance_id}: {context}"),
        },
        InterceptProof::Capability { token_id } => AuthorizationProof::Capability {
            token_id: token_id.clone(),
            // InterceptProof only carries the token_id, not the full token bytes.
            // Hash the token_id string as a deterministic fingerprint so the audit
            // entry is at least tied to a specific token, even though we cannot
            // compute the true content hash without the full token.
            token_hash: astrid_crypto::ContentHash::hash(token_id.to_string().as_bytes()),
        },
        InterceptProof::Allowance { .. } => AuthorizationProof::NotRequired {
            reason: format!("pre-existing allowance: {context}"),
        },
    }
}

/// Convert an internal approval request to a frontend-facing [`ApprovalRequest`].
fn to_frontend_request(internal: &InternalApprovalRequest) -> ApprovalRequest {
    ApprovalRequest::new(
        internal.action.action_type().to_string(),
        internal.action.summary(),
    )
    .with_risk_level(internal.assessment.level)
    .with_resource(format!("{}", internal.action))
}

/// Convert a frontend [`ApprovalDecision`] to an internal [`ApprovalResponse`].
fn to_internal_response(
    request: &InternalApprovalRequest,
    decision: &ApprovalDecision,
) -> InternalApprovalResponse {
    let internal_decision = match decision.decision {
        ApprovalOption::AllowOnce => InternalApprovalDecision::Approve,
        ApprovalOption::AllowSession => InternalApprovalDecision::ApproveSession,
        ApprovalOption::AllowWorkspace => InternalApprovalDecision::ApproveWorkspace,
        ApprovalOption::AllowAlways => InternalApprovalDecision::ApproveAlways,
        ApprovalOption::Deny => InternalApprovalDecision::Deny {
            reason: decision
                .reason
                .clone()
                .unwrap_or_else(|| "denied by user".to_string()),
        },
    };
    InternalApprovalResponse::new(request.id.clone(), internal_decision)
}

/// Classify a built-in tool call into a [`SensitiveAction`].
///
/// Every tool — including read-only ones — goes through the interceptor because
/// even reads can expose sensitive data (credentials, private keys, PII).
fn classify_builtin_tool_call(tool_name: &str, args: &serde_json::Value) -> SensitiveAction {
    match tool_name {
        "bash" => {
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("bash")
                .to_string();
            SensitiveAction::ExecuteCommand {
                command,
                args: Vec::new(),
            }
        },
        "write_file" | "edit_file" => {
            let path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            SensitiveAction::FileWriteOutsideSandbox { path }
        },
        "read_file" | "glob" | "grep" | "list_directory" => {
            let path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .or_else(|| args.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or(".")
                .to_string();
            SensitiveAction::FileRead { path }
        },
        // Unknown built-in tool — treat as MCP tool call requiring approval
        other => SensitiveAction::McpToolCall {
            server: "builtin".to_string(),
            tool: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// FrontendApprovalHandler — bridges Frontend::request_approval() to ApprovalHandler
// ---------------------------------------------------------------------------

/// Adapter that bridges a [`Frontend`] to the [`ApprovalHandler`] trait
/// used internally by the approval system.
struct FrontendApprovalHandler<F: Frontend> {
    frontend: Arc<F>,
}

#[async_trait]
impl<F: Frontend> ApprovalHandler for FrontendApprovalHandler<F> {
    async fn request_approval(
        &self,
        request: InternalApprovalRequest,
    ) -> Option<InternalApprovalResponse> {
        let frontend_request = to_frontend_request(&request);
        match self.frontend.request_approval(frontend_request).await {
            Ok(decision) => Some(to_internal_response(&request, &decision)),
            Err(_) => None,
        }
    }

    fn is_available(&self) -> bool {
        true
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
fn tokens_to_usd(input_tokens: usize, output_tokens: usize) -> f64 {
    let input_cost = (input_tokens as f64 / 1000.0) * INPUT_RATE_PER_1K;
    let output_cost = (output_tokens as f64 / 1000.0) * OUTPUT_RATE_PER_1K;
    input_cost + output_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_paths_from_args() {
        let args = serde_json::json!({
            "path": "/home/user/file.txt",
            "content": "some data",
            "count": 42
        });
        let paths = extract_paths_from_args(&args);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("/home/user/file.txt"));
    }

    #[test]
    fn test_extract_paths_ignores_non_path_values() {
        let args = serde_json::json!({
            "path": "not-a-path",
            "url": "https://example.com",
        });
        let paths = extract_paths_from_args(&args);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_extract_paths_file_uri() {
        let args = serde_json::json!({
            "uri": "file:///tmp/test.txt"
        });
        let paths = extract_paths_from_args(&args);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("/tmp/test.txt"));
    }

    #[test]
    fn test_extract_paths_relative() {
        let args = serde_json::json!({
            "file": "./src/main.rs",
            "dir": "../other"
        });
        let paths = extract_paths_from_args(&args);
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn test_infer_operation() {
        use astrid_workspace::escape::EscapeOperation;
        assert_eq!(infer_operation("read_file"), EscapeOperation::Read);
        assert_eq!(infer_operation("write_file"), EscapeOperation::Write);
        assert_eq!(infer_operation("create_directory"), EscapeOperation::Create);
        assert_eq!(infer_operation("delete_file"), EscapeOperation::Delete);
        assert_eq!(infer_operation("execute_command"), EscapeOperation::Execute);
        assert_eq!(infer_operation("list_files"), EscapeOperation::List);
        assert_eq!(infer_operation("unknown_tool"), EscapeOperation::Read);
    }

    #[test]
    fn test_risk_level_for_operation() {
        use astrid_workspace::escape::EscapeOperation;
        assert_eq!(
            risk_level_for_operation(EscapeOperation::Read),
            RiskLevel::Medium
        );
        assert_eq!(
            risk_level_for_operation(EscapeOperation::Write),
            RiskLevel::High
        );
        assert_eq!(
            risk_level_for_operation(EscapeOperation::Delete),
            RiskLevel::Critical
        );
    }

    #[test]
    fn test_intercept_proof_to_auth_proof_policy_allowed() {
        use astrid_approval::InterceptProof;
        let proof = intercept_proof_to_auth_proof(
            &InterceptProof::PolicyAllowed,
            [1; 8],
            "plugin:test:echo",
        );
        match proof {
            AuthorizationProof::NotRequired { reason } => {
                assert!(reason.contains("policy auto-approved"));
                assert!(reason.contains("plugin:test:echo"));
            },
            other => panic!("expected NotRequired, got {other:?}"),
        }
    }

    #[test]
    fn test_intercept_proof_to_auth_proof_user_approval() {
        use astrid_approval::InterceptProof;
        let audit_id = AuditEntryId::new();
        let proof = intercept_proof_to_auth_proof(
            &InterceptProof::UserApproval {
                approval_audit_id: audit_id.clone(),
            },
            [2; 8],
            "ctx",
        );
        match proof {
            AuthorizationProof::UserApproval {
                user_id,
                approval_entry_id,
            } => {
                assert_eq!(user_id, [2; 8]);
                assert_eq!(approval_entry_id, audit_id);
            },
            other => panic!("expected UserApproval, got {other:?}"),
        }
    }

    #[test]
    fn test_intercept_proof_to_auth_proof_session_approval() {
        use astrid_approval::InterceptProof;
        let proof = intercept_proof_to_auth_proof(
            &InterceptProof::SessionApproval {
                allowance_id: astrid_approval::AllowanceId::new(),
            },
            [3; 8],
            "ctx",
        );
        match proof {
            AuthorizationProof::NotRequired { reason } => {
                assert!(reason.contains("session-scoped allowance"));
            },
            other => panic!("expected NotRequired for session approval, got {other:?}"),
        }
    }

    #[test]
    fn test_intercept_proof_to_auth_proof_workspace_approval() {
        use astrid_approval::InterceptProof;
        let proof = intercept_proof_to_auth_proof(
            &InterceptProof::WorkspaceApproval {
                allowance_id: astrid_approval::AllowanceId::new(),
            },
            [4; 8],
            "ctx",
        );
        match proof {
            AuthorizationProof::NotRequired { reason } => {
                assert!(reason.contains("workspace-scoped allowance"));
            },
            other => panic!("expected NotRequired for workspace approval, got {other:?}"),
        }
    }

    #[test]
    fn test_intercept_proof_to_auth_proof_capability() {
        use astrid_approval::InterceptProof;
        let token_id = astrid_core::TokenId::new();
        let proof = intercept_proof_to_auth_proof(
            &InterceptProof::Capability {
                token_id: token_id.clone(),
            },
            [5; 8],
            "ctx",
        );
        match proof {
            AuthorizationProof::Capability {
                token_id: id,
                token_hash,
            } => {
                assert_eq!(id, token_id);
                // Hash should be derived from token_id string, not empty bytes.
                let expected = astrid_crypto::ContentHash::hash(token_id.to_string().as_bytes());
                assert_eq!(token_hash, expected);
            },
            other => panic!("expected Capability, got {other:?}"),
        }
    }

    #[test]
    fn test_intercept_proof_to_auth_proof_allowance() {
        use astrid_approval::InterceptProof;
        let proof = intercept_proof_to_auth_proof(
            &InterceptProof::Allowance {
                allowance_id: astrid_approval::AllowanceId::new(),
            },
            [6; 8],
            "plugin:test:echo",
        );
        match proof {
            AuthorizationProof::NotRequired { reason } => {
                assert!(reason.contains("pre-existing allowance"));
                assert!(reason.contains("plugin:test:echo"));
            },
            other => panic!("expected NotRequired for allowance, got {other:?}"),
        }
    }
}
