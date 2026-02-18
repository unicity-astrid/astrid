//! Sampling capability types and handler trait.
//!
//! Implements the MCP Nov 2025 sampling capability: server-initiated LLM calls.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Request for LLM sampling from a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingRequest {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// Server making the request.
    pub server: String,
    /// Messages to send to the LLM.
    pub messages: Vec<SamplingMessage>,
    /// Optional system prompt.
    pub system: Option<String>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Temperature setting.
    pub temperature: Option<f64>,
    /// Model preference (hint, not requirement).
    pub model_hint: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

/// Message in a sampling request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingMessage {
    /// Role: "user", "assistant", or "system".
    pub role: String,
    /// Message content.
    pub content: SamplingContent,
}

/// Content in a sampling message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SamplingContent {
    /// Text content.
    Text {
        /// The text.
        text: String,
    },
    /// Image content.
    Image {
        /// Base64-encoded image data.
        data: String,
        /// MIME type.
        mime_type: String,
    },
}

/// Response to a sampling request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingResponse {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// Whether the request was successful.
    pub success: bool,
    /// Generated content.
    pub content: Option<String>,
    /// Model used.
    pub model: Option<String>,
    /// Stop reason.
    pub stop_reason: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Handler for server-initiated LLM sampling requests.
#[async_trait]
pub trait SamplingHandler: Send + Sync {
    /// Handle a sampling request from a server.
    ///
    /// The implementation should:
    /// 1. Validate the request is within allowed parameters
    /// 2. Forward to the LLM if authorized
    /// 3. Return the response
    async fn handle_sampling(&self, request: SamplingRequest) -> SamplingResponse;

    /// Check if sampling is enabled for a server.
    fn is_enabled(&self, server: &str) -> bool;

    /// Get the maximum tokens allowed for a server.
    fn max_tokens(&self, server: &str) -> Option<u32>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sampling_request_serialization() {
        let request = SamplingRequest {
            request_id: Uuid::new_v4(),
            server: "test".to_string(),
            messages: vec![SamplingMessage {
                role: "user".to_string(),
                content: SamplingContent::Text {
                    text: "Hello".to_string(),
                },
            }],
            system: None,
            max_tokens: Some(100),
            temperature: Some(0.7),
            model_hint: None,
            metadata: HashMap::new(),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"server\":\"test\""));
        assert!(json.contains("\"max_tokens\":100"));
    }
}
