#![allow(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    pub topic: String,
    pub payload: IpcPayload,
    pub signature: Option<Vec<u8>>,
    pub source_id: uuid::Uuid,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcPayload {
    ToolExecuteRequest {
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ToolExecuteResult {
        call_id: String,
        result: serde_json::Value, // ToolCallResult maps to JSON object
    },
    #[serde(other)]
    Other,
}

fn main() {
    // 1. Subscribe to the IPC topic for tool requests.
    let req_handle = match ipc::subscribe("tool.request.execute") {
        Ok(h) => h,
        Err(e) => {
            let _ = sys::log(
                "error",
                format!(
                    "Router failed to subscribe to tool.request.execute: {:?}",
                    e
                ),
            );
            return;
        },
    };

    // 2. Subscribe to the IPC topic for tool results.
    let res_handle = match ipc::subscribe("tool.execute.*.result") {
        Ok(h) => h,
        Err(e) => {
            let _ = sys::log(
                "error",
                format!(
                    "Router failed to subscribe to tool.execute.*.result: {:?}",
                    e
                ),
            );
            return;
        },
    };

    let _ = sys::log("info", "Tool router middleware started.");

    // 3. Continuous polling loop
    loop {
        // Poll for requests
        if let Ok(bytes) = ipc::poll_bytes(&req_handle) {
            if !bytes.is_empty() {
                if let Ok(msg) = serde_json::from_slice::<IpcMessage>(&bytes) {
                    if let IpcPayload::ToolExecuteRequest {
                        call_id,
                        tool_name,
                        arguments,
                    } = msg.payload
                    {
                        // Forward payload to the specific tool capsule
                        let forward_topic = format!("tool.execute.{}", tool_name);
                        let forward_payload = IpcPayload::ToolExecuteRequest {
                            call_id,
                            tool_name,
                            arguments,
                        };
                        let new_msg = IpcMessage {
                            topic: forward_topic.clone(),
                            payload: forward_payload,
                            signature: None,
                            source_id: msg.source_id,
                            timestamp: msg.timestamp,
                        };
                        let _ = ipc::publish_json(&forward_topic, &new_msg);
                    }
                }
            }
        }

        // Poll for results
        if let Ok(bytes) = ipc::poll_bytes(&res_handle) {
            if !bytes.is_empty() {
                if let Ok(msg) = serde_json::from_slice::<IpcMessage>(&bytes) {
                    if let IpcPayload::ToolExecuteResult { call_id, result } = msg.payload {
                        // Forward result back to the orchestrator
                        let new_msg = IpcMessage {
                            topic: "tool.execute.result".to_string(),
                            payload: IpcPayload::ToolExecuteResult { call_id, result },
                            signature: None,
                            source_id: msg.source_id,
                            timestamp: msg.timestamp,
                        };
                        let _ = ipc::publish_json("tool.execute.result", &new_msg);
                    }
                }
            }
        }

        // Yield/sleep to avoid 100% CPU usage
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
