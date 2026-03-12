//! Event types for the Astrid event bus.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Metadata attached to every event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMetadata {
    /// Unique event identifier.
    pub event_id: Uuid,
    /// When the event was created.
    pub timestamp: DateTime<Utc>,
    /// Correlation ID for tracing related events.
    pub correlation_id: Option<Uuid>,
    /// Session ID if applicable.
    pub session_id: Option<Uuid>,
    /// User ID if applicable.
    pub user_id: Option<Uuid>,
    /// Source component that generated the event.
    pub source: String,
}

impl EventMetadata {
    /// Create new event metadata.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            correlation_id: None,
            session_id: None,
            user_id: None,
            source: source.into(),
        }
    }

    /// Set correlation ID.
    #[must_use]
    pub fn with_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set session ID.
    #[must_use]
    pub fn with_session_id(mut self, id: Uuid) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Set user ID.
    #[must_use]
    pub fn with_user_id(mut self, id: Uuid) -> Self {
        self.user_id = Some(id);
        self
    }
}

impl Default for EventMetadata {
    fn default() -> Self {
        Self::new("unknown")
    }
}

/// All events that can occur in the Astrid runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AstridEvent {
    // ========== Agent Lifecycle ==========
    /// Runtime started.
    RuntimeStarted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Runtime version.
        version: String,
    },

    /// Runtime stopped.
    RuntimeStopped {
        /// Event metadata.
        metadata: EventMetadata,
        /// Reason for stopping.
        reason: Option<String>,
    },

    /// Agent started within the runtime.
    AgentStarted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Agent ID.
        agent_id: Uuid,
        /// Agent name.
        agent_name: String,
    },

    /// Agent stopped.
    AgentStopped {
        /// Event metadata.
        metadata: EventMetadata,
        /// Agent ID.
        agent_id: Uuid,
        /// Reason for stopping.
        reason: Option<String>,
    },

    // ========== Session Events ==========
    /// Session created.
    SessionCreated {
        /// Event metadata.
        metadata: EventMetadata,
        /// Session ID.
        session_id: Uuid,
    },

    /// Session ended.
    SessionEnded {
        /// Event metadata.
        metadata: EventMetadata,
        /// Session ID.
        session_id: Uuid,
        /// Reason for ending.
        reason: Option<String>,
    },

    /// Session resumed from persisted state.
    SessionResumed {
        /// Event metadata.
        metadata: EventMetadata,
        /// Session ID.
        session_id: Uuid,
    },

    // ========== Message Flow ==========
    /// User message received by the runtime.
    MessageReceived {
        /// Event metadata.
        metadata: EventMetadata,
        /// Message ID.
        message_id: Uuid,
        /// Platform the message came from.
        platform: String,
    },

    /// Response message has been delivered to the user/platform.
    ///
    /// Fired after the message is confirmed sent. Useful for auditing,
    /// logging, or triggering post-delivery side effects.
    MessageSent {
        /// Event metadata.
        metadata: EventMetadata,
        /// Message ID.
        message_id: Uuid,
        /// Target platform.
        platform: String,
    },

    /// Message fully processed (response sent).
    MessageProcessed {
        /// Event metadata.
        metadata: EventMetadata,
        /// Message ID.
        message_id: Uuid,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    // ========== Prompt / Cognitive Loop Events ==========
    /// Prompt is being assembled before an LLM call.
    ///
    /// Capsules can inspect or modify the prompt context before it is sent
    /// to the model.
    PromptBuilding {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID correlating to the upcoming LLM call.
        request_id: Uuid,
    },

    /// A response message is about to be sent to the user/platform.
    ///
    /// Allows capsules to intercept or transform outbound messages.
    MessageSending {
        /// Event metadata.
        metadata: EventMetadata,
        /// Message ID.
        message_id: Uuid,
        /// Target platform.
        platform: String,
    },

    /// Context compaction is starting (trimming conversation history).
    ContextCompactionStarted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Session ID being compacted.
        session_id: Uuid,
        /// Number of messages before compaction.
        message_count: u32,
    },

    /// Context compaction completed.
    ContextCompactionCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Session ID that was compacted.
        session_id: Uuid,
        /// Messages remaining after compaction.
        messages_remaining: u32,
    },

    /// Session is being reset (conversation history cleared).
    SessionResetting {
        /// Event metadata.
        metadata: EventMetadata,
        /// Session ID being reset.
        session_id: Uuid,
    },

    /// Model selection is being resolved before an LLM call.
    ///
    /// Capsules can influence which model/provider is selected for a request.
    ModelResolving {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Candidate provider (may be overridden by capsule).
        provider: Option<String>,
        /// Candidate model (may be overridden by capsule).
        model: Option<String>,
    },

    /// The agent's cognitive loop has finished its run.
    ///
    /// Fired after the final response is produced, before session teardown.
    /// Capsules can inspect the complete run for logging or analytics.
    AgentLoopCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Agent ID.
        agent_id: Uuid,
        /// Total turns in the loop.
        turns: u32,
        /// Duration of the full loop in milliseconds.
        duration_ms: u64,
    },

    /// A tool result is about to be persisted to conversation history.
    ///
    /// Capsules can intercept, redact, or transform the result before
    /// it is stored.
    ToolResultPersisting {
        /// Event metadata.
        metadata: EventMetadata,
        /// Tool call ID.
        call_id: Uuid,
        /// Tool name.
        tool_name: String,
    },

    // ========== LLM Events ==========
    /// LLM request started.
    LlmRequestStarted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Provider name.
        provider: String,
        /// Model name.
        model: String,
    },

    /// LLM request completed (non-streaming or final).
    LlmRequestCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Whether the request succeeded.
        success: bool,
        /// Input tokens used.
        input_tokens: Option<u32>,
        /// Output tokens used.
        output_tokens: Option<u32>,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// LLM streaming response started.
    LlmStreamStarted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Model name.
        model: String,
    },

    /// LLM stream chunk received.
    LlmStreamChunk {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Chunk index (0-based).
        chunk_index: u32,
        /// Number of tokens in this chunk.
        token_count: u32,
    },

    /// LLM streaming response completed.
    LlmStreamCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Total input tokens.
        input_tokens: Option<u32>,
        /// Total output tokens.
        output_tokens: Option<u32>,
        /// Total duration in milliseconds.
        duration_ms: u64,
    },

    // ========== Tool Events ==========
    /// Tool call started (generic, any tool source).
    ToolCallStarted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Tool call ID.
        call_id: Uuid,
        /// Tool name.
        tool_name: String,
        /// Server name (if MCP tool).
        server_name: Option<String>,
    },

    /// Tool call completed successfully.
    ToolCallCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Tool call ID.
        call_id: Uuid,
        /// Tool name.
        tool_name: String,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// Tool call failed.
    ToolCallFailed {
        /// Event metadata.
        metadata: EventMetadata,
        /// Tool call ID.
        call_id: Uuid,
        /// Tool name.
        tool_name: String,
        /// Error message.
        error: String,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    // ========== MCP Events ==========
    /// MCP server connected.
    McpServerConnected {
        /// Event metadata.
        metadata: EventMetadata,
        /// Server name.
        server_name: String,
        /// Protocol version.
        protocol_version: String,
    },

    /// MCP server disconnected.
    McpServerDisconnected {
        /// Event metadata.
        metadata: EventMetadata,
        /// Server name.
        server_name: String,
        /// Reason for disconnection.
        reason: Option<String>,
    },

    /// MCP tool called.
    McpToolCalled {
        /// Event metadata.
        metadata: EventMetadata,
        /// Server name.
        server_name: String,
        /// Tool name.
        tool_name: String,
        /// Tool arguments (may be redacted for security).
        arguments: Option<Value>,
    },

    /// MCP tool completed.
    McpToolCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Server name.
        server_name: String,
        /// Tool name.
        tool_name: String,
        /// Whether the call succeeded.
        success: bool,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    // ========== SubAgent Events ==========
    /// Sub-agent spawned by a parent agent.
    SubAgentSpawned {
        /// Event metadata.
        metadata: EventMetadata,
        /// Sub-agent ID.
        subagent_id: Uuid,
        /// Parent agent ID.
        parent_id: Uuid,
        /// Task description.
        task: String,
        /// Depth in the agent tree.
        depth: u32,
    },

    /// Sub-agent progress update.
    SubAgentProgress {
        /// Event metadata.
        metadata: EventMetadata,
        /// Sub-agent ID.
        subagent_id: Uuid,
        /// Progress message.
        message: String,
    },

    /// Sub-agent completed successfully.
    SubAgentCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Sub-agent ID.
        subagent_id: Uuid,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// Sub-agent failed.
    SubAgentFailed {
        /// Event metadata.
        metadata: EventMetadata,
        /// Sub-agent ID.
        subagent_id: Uuid,
        /// Error message.
        error: String,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// Sub-agent cancelled.
    SubAgentCancelled {
        /// Event metadata.
        metadata: EventMetadata,
        /// Sub-agent ID.
        subagent_id: Uuid,
        /// Reason for cancellation.
        reason: Option<String>,
    },

    // ========== Security Events ==========
    /// Capability granted.
    CapabilityGranted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Capability ID.
        capability_id: Uuid,
        /// Resource being accessed.
        resource: String,
        /// Action being performed.
        action: String,
    },

    /// Capability revoked.
    CapabilityRevoked {
        /// Event metadata.
        metadata: EventMetadata,
        /// Capability ID.
        capability_id: Uuid,
        /// Reason for revocation.
        reason: Option<String>,
    },

    /// Capability check performed.
    CapabilityChecked {
        /// Event metadata.
        metadata: EventMetadata,
        /// Resource being accessed.
        resource: String,
        /// Action being performed.
        action: String,
        /// Whether the check passed.
        allowed: bool,
    },

    /// Authorization denied.
    AuthorizationDenied {
        /// Event metadata.
        metadata: EventMetadata,
        /// Resource being accessed.
        resource: String,
        /// Action being performed.
        action: String,
        /// Reason for denial.
        reason: String,
    },

    /// Security violation detected.
    SecurityViolation {
        /// Event metadata.
        metadata: EventMetadata,
        /// Violation type.
        violation_type: String,
        /// Details of the violation.
        details: String,
    },

    // ========== Approval Events ==========
    /// Approval requested.
    ApprovalRequested {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Resource being accessed.
        resource: String,
        /// Action being performed.
        action: String,
        /// Description of what's being requested.
        description: String,
    },

    /// Approval granted.
    ApprovalGranted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Duration of approval (if limited).
        duration: Option<String>,
    },

    /// Approval denied.
    ApprovalDenied {
        /// Event metadata.
        metadata: EventMetadata,
        /// Request ID.
        request_id: Uuid,
        /// Reason for denial.
        reason: Option<String>,
    },

    // ========== Budget Events ==========
    /// Budget allocated for a session or agent.
    BudgetAllocated {
        /// Event metadata.
        metadata: EventMetadata,
        /// Budget ID.
        budget_id: Uuid,
        /// Amount allocated (in smallest currency unit, e.g. cents).
        amount_cents: u64,
        /// Currency code.
        currency: String,
    },

    /// Budget threshold warning.
    BudgetWarning {
        /// Event metadata.
        metadata: EventMetadata,
        /// Budget ID.
        budget_id: Uuid,
        /// Amount remaining (cents).
        remaining_cents: u64,
        /// Percentage used.
        percent_used: f64,
    },

    /// Budget exceeded.
    BudgetExceeded {
        /// Event metadata.
        metadata: EventMetadata,
        /// Budget ID.
        budget_id: Uuid,
        /// Amount over budget (cents).
        overage_cents: u64,
    },

    // ========== Capsule Events ==========
    /// Capsule loaded successfully.
    CapsuleLoaded {
        /// Event metadata.
        metadata: EventMetadata,
        /// Capsule identifier.
        capsule_id: String,
        /// Capsule name.
        capsule_name: String,
    },

    /// Capsule failed to load.
    CapsuleFailed {
        /// Event metadata.
        metadata: EventMetadata,
        /// Capsule identifier.
        capsule_id: String,
        /// Error message.
        error: String,
    },

    /// Capsule unloaded.
    CapsuleUnloaded {
        /// Event metadata.
        metadata: EventMetadata,
        /// Capsule identifier.
        capsule_id: String,
        /// Capsule name.
        capsule_name: String,
    },

    // ========== System Events ==========
    /// Kernel daemon started.
    KernelStarted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Kernel version.
        version: String,
    },

    /// Kernel daemon shutting down.
    KernelShutdown {
        /// Event metadata.
        metadata: EventMetadata,
        /// Reason for shutdown.
        reason: Option<String>,
    },

    /// Configuration reloaded from disk.
    ConfigReloaded {
        /// Event metadata.
        metadata: EventMetadata,
    },

    /// Configuration value changed.
    ConfigChanged {
        /// Event metadata.
        metadata: EventMetadata,
        /// Config key that changed.
        key: String,
    },

    /// Health check completed.
    HealthCheckCompleted {
        /// Event metadata.
        metadata: EventMetadata,
        /// Overall health state.
        healthy: bool,
        /// Number of checks performed.
        checks_performed: u32,
        /// Number of checks that failed.
        checks_failed: u32,
    },

    // ========== Audit Events ==========
    /// Audit entry created.
    AuditEntryCreated {
        /// Event metadata.
        metadata: EventMetadata,
        /// Audit entry ID.
        entry_id: Uuid,
        /// Entry type.
        entry_type: String,
    },

    // ========== Error Events ==========
    /// Error occurred.
    ErrorOccurred {
        /// Event metadata.
        metadata: EventMetadata,
        /// Error code.
        code: String,
        /// Error message.
        message: String,
        /// Stack trace if available.
        stack_trace: Option<String>,
    },

    // ========== IPC Events ==========
    /// An IPC message routed from a WASM guest or host.
    Ipc {
        /// Event metadata.
        metadata: EventMetadata,
        /// The decoded IPC message.
        message: crate::ipc::IpcMessage,
    },

    // ========== Custom Events ==========
    /// Custom event for extensions.
    Custom {
        /// Event metadata.
        metadata: EventMetadata,
        /// Event name.
        name: String,
        /// Event data.
        data: Value,
    },
}

