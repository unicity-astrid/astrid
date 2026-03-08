#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![warn(missing_docs)]

//! Hook Bridge capsule — maps lifecycle events to semantic hooks.
//!
//! The kernel dispatches lifecycle events (e.g. `tool_call_started`,
//! `session_created`) to this capsule via interceptors. The Hook Bridge
//! maps each event to a semantic hook name, calls `hooks::trigger` to
//! fan out to all subscriber capsules, and applies merge strategies to
//! the collected responses.
//!
//! # Architecture
//!
//! ```text
//! Kernel EventBus → EventDispatcher → Hook Bridge (this capsule)
//!                                        ↓ hooks::trigger("before_tool_call", payload)
//!                                     Kernel astrid_trigger_hook host fn
//!                                        ↓ iterates CapsuleRegistry
//!                                     Subscriber capsule A, B, C...
//!                                        ↑ collect responses
//!                                     Hook Bridge applies merge strategy
//! ```
//!
//! This is a **policy** capsule: it defines which events map to which
//! hooks and how responses are merged. The **mechanism** (fan-out and
//! response collection) lives in the kernel's `astrid_trigger_hook`
//! host function.

use astrid_sdk::prelude::*;
use serde::Serialize;

// ── Merge Semantics ────────────────────────────────────────────────

/// How interceptor responses are merged for a hook.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MergeSemantics {
    /// Fire-and-forget: responses are discarded.
    None,
    /// `before_tool_call` specific: any `skip: true` → skip,
    /// last non-null `modified_params` wins.
    ToolCallBefore,
    /// Last non-null value for the named field wins.
    LastNonNull { field: &'static str },
}

/// A mapping from a lifecycle event to a hook name and merge strategy.
struct HookMapping {
    hook_name: &'static str,
    merge: MergeSemantics,
}

// ── Hook Trigger Protocol ──────────────────────────────────────────

/// Request payload sent to `hooks::trigger` (consumed by the kernel's
/// `astrid_trigger_hook` host function).
#[derive(Serialize)]
struct TriggerRequest<'a> {
    hook: &'a str,
    payload: &'a serde_json::Value,
}

