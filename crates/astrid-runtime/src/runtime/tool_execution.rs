//! Tool call dispatch: built-in, MCP, and capsule tools.

use astrid_approval::SensitiveAction;
use astrid_audit::{AuditAction, AuditOutcome, AuthorizationProof};
use astrid_core::Frontend;
use astrid_hooks::HookEvent;
use astrid_llm::{LlmProvider, ToolCall, ToolCallResult};
use astrid_storage::{MemoryKvStore, ScopedKvStore};
use astrid_tools::{ToolContext, ToolRegistry, truncate_output};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::error::RuntimeResult;
use crate::session::AgentSession;

use super::AgentRuntime;
use super::security::{
    classify_builtin_tool_call, classify_tool_call, intercept_proof_to_auth_proof,
};

impl<P: LlmProvider + 'static> AgentRuntime<P> {
    /// Execute a tool call with security checks via the `SecurityInterceptor`.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn execute_tool_call<F: Frontend>(
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

        // Check for capsule tool (capsule:{capsule_id}:{tool_name})
        if astrid_capsule::registry::CapsuleRegistry::is_capsule_tool(&call.name) {
            return self.execute_capsule_tool(session, call, frontend).await;
        }

        let (server, tool) = call.parse_name().ok_or_else(|| {
            crate::error::RuntimeError::McpError(astrid_mcp::McpError::ToolNotFound {
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

    /// Execute a built-in tool with workspace boundary checks, interceptor, and hooks.
    pub(super) async fn execute_builtin_tool<F: Frontend>(
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

    /// Execute a capsule tool with security checks, interceptor, and hooks.
    ///
    /// Plugin tool names follow the format `plugin:{capsule_id}:{tool_name}`.
    /// The qualified name is used as-is for `PluginRegistry::find_tool()`.
    #[allow(clippy::too_many_lines)]
    pub(super) async fn execute_capsule_tool<F: Frontend>(
        &self,
        session: &mut AgentSession,
        call: &ToolCall,
        frontend: &F,
    ) -> RuntimeResult<ToolCallResult> {
        let Some(ref registry_lock) = self.capsule_registry else {
            return Ok(ToolCallResult::error(
                &call.id,
                "Plugin tools are not available (no plugin registry configured)",
            ));
        };

        // Parse the qualified name into capsule ID and tool name.
        let (capsule_id_str, tool) = match call.name.strip_prefix("capsule:") {
            Some(rest) => match rest.split_once(':') {
                Some((id, tool_name)) => (id, tool_name),
                None => {
                    return Ok(ToolCallResult::error(
                        &call.id,
                        format!(
                            "Malformed capsule tool name (missing tool segment): {}",
                            call.name
                        ),
                    ));
                },
            },
            None => {
                return Ok(ToolCallResult::error(
                    &call.id,
                    format!(
                        "Malformed capsule tool name (missing capsule: prefix): {}",
                        call.name
                    ),
                ));
            },
        };
        // Server-like prefix used for hooks, interceptor, and audit metadata.
        let server = format!("capsule:{capsule_id_str}");

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

        // Classify the capsule tool call as a CapsuleExecution (not McpToolCall).
        // This routes through SecurityPolicy::check_plugin_action, which checks
        // blocked_plugins and always requires approval — more appropriate than
        // the generic MCP tool classification.
        let action = SensitiveAction::CapsuleExecution {
            capsule_id: capsule_id_str.to_string(),
            capability: tool.to_string(),
        };

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
                let (capsule_tool, _plugin_config) = {
                    let registry = registry_lock.read().await;
                    match registry.find_tool(&call.name) {
                        Some((plugin, tool_arc)) => {
                            let config = plugin
                                .manifest()
                                .env
                                .iter()
                                .filter_map(|(k, v)| v.default.clone().map(|d| (k.clone(), d)))
                                .collect();
                            (Some(tool_arc), config)
                        },
                        None => (None, std::collections::HashMap::new()),
                    }
                    // Read lock dropped here.
                };

                let result = match capsule_tool {
                    Some(capsule_tool) => {
                        // Get or create a persistent KV store for this plugin+session.
                        // Keyed by "{session_id}:{server}" so different sessions are
                        // isolated from each other (prevents cross-session data leaks).
                        // MCP plugins ignore the KV context (call peer.call_tool()
                        // directly), but WASM plugins can use it for cross-call state.
                        let plugin_kv = {
                            let kv_key = format!("{}:{server}", session.id);
                            // SAFETY: no .await while this std::sync::Mutex lock is held.
                            // The critical section is a synchronous HashMap lookup/insert.
                            let mut stores = self
                                .capsule_kv_stores
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

                        let capsule_id = astrid_capsule::capsule::CapsuleId::new(capsule_id_str)
                            .expect("capsule_id_str already validated by find_tool");

                        let user_uuid = Self::user_uuid(session.user_id);

                        let tool_ctx = astrid_capsule::context::CapsuleToolContext::new(
                            capsule_id,
                            self.config.workspace.root.clone(),
                            scoped_kv,
                        )
                        // .with_config(plugin_config)
                        .with_session(session.id.clone())
                        .with_user(user_uuid);

                        match capsule_tool
                            .execute(call.arguments.clone(), &tool_ctx)
                            .await
                        {
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

        // Audit the capsule tool call.
        // Note: the interceptor also writes an authorization-level audit entry.
        // This explicit entry records richer metadata (capsule_id, tool, args_hash)
        // and the execution outcome (success/failure) — complementary, not redundant.
        {
            let outcome = if tool_result.is_error {
                AuditOutcome::failure(&tool_result.content)
            } else {
                AuditOutcome::success()
            };
            let args_hash = astrid_crypto::ContentHash::hash(call.arguments.to_string().as_bytes());
            if let Err(e) = self.audit.append(
                session.id.clone(),
                AuditAction::CapsuleToolCall {
                    capsule_id: capsule_id_str.to_string(),
                    tool: tool.to_string(),
                    args_hash,
                },
                auth_proof,
                outcome,
            ) {
                warn!(
                    error = %e,
                    tool_name = %call.name,
                    "Failed to audit capsule tool call"
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
}
