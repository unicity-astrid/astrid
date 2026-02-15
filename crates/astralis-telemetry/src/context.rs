//! Request context for correlation and tracing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Request context for correlation across operations.
///
/// This struct carries context information that should be propagated
/// through the system for tracing and debugging purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    /// Unique request identifier.
    pub request_id: Uuid,
    /// Correlation ID for tracing related requests.
    pub correlation_id: Uuid,
    /// Parent request ID if this is a sub-request.
    pub parent_id: Option<Uuid>,
    /// Session ID if within a session.
    pub session_id: Option<Uuid>,
    /// User ID if authenticated.
    pub user_id: Option<Uuid>,
    /// When the request started.
    pub started_at: DateTime<Utc>,
    /// Source component that created this context.
    pub source: String,
    /// Operation being performed.
    pub operation: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl RequestContext {
    /// Create a new request context.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            request_id: id,
            correlation_id: id,
            parent_id: None,
            session_id: None,
            user_id: None,
            started_at: Utc::now(),
            source: source.into(),
            operation: None,
            metadata: HashMap::new(),
        }
    }

    /// Create a child context that inherits correlation info.
    #[must_use]
    pub fn child(&self, source: impl Into<String>) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            correlation_id: self.correlation_id,
            parent_id: Some(self.request_id),
            session_id: self.session_id,
            user_id: self.user_id,
            started_at: Utc::now(),
            source: source.into(),
            operation: None,
            metadata: self.metadata.clone(),
        }
    }

    /// Set the correlation ID.
    #[must_use]
    pub fn with_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = id;
        self
    }

    /// Set the session ID.
    #[must_use]
    pub fn with_session_id(mut self, id: Uuid) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Set the user ID.
    #[must_use]
    pub fn with_user_id(mut self, id: Uuid) -> Self {
        self.user_id = Some(id);
        self
    }

    /// Set the operation name.
    #[must_use]
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Add metadata.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Get elapsed time since the request started.
    #[must_use]
    pub fn elapsed(&self) -> chrono::Duration {
        // Utc::now() >= self.started_at by construction (started_at is set at creation time)
        #[allow(clippy::arithmetic_side_effects)]
        let elapsed = Utc::now() - self.started_at;
        elapsed
    }

    /// Get elapsed time in milliseconds.
    #[must_use]
    pub fn elapsed_ms(&self) -> i64 {
        self.elapsed().num_milliseconds()
    }

    /// Create a tracing span with this context.
    #[must_use]
    pub fn span(&self) -> tracing::Span {
        tracing::info_span!(
            "request",
            request_id = %self.request_id,
            correlation_id = %self.correlation_id,
            source = %self.source,
            operation = self.operation.as_deref(),
        )
    }

    /// Check if this context has a parent.
    #[must_use]
    pub fn has_parent(&self) -> bool {
        self.parent_id.is_some()
    }

    /// Get a short identifier for logging.
    #[must_use]
    pub fn short_id(&self) -> String {
        self.request_id.to_string()[..8].to_string()
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        Self::new("unknown")
    }
}

/// Guard that logs when a request completes.
pub struct RequestGuard {
    context: RequestContext,
    /// Held to keep the span active until the guard is dropped.
    #[allow(dead_code)]
    span: tracing::span::EnteredSpan,
}

impl RequestGuard {
    /// Create a new request guard.
    #[must_use]
    pub fn new(context: RequestContext) -> Self {
        let span = context.span().entered();
        tracing::debug!("Request started");
        Self { context, span }
    }

    /// Get the request context.
    #[must_use]
    pub fn context(&self) -> &RequestContext {
        &self.context
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        tracing::debug!(elapsed_ms = self.context.elapsed_ms(), "Request completed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_context_creation() {
        let ctx = RequestContext::new("test");
        assert_eq!(ctx.source, "test");
        assert_eq!(ctx.request_id, ctx.correlation_id);
        assert!(ctx.parent_id.is_none());
        assert!(ctx.session_id.is_none());
        assert!(ctx.user_id.is_none());
    }

    #[test]
    fn test_request_context_builder() {
        let session = Uuid::new_v4();
        let user = Uuid::new_v4();
        let correlation = Uuid::new_v4();

        let ctx = RequestContext::new("test")
            .with_correlation_id(correlation)
            .with_session_id(session)
            .with_user_id(user)
            .with_operation("test_op")
            .with_metadata("key", "value");

        assert_eq!(ctx.correlation_id, correlation);
        assert_eq!(ctx.session_id, Some(session));
        assert_eq!(ctx.user_id, Some(user));
        assert_eq!(ctx.operation, Some("test_op".to_string()));
        assert_eq!(ctx.metadata.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_child_context() {
        let session = Uuid::new_v4();
        let parent = RequestContext::new("parent")
            .with_session_id(session)
            .with_metadata("inherited", "yes");

        let child = parent.child("child");

        // Child should have new request_id
        assert_ne!(child.request_id, parent.request_id);

        // Child should inherit correlation_id
        assert_eq!(child.correlation_id, parent.correlation_id);

        // Child should have parent_id
        assert_eq!(child.parent_id, Some(parent.request_id));

        // Child should inherit session_id
        assert_eq!(child.session_id, Some(session));

        // Child should inherit metadata
        assert_eq!(child.metadata.get("inherited"), Some(&"yes".to_string()));
    }

    #[test]
    fn test_elapsed() {
        let ctx = RequestContext::new("test");
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(ctx.elapsed_ms() >= 10);
    }

    #[test]
    fn test_short_id() {
        let ctx = RequestContext::new("test");
        let short = ctx.short_id();
        assert_eq!(short.len(), 8);
    }

    #[test]
    fn test_serialization() {
        let ctx = RequestContext::new("test")
            .with_operation("test_op")
            .with_metadata("key", "value");

        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("\"source\":\"test\""));
        assert!(json.contains("\"operation\":\"test_op\""));

        let parsed: RequestContext = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.source, "test");
        assert_eq!(parsed.operation, Some("test_op".to_string()));
    }
}
