#![deny(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(unreachable_pub)]

//! Hook Bridge capsule — translates kernel `AstridEvent` broadcasts into
//! plugin hook interceptor calls.
//!
//! The kernel fires raw events. Plugin capsules expect named hooks with
//! request-response semantics. The Hook Bridge sits between them, owning
//! the event-to-hook mapping and response merge semantics.
//!
//! # Architecture
//!
//! ```text
//! Kernel emits AstridEvent::ToolCallStarted
//!     → Hook Bridge receives broadcast
//!     → Hook Bridge fires "before_tool_call" interceptor on all subscriber capsules
//!     → Plugin capsules respond (e.g., { skip: true })
//!     → Hook Bridge merges responses and publishes result event
//! ```

mod mapping;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use astrid_capsule::capsule::{Capsule, CapsuleId, CapsuleState};
use astrid_capsule::context::CapsuleContext;
use astrid_capsule::error::{CapsuleError, CapsuleResult};
use astrid_capsule::manifest::{CapabilitiesDef, CapsuleManifest, PackageDef};
use astrid_capsule::registry::CapsuleRegistry;
use astrid_capsule::tool::CapsuleTool;
use astrid_events::{AstridEvent, EventBus, EventMetadata, IpcMessage, IpcPayload};

use mapping::{HookMapping, MergeSemantics};

/// The Hook Bridge capsule ID.
const CAPSULE_ID: &str = "hook-bridge";

/// Maximum time to wait for a single interceptor invocation before giving up.
/// Prevents a hung WASM guest from blocking the dispatch task indefinitely.
const INTERCEPTOR_TIMEOUT: Duration = Duration::from_secs(30);

/// Hook Bridge capsule.
///
/// A native Rust capsule that subscribes to kernel `AstridEvent` broadcasts
/// and translates them into interceptor calls on plugin capsules that
/// registered for the corresponding hook names.
pub struct HookBridgeCapsule {
    id: CapsuleId,
    manifest: CapsuleManifest,
    state: CapsuleState,
    registry: Arc<RwLock<CapsuleRegistry>>,
    /// Handle to the background dispatch task; aborted on unload.
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl HookBridgeCapsule {
    /// Create a new Hook Bridge capsule.
    ///
    /// # Arguments
    ///
    /// * `registry` — shared capsule registry for looking up subscriber capsules
    #[must_use]
    pub fn new(registry: Arc<RwLock<CapsuleRegistry>>) -> Self {
        Self {
            id: CapsuleId::from_static(CAPSULE_ID),
            manifest: build_manifest(),
            state: CapsuleState::Unloaded,
            registry,
            task_handle: None,
        }
    }
}

#[async_trait]
impl Capsule for HookBridgeCapsule {
    fn id(&self) -> &CapsuleId {
        &self.id
    }

    fn manifest(&self) -> &CapsuleManifest {
        &self.manifest
    }

    fn state(&self) -> CapsuleState {
        self.state.clone()
    }

    async fn load(&mut self, ctx: &CapsuleContext) -> CapsuleResult<()> {
        self.state = CapsuleState::Loading;

        let event_bus = Arc::clone(&ctx.event_bus);
        let registry = Arc::clone(&self.registry);

        let handle = tokio::spawn(dispatch_loop(event_bus, registry));
        self.task_handle = Some(handle);

        self.state = CapsuleState::Ready;
        debug!("Hook Bridge capsule loaded");
        Ok(())
    }

    async fn unload(&mut self) -> CapsuleResult<()> {
        self.state = CapsuleState::Unloading;
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
        self.state = CapsuleState::Unloaded;
        debug!("Hook Bridge capsule unloaded");
        Ok(())
    }

    fn tools(&self) -> &[Arc<dyn CapsuleTool>] {
        &[]
    }
}

/// Main dispatch loop: subscribes to all events on the bus and routes them
/// to interceptors via the hook mapping.
async fn dispatch_loop(event_bus: Arc<EventBus>, registry: Arc<RwLock<CapsuleRegistry>>) {
    let mut receiver = event_bus.subscribe();
    debug!("Hook Bridge dispatch loop started");

    while let Some(event) = receiver.recv().await {
        // Skip IPC events — those are handled by the existing EventDispatcher.
        if matches!(&*event, AstridEvent::Ipc { .. }) {
            continue;
        }

        let Some(mapping) = HookMapping::from_event(&event) else {
            continue;
        };

        let registry = Arc::clone(&registry);
        let event_bus = Arc::clone(&event_bus);

        // Spawn each hook dispatch as an independent task so the event loop
        // is never blocked by slow interceptors.
        tokio::spawn(async move {
            dispatch_hook(&event, &mapping, &registry, &event_bus).await;
        });
    }

    debug!("Hook Bridge dispatch loop stopped (event bus closed)");
}

/// Dispatch a single hook: find subscriber capsules, invoke interceptors,
/// merge responses, and publish results.
async fn dispatch_hook(
    event: &AstridEvent,
    mapping: &HookMapping,
    registry: &Arc<RwLock<CapsuleRegistry>>,
    event_bus: &Arc<EventBus>,
) {
    let hook_name = mapping.hook_name;
    let payload = build_hook_payload(event);

    let payload_bytes = match serde_json::to_vec(&payload) {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(hook = hook_name, error = %e, "Failed to serialize hook payload");
            return;
        },
    };

    let subscribers = collect_subscribers(registry, hook_name).await;

    if subscribers.is_empty() {
        return;
    }

    debug!(
        hook = hook_name,
        subscriber_count = subscribers.len(),
        "Dispatching hook to subscribers"
    );

