//! Event dispatcher for routing IPC events to capsule interceptors.
//!
//! The dispatcher is a host-side async task that subscribes to the global
//! `EventBus`, matches incoming IPC event topics against capsule interceptor
//! patterns (from `Capsule.toml`), and invokes the corresponding WASM
//! `astrid_hook_trigger` export on each matching capsule.
//!
//! # Topic Matching
//!
//! Interceptor event patterns support:
//! - Exact match: `user.prompt` matches only `user.prompt`
//! - Single-segment wildcard: `tool.execute.*.result` matches
//!   `tool.execute.search.result` but not `tool.execute.result`

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::capsule::CapsuleId;
use crate::registry::CapsuleRegistry;
use astrid_events::{AstridEvent, EventBus};

/// Routes IPC events from the `EventBus` to capsule interceptors.
pub struct EventDispatcher {
    registry: Arc<RwLock<CapsuleRegistry>>,
    event_bus: Arc<EventBus>,
}

impl EventDispatcher {
    /// Create a new event dispatcher.
    #[must_use]
    pub fn new(registry: Arc<RwLock<CapsuleRegistry>>, event_bus: Arc<EventBus>) -> Self {
        Self {
            registry,
            event_bus,
        }
    }

    /// Run the dispatch loop. Blocks until the event bus is closed.
    ///
    /// Subscribes to all events on the bus, filters for IPC events, and
    /// dispatches matching interceptors. This method should be spawned as
    /// a background tokio task.
    pub async fn run(self) {
        let mut receiver = self.event_bus.subscribe();
        debug!("Event dispatcher started");

        while let Some(event) = receiver.recv().await {
            if let AstridEvent::Ipc { message, .. } = &*event {
                self.dispatch(&message.topic, &message.payload).await;
            }
        }

        debug!("Event dispatcher stopped (event bus closed)");
    }

    /// Match an IPC event against all registered interceptors and invoke matches.
    async fn dispatch(&self, topic: &str, payload: &astrid_events::ipc::IpcPayload) {
        // Phase 1: collect matches under a brief read lock.
        let matches: Vec<(CapsuleId, String)> = {
            let registry = self.registry.read().await;
            let mut matches = Vec::new();
            for capsule_id in registry.list() {
                if let Some(capsule) = registry.get(capsule_id) {
                    if !matches!(capsule.state(), crate::capsule::CapsuleState::Ready) {
                        continue;
                    }
                    for interceptor in &capsule.manifest().interceptors {
                        if topic_matches(topic, &interceptor.event) {
                            matches.push((capsule_id.clone(), interceptor.action.clone()));
                        }
                    }
                }
            }
            matches
        };

        if matches.is_empty() {
            return;
        }

        // Serialize payload once for all invocations.
        let payload_bytes = match serde_json::to_vec(payload) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(topic, error = %e, "Failed to serialize IPC payload for dispatch");
                return;
            },
        };

        // Phase 2: invoke each matching interceptor (re-acquire read lock per invocation
        // to avoid holding it across WASM execution).
        for (capsule_id, action) in matches {
            debug!(
                capsule_id = %capsule_id,
                action = %action,
                topic,
                "Dispatching interceptor"
            );

            let result = {
                let registry = self.registry.read().await;
                registry
                    .get(&capsule_id)
                    .map(|capsule| capsule.invoke_interceptor(&action, &payload_bytes))
            };

            match result {
                Some(Ok(_)) => {
                    debug!(
                        capsule_id = %capsule_id,
                        action = %action,
                        "Interceptor completed"
                    );
                },
                Some(Err(e)) => {
                    warn!(
                        capsule_id = %capsule_id,
                        action = %action,
                        topic,
                        error = %e,
                        "Interceptor invocation failed"
                    );
                },
                None => {
                    debug!(
                        capsule_id = %capsule_id,
                        "Capsule no longer registered, skipping interceptor"
                    );
                },
            }
        }
    }
}

/// Check if an IPC topic matches an interceptor event pattern.
///
/// Supports exact matches and single-segment wildcards (`*`).
/// Both strings are split on `.` and compared segment by segment.
/// A `*` in the pattern matches exactly one segment.
///
/// # Examples
///
/// ```ignore
/// assert!(topic_matches("user.prompt", "user.prompt"));
/// assert!(topic_matches("tool.execute.search.result", "tool.execute.*.result"));
/// assert!(!topic_matches("tool.execute.result", "tool.execute.*.result"));
/// assert!(!topic_matches("user.prompt.extra", "user.prompt"));
/// ```
pub(crate) fn topic_matches(topic: &str, pattern: &str) -> bool {
    let topic_parts: Vec<&str> = topic.split('.').collect();
    let pattern_parts: Vec<&str> = pattern.split('.').collect();

    if topic_parts.len() != pattern_parts.len() {
        return false;
    }

    topic_parts
        .iter()
        .zip(pattern_parts.iter())
        .all(|(t, p)| *p == "*" || t == p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert!(topic_matches("user.prompt", "user.prompt"));
        assert!(topic_matches(
            "llm.stream.anthropic",
            "llm.stream.anthropic"
        ));
    }

    #[test]
    fn wildcard_single_segment() {
        assert!(topic_matches(
            "tool.execute.search.result",
            "tool.execute.*.result"
        ));
        assert!(topic_matches(
            "tool.execute.code-run.result",
            "tool.execute.*.result"
        ));
    }

    #[test]
    fn wildcard_does_not_match_missing_segment() {
        // Pattern has 4 segments but topic has only 3
        assert!(!topic_matches(
            "tool.execute.result",
            "tool.execute.*.result"
        ));
    }

    #[test]
    fn no_match_different_topic() {
        assert!(!topic_matches("user.prompt", "llm.stream.anthropic"));
    }

    #[test]
    fn no_match_extra_segment() {
        assert!(!topic_matches("user.prompt.extra", "user.prompt"));
    }

    #[test]
    fn no_match_fewer_segments() {
        assert!(!topic_matches("user", "user.prompt"));
    }

    #[test]
    fn single_segment_exact() {
        assert!(topic_matches("ping", "ping"));
        assert!(!topic_matches("ping", "pong"));
    }

    #[test]
    fn wildcard_at_start() {
        assert!(topic_matches("foo.bar.baz", "*.bar.baz"));
    }

    #[test]
    fn wildcard_at_end() {
        assert!(topic_matches("foo.bar.baz", "foo.bar.*"));
    }

    #[test]
    fn multiple_wildcards() {
        assert!(topic_matches("a.b.c", "*.b.*"));
        assert!(topic_matches("x.b.z", "*.b.*"));
        assert!(!topic_matches("x.c.z", "*.b.*"));
    }
}
