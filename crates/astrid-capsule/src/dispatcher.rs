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

use crate::capsule::CapsuleId;
use crate::registry::CapsuleRegistry;
use astrid_events::{AstridEvent, EventBus};

/// Maximum time the dispatcher waits for a single interceptor invocation
/// before detaching and continuing. This is defense-in-depth: the primary
/// WASM kill mechanism is Extism's per-call timeout (~10s). The dispatch
/// timeout covers lock contention, host-function hangs, and ensures the
/// dispatcher continues regardless. It does NOT kill the blocked thread —
/// it detaches it.
const INTERCEPTOR_TIMEOUT: Duration = Duration::from_secs(15);

/// Routes IPC events from the `EventBus` to capsule interceptors.
pub struct EventDispatcher {
    registry: Arc<RwLock<CapsuleRegistry>>,
    event_bus: Arc<EventBus>,
    /// Per-interceptor timeout. Defaults to [`INTERCEPTOR_TIMEOUT`].
    timeout: Duration,
}

impl EventDispatcher {
    /// Create a new event dispatcher with the default timeout.
    #[must_use]
    pub fn new(registry: Arc<RwLock<CapsuleRegistry>>, event_bus: Arc<EventBus>) -> Self {
        Self {
            registry,
            event_bus,
            timeout: INTERCEPTOR_TIMEOUT,
        }
    }

    /// Override the per-interceptor timeout (useful for testing).
    #[cfg(test)]
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
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
                self.dispatch(message).await;
            }
        }

        debug!("Event dispatcher stopped (event bus closed)");
    }

    /// Match an IPC event against all registered interceptors and invoke matches.
    async fn dispatch(&self, message: &astrid_events::ipc::IpcMessage) {
        let topic = &message.topic;
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

        // Serialize the FULL message once for all invocations so capsules get metadata.
        let payload_bytes = match serde_json::to_vec(message) {
            Ok(bytes) => std::sync::Arc::new(bytes),
            Err(e) => {
                warn!(topic, error = %e, "Failed to serialize IPC message for dispatch");
                return;
            },
        };

        // Phase 2: invoke each matching interceptor via spawn_blocking so that
        // WASM execution (which uses block_in_place internally) doesn't stall
        // the dispatcher's async task. Each invocation is wrapped in a timeout.
        for (capsule_id, action) in matches {
            debug!(
                capsule_id = %capsule_id,
                action = %action,
                topic,
                "Dispatching interceptor"
            );

            let registry = Arc::clone(&self.registry);
            let payload = Arc::clone(&payload_bytes);
            let cid = capsule_id.clone();
            let act = action.clone();

            let handle = tokio::task::spawn_blocking(move || {
                let registry = registry.blocking_read();
                registry
                    .get(&cid)
                    .map(|capsule| capsule.invoke_interceptor(&act, &payload))
            });

            match tokio::time::timeout(self.timeout, handle).await {
                Ok(Ok(Some(Ok(_)))) => {
                    debug!(
                        capsule_id = %capsule_id,
                        action = %action,
                        "Interceptor completed"
                    );
                },
                Ok(Ok(Some(Err(e)))) => {
                    warn!(
                        capsule_id = %capsule_id,
                        action = %action,
                        topic,
                        error = %e,
                        "Interceptor invocation failed"
                    );
                },
                Ok(Ok(None)) => {
                    debug!(
                        capsule_id = %capsule_id,
                        "Capsule no longer registered, skipping interceptor"
                    );
                },
                Ok(Err(e)) => {
                    warn!(
                        capsule_id = %capsule_id,
                        action = %action,
                        error = %e,
                        "Interceptor task panicked"
                    );
                },
                Err(_) => {
                    warn!(
                        capsule_id = %capsule_id,
                        action = %action,
                        topic,
                        "Interceptor timed out after {}s (blocked thread detached)",
                        self.timeout.as_secs()
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

    // ── Dispatch integration tests ──────────────────────────────────

    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;

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
                // Simulate a hung interceptor. The dispatcher's timeout should
                // detach this thread and continue processing. Uses a sleep
                // longer than the test timeout (1s) so the dispatcher must
                // detach rather than wait. Kept short (3s) to avoid blocking
                // test runtime shutdown.
                std::thread::sleep(Duration::from_secs(3));
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
    async fn dispatch_timeout_does_not_block_dispatcher() {
        // Create a capsule that blocks for 60s on invoke.
        let (blocking_capsule, blocking_invoked) =
            MockCapsule::new("blocking-capsule", "block.topic", true);
        // Create a normal capsule on a different topic.
        let (normal_capsule, normal_invoked) =
            MockCapsule::new("normal-capsule", "normal.topic", false);

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(blocking_capsule)).unwrap();
        registry.register(Box::new(normal_capsule)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let bus = Arc::new(EventBus::with_capacity(64));
        // Use a 1-second timeout for fast tests.
        let dispatcher = EventDispatcher::new(Arc::clone(&registry), Arc::clone(&bus))
            .with_timeout(Duration::from_secs(1));
        let handle = tokio::spawn(dispatcher.run());

        tokio::task::yield_now().await;

        // Publish to the blocking capsule first, then the normal one.
        publish_ipc(&bus, "block.topic");
        tokio::time::sleep(Duration::from_millis(50)).await;
        publish_ipc(&bus, "normal.topic");

        // The timeout is 1s. The normal capsule should be invoked shortly
        // after the blocking capsule's timeout fires. Wait up to 5s.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !normal_invoked.load(Ordering::SeqCst) {
            if tokio::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        assert!(
            blocking_invoked.load(Ordering::SeqCst),
            "blocking interceptor should have been entered"
        );
        assert!(
            normal_invoked.load(Ordering::SeqCst),
            "normal interceptor should have been invoked despite blocking capsule"
        );

        handle.abort();
    }
}