/// Merged result from hook fan-out.
#[derive(Serialize)]
struct HookResult {
    /// Whether the operation should be skipped (ToolCallBefore semantics).
    #[serde(skip_serializing_if = "Option::is_none")]
    skip: Option<bool>,
    /// Merged data from interceptor responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

// ── Event-to-Hook Mapping Table ────────────────────────────────────

/// Resolve the hook mapping for a given event type string.
///
/// Returns `None` for events that have no corresponding hook.
fn mapping_for_event(event_type: &str) -> Option<HookMapping> {
    match event_type {
        // Session lifecycle
        "session_created" => Some(HookMapping {
            hook_name: "session_start",
            merge: MergeSemantics::None,
        }),
        "session_ended" => Some(HookMapping {
            hook_name: "session_end",
            merge: MergeSemantics::None,
        }),

        // Tool hooks
        "tool_call_started" => Some(HookMapping {
            hook_name: "before_tool_call",
            merge: MergeSemantics::ToolCallBefore,
        }),
        "tool_call_completed" => Some(HookMapping {
            hook_name: "after_tool_call",
            merge: MergeSemantics::LastNonNull {
                field: "modified_result",
            },
        }),
        "tool_result_persisting" => Some(HookMapping {
            hook_name: "tool_result_persist",
            merge: MergeSemantics::LastNonNull {
                field: "transformed_result",
            },
        }),

        // Message hooks
        "message_received" => Some(HookMapping {
            hook_name: "message_received",
            merge: MergeSemantics::None,
        }),
        "message_sending" => Some(HookMapping {
            hook_name: "message_sending",
            merge: MergeSemantics::LastNonNull {
                field: "modified_content",
            },
        }),
        "message_sent" => Some(HookMapping {
            hook_name: "message_sent",
            merge: MergeSemantics::None,
        }),

        // Sub-agent hooks
        "sub_agent_spawned" => Some(HookMapping {
            hook_name: "subagent_start",
            merge: MergeSemantics::None,
        }),
        "sub_agent_completed" | "sub_agent_failed" | "sub_agent_cancelled" => Some(HookMapping {
            hook_name: "subagent_stop",
            merge: MergeSemantics::None,
        }),

        // Context compaction (broadcast-only observation hooks)
        "context_compaction_started" => Some(HookMapping {
            hook_name: "on_compaction_started",
            merge: MergeSemantics::None,
        }),
        "context_compaction_completed" => Some(HookMapping {
            hook_name: "on_compaction_completed",
            merge: MergeSemantics::None,
        }),

        // Kernel lifecycle
        "kernel_started" => Some(HookMapping {
            hook_name: "kernel_start",
            merge: MergeSemantics::None,
        }),
        "kernel_shutdown" => Some(HookMapping {
            hook_name: "kernel_stop",
            merge: MergeSemantics::None,
        }),

        _ => Option::None,
    }
}

// ── Merge Logic ────────────────────────────────────────────────────

/// Apply merge semantics to a list of interceptor responses.
fn apply_merge(merge: &MergeSemantics, responses: &[serde_json::Value]) -> HookResult {
    match merge {
        MergeSemantics::None => HookResult {
            skip: Option::None,
            data: Option::None,
        },

        MergeSemantics::ToolCallBefore => {
            let mut skip = false;
            let mut last_params: Option<serde_json::Value> = Option::None;

            for resp in responses {
                // Any response with skip: true wins
                if resp.get("skip").and_then(|v| v.as_bool()).unwrap_or(false) {
                    skip = true;
                }
                // Last non-null modified_params wins
                if let Some(params) = resp.get("modified_params")
                    && !params.is_null()
                {
                    last_params = Some(params.clone());
                }
            }

            HookResult {
                skip: if skip { Some(true) } else { Option::None },
                data: last_params,
            }
        },

        MergeSemantics::LastNonNull { field } => {
            let mut last_value: Option<serde_json::Value> = Option::None;

            for resp in responses {
                if let Some(val) = resp.get(*field)
                    && !val.is_null()
                {
                    last_value = Some(val.clone());
                }
            }

            HookResult {
                skip: Option::None,
                data: last_value,
            }
        },
    }
}

// ── Core Dispatch ──────────────────────────────────────────────────

/// Dispatch a lifecycle event through the hook system.
///
/// 1. Look up the event-to-hook mapping
/// 2. Call `hooks::trigger` to fan out to subscriber capsules
/// 3. Apply merge strategy to collected responses
/// 4. Return the merged result
fn dispatch_hook(event_type: &str, payload: &serde_json::Value) -> Result<Vec<u8>, SysError> {
    let Some(mapping) = mapping_for_event(event_type) else {
        // No hook mapping for this event — nothing to do.
        return Ok(Vec::new());
    };

    let request = TriggerRequest {
        hook: mapping.hook_name,
        payload,
    };
    let request_bytes = serde_json::to_vec(&request)
        .map_err(|e| SysError::ApiError(format!("failed to serialize trigger request: {e}")))?;

    let response_bytes = hooks::trigger(&request_bytes)?;

    // Parse the response array from the kernel.
    let responses: Vec<serde_json::Value> =
        serde_json::from_slice(&response_bytes).unwrap_or_default();

    if responses.is_empty() && matches!(mapping.merge, MergeSemantics::None) {
        return Ok(Vec::new());
    }

    let result = apply_merge(&mapping.merge, &responses);
    serde_json::to_vec(&result)
        .map_err(|e| SysError::ApiError(format!("failed to serialize hook result: {e}")))
}

// ── Capsule Implementation ─────────────────────────────────────────

/// Hook Bridge capsule.
///
/// Maps lifecycle events to semantic hooks, fans out to subscribers via
/// `hooks::trigger`, and applies merge strategies to the responses.
#[derive(Default)]
pub struct HookBridge;

/// Extract event type and dispatch the hook. Used by all interceptor handlers.
fn handle_lifecycle(event_type: &str, payload: serde_json::Value) -> Result<Vec<u8>, SysError> {
    dispatch_hook(event_type, &payload)
}

#[capsule]
impl HookBridge {
    // ── Session lifecycle ──

