//! LLM provider trait.
//!
//! Defines the interface that all LLM providers must implement.

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

use crate::error::LlmResult;
use crate::types::{LlmResponse, LlmToolDefinition, Message, StreamEvent};

/// Type alias for boxed streams.
pub type StreamBox = Pin<Box<dyn Stream<Item = LlmResult<StreamEvent>> + Send>>;

/// LLM provider trait.
///
/// Implementors provide access to language models with streaming support.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the provider name.
    fn name(&self) -> &str;

    /// Get the model being used.
    fn model(&self) -> &str;

    /// Stream a completion.
    ///
    /// Returns a stream of events as the model generates output.
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<StreamBox>;

    /// Complete without streaming.
    ///
    /// Returns the full response once generation is complete.
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<LlmResponse>;

    /// Simple text completion (no tools).
    async fn complete_simple(&self, prompt: &str) -> LlmResult<String> {
        let messages = vec![Message::user(prompt)];
        let response = self.complete(&messages, &[], "").await?;
        Ok(response.message.text().unwrap_or("").to_string())
    }

    /// Count tokens in text (approximate).
    fn count_tokens(&self, text: &str) -> usize {
        // Rough approximation: ~4 chars per token
        text.len() / 4
    }

    /// Get maximum context length.
    fn max_context_length(&self) -> usize;
}

/// Blanket implementation allowing `Box<dyn LlmProvider>` to be used as
/// a type parameter wherever `P: LlmProvider` is required (e.g. `AgentRuntime<P>`).
#[async_trait]
impl LlmProvider for Box<dyn LlmProvider> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn model(&self) -> &str {
        (**self).model()
    }

    async fn stream(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<StreamBox> {
        (**self).stream(messages, tools, system).await
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[LlmToolDefinition],
        system: &str,
    ) -> LlmResult<LlmResponse> {
        (**self).complete(messages, tools, system).await
    }

    fn count_tokens(&self, text: &str) -> usize {
        (**self).count_tokens(text)
    }

    fn max_context_length(&self) -> usize {
        (**self).max_context_length()
    }
}

/// Configuration for LLM providers.
#[derive(Clone)]
pub struct ProviderConfig {
    /// API key.
    pub api_key: String,
    /// Model name.
    pub model: String,
    /// Maximum tokens to generate.
    pub max_tokens: usize,
    /// Temperature (0.0 - 1.0).
    pub temperature: f64,
    /// API base URL (for custom endpoints).
    pub base_url: Option<String>,
    /// Context window size override. When set, the provider uses this instead
    /// of its built-in default for the model.
    pub context_window: Option<usize>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("has_api_key", &!self.api_key.is_empty())
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("has_base_url", &self.base_url.is_some())
            .field("context_window", &self.context_window)
            .finish()
    }
}

impl ProviderConfig {
    /// Create a new config with API key and model.
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: 4096,
            temperature: 0.7,
            base_url: None,
            context_window: None,
        }
    }

    /// Set max tokens.
    #[must_use]
    pub fn max_tokens(mut self, max: usize) -> Self {
        self.max_tokens = max;
        self
    }

    /// Set temperature.
    #[must_use]
    pub fn temperature(mut self, temp: f64) -> Self {
        self.temperature = temp.clamp(0.0, 1.0);
        self
    }

    /// Set base URL.
    #[must_use]
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Set context window size override.
    #[must_use]
    pub fn context_window(mut self, size: usize) -> Self {
        self.context_window = Some(size);
        self
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            base_url: None,
            context_window: None,
        }
    }
}