impl AstridEvent {
    /// Get the event metadata.
    #[must_use]
    pub fn metadata(&self) -> &EventMetadata {
        match self {
            Self::RuntimeStarted { metadata, .. }
            | Self::RuntimeStopped { metadata, .. }
            | Self::AgentStarted { metadata, .. }
            | Self::AgentStopped { metadata, .. }
            | Self::SessionCreated { metadata, .. }
            | Self::SessionEnded { metadata, .. }
            | Self::SessionResumed { metadata, .. }
            | Self::PromptBuilding { metadata, .. }
            | Self::MessageSending { metadata, .. }
            | Self::ContextCompactionStarted { metadata, .. }
            | Self::ContextCompactionCompleted { metadata, .. }
            | Self::SessionResetting { metadata, .. }
            | Self::ModelResolving { metadata, .. }
            | Self::AgentLoopCompleted { metadata, .. }
            | Self::ToolResultPersisting { metadata, .. }
            | Self::MessageReceived { metadata, .. }
            | Self::MessageSent { metadata, .. }
            | Self::MessageProcessed { metadata, .. }
            | Self::LlmRequestStarted { metadata, .. }
            | Self::LlmRequestCompleted { metadata, .. }
            | Self::LlmStreamStarted { metadata, .. }
            | Self::LlmStreamChunk { metadata, .. }
            | Self::LlmStreamCompleted { metadata, .. }
            | Self::ToolCallStarted { metadata, .. }
            | Self::ToolCallCompleted { metadata, .. }
            | Self::ToolCallFailed { metadata, .. }
            | Self::McpServerConnected { metadata, .. }
            | Self::McpServerDisconnected { metadata, .. }
            | Self::McpToolCalled { metadata, .. }
            | Self::McpToolCompleted { metadata, .. }
            | Self::SubAgentSpawned { metadata, .. }
            | Self::SubAgentProgress { metadata, .. }
            | Self::SubAgentCompleted { metadata, .. }
            | Self::SubAgentFailed { metadata, .. }
            | Self::SubAgentCancelled { metadata, .. }
            | Self::CapsuleLoaded { metadata, .. }
            | Self::CapsuleFailed { metadata, .. }
            | Self::CapsuleUnloaded { metadata, .. }
            | Self::CapabilityGranted { metadata, .. }
            | Self::CapabilityRevoked { metadata, .. }
            | Self::CapabilityChecked { metadata, .. }
            | Self::AuthorizationDenied { metadata, .. }
            | Self::SecurityViolation { metadata, .. }
            | Self::ApprovalRequested { metadata, .. }
            | Self::ApprovalGranted { metadata, .. }
            | Self::ApprovalDenied { metadata, .. }
            | Self::BudgetAllocated { metadata, .. }
            | Self::BudgetWarning { metadata, .. }
            | Self::BudgetExceeded { metadata, .. }
            | Self::KernelStarted { metadata, .. }
            | Self::KernelShutdown { metadata, .. }
            | Self::ConfigReloaded { metadata, .. }
            | Self::ConfigChanged { metadata, .. }
            | Self::HealthCheckCompleted { metadata, .. }
            | Self::AuditEntryCreated { metadata, .. }
            | Self::ErrorOccurred { metadata, .. }
            | Self::Ipc { metadata, .. }
            | Self::Custom { metadata, .. } => metadata,
        }
    }

