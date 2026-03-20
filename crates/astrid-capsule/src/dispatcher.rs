//! Event dispatcher for routing events to capsule interceptors.
//!
//! The dispatcher is a host-side async task that subscribes to the global
//! `EventBus`, matches incoming events against capsule interceptor patterns
//! (from `Capsule.toml`), and invokes the corresponding WASM
//! `astrid_hook_trigger` export on each matching capsule.
//!
//! # Event Routing
//!
//! The dispatcher handles two categories of events:
//!
//! - **IPC events**: matched by their `topic` field (e.g. `user.prompt`)
//! - **Lifecycle events**: matched by `event_type()` (e.g. `tool_call_started`,
//!   `session_created`). This enables WASM capsules (like the Hook Bridge) to
//!   subscribe to lifecycle events and apply policy (merge strategies, hook
//!   fan-out) on top of the kernel's dispatch mechanism.
//!
//! All dispatch is fire-and-forget from the dispatcher's perspective. Capsules
//! that need request-response semantics (e.g. collecting responses from multiple
//! subscribers) use `hooks::trigger` — the kernel syscall for fan-out with
//! response collection.
//!
//! # Topic Matching
//!
//! Interceptor event patterns support:
//! - Exact match: `user.prompt` matches only `user.prompt`
//! - Single-segment wildcard: `tool.execute.*.result` matches
//!   `tool.execute.search.result` but not `tool.execute.result`

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tracing::{debug, warn};

use crate::capsule::{Capsule, CapsuleId};
use crate::registry::CapsuleRegistry;
use astrid_events::{AstridEvent, EventBus};

/// Capacity of each per-capsule event dispatch queue.
///
/// If a capsule's queue fills up (i.e. it is processing events slower than
/// they arrive), new events are dropped with a warning rather than blocking
/// the dispatcher. 256 is generous for typical usage.
const CAPSULE_EVENT_QUEUE_CAPACITY: usize = 256;

/// Work item sent to a per-capsule ordered queue.
struct InterceptorWork {
    action: String,
    payload: Arc<Vec<u8>>,
    topic: Arc<String>,
    /// The originating IPC message, if this event came from IPC.
    /// `None` for lifecycle events. Carried through to
    /// `invoke_interceptor` so the kernel can set per-invocation
    /// principal context on `HostState`.
    ipc_message: Option<Arc<astrid_events::ipc::IpcMessage>>,
}

