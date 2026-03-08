//! Event dispatcher for routing IPC events to capsule interceptors.
//!
//! The dispatcher is a host-side async task that subscribes to the global
//! `EventBus`, matches incoming **IPC** events against capsule interceptor
//! patterns (from `Capsule.toml`), and invokes the corresponding WASM
//! `astrid_hook_trigger` export on each matching capsule.
//!
//! # Event Routing
//!
//! The dispatcher handles only **IPC events**, matched by their `topic` field
//! (e.g. `user.prompt`). All other event variants (lifecycle events like
//! `SessionCreated`, `ToolCallStarted`, etc.) are routed by the **Hook Bridge
//! capsule** (`astrid-capsule-hook-bridge`), which adds response collection
//! and merge semantics on top of interceptor invocation.
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

use crate::registry::CapsuleRegistry;
use astrid_events::{AstridEvent, EventBus};

/// Routes IPC events from the `EventBus` to capsule interceptors.
///
/// Lifecycle events (e.g. `SessionCreated`, `ToolCallStarted`) are **not**
/// handled here — they are routed by the Hook Bridge capsule which adds
/// response collection and merge semantics.
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
    /// Subscribes to all events on the bus and routes **IPC events** by topic.
    /// Lifecycle events are silently skipped — they are handled by the Hook
    /// Bridge capsule. This method should be spawned as a background tokio task.
    pub async fn run(self) {
        let mut receiver = self.event_bus.subscribe();
        debug!("Event dispatcher started");

        while let Some(event) = receiver.recv().await {
            if let AstridEvent::Ipc { message, .. } = &*event {
                self.dispatch_ipc(message);
            }
            // Lifecycle events are handled by the Hook Bridge capsule.
        }

        debug!("Event dispatcher stopped (event bus closed)");
    }

    /// Match an IPC event against all registered interceptors and invoke matches.
    ///
    /// Interceptors are dispatched concurrently — each gets its own spawned task
    /// that runs to completion. This method returns immediately after spawning,
    /// so the event loop is never blocked by slow or long-running interceptors.
    fn dispatch_ipc(&self, message: &astrid_events::ipc::IpcMessage) {
        let topic = Arc::new(message.topic.clone());
        let registry = Arc::clone(&self.registry);

        // Serialize payload eagerly so all interceptors share the same bytes.
        let payload_bytes = match serde_json::to_vec(message) {
            Ok(bytes) => Arc::new(bytes),
            Err(e) => {
                warn!(topic = %topic, error = %e, "Failed to serialize IPC message for dispatch");
                return;
            },
        };

        // Spawn a lightweight coordinator task that collects matches under a
        // brief read lock, then fans out each interceptor as its own task.
        tokio::task::spawn(async move {
            let matches: Vec<(Arc<dyn crate::capsule::Capsule>, String)> = {
                let registry = registry.read().await;
                let mut matches = Vec::new();
                for capsule_id in registry.list() {
                    if let Some(capsule) = registry.get(capsule_id) {
                        if !matches!(capsule.state(), crate::capsule::CapsuleState::Ready) {
                            continue;
                        }
                        for interceptor in &capsule.manifest().interceptors {
                            if topic_matches(&topic, &interceptor.event) {
                                matches.push((Arc::clone(&capsule), interceptor.action.clone()));
                            }
                        }
                    }
                }
                matches
                // Read lock dropped here.
            };

            for (capsule, action) in matches {
                let capsule_id = capsule.id().clone();
                let payload = Arc::clone(&payload_bytes);
                let topic = Arc::clone(&topic);

                // Each interceptor runs independently to completion. Spawned on
                // a Tokio worker thread so block_in_place (used by
                // invoke_interceptor and WASM host functions) works correctly.
                // Requires a multi-thread Tokio runtime.
                tokio::task::spawn(async move {
                    debug!(
                        capsule_id = %capsule_id,
                        action = %action,
                        topic = %topic,
                        "Dispatching interceptor"
                    );

                    match capsule.invoke_interceptor(&action, &payload) {
                        Ok(_) => {
                            debug!(
                                capsule_id = %capsule_id,
                                action = %action,
                                "Interceptor completed"
                            );
                        },
                        Err(e) => {
                            warn!(
                                capsule_id = %capsule_id,
                                action = %action,
                                topic = %topic,
                                error = %e,
                                "Interceptor invocation failed"
                            );
                        },
                    }
                });
            }
        });
    }
}