    /// Handle `session_created` lifecycle event.
    #[astrid::interceptor("on_session_created")]
    pub fn on_session_created(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("session_created", payload)?;
        Ok(())
    }

    /// Handle `session_ended` lifecycle event.
    #[astrid::interceptor("on_session_ended")]
    pub fn on_session_ended(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("session_ended", payload)?;
        Ok(())
    }

    // ── Tool hooks ──

    /// Handle `tool_call_started` — maps to `before_tool_call` hook.
    ///
    /// Returns merged result with potential skip/modified_params.
    #[astrid::interceptor("on_tool_call_started")]
    pub fn on_tool_call_started(&self, payload: serde_json::Value) -> Result<Vec<u8>, SysError> {
        handle_lifecycle("tool_call_started", payload)
    }

    /// Handle `tool_call_completed` — maps to `after_tool_call` hook.
    #[astrid::interceptor("on_tool_call_completed")]
    pub fn on_tool_call_completed(&self, payload: serde_json::Value) -> Result<Vec<u8>, SysError> {
        handle_lifecycle("tool_call_completed", payload)
    }

    /// Handle `tool_result_persisting` — maps to `tool_result_persist` hook.
    #[astrid::interceptor("on_tool_result_persisting")]
    pub fn on_tool_result_persisting(
        &self,
        payload: serde_json::Value,
    ) -> Result<Vec<u8>, SysError> {
        handle_lifecycle("tool_result_persisting", payload)
    }

    // ── Message hooks ──

    /// Handle `message_received` lifecycle event.
    #[astrid::interceptor("on_message_received")]
    pub fn on_message_received(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("message_received", payload)?;
        Ok(())
    }

    /// Handle `message_sending` — maps to `message_sending` hook.
    #[astrid::interceptor("on_message_sending")]
    pub fn on_message_sending(&self, payload: serde_json::Value) -> Result<Vec<u8>, SysError> {
        handle_lifecycle("message_sending", payload)
    }

    /// Handle `message_sent` lifecycle event.
    #[astrid::interceptor("on_message_sent")]
    pub fn on_message_sent(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("message_sent", payload)?;
        Ok(())
    }

    // ── Sub-agent hooks ──

    /// Handle `sub_agent_spawned` lifecycle event.
    #[astrid::interceptor("on_subagent_spawned")]
    pub fn on_subagent_spawned(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("sub_agent_spawned", payload)?;
        Ok(())
    }

    /// Handle `sub_agent_completed` lifecycle event.
    #[astrid::interceptor("on_subagent_completed")]
    pub fn on_subagent_completed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("sub_agent_completed", payload)?;
        Ok(())
    }

    /// Handle `sub_agent_failed` lifecycle event.
    #[astrid::interceptor("on_subagent_failed")]
    pub fn on_subagent_failed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("sub_agent_failed", payload)?;
        Ok(())
    }

    /// Handle `sub_agent_cancelled` lifecycle event.
    #[astrid::interceptor("on_subagent_cancelled")]
    pub fn on_subagent_cancelled(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("sub_agent_cancelled", payload)?;
        Ok(())
    }

    // ── Context compaction ──

    /// Handle `context_compaction_started` lifecycle event.
    #[astrid::interceptor("on_compaction_started")]
    pub fn on_compaction_started(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("context_compaction_started", payload)?;
        Ok(())
    }

    /// Handle `context_compaction_completed` lifecycle event.
    #[astrid::interceptor("on_compaction_completed")]
    pub fn on_compaction_completed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("context_compaction_completed", payload)?;
        Ok(())
    }

    // ── Kernel lifecycle ──

    /// Handle `kernel_started` lifecycle event.
    #[astrid::interceptor("on_kernel_started")]
    pub fn on_kernel_started(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("kernel_started", payload)?;
        Ok(())
    }

    /// Handle `kernel_shutdown` lifecycle event.
    #[astrid::interceptor("on_kernel_shutdown")]
    pub fn on_kernel_shutdown(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("kernel_shutdown", payload)?;
        Ok(())
    }
}