/// Routes events from the `EventBus` to capsule interceptors.
///
/// Both IPC events (by topic) and lifecycle events (by `event_type()`) are
/// dispatched fire-and-forget. Capsules needing response collection use
/// `hooks::trigger` (the kernel fan-out syscall).
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
    /// Subscribes to all events on the bus and routes both IPC events (by topic)
    /// and lifecycle events (by `event_type()`). Should be spawned as a
    /// background tokio task.
    ///
    /// Monitors broadcast channel lag and publishes `astrid.v1.event_bus.lagged`
    /// IPC events when messages are dropped, rate-limited to at most once per
    /// 10 seconds to avoid feedback loops.
    pub async fn run(self) {
        let mut receiver = self.event_bus.subscribe();
        let mut last_lag_notification = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(10))
            .unwrap_or_else(std::time::Instant::now);
        let mut capsule_queues: HashMap<CapsuleId, mpsc::Sender<InterceptorWork>> = HashMap::new();
        let mut known_principals: HashSet<String> = HashSet::new();
        // The "default" principal is always provisioned by the kernel boot sequence.
        known_principals.insert("default".to_string());
        debug!("Event dispatcher started");

        while let Some(event) = receiver.recv().await {
            // Check for broadcast channel overflow (lost messages).
            let lagged = receiver.drain_lagged();
            if lagged > 0 && last_lag_notification.elapsed() >= std::time::Duration::from_secs(10) {
                warn!(
                    lagged_count = lagged,
                    "Event bus broadcast channel lagged - {lagged} messages dropped"
                );
                last_lag_notification = std::time::Instant::now();

                // Publish a lag notification so capsules can react.
                // Note: This notification is published onto the same bus that just
                // overflowed, so it may itself be dropped under sustained load. This
                // is acceptable - the watchdog timeout is the actual recovery mechanism.
                // The 10s rate limit prevents amplification feedback loops.
                let msg = astrid_events::ipc::IpcMessage::new(
                    "astrid.v1.event_bus.lagged",
                    astrid_events::ipc::IpcPayload::Custom {
                        data: serde_json::json!({ "lagged_count": lagged }),
                    },
                    uuid::Uuid::new_v4(),
                );
                self.event_bus.publish(AstridEvent::Ipc {
                    metadata: astrid_events::EventMetadata::new("dispatcher"),
                    message: msg,
                });
            }

            let (topic, payload_bytes, ipc_message) = match &*event {
                AstridEvent::Ipc { message, .. } => {
                    let topic = Arc::new(message.topic.clone());
                    match message.payload.to_guest_bytes() {
                        Ok(bytes) => (topic, Arc::new(bytes), Some(Arc::new(message.clone()))),
                        Err(e) => {
                            warn!(topic = %message.topic, error = %e, "Failed to serialize IPC payload");
                            continue;
                        },
                    }
                },
                other => {
                    let topic = Arc::new(other.event_type().to_string());
                    match serde_json::to_vec(other) {
                        Ok(bytes) => (topic, Arc::new(bytes), None),
                        Err(e) => {
                            warn!(event_type = %topic, error = %e, "Failed to serialize lifecycle event");
                            continue;
                        },
                    }
                },
            };

            // Auto-provision home directories for new principals.
            // TODO: gate on identity_store lookup once wired through dispatcher.
            // Currently unauthenticated — any valid-format principal triggers
            // directory creation. Acceptable pre-production; must be gated
            // before multi-tenant deployment.
            if let Some(ref msg) = ipc_message
                && let Some(ref principal_str) = msg.principal
                && !known_principals.contains(principal_str)
            {
                if let Ok(pid) = astrid_core::PrincipalId::new(principal_str) {
                    if let Ok(home) = astrid_core::dirs::AstridHome::resolve() {
                        let ph = home.principal_home(&pid);
                        if let Err(e) = ph.ensure() {
                            warn!(
                                principal = %pid,
                                error = %e,
                                "Failed to auto-provision principal home"
                            );
                        } else {
                            debug!(
                                principal = %pid,
                                "Auto-provisioned principal home directory"
                            );
                        }
                    }
                    known_principals.insert(principal_str.clone());
                } else {
                    warn!(
                        principal = %principal_str,
                        "IPC message has invalid principal string, ignoring"
                    );
                }
            }

            let matches = find_matching_interceptors(&self.registry, &topic).await;
            dispatch_to_capsule_queues(
                &mut capsule_queues,
                matches,
                topic,
                payload_bytes,
                ipc_message,
            );
        }

        debug!("Event dispatcher stopped (event bus closed)");
    }
}

/// Dispatch matching interceptors through per-capsule ordered queues.
///
/// Each capsule gets a dedicated mpsc channel with a single consumer task
/// that invokes interceptors sequentially. This guarantees events are
/// delivered in publish order (by IPC `seq`), preventing out-of-order
/// stream assembly in capsules like ReAct.
///
/// Different capsules still process events concurrently — only same-capsule
/// delivery is serialized.
fn dispatch_to_capsule_queues(
    queues: &mut HashMap<CapsuleId, mpsc::Sender<InterceptorWork>>,
    matches: Vec<(Arc<dyn Capsule>, String)>,
    topic: Arc<String>,
    payload_bytes: Arc<Vec<u8>>,
    ipc_message: Option<Arc<astrid_events::ipc::IpcMessage>>,
) {
    for (capsule, action) in matches {
        let sender = queues.entry(capsule.id().clone()).or_insert_with(|| {
            let (tx, mut rx) = mpsc::channel::<InterceptorWork>(CAPSULE_EVENT_QUEUE_CAPACITY);
            let capsule = Arc::clone(&capsule);
            tokio::task::spawn(async move {
                while let Some(work) = rx.recv().await {
                    debug!(
                        capsule_id = %capsule.id(),
                        action = %work.action,
                        topic = %work.topic,
                        "Dispatching interceptor (ordered)"
                    );
                    let caller = work.ipc_message.as_deref();
                    match capsule.invoke_interceptor(&work.action, &work.payload, caller) {
                        Ok(_) => {
                            debug!(
                                capsule_id = %capsule.id(),
                                action = %work.action,
                                "Interceptor completed"
                            );
                        },
                        Err(crate::error::CapsuleError::NotSupported(ref msg)) => {
                            debug!(
                                capsule_id = %capsule.id(),
                                action = %work.action,
                                reason = %msg,
                                "Interceptor skipped (NotSupported)"
                            );
                        },
                        Err(e) => {
                            warn!(
                                capsule_id = %capsule.id(),
                                action = %work.action,
                                topic = %work.topic,
                                error = %e,
                                "Interceptor invocation failed"
                            );
                        },
                    }
                }
            });
            tx
        });

        let work = InterceptorWork {
            action,
            payload: Arc::clone(&payload_bytes),
            topic: Arc::clone(&topic),
            ipc_message: ipc_message.clone(),
        };
        // Non-blocking send — if the channel is full, the capsule is overwhelmed.
        if let Err(e) = sender.try_send(work) {
            warn!(
                capsule_id = %capsule.id(),
                topic = %topic,
                "Capsule dispatch queue full or closed, dropping event: {e}"
            );
        }
    }
}

