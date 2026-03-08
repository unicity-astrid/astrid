//! Event-to-hook mapping table.
//!
//! Maps kernel `AstridEvent` variants to OpenClaw-compatible hook names
//! and defines the merge semantics for each hook's interceptor responses.

use astrid_events::AstridEvent;

/// Describes how interceptor responses should be merged for a hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeSemantics {
    /// Fire-and-forget: responses are discarded.
    None,

    /// `before_tool_call` specific: any `skip: true` → skip,
    /// last non-null `modified_params` wins.
    ToolCallBefore,

    /// Last non-null value for the named field wins.
    LastNonNull {
        /// The response field to merge on.
        field: &'static str,
    },
}

/// A mapping from a kernel event to a hook name and merge semantics.
#[derive(Debug)]
pub(crate) struct HookMapping {
    /// The hook name to fire (e.g. `"before_tool_call"`).
    pub hook_name: &'static str,
    /// How to merge responses from multiple interceptors.
    pub merge: MergeSemantics,
}

impl HookMapping {
    /// Resolve the hook mapping for a given `AstridEvent`.
    ///
    /// Returns `None` for events that have no corresponding hook.
    #[must_use]
    pub(crate) fn from_event(event: &AstridEvent) -> Option<Self> {
        match event {
            // ── Session lifecycle ──
            AstridEvent::SessionCreated { .. } => Some(Self {
                hook_name: "session_start",
                merge: MergeSemantics::None,
            }),
            AstridEvent::SessionEnded { .. } => Some(Self {
                hook_name: "session_end",
                merge: MergeSemantics::None,
            }),

            // ── Tool hooks ──
            AstridEvent::ToolCallStarted { .. } => Some(Self {
                hook_name: "before_tool_call",
                merge: MergeSemantics::ToolCallBefore,
            }),
            AstridEvent::ToolCallCompleted { .. } => Some(Self {
                hook_name: "after_tool_call",
                merge: MergeSemantics::LastNonNull {
                    field: "modified_result",
                },
            }),
            AstridEvent::ToolResultPersisting { .. } => Some(Self {
                hook_name: "tool_result_persist",
                merge: MergeSemantics::LastNonNull {
                    field: "transformed_result",
                },
            }),

            // ── Message hooks ──
            AstridEvent::MessageReceived { .. } => Some(Self {
                hook_name: "message_received",
                merge: MergeSemantics::None,
            }),
            AstridEvent::MessageSending { .. } => Some(Self {
                hook_name: "message_sending",
                merge: MergeSemantics::LastNonNull {
                    field: "modified_content",
                },
            }),
            AstridEvent::MessageSent { .. } => Some(Self {
                hook_name: "message_sent",
                merge: MergeSemantics::None,
            }),

            // ── Sub-agent hooks ──
            AstridEvent::SubAgentSpawned { .. } => Some(Self {
                hook_name: "subagent_start",
                merge: MergeSemantics::None,
            }),
            AstridEvent::SubAgentCompleted { .. }
            | AstridEvent::SubAgentFailed { .. }
            | AstridEvent::SubAgentCancelled { .. } => Some(Self {
                hook_name: "subagent_stop",
                merge: MergeSemantics::None,
            }),

            // ── Kernel lifecycle ──
            AstridEvent::KernelStarted { .. } => Some(Self {
                hook_name: "kernel_start",
                merge: MergeSemantics::None,
            }),
            AstridEvent::KernelShutdown { .. } => Some(Self {
                hook_name: "kernel_stop",
                merge: MergeSemantics::None,
            }),

            // All other events have no hook mapping.
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrid_events::EventMetadata;
    use uuid::Uuid;

    #[test]
    fn session_created_maps_to_session_start() {
        let event = AstridEvent::SessionCreated {
            metadata: EventMetadata::new("test"),
            session_id: Uuid::new_v4(),
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "session_start");
        assert_eq!(mapping.merge, MergeSemantics::None);
    }

    #[test]
    fn session_ended_maps_to_session_end() {
        let event = AstridEvent::SessionEnded {
            metadata: EventMetadata::new("test"),
            session_id: Uuid::new_v4(),
            reason: None,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "session_end");
        assert_eq!(mapping.merge, MergeSemantics::None);
    }

    #[test]
    fn tool_call_started_maps_to_before_tool_call() {
        let event = AstridEvent::ToolCallStarted {
            metadata: EventMetadata::new("test"),
            call_id: Uuid::new_v4(),
            tool_name: "search".into(),
            server_name: None,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "before_tool_call");
        assert_eq!(mapping.merge, MergeSemantics::ToolCallBefore);
    }

    #[test]
    fn tool_call_completed_maps_to_after_tool_call() {
        let event = AstridEvent::ToolCallCompleted {
            metadata: EventMetadata::new("test"),
            call_id: Uuid::new_v4(),
            tool_name: "search".into(),
            duration_ms: 100,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "after_tool_call");
        assert_eq!(
            mapping.merge,
            MergeSemantics::LastNonNull {
                field: "modified_result"
            }
        );
    }

    #[test]
    fn tool_result_persisting_maps_to_tool_result_persist() {
        let event = AstridEvent::ToolResultPersisting {
            metadata: EventMetadata::new("test"),
            call_id: Uuid::new_v4(),
            tool_name: "search".into(),
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "tool_result_persist");
        assert_eq!(
            mapping.merge,
            MergeSemantics::LastNonNull {
                field: "transformed_result"
            }
        );
    }

    #[test]
    fn message_received_maps_correctly() {
        let event = AstridEvent::MessageReceived {
            metadata: EventMetadata::new("test"),
            message_id: Uuid::new_v4(),
            frontend: "cli".into(),
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "message_received");
        assert_eq!(mapping.merge, MergeSemantics::None);
    }

    #[test]
    fn message_sending_maps_correctly() {
        let event = AstridEvent::MessageSending {
            metadata: EventMetadata::new("test"),
            message_id: Uuid::new_v4(),
            frontend: "cli".into(),
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "message_sending");
        assert_eq!(
            mapping.merge,
            MergeSemantics::LastNonNull {
                field: "modified_content"
            }
        );
    }

    #[test]
    fn message_sent_maps_correctly() {
        let event = AstridEvent::MessageSent {
            metadata: EventMetadata::new("test"),
            message_id: Uuid::new_v4(),
            frontend: "cli".into(),
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "message_sent");
        assert_eq!(mapping.merge, MergeSemantics::None);
    }

    #[test]
    fn subagent_spawned_maps_correctly() {
        let event = AstridEvent::SubAgentSpawned {
            metadata: EventMetadata::new("test"),
            subagent_id: Uuid::new_v4(),
            parent_id: Uuid::new_v4(),
            task: "test task".into(),
            depth: 1,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "subagent_start");
        assert_eq!(mapping.merge, MergeSemantics::None);
    }

    #[test]
    fn subagent_completed_maps_to_subagent_stop() {
        let event = AstridEvent::SubAgentCompleted {
            metadata: EventMetadata::new("test"),
            subagent_id: Uuid::new_v4(),
            duration_ms: 500,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "subagent_stop");
    }

    #[test]
    fn subagent_failed_maps_to_subagent_stop() {
        let event = AstridEvent::SubAgentFailed {
            metadata: EventMetadata::new("test"),
            subagent_id: Uuid::new_v4(),
            error: "boom".into(),
            duration_ms: 100,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "subagent_stop");
    }

    #[test]
    fn subagent_cancelled_maps_to_subagent_stop() {
        let event = AstridEvent::SubAgentCancelled {
            metadata: EventMetadata::new("test"),
            subagent_id: Uuid::new_v4(),
            reason: None,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "subagent_stop");
    }

    #[test]
    fn kernel_started_maps_correctly() {
        let event = AstridEvent::KernelStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".into(),
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "kernel_start");
        assert_eq!(mapping.merge, MergeSemantics::None);
    }

    #[test]
    fn kernel_shutdown_maps_correctly() {
        let event = AstridEvent::KernelShutdown {
            metadata: EventMetadata::new("test"),
            reason: None,
        };
        let mapping = HookMapping::from_event(&event).unwrap();
        assert_eq!(mapping.hook_name, "kernel_stop");
        assert_eq!(mapping.merge, MergeSemantics::None);
    }

    #[test]
    fn unmapped_event_returns_none() {
        let event = AstridEvent::RuntimeStarted {
            metadata: EventMetadata::new("test"),
            version: "0.1.0".into(),
        };
        assert!(HookMapping::from_event(&event).is_none());
    }

    #[test]
    fn ipc_event_returns_none() {
        // IPC events are handled by the existing EventDispatcher, not the hook bridge.
        let event = AstridEvent::Custom {
            metadata: EventMetadata::new("test"),
            name: "custom".into(),
            data: serde_json::json!({}),
        };
        assert!(HookMapping::from_event(&event).is_none());
    }
}
