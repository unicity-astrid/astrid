//! Agent loop: `run_turn_streaming`, `run_subagent_turn`, and `run_loop`.

use astrid_approval::manager::ApprovalHandler;
use astrid_audit::{AuditAction, AuditOutcome, AuthorizationProof};
use astrid_core::Frontend;
use astrid_hooks::{HookEvent, HookResult};
use astrid_llm::{LlmProvider, LlmToolDefinition, Message, StreamEvent, ToolCall};
use astrid_tools::ToolContext;
use futures::StreamExt;
use std::sync::Arc;
use tracing::{debug, error};

use crate::error::{RuntimeError, RuntimeResult};
use crate::session::AgentSession;
use crate::subagent::SubAgentId;

use super::security::FrontendApprovalHandler;
use super::{AgentRuntime, tokens_to_usd};

impl<P: LlmProvider + 'static> AgentRuntime<P> {
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
            if let HookResult::Block { reason } = result {
                return Err(RuntimeError::ApprovalDenied { reason });
            }
            if let HookResult::ContinueWith { modifications } = &result {
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

        // Collect agent context from plugins if not already collected this turn.
        // It is held in session.plugin_context and dynamically injected into the prompt.
        #[allow(clippy::collapsible_if)]
        if session.plugin_context.is_none() {
            if let Some(ref registry_lock) = self.capsule_registry {
                let mut combined_context = String::new();
                let active_plugins: Vec<astrid_capsule::capsule::CapsuleId> = {
                    let registry = registry_lock.read().await;
                    registry.list().into_iter().cloned().collect()
                };

                for plugin_id in active_plugins {
                    // Discover if it exposes the context tool
                    let (tool_arc, _tool_config) = {
                        let registry = registry_lock.read().await;
                        let tool_name = format!("plugin:{plugin_id}:__astrid_get_agent_context");
                        match registry.find_tool(&tool_name) {
                            Some((plugin, t)) => {
                                let config = plugin
                                    .manifest()
                                    .env
                                    .iter()
                                    .filter_map(|(k, v)| v.default.clone().map(|d| (k.clone(), d)))
                                    .collect();
                                (Some(t), config)
                            },
                            None => (None, std::collections::HashMap::new()),
                        }
                    };

                    // Execute the tool if present with a 5-second timeout
                    if let Some(tool) = tool_arc {
                        let plugin_kv =
                            {
                                let kv_key = format!("{}:plugin:{plugin_id}", session.id);
                                let mut stores = self
                                    .plugin_kv_stores
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                Arc::clone(stores.entry(kv_key).or_insert_with(|| {
                                    Arc::new(astrid_storage::MemoryKvStore::new())
                                }))
                            };

                        let scoped_name = format!("plugin-tool:plugin:{plugin_id}");
                        if let Ok(scoped_kv) =
                            astrid_storage::ScopedKvStore::new(plugin_kv, scoped_name)
                        {
                            let user_uuid = Self::user_uuid(session.user_id);
                            let tool_ctx = astrid_capsule::context::CapsuleToolContext::new(
                                plugin_id.clone(),
                                self.config.workspace.root.clone(),
                                scoped_kv,
                            )
                            // .with_config(tool_config) // Context tools do not take config directly in capsule implementation
                            .with_session(session.id.clone())
                            .with_user(user_uuid);

                            let execute_future = tool.execute(
                                serde_json::Value::Object(serde_json::Map::default()),
                                &tool_ctx,
                            );
                            if let Ok(Ok(ctx_result)) = tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                execute_future,
                            )
                            .await
                            {
                                let trimmed = ctx_result.trim();
                                if !trimmed.is_empty() {
                                    combined_context.push_str(trimmed);
                                    combined_context.push_str("\n\n");
                                }
                            } else {
                                tracing::warn!(%plugin_id, "Context tool execution timed out or failed");
                            }
                        }
                    }
                }

                if combined_context.is_empty() {
                    session.plugin_context = Some(String::new()); // Mark as collected but empty
                } else {
                    session.plugin_context = Some(combined_context);
                }
            }
        }

        // Create per-turn ToolContext (shares cwd, owns its own spawner slot)
        let tool_ctx = ToolContext::with_shared_cwd(
            self.config.workspace.root.clone(),
            Arc::clone(&self.shared_cwd),
            self.config.spark_file.clone(),
        );

        // Inject sub-agent spawner (if self_arc is available)
        self.inject_subagent_spawner(&tool_ctx, session, &frontend, None)
            .await;

        // Run the agentic loop (tool_ctx is dropped at turn end — no cleanup needed)
        let loop_result = self.run_loop(session, &*frontend, &tool_ctx).await;

        let save_result = self.sessions.save(session);

        loop_result?;
        save_result?;

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
        parent_subagent_id: Option<SubAgentId>,
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
            self.config.spark_file.clone(),
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
    pub(super) async fn run_loop<F: Frontend>(
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

            // Capsule tools (snapshot under a brief read lock).
            if let Some(ref registry) = self.capsule_registry {
                let registry = registry.read().await;
                llm_tools.extend(registry.all_tool_definitions().into_iter().map(|td| {
                    LlmToolDefinition::new(td.name)
                        .with_description(td.description)
                        .with_schema(td.input_schema)
                }));
            }

            // Re-read spark for hot-reload (cheap: ~1KB file read per loop iteration).
            // Sub-agents skip this: their identity is baked into session.system_prompt
            // by SubAgentExecutor to avoid contradictory double injection.
            let mut effective_prompt = if session.is_subagent {
                session.system_prompt.clone()
            } else if let Some(spark) = self.read_effective_spark() {
                if let Some(preamble) = spark.build_preamble() {
                    format!("{preamble}\n\n{}", session.system_prompt)
                } else {
                    session.system_prompt.clone()
                }
            } else {
                session.system_prompt.clone()
            };

            // Inject dynamic plugin context if present
            if let Some(ctx) = session.plugin_context.as_ref().filter(|c| !c.is_empty()) {
                effective_prompt = format!("{ctx}\n\n{effective_prompt}");
            }

            // Stream from LLM
            let mut stream = self
                .llm
                .stream(&session.messages, &llm_tools, &effective_prompt)
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
}
