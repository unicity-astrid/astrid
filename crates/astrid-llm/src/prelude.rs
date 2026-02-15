//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_llm::prelude::*;` to import all essential types.
//!
//! # Example with Claude
//!
//! ```rust,no_run
//! use astrid_llm::prelude::*;
//!
//! # async fn example() -> LlmResult<()> {
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
//! use astrid_llm::prelude::*;
//!
//! # async fn example() -> LlmResult<()> {
//! // Connect to LM Studio running locally
//! let provider = OpenAiCompatProvider::lm_studio();
//!
//! let response = provider.complete_simple("Hello!").await?;
//! println!("Response: {}", response);
//! # Ok(())
//! # }
//! ```

// Errors
pub use crate::{LlmError, LlmResult};

// Provider trait and config
pub use crate::{LlmProvider, ProviderConfig, StreamBox};

// Providers
pub use crate::ClaudeProvider;
pub use crate::OpenAiCompatProvider;
pub use crate::ZaiProvider;

// Message types
pub use crate::{ContentPart, Message, MessageContent, MessageRole};

// Response types
pub use crate::{LlmResponse, StopReason, StreamEvent, Usage};

// Tool types
pub use crate::{LlmToolDefinition, ToolCall, ToolCallResult};
