#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![allow(missing_docs)]

//! The Tool Router capsule.
//!
//! This middleware receives tool execution requests from the Orchestrator
//! and forwards them to the appropriate tool capsule, then routes the results back.

use astrid_events::ipc::IpcPayload;
use astrid_sdk::prelude::*;

#[derive(Default)]
pub struct ToolRouter;

#[capsule]
impl ToolRouter {
    /// Intercepts tool execution requests from the orchestrator and forwards them.
    #[astrid::interceptor("handle_execute_request")]
    pub fn handle_execute_request(&self, req: IpcPayload) -> Result<(), SysError> {
        if let IpcPayload::ToolExecuteRequest {
            call_id,
            tool_name,
            arguments,
        } = req
        {
            let forward_topic = format!("tool.execute.{}", tool_name);
            let forward_payload = IpcPayload::ToolExecuteRequest {
                call_id,
                tool_name,
                arguments,
            };
            
            sys::log("info", format!("Routing tool request to topic: {}", forward_topic))?;
            
            // Forward payload to the specific tool capsule
            let _ = ipc::publish_json(forward_topic, &forward_payload);
        }
        Ok(())
    }

    /// Intercepts tool results from tool capsules and forwards them back to the orchestrator.
    #[astrid::interceptor("handle_execute_result")]
    pub fn handle_execute_result(&self, res: IpcPayload) -> Result<(), SysError> {
        if let IpcPayload::ToolExecuteResult { call_id, result } = res {
            sys::log("info", format!("Routing tool result for call_id: {}", call_id))?;
            
            let result_payload = IpcPayload::ToolExecuteResult { call_id, result };
            
            // Forward result back to the orchestrator
            let _ = ipc::publish_json("tool.execute.result", &result_payload);
        }
        Ok(())
    }
}
