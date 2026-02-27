#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![allow(missing_docs)]

//! The default LLM Orchestrator capsule.
//!
//! This capsule replaces the hardcoded `astrid-runtime::execution` loop.
//! It maintains conversational state, sends `LlmRequest` payloads to provider capsules,
//! and dispatches `ToolExecuteRequest` payloads to the Tool Router.

use astrid_events::ipc::IpcPayload;
use astrid_events::llm::{Message, MessageContent, MessageRole};
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Default, Serialize, Deserialize)]
pub struct OrchestratorState {
    /// The conversation history for the current session.
    pub messages: Vec<Message>,
}

#[capsule(state)]
impl OrchestratorState {
    /// Listens for new human inputs from the Telegram/CLI Uplink.
    #[astrid::tool("handle_user_prompt")]
    pub fn handle_user_prompt(&mut self, text: String) -> Result<(), SysError> {
        // 1. Add user message to history
        self.messages.push(Message {
            role: MessageRole::User,
            content: MessageContent::Text(text),
        });

        // 2. We need the system prompt from the Identity capsule.
        // For now, we stub this out as a direct IPC request and wait for the async flow.
        // The orchestrator will emit an `identity.request.build` event and yield.
        // When the Identity capsule responds, we will resume execution.
        
        ipc::publish_json("identity.request.build", &{})?;
        Ok(())
    }

    /// Handles the identity response to build the LlmRequest.
    #[astrid::tool("resume_with_identity")]
    pub fn resume_with_identity(&mut self, system_prompt: String) -> Result<(), SysError> {
        // We have the system prompt. Now we ask the LLM Provider to generate.
        let request_id = Uuid::new_v4();
        
        let llm_request = IpcPayload::LlmRequest {
            request_id,
            model: "claude-3-5-sonnet-20241022".to_string(), // In reality, pulled from OS config
            messages: self.messages.clone(),
            tools: vec![], // For now, empty until Tool Router gives us schemas
            system: system_prompt,
        };

        ipc::publish_json("llm.request.generate.anthropic", &llm_request)?;
        Ok(())
    }
}
