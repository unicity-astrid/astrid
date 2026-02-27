#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![warn(missing_docs)]

//! Tool router capsule for Astrid OS.
//!
//! Receives `tool.request.execute` events from the orchestrator, validates
//! the tool name, forwards the request to the appropriate tool capsule's
//! topic, and routes results back to `tool.execute.result`.

use astrid_events::ipc::IpcPayload;
use astrid_events::llm::ToolCallResult;
use astrid_sdk::prelude::*;

/// Tool router capsule. Stateless middleware.
#[derive(Default)]
pub struct ToolRouter;

#[capsule]
impl ToolRouter {
    /// Handles `tool.request.execute` events from the orchestrator.
    ///
    /// Validates the tool name, builds the forward topic, and publishes the
    /// request to the specific tool capsule. If the tool name is invalid or
    /// the publish fails, returns an error result to the orchestrator.
    #[astrid::interceptor("handle_execute_request")]
    pub fn handle_execute_request(&self, req: IpcPayload) -> Result<(), SysError> {
        let (call_id, tool_name, arguments) = match req {
            IpcPayload::ToolExecuteRequest {
                call_id,
                tool_name,
                arguments,
            } => (call_id, tool_name, arguments),
            _ => return Ok(()),
        };

        // Validate tool name: must be non-empty, alphanumeric with hyphens/underscores/colons.
        // Reject dots to prevent topic injection (e.g., "foo.bar.baz" becoming nested topics).
        if tool_name.is_empty()
            || tool_name
                .chars()
                .any(|c| !c.is_alphanumeric() && c != '-' && c != '_' && c != ':')
        {
            let _ = sys::log(
                "warn",
                format!("Rejected invalid tool name: {tool_name}"),
            );
            return Self::publish_error_result(
                &call_id,
                format!("Invalid tool name: {tool_name}"),
            );
        }

        let forward_topic = format!("tool.execute.{tool_name}");

        let _ = sys::log("info", format!("Routing tool request: {tool_name} -> {forward_topic}"));

        let forward_payload = IpcPayload::ToolExecuteRequest {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            arguments,
        };

        if let Err(e) = ipc::publish_json(&forward_topic, &forward_payload) {
            let _ = sys::log(
                "error",
                format!("Failed to forward tool request for {tool_name}: {e}"),
            );
            return Self::publish_error_result(
                &call_id,
                format!("Failed to route tool request: {e}"),
            );
        }

        Ok(())
    }

    /// Handles tool execution results from tool capsules.
    ///
    /// Forwards the result back to the orchestrator via `tool.execute.result`.
    #[astrid::interceptor("handle_execute_result")]
    pub fn handle_execute_result(&self, res: IpcPayload) -> Result<(), SysError> {
        let (call_id, result) = match res {
            IpcPayload::ToolExecuteResult { call_id, result } => (call_id, result),
            _ => return Ok(()),
        };

        let _ = sys::log(
            "info",
            format!("Routing tool result for call_id: {call_id}"),
        );

        ipc::publish_json(
            "tool.execute.result",
            &IpcPayload::ToolExecuteResult { call_id, result },
        )
    }
}

impl ToolRouter {
    /// Publish an error result back to the orchestrator for a failed tool dispatch.
    fn publish_error_result(call_id: &str, error_message: String) -> Result<(), SysError> {
        ipc::publish_json(
            "tool.execute.result",
            &IpcPayload::ToolExecuteResult {
                call_id: call_id.to_string(),
                result: ToolCallResult {
                    call_id: call_id.to_string(),
                    content: error_message,
                    is_error: true,
                },
            },
        )
    }
}