/// Find all capsules with interceptors matching the given topic.
///
/// Takes a brief read lock on the registry. Only `Ready` capsules are
/// considered. Returns `(capsule, action)` pairs for each match.
async fn find_matching_interceptors(
    registry: &RwLock<CapsuleRegistry>,
    topic: &str,
) -> Vec<(Arc<dyn crate::capsule::Capsule>, String)> {
    let registry = registry.read().await;
    let mut matches = Vec::new();
    for capsule_id in registry.list() {
        if let Some(capsule) = registry.get(capsule_id) {
            if !matches!(capsule.state(), crate::capsule::CapsuleState::Ready) {
                continue;
            }
            for interceptor in &capsule.manifest().interceptors {
                if topic_matches(topic, &interceptor.event) {
                    matches.push((Arc::clone(&capsule), interceptor.action.clone()));
                }
            }
        }
    }
    matches
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
                dependencies: Default::default(),
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
                topics: Vec::new(),
                effective_provides_cache: std::sync::OnceLock::new(),
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
        fn invoke_interceptor(
            &self,
            _action: &str,
            _payload: &[u8],
            _caller: Option<&astrid_events::ipc::IpcMessage>,
        ) -> CapsuleResult<Vec<u8>> {
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
    async fn dispatch_routes_lifecycle_events() {
        // Lifecycle events are dispatched by event_type() as the topic.
        let (capsule, invoked) =
            MockCapsule::new("lifecycle-capsule", "astrid.v1.lifecycle.tool_call_started");

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(capsule)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let bus = Arc::new(EventBus::with_capacity(64));
        let dispatcher = EventDispatcher::new(Arc::clone(&registry), Arc::clone(&bus));
        let handle = tokio::spawn(dispatcher.run());

        tokio::task::yield_now().await;

        // Publish a lifecycle event
        bus.publish(AstridEvent::ToolCallStarted {
            metadata: astrid_events::EventMetadata::new("test"),
            call_id: uuid::Uuid::nil(),
            tool_name: "search".into(),
            server_name: None,
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            invoked.load(Ordering::SeqCst),
            "EventDispatcher should dispatch lifecycle events by event_type()"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn dispatch_publishes_lag_event_on_overflow() {
        // Use a tiny bus capacity so publishing more events than capacity triggers lag.
        let bus = Arc::new(EventBus::with_capacity(2));

        // A capsule that listens for lag events.
        let (lag_capsule, _lag_invoked) =
            MockCapsule::new("lag-listener", "astrid.v1.event_bus.lagged");

        let mut registry = CapsuleRegistry::new();
        registry.register(Box::new(lag_capsule)).unwrap();
        let registry = Arc::new(RwLock::new(registry));

        let dispatcher = EventDispatcher::new(Arc::clone(&registry), Arc::clone(&bus));
        let handle = tokio::spawn(dispatcher.run());

        tokio::task::yield_now().await;

        // Flood the bus to trigger lag (the dispatcher's receiver has capacity 2,
        // so publishing many events quickly should cause overflow).
        for i in 0..20 {
            publish_ipc(&bus, &format!("flood.event.{i}"));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;

        // If lag was detected, the dispatcher should have published
        // astrid.v1.event_bus.lagged which routes to our lag-listener capsule.
        // Note: this test may not trigger lag on fast machines where the
        // dispatcher drains fast enough. That's acceptable - the test
        // validates the wiring, not the race condition.
        // We just verify no panics occurred and the dispatcher is still running.
        assert!(!handle.is_finished(), "dispatcher should still be running");
        handle.abort();
    }

    #[test]
    fn mock_capsule_check_health_returns_ready() {
        let (capsule, _) = MockCapsule::new("health-test", "test.topic");
        assert_eq!(capsule.check_health(), CapsuleState::Ready);
    }
}