    /// Get the event type as a string.
    #[must_use]
    pub fn event_type(&self) -> &'static str {
        match self {
            // Agent Lifecycle
            Self::RuntimeStarted { .. } => "astrid.v1.lifecycle.runtime_started",
            Self::RuntimeStopped { .. } => "astrid.v1.lifecycle.runtime_stopped",
            Self::AgentStarted { .. } => "astrid.v1.lifecycle.agent_started",
            Self::AgentStopped { .. } => "astrid.v1.lifecycle.agent_stopped",
            // Session
            Self::SessionCreated { .. } => "astrid.v1.lifecycle.session_created",
            Self::SessionEnded { .. } => "astrid.v1.lifecycle.session_ended",
            Self::SessionResumed { .. } => "astrid.v1.lifecycle.session_resumed",
            // Prompt / Cognitive Loop
            Self::PromptBuilding { .. } => "astrid.v1.lifecycle.prompt_building",
            Self::MessageSending { .. } => "astrid.v1.lifecycle.message_sending",
            Self::ContextCompactionStarted { .. } => {
                "astrid.v1.lifecycle.context_compaction_started"
            },
            Self::ContextCompactionCompleted { .. } => {
                "astrid.v1.lifecycle.context_compaction_completed"
            },
            Self::SessionResetting { .. } => "astrid.v1.lifecycle.session_resetting",
            Self::ModelResolving { .. } => "astrid.v1.lifecycle.model_resolving",
            Self::AgentLoopCompleted { .. } => "astrid.v1.lifecycle.agent_loop_completed",
            Self::ToolResultPersisting { .. } => "astrid.v1.lifecycle.tool_result_persisting",
            // Message Flow
            Self::MessageReceived { .. } => "astrid.v1.lifecycle.message_received",
            Self::MessageSent { .. } => "astrid.v1.lifecycle.message_sent",
            Self::MessageProcessed { .. } => "astrid.v1.lifecycle.message_processed",
            // LLM
            Self::LlmRequestStarted { .. } => "astrid.v1.lifecycle.llm_request_started",
            Self::LlmRequestCompleted { .. } => "astrid.v1.lifecycle.llm_request_completed",
            Self::LlmStreamStarted { .. } => "astrid.v1.lifecycle.llm_stream_started",
            Self::LlmStreamChunk { .. } => "astrid.v1.lifecycle.llm_stream_chunk",
            Self::LlmStreamCompleted { .. } => "astrid.v1.lifecycle.llm_stream_completed",
            // Tool
            Self::ToolCallStarted { .. } => "astrid.v1.lifecycle.tool_call_started",
            Self::ToolCallCompleted { .. } => "astrid.v1.lifecycle.tool_call_completed",
            Self::ToolCallFailed { .. } => "astrid.v1.lifecycle.tool_call_failed",
            // MCP
            Self::McpServerConnected { .. } => "astrid.v1.lifecycle.mcp_server_connected",
            Self::McpServerDisconnected { .. } => "astrid.v1.lifecycle.mcp_server_disconnected",
            Self::McpToolCalled { .. } => "astrid.v1.lifecycle.mcp_tool_called",
            Self::McpToolCompleted { .. } => "astrid.v1.lifecycle.mcp_tool_completed",
            // SubAgent
            Self::SubAgentSpawned { .. } => "astrid.v1.lifecycle.sub_agent_spawned",
            Self::SubAgentProgress { .. } => "astrid.v1.lifecycle.sub_agent_progress",
            Self::SubAgentCompleted { .. } => "astrid.v1.lifecycle.sub_agent_completed",
            Self::SubAgentFailed { .. } => "astrid.v1.lifecycle.sub_agent_failed",
            Self::SubAgentCancelled { .. } => "astrid.v1.lifecycle.sub_agent_cancelled",
            // Capsule
            Self::CapsuleLoaded { .. } => "astrid.v1.lifecycle.capsule_loaded",
            Self::CapsuleFailed { .. } => "astrid.v1.lifecycle.capsule_failed",
            Self::CapsuleUnloaded { .. } => "astrid.v1.lifecycle.capsule_unloaded",
            // Security
            Self::CapabilityGranted { .. } => "astrid.v1.lifecycle.capability_granted",
            Self::CapabilityRevoked { .. } => "astrid.v1.lifecycle.capability_revoked",
            Self::CapabilityChecked { .. } => "astrid.v1.lifecycle.capability_checked",
            Self::AuthorizationDenied { .. } => "astrid.v1.lifecycle.authorization_denied",
            Self::SecurityViolation { .. } => "astrid.v1.lifecycle.security_violation",
            // Approval
            Self::ApprovalRequested { .. } => "astrid.v1.lifecycle.approval_requested",
            Self::ApprovalGranted { .. } => "astrid.v1.lifecycle.approval_granted",
            Self::ApprovalDenied { .. } => "astrid.v1.lifecycle.approval_denied",
            // Budget
            Self::BudgetAllocated { .. } => "astrid.v1.lifecycle.budget_allocated",
            Self::BudgetWarning { .. } => "astrid.v1.lifecycle.budget_warning",
            Self::BudgetExceeded { .. } => "astrid.v1.lifecycle.budget_exceeded",
            // System
            Self::KernelStarted { .. } => "astrid.v1.lifecycle.kernel_started",
            Self::KernelShutdown { .. } => "astrid.v1.lifecycle.kernel_shutdown",
            Self::ConfigReloaded { .. } => "astrid.v1.lifecycle.config_reloaded",
            Self::ConfigChanged { .. } => "astrid.v1.lifecycle.config_changed",
            Self::HealthCheckCompleted { .. } => "astrid.v1.lifecycle.health_check_completed",
            // Audit
            Self::AuditEntryCreated { .. } => "astrid.v1.lifecycle.audit_entry_created",
            // Error
            Self::ErrorOccurred { .. } => "astrid.v1.lifecycle.error_occurred",
            // IPC
            Self::Ipc { .. } => "ipc",
            // Custom
            Self::Custom { .. } => "custom",
        }
    }

    /// Check if this is a security-related event (test-only).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_security_event(&self) -> bool {
        matches!(
            self,
            Self::CapabilityGranted { .. }
                | Self::CapabilityRevoked { .. }
                | Self::CapabilityChecked { .. }
                | Self::AuthorizationDenied { .. }
                | Self::SecurityViolation { .. }
                | Self::ApprovalRequested { .. }
                | Self::ApprovalGranted { .. }
                | Self::ApprovalDenied { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_metadata_creation() {
        let meta = EventMetadata::new("test_source");
        assert_eq!(meta.source, "test_source");
        assert!(meta.correlation_id.is_none());
        assert!(meta.session_id.is_none());
        assert!(meta.user_id.is_none());
    }

    #[test]
    fn test_event_metadata_builder() {
        let correlation = Uuid::new_v4();
        let session = Uuid::new_v4();
        let user = Uuid::new_v4();

        let meta = EventMetadata::new("test")
            .with_correlation_id(correlation)
            .with_session_id(session)
            .with_user_id(user);

        assert_eq!(meta.correlation_id, Some(correlation));
        assert_eq!(meta.session_id, Some(session));
        assert_eq!(meta.user_id, Some(user));
    }

    #[test]
    fn test_event_type() {
        let event = AstridEvent::RuntimeStarted {
            metadata: EventMetadata::new("runtime"),
            version: "0.1.0".to_string(),
        };
        assert_eq!(event.event_type(), "astrid.v1.lifecycle.runtime_started");
    }

    #[test]
    fn test_security_event_detection() {
        let security_event = AstridEvent::CapabilityGranted {
            metadata: EventMetadata::new("security"),
            capability_id: Uuid::new_v4(),
            resource: "tool:test".to_string(),
            action: "execute".to_string(),
        };
        assert!(security_event.is_security_event());

        let non_security_event = AstridEvent::RuntimeStarted {
            metadata: EventMetadata::new("runtime"),
            version: "0.1.0".to_string(),
        };
        assert!(!non_security_event.is_security_event());
    }

    #[test]
    fn test_event_serialization() {
        let event = AstridEvent::McpToolCalled {
            metadata: EventMetadata::new("mcp"),
            server_name: "filesystem".to_string(),
            tool_name: "read_file".to_string(),
            arguments: Some(serde_json::json!({"path": "/tmp/test.txt"})),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("mcp_tool_called"));
        assert!(json.contains("filesystem"));
    }
}