    // Invoke all interceptors and collect responses.
    // Each interceptor is called via `block_in_place` because
    // `invoke_interceptor` is synchronous (may block on WASM execution).
    // A timeout prevents hung guests from blocking the dispatch task.
    // Errors from one capsule never short-circuit the remaining capsules.
    let mut responses: Vec<serde_json::Value> = Vec::new();

    for (capsule, action) in &subscribers {
        let capsule_id_str = capsule.id().to_string();
        let capsule = Arc::clone(capsule);
        let action = action.clone();
        let payload = payload_bytes.clone();

        let result = tokio::time::timeout(INTERCEPTOR_TIMEOUT, async {
            tokio::task::spawn_blocking(move || capsule.invoke_interceptor(&action, &payload)).await
        })
        .await;

        match result {
            Ok(Ok(Ok(bytes))) if bytes.is_empty() => {},
            Ok(Ok(Ok(bytes))) => {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    responses.push(val);
                }
            },
            Ok(Ok(Err(CapsuleError::NotSupported(_)))) => {},
            Ok(Ok(Err(e))) => {
                warn!(
                    hook = hook_name,
                    capsule_id = %capsule_id_str,
                    error = %e,
                    "Interceptor invocation failed"
                );
            },
            Ok(Err(e)) => {
                warn!(
                    hook = hook_name,
                    capsule_id = %capsule_id_str,
                    error = %e,
                    "Interceptor task panicked"
                );
            },
            Err(_) => {
                warn!(
                    hook = hook_name,
                    capsule_id = %capsule_id_str,
                    timeout_secs = INTERCEPTOR_TIMEOUT.as_secs(),
                    "Interceptor timed out"
                );
            },
        }
    }

    // Fire-and-forget hooks: no merge, no result event.
    if matches!(mapping.merge, MergeSemantics::None) {
        return;
    }

    // Merge responses and publish the decision event.
    let merged = merge_responses(&responses, &mapping.merge);

    let result_topic = format!("hook_bridge.{hook_name}.decision");
    let msg = IpcMessage::new(
        &result_topic,
        IpcPayload::RawJson(merged),
        uuid::Uuid::nil(),
    );

    event_bus.publish(AstridEvent::Ipc {
        metadata: EventMetadata::new("hook-bridge"),
        message: msg,
    });

    debug!(hook = hook_name, topic = %result_topic, "Published hook decision");
}

/// Collect capsules that registered interceptors for the given hook name.
///
/// Results are sorted by `CapsuleId` for deterministic iteration order so that
/// "last non-null wins" merge semantics produce predictable results. The hook
/// bridge itself is excluded to prevent infinite dispatch loops.
async fn collect_subscribers(
    registry: &Arc<RwLock<CapsuleRegistry>>,
    hook_name: &str,
) -> Vec<(Arc<dyn Capsule>, String)> {
    let reg = registry.read().await;
    let mut subs = Vec::new();
    for capsule_id in reg.list() {
        if let Some(capsule) = reg.get(capsule_id) {
            if !matches!(capsule.state(), CapsuleState::Ready) {
                continue;
            }
            // Skip ourselves to avoid infinite loops.
            if capsule.id().as_str() == CAPSULE_ID {
                continue;
            }
            for interceptor in &capsule.manifest().interceptors {
                if interceptor.event == hook_name {
                    subs.push((Arc::clone(&capsule), interceptor.action.clone()));
                }
            }
        }
    }
    // Sort by capsule ID for deterministic merge order.
    subs.sort_by(|(a, _), (b, _)| a.id().as_str().cmp(b.id().as_str()));
    subs
}

/// Build the JSON payload from an `AstridEvent` to send to interceptors.
fn build_hook_payload(event: &AstridEvent) -> serde_json::Value {
    // Serialize the full event — interceptors get all fields.
    serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({}))
}

/// Merge interceptor responses according to the hook's merge semantics.
fn merge_responses(
    responses: &[serde_json::Value],
    semantics: &MergeSemantics,
) -> serde_json::Value {
    match semantics {
        MergeSemantics::None => serde_json::json!({}),

        MergeSemantics::ToolCallBefore => {
            // Any `skip: true` → skip. Last non-null `modified_params` wins.
            let mut skip = false;
            let mut modified_params: Option<&serde_json::Value> = None;

            for resp in responses {
                if resp.get("skip").and_then(serde_json::Value::as_bool) == Some(true) {
                    skip = true;
                }
                if let Some(params) = resp.get("modified_params")
                    && !params.is_null()
                {
                    modified_params = Some(params);
                }
            }

            serde_json::json!({
                "skip": skip,
                "modified_params": modified_params,
            })
        },

        MergeSemantics::LastNonNull { field } => {
            // Last non-null value for the given field wins.
            let mut result: Option<&serde_json::Value> = None;
            for resp in responses {
                if let Some(val) = resp.get(*field)
                    && !val.is_null()
                {
                    result = Some(val);
                }
            }

            let mut obj = serde_json::Map::new();
            if let Some(val) = result {
                obj.insert((*field).to_string(), val.clone());
            }
            serde_json::Value::Object(obj)
        },
    }
}

/// Build the capsule manifest for the Hook Bridge.
///
/// The manifest declares no interceptors, tools, or capabilities — the bridge
/// directly subscribes to the `EventBus` rather than using the topic-matching
/// interceptor pattern.
fn build_manifest() -> CapsuleManifest {
    CapsuleManifest {
        package: PackageDef {
            name: CAPSULE_ID.to_string(),
            version: "0.1.0".to_string(),
            description: Some(
                "Translates kernel events into plugin hook interceptor calls".to_string(),
            ),
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
            publish: Some(false),
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
        interceptors: Vec::new(),
        cron_jobs: Vec::new(),
        tools: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
