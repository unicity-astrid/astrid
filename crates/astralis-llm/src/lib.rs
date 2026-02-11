//! Astralis LLM - LLM provider abstraction with streaming support.
//!
//! This crate provides:
//! - LLM provider trait for abstraction
//! - Claude (Anthropic) implementation
//! - `OpenAI`-compatible implementation (LM Studio, `OpenAI`, vLLM, etc.)
//! - Streaming response support
//! - Tool use support
//!
//! # Example with Claude
//!
//! ```rust,no_run
//! use astralis_llm::{ClaudeProvider, LlmProvider, Message, ProviderConfig};
//!
//! # async fn example() -> Result<(), astralis_llm::LlmError> {
//! // Create provider
//! let config = ProviderConfig::new("your-api-key", "claude-sonnet-4-20250514");
//! let provider = ClaudeProvider::new(config);
//!
//! // Simple completion
//! let response = provider.complete_simple("What is 2+2?").await?;
//! println!("Response: {}", response);
//! # Ok(())
//! # }
//! ```
//!
//! # Example with LM Studio
//!
//! ```rust,no_run
//! use astralis_llm::{OpenAiCompatProvider, LlmProvider, Message};
//!
//! # async fn example() -> Result<(), astralis_llm::LlmError> {
//! // Connect to LM Studio running locally
//! let provider = OpenAiCompatProvider::lm_studio();
//!
//! // Or with a specific model
//! let provider = OpenAiCompatProvider::lm_studio_with_model("llama-3.1-8b");
//!
//! let response = provider.complete_simple("Hello!").await?;
//! println!("Response: {}", response);
//! # Ok(())
//! # }
//! ```
//!
//! # Streaming
//!
//! ```rust,no_run
//! use astralis_llm::{ClaudeProvider, LlmProvider, Message, ProviderConfig, StreamEvent};
//! use futures::StreamExt;
//!
//! # async fn example() -> Result<(), astralis_llm::LlmError> {
//! let provider = ClaudeProvider::new(ProviderConfig::new("api-key", "claude-sonnet-4-20250514"));
//! let messages = vec![Message::user("Tell me a story")];
//!
//! let mut stream = provider.stream(&messages, &[], "").await?;
//!
//! while let Some(event) = stream.next().await {
//!     match event? {
//!         StreamEvent::TextDelta(text) => print!("{}", text),
//!         StreamEvent::Done => println!("\n[Done]"),
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

mod claude;
mod error;
mod openai_compat;
mod provider;
mod types;
mod zai;

pub use claude::ClaudeProvider;
pub use error::{LlmError, LlmResult};
pub use openai_compat::OpenAiCompatProvider;
pub use provider::{LlmProvider, ProviderConfig, StreamBox};
pub use types::{
    ContentPart, LlmResponse, LlmToolDefinition, Message, MessageContent, MessageRole, StopReason,
    StreamEvent, ToolCall, ToolCallResult, Usage,
};
pub use zai::ZaiProvider;
