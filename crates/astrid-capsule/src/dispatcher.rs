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
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Default timeout for interceptor invocations.
const DEFAULT_INTERCEPTOR_TIMEOUT: Duration = Duration::from_secs(15);

use crate::registry::CapsuleRegistry;
use astrid_events::{AstridEvent, EventBus};

/// Routes IPC events from the `EventBus` to capsule interceptors.
pub struct EventDispatcher {
    registry: Arc<RwLock<CapsuleRegistry>>,
    event_bus: Arc<EventBus>,
    interceptor_timeout: Duration,
}

impl EventDispatcher {
    /// Create a new event dispatcher.
    #[must_use]
    pub fn new(registry: Arc<RwLock<CapsuleRegistry>>, event_bus: Arc<EventBus>) -> Self {
        Self {
            registry,
            event_bus,
            interceptor_timeout: DEFAULT_INTERCEPTOR_TIMEOUT,
        }
    }

    /// Create a new event dispatcher with a custom interceptor timeout.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn with_timeout(
        registry: Arc<RwLock<CapsuleRegistry>>,
        event_bus: Arc<EventBus>,
        interceptor_timeout: Duration,
    ) -> Self {
        Self {
            registry,
            event_bus,
            interceptor_timeout,
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
                self.dispatch(message);
            }
        }

        debug!("Event dispatcher stopped (event bus closed)");
    }

    /// Match an IPC event against all registered interceptors and invoke matches.
    ///
    /// Interceptors are dispatched concurrently — each gets its own spawned task
    /// with an independent timeout. This method returns immediately after spawning,
    /// so the event loop is never blocked by slow or misbehaving interceptors.
    fn dispatch(&self, message: &astrid_events::ipc::IpcMessage) {
        let topic = message.topic.clone();
        let registry = Arc::clone(&self.registry);
        let timeout = self.interceptor_timeout;

        // Serialize payload eagerly so all interceptors share the same bytes.
        let payload_bytes = match serde_json::to_vec(message) {
            Ok(bytes) => Arc::new(bytes),
            Err(e) => {
                warn!(topic, error = %e, "Failed to serialize IPC message for dispatch");
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
                let act = action.clone();
                let payload = Arc::clone(&payload_bytes);
                let topic = topic.clone();

                // Each interceptor runs independently with its own timeout.
                // Spawned on a Tokio worker thread so block_in_place (used by
                // invoke_interceptor and WASM host functions) works correctly.
                // Requires a multi-thread Tokio runtime.
                tokio::task::spawn(async move {
                    debug!(
                        capsule_id = %capsule_id,
                        action = %act,
                        topic,
                        "Dispatching interceptor"
                    );

                    let mut handle =
                        tokio::task::spawn(
                            async move { capsule.invoke_interceptor(&act, &payload) },
                        );

                    match tokio::time::timeout(timeout, &mut handle).await {
                        Ok(Ok(Ok(_))) => {
                            debug!(
                                capsule_id = %capsule_id,
                                action,
                                "Interceptor completed"
                            );
                        },
                        Ok(Ok(Err(e))) => {
                            warn!(
                                capsule_id = %capsule_id,
                                action,
                                topic,
                                error = %e,
                                "Interceptor invocation failed"
                            );
                        },
                        Ok(Err(e)) => {
                            warn!(
                                capsule_id = %capsule_id,
                                action,
                                error = %e,
                                "Interceptor task panicked"
                            );
                        },
                        Err(_) => {
                            handle.abort();
                            warn!(
                                capsule_id = %capsule_id,
                                action,
                                topic,
                                "Interceptor timed out after {timeout:?}, aborting task"
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
        /// When `true`, `invoke_interceptor` blocks forever (for timeout tests).
        block_forever: bool,
    }

    impl MockCapsule {
        fn new(
            name: &str,
            interceptor_event: &str,
            block_forever: bool,
        ) -> (Self, Arc<AtomicBool>) {
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
                block_forever,
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
            if self.block_forever {
                // Simulate a hung interceptor. Sleeps for 5s which is longer
                // than the test timeout (500ms), so the dispatcher must abort
                // this task and continue processing.
                std::thread::sleep(Duration::from_secs(5));
            }
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
        let (capsule, invoked) = MockCapsule::new("test-capsule", "test.topic", false);

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
        let (capsule, invoked) = MockCapsule::new("test-capsule-skip", "specific.topic", false);

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
    async fn dispatch_timeout_aborts_and_continues() {
        // Blocking capsule sleeps for 5s, but we set the timeout to 500ms.
        let (blocking_capsule, blocking_invoked) =
            MockCapsule::new("blocking-capsule", "block.topic", true);
        let (normal_capsule, normal_invoked) =
            MockCapsule::new("normal-capsule", "normal.topic", false);

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(blocking_capsule)).unwrap();
        registry.register(Box::new(normal_capsule)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let bus = Arc::new(EventBus::with_capacity(64));
        let dispatcher = EventDispatcher::with_timeout(
            Arc::clone(&registry),
            Arc::clone(&bus),
            Duration::from_millis(500),
        );
        let handle = tokio::spawn(dispatcher.run());

        tokio::task::yield_now().await;

        // Publish to the blocking capsule first, then the normal one.
        publish_ipc(&bus, "block.topic");
        tokio::time::sleep(Duration::from_millis(50)).await;
        publish_ipc(&bus, "normal.topic");

        // The 500ms timeout fires, the blocking task is aborted, and the
        // dispatcher continues to process the normal event. Wait up to 2s.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !normal_invoked.load(Ordering::SeqCst) {
            if tokio::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        assert!(
            blocking_invoked.load(Ordering::SeqCst),
            "blocking interceptor should have been entered before timeout"
        );
        assert!(
            normal_invoked.load(Ordering::SeqCst),
            "normal interceptor should have been invoked after timeout abort"
        );

        handle.abort();
    }
}