/// Returns `true` if `s` has no empty segments — i.e. no leading/trailing dots
/// and no consecutive dots. An empty string is also rejected.
///
/// Used crate-wide: `discovery.rs` (manifest validation) and `engine/wasm/host/ipc.rs`
/// (runtime boundary checks) both depend on this function.
pub(crate) fn has_valid_segments(s: &str) -> bool {
    !s.is_empty() && s.split('.').all(|seg| !seg.is_empty())
}

/// Check if an IPC topic matches an interceptor event pattern.
///
/// Supports exact matches and single-segment wildcards (`*`).
/// Both strings are split on `.` and compared segment by segment.
/// A `*` in the pattern matches exactly one segment.
/// Topics and patterns with empty segments are rejected (defense in depth).
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
    if !has_valid_segments(topic) || !has_valid_segments(pattern) {
        return false;
    }

    if topic.split('.').count() != pattern.split('.').count() {
        return false;
    }

    topic
        .split('.')
        .zip(pattern.split('.'))
        .all(|(t, p)| p == "*" || t == p)
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

    #[test]
    fn empty_segments_rejected() {
        // Consecutive dots — empty middle segment
        assert!(!topic_matches("a..b", "a.*.b"));
        assert!(!topic_matches("a.x.b", "a..b"));

        // Leading dot — empty first segment
        assert!(!topic_matches(".a.b", "*.a.b"));
        assert!(!topic_matches("x.a.b", ".a.b"));

        // Trailing dot — empty last segment
        assert!(!topic_matches("a.b.", "a.b.*"));
        assert!(!topic_matches("a.b.x", "a.b."));

        // Single dot — two empty segments
        assert!(!topic_matches(".", "*.*"));

        // Empty string
        assert!(!topic_matches("", ""));
        assert!(!topic_matches("", "a"));
        assert!(!topic_matches("a", ""));
    }

    #[test]
    fn has_valid_segments_accepts_valid() {
        assert!(has_valid_segments("a"));
        assert!(has_valid_segments("a.b"));
        assert!(has_valid_segments("a.b.c"));
        assert!(has_valid_segments("*"));
        assert!(has_valid_segments("a.*.b"));
    }

    #[test]
    fn has_valid_segments_rejects_invalid() {
        assert!(!has_valid_segments(""));
        assert!(!has_valid_segments("."));
        assert!(!has_valid_segments(".."));
        assert!(!has_valid_segments("a..b"));
        assert!(!has_valid_segments(".a"));
        assert!(!has_valid_segments("a."));
        assert!(!has_valid_segments(".a.b"));
        assert!(!has_valid_segments("a.b."));
        assert!(!has_valid_segments("a...b"));
    }

    // ── Dispatch integration tests ──────────────────────────────────

    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use crate::capsule::{Capsule, CapsuleId, CapsuleState};
    use crate::context::CapsuleContext;
    use crate::error::CapsuleResult;
    use crate::manifest::{CapabilitiesDef, CapsuleManifest, InterceptorDef, PackageDef};
    use crate::tool::CapsuleTool;
    use astrid_events::ipc::IpcPayload;

    /// A minimal mock capsule for dispatch tests.
    struct MockCapsule {
        id: CapsuleId,
        manifest: CapsuleManifest,
        invoked: Arc<AtomicBool>,
    }

    impl MockCapsule {
        fn new(name: &str, interceptor_event: &str) -> (Self, Arc<AtomicBool>) {
            let invoked = Arc::new(AtomicBool::new(false));
            let manifest = CapsuleManifest {
                package: PackageDef {
                    name: name.to_string(),
                    version: "0.0.1".to_string(),
                    description: None,
                    authors: Vec::new(),
                    repository: None,
                    homepage: None,
                    documentation: None,
                    license: None,
                    license_file: None,
                    readme: None,
                    keywords: Vec::new(),
                    categories: Vec::new(),
                    astrid_version: None,
                    publish: None,
                    include: None,
                    exclude: None,
                    metadata: None,
                },
                components: Vec::new(),
                dependencies: std::collections::HashMap::new(),
                capabilities: CapabilitiesDef::default(),
                env: std::collections::HashMap::new(),
                context_files: Vec::new(),
                commands: Vec::new(),
                mcp_servers: Vec::new(),
                skills: Vec::new(),
                uplinks: Vec::new(),
                llm_providers: Vec::new(),
                interceptors: vec![InterceptorDef {
                    event: interceptor_event.to_string(),
                    action: "test_action".to_string(),
                }],
                cron_jobs: Vec::new(),
                tools: Vec::new(),
            };
            let capsule = Self {
                id: CapsuleId::from_static(name),
                manifest,
                invoked: Arc::clone(&invoked),
            };
            (capsule, invoked)
        }
    }

    #[async_trait]
    impl Capsule for MockCapsule {
        fn id(&self) -> &CapsuleId {
            &self.id
        }
        fn manifest(&self) -> &CapsuleManifest {
            &self.manifest
        }
        fn state(&self) -> CapsuleState {
            CapsuleState::Ready
        }
        async fn load(&mut self, _ctx: &CapsuleContext) -> CapsuleResult<()> {
            Ok(())
        }
        async fn unload(&mut self) -> CapsuleResult<()> {
            Ok(())
        }
        fn tools(&self) -> &[Arc<dyn CapsuleTool>] {
            &[]
        }
        fn invoke_interceptor(&self, _action: &str, _payload: &[u8]) -> CapsuleResult<Vec<u8>> {
            self.invoked.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    /// Helper: publish an IPC event on the bus.
    fn publish_ipc(bus: &EventBus, topic: &str) {
        let msg = astrid_events::ipc::IpcMessage::new(
            topic,
            IpcPayload::Custom {
                data: serde_json::json!({}),
            },
            uuid::Uuid::nil(),
        );
        bus.publish(AstridEvent::Ipc {
            metadata: astrid_events::EventMetadata::new("test"),
            message: msg,
        });
    }

    #[tokio::test]
    async fn dispatch_routes_to_matching_interceptor() {
        let (capsule, invoked) = MockCapsule::new("test-capsule", "test.topic");

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(capsule)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let bus = Arc::new(EventBus::with_capacity(64));
        let dispatcher = EventDispatcher::new(Arc::clone(&registry), Arc::clone(&bus));
        let handle = tokio::spawn(dispatcher.run());

        // Yield to let the dispatcher subscribe before publishing.
        tokio::task::yield_now().await;

        publish_ipc(&bus, "test.topic");

        // Give the dispatcher time to process.
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            invoked.load(Ordering::SeqCst),
            "interceptor should have been invoked for matching topic"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn dispatch_skips_non_matching_topic() {
        let (capsule, invoked) = MockCapsule::new("test-capsule-skip", "specific.topic");

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(capsule)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let bus = Arc::new(EventBus::with_capacity(64));
        let dispatcher = EventDispatcher::new(Arc::clone(&registry), Arc::clone(&bus));
        let handle = tokio::spawn(dispatcher.run());

        tokio::task::yield_now().await;

        publish_ipc(&bus, "other.topic");

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            !invoked.load(Ordering::SeqCst),
            "interceptor should NOT have been invoked for non-matching topic"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn dispatch_concurrent_does_not_block() {
        // Both capsules match different topics. With concurrent dispatch,
        // the second event is processed immediately without waiting for
        // the first interceptor to complete.
        let (cap_a, invoked_a) = MockCapsule::new("capsule-a", "topic.a");
        let (cap_b, invoked_b) = MockCapsule::new("capsule-b", "topic.b");

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(cap_a)).unwrap();
        registry.register(Box::new(cap_b)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let bus = Arc::new(EventBus::with_capacity(64));
        let dispatcher = EventDispatcher::new(Arc::clone(&registry), Arc::clone(&bus));
        let handle = tokio::spawn(dispatcher.run());

        tokio::task::yield_now().await;

        publish_ipc(&bus, "topic.a");
        publish_ipc(&bus, "topic.b");

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            invoked_a.load(Ordering::SeqCst),
            "capsule-a interceptor should have been invoked"
        );
        assert!(
            invoked_b.load(Ordering::SeqCst),
            "capsule-b interceptor should have been invoked"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn dispatch_ignores_lifecycle_events() {
        // Lifecycle events are handled by the Hook Bridge capsule, not the
        // EventDispatcher. Verify the dispatcher silently skips them.
        let (capsule, invoked) = MockCapsule::new("lifecycle-capsule", "tool_call_started");

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(capsule)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let bus = Arc::new(EventBus::with_capacity(64));
        let dispatcher = EventDispatcher::new(Arc::clone(&registry), Arc::clone(&bus));
        let handle = tokio::spawn(dispatcher.run());

        tokio::task::yield_now().await;

        // Publish a lifecycle event (not IPC)
        bus.publish(AstridEvent::ToolCallStarted {
            metadata: astrid_events::EventMetadata::new("test"),
            call_id: uuid::Uuid::nil(),
            tool_name: "search".into(),
            server_name: None,
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            !invoked.load(Ordering::SeqCst),
            "EventDispatcher should NOT dispatch lifecycle events (Hook Bridge handles those)"
        );

        handle.abort();
    }
}
