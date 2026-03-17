use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use astrid_events::ipc::{IpcMessage, IpcPayload};
use astrid_events::kernel_api::{KernelRequest, KernelResponse};
use tracing::{debug, info, warn};

/// Spawns background tasks for the kernel management API and connection tracking.
///
/// Two listeners:
/// 1. `astrid.v1.request.*` - handles management commands (list capsules, reload, etc.)
/// 2. `client.v1.disconnect` - decrements the active connection counter on graceful disconnect.
///
/// Connection *increment* happens when the WASM proxy capsule accepts a socket
/// connection (it publishes a `client.v1.connected` event). For ungraceful disconnects,
/// the idle monitor uses `EventBus::subscriber_count()` as a secondary signal.
#[must_use]
pub(crate) fn spawn_kernel_router(kernel: Arc<crate::Kernel>) -> tokio::task::JoinHandle<()> {
    // Spawn the connection tracker as a sibling task.
    drop(spawn_connection_tracker(Arc::clone(&kernel)));

    let mut receiver = kernel.event_bus.subscribe_topic("astrid.v1.request.*");

    tokio::spawn(async move {
        let mut rate_limiter = ManagementRateLimiter::new();

        while let Some(event) = receiver.recv().await {
            let astrid_events::AstridEvent::Ipc { message, .. } = &*event else {
                continue;
            };

            // Only process standard IPC messages that contain JSON payloads.
            let IpcPayload::RawJson(val) = &message.payload else {
                continue;
            };

            match serde_json::from_value::<KernelRequest>(val.clone()) {
                Ok(req) => {
                    let (method, limit) = rate_limit_for_request(&req);
                    if let Some(max) = limit
                        && !rate_limiter.check(method, max)
                    {
                        warn!(
                            security_event = true,
                            method = method,
                            "Rate limited kernel management request"
                        );
                        let response_topic =
                            message.topic.replace("kernel.request.", "kernel.response.");
                        publish_response(
                            &kernel,
                            response_topic,
                            KernelResponse::Error(format!(
                                "Rate limited: max {max} {method} requests per minute"
                            )),
                        );
                        continue;
                    }
                    handle_request(&kernel, message.topic.clone(), req).await;
                },
                Err(e) => {
                    warn!(error = %e, topic = %message.topic, "Failed to parse KernelRequest from IPC");
                },
            }
        }
    })
}

/// Tracks client connection lifecycle events.
///
/// Listens on `client.v1.*` topics:
/// - `client.v1.connected` - a new socket connection was accepted.
/// - `client.v1.disconnect` - a client sent a graceful disconnect.
fn spawn_connection_tracker(kernel: Arc<crate::Kernel>) -> tokio::task::JoinHandle<()> {
    let mut receiver = kernel.event_bus.subscribe_topic("client.v1.*");

    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let astrid_events::AstridEvent::Ipc { message, .. } = &*event else {
                continue;
            };
            match &message.payload {
                IpcPayload::Disconnect { reason } => {
                    kernel.connection_closed();
                    debug!(reason = ?reason, "Client disconnected");
                },
                IpcPayload::Connect => {
                    kernel.connection_opened();
                    debug!("New client connection accepted");
                },
                _ => {},
            }
        }
    })
}

#[expect(clippy::too_many_lines)]
async fn handle_request(kernel: &Arc<crate::Kernel>, topic: String, req: KernelRequest) {
    let response_topic = if let Some(suffix) = topic.strip_prefix("astrid.v1.request.") {
        format!("astrid.v1.response.{suffix}")
    } else {
        topic.clone()
    };

    let res = match req {
        KernelRequest::InstallCapsule { source, workspace } => {
            info!(source = %source, workspace, "Kernel received install request");
            // Here the kernel would verify identity, parse the capsule, and potentially
            // return ApprovalRequired if it needs dangerous capabilities!
            KernelResponse::Error(
                "Installation logic not yet implemented in kernel router".to_string(),
            )
        },
        KernelRequest::ApproveCapability {
            request_id,
            signature: _,
        } => {
            info!(request_id = %request_id, "Kernel received capability approval");
            KernelResponse::Error("Approval logic not yet implemented in kernel router".to_string())
        },
        KernelRequest::ListCapsules => {
            let reg = kernel.capsules.read().await;
            let mut list = Vec::new();
            for c in reg.list() {
                list.push(c.to_string());
            }
            KernelResponse::Success(serde_json::json!(list))
        },
        KernelRequest::GetCommands => {
            let reg = kernel.capsules.read().await;
            let mut commands = Vec::new();
            for c in reg.values() {
                for cmd in &c.manifest().commands {
                    commands.push(astrid_events::kernel_api::CommandInfo {
                        name: cmd.name.clone(),
                        description: cmd
                            .description
                            .clone()
                            .unwrap_or_else(|| "No description".to_string()),
                        provider_capsule: c.id().to_string(),
                    });
                }
            }
            info!(
                count = commands.len(),
                capsules = reg.len(),
                "GetCommands: returning {} commands from {} capsules",
                commands.len(),
                reg.len()
            );
            KernelResponse::Commands(commands)
        },
        KernelRequest::ReloadCapsules => {
            // Unregister capsules in a Failed state so they can be re-loaded
            // with fresh configuration (e.g. after onboarding writes .env.json).
            {
                let reg = kernel.capsules.read().await;
                let failed_ids: Vec<_> = reg
                    .list()
                    .into_iter()
                    .filter(|id| {
                        reg.get(id).is_some_and(|c| {
                            matches!(c.state(), astrid_capsule::capsule::CapsuleState::Failed(_))
                        })
                    })
                    .cloned()
                    .collect();
                drop(reg);

                let mut reg = kernel.capsules.write().await;
                for id in failed_ids {
                    let _ = reg.unregister(&id);
                }
            }

            kernel.load_all_capsules().await;
            KernelResponse::Success(serde_json::json!({"status": "reloaded"}))
        },
        KernelRequest::Shutdown { reason } => {
            info!(
                reason = reason.as_deref().unwrap_or("none"),
                "Kernel received shutdown request via management API"
            );
            // Publish response before signaling shutdown so the client gets confirmation.
            publish_response(
                kernel,
                response_topic.clone(),
                KernelResponse::Success(serde_json::json!({"status": "shutting_down"})),
            );
            // Signal the daemon's main loop to exit gracefully.
            let _ = kernel.shutdown_tx.send(true);
            // Return early — the daemon will call kernel.shutdown() from its main loop.
            return;
        },
        KernelRequest::GetStatus => {
            let uptime = kernel.boot_time.elapsed().as_secs();
            let reg = kernel.capsules.read().await;
            let loaded: Vec<String> = reg.list().iter().map(ToString::to_string).collect();
            let status = astrid_events::kernel_api::DaemonStatus {
                pid: std::process::id(),
                uptime_secs: uptime,
                version: env!("CARGO_PKG_VERSION").to_string(),
                ephemeral: false, // The kernel doesn't know; daemon sets this via response override if needed
                connected_clients: u32::try_from(kernel.connection_count()).unwrap_or(u32::MAX),
                loaded_capsules: loaded,
            };
            KernelResponse::Status(status)
        },
        KernelRequest::GetCapsuleMetadata => {
            let reg = kernel.capsules.read().await;
            let mut entries = Vec::new();
            for capsule in reg.values() {
                let manifest = capsule.manifest();
                entries.push(astrid_events::kernel_api::CapsuleMetadataEntry {
                    name: manifest.package.name.clone(),
                    llm_providers: manifest
                        .llm_providers
                        .iter()
                        .map(|p| astrid_events::kernel_api::LlmProviderInfo {
                            id: p.id.clone(),
                            description: p.description.clone().unwrap_or_default(),
                            capabilities: p.capabilities.clone(),
                        })
                        .collect(),
                    interceptor_events: manifest
                        .interceptors
                        .iter()
                        .map(|i| i.event.clone())
                        .collect(),
                });
            }
            KernelResponse::CapsuleMetadata(entries)
        },
    };

    publish_response(kernel, response_topic, res);
}

fn publish_response(kernel: &Arc<crate::Kernel>, response_topic: String, res: KernelResponse) {
    if let Ok(val) = serde_json::to_value(res) {
        let msg = IpcMessage::new(
            response_topic,
            IpcPayload::RawJson(val),
            kernel.session_id.0,
        );
        let _ = kernel.event_bus.publish(astrid_events::AstridEvent::Ipc {
            metadata: astrid_events::EventMetadata::new("kernel_router"),
            message: msg,
        });
    }
}

// ---------------------------------------------------------------------------
// Management API rate limiting
// ---------------------------------------------------------------------------

/// Sliding window rate limiter for management API requests.
/// Tracks per-request timestamps and evicts entries older than 60 seconds,
/// preventing the 2x burst possible with fixed-window designs.
/// Single-consumer (owned by the router task), no concurrency concerns.
struct ManagementRateLimiter {
    buckets: HashMap<&'static str, VecDeque<Instant>>,
}

impl ManagementRateLimiter {
    fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// Check if a request of the given type is within the rate limit.
    /// Returns `true` if allowed, `false` if rate-limited.
    fn check(&mut self, method: &'static str, max_per_minute: u32) -> bool {
        let now = Instant::now();
        let window = std::time::Duration::from_secs(60);
        let timestamps = self.buckets.entry(method).or_default();

        // Evict timestamps older than the 60-second sliding window.
        while let Some(&oldest) = timestamps.front() {
            if now.saturating_duration_since(oldest) >= window {
                timestamps.pop_front();
            } else {
                break;
            }
        }

        if timestamps.len() >= max_per_minute as usize {
            return false;
        }
        timestamps.push_back(now);
        true
    }
}

/// Return the rate limit label and max-per-minute for a request type.
/// Returns `None` for the limit if the request type is not rate-limited.
fn rate_limit_for_request(req: &KernelRequest) -> (&'static str, Option<u32>) {
    match req {
        KernelRequest::ReloadCapsules => ("ReloadCapsules", Some(5)),
        KernelRequest::InstallCapsule { .. } => ("InstallCapsule", Some(10)),
        KernelRequest::ApproveCapability { .. } => ("ApproveCapability", Some(10)),
        // Read-only operations are cheap - no rate limit.
        KernelRequest::ListCapsules => ("ListCapsules", None),
        KernelRequest::GetCommands => ("GetCommands", None),
        KernelRequest::GetCapsuleMetadata => ("GetCapsuleMetadata", None),
        KernelRequest::Shutdown { .. } => ("Shutdown", Some(1)),
        KernelRequest::GetStatus => ("GetStatus", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_within_limit() {
        let mut limiter = ManagementRateLimiter::new();
        for _ in 0..5 {
            assert!(limiter.check("ReloadCapsules", 5));
        }
        // 6th should be rejected
        assert!(!limiter.check("ReloadCapsules", 5));
    }

    #[test]
    fn rate_limiter_independent_buckets() {
        let mut limiter = ManagementRateLimiter::new();
        // Fill ReloadCapsules
        for _ in 0..5 {
            assert!(limiter.check("ReloadCapsules", 5));
        }
        assert!(!limiter.check("ReloadCapsules", 5));

        // InstallCapsule should still be allowed
        assert!(limiter.check("InstallCapsule", 10));
    }

    #[test]
    fn rate_limiter_sliding_window_eviction() {
        let mut limiter = ManagementRateLimiter::new();
        // Fill the bucket
        for _ in 0..5 {
            assert!(limiter.check("ReloadCapsules", 5));
        }
        assert!(!limiter.check("ReloadCapsules", 5));

        // Manually set all timestamps to 61 seconds ago to simulate expiry.
        if let Some(timestamps) = limiter.buckets.get_mut("ReloadCapsules") {
            let past = Instant::now() - std::time::Duration::from_secs(61);
            for ts in timestamps.iter_mut() {
                *ts = past;
            }
        }

        // Should be allowed again after old entries are evicted
        assert!(limiter.check("ReloadCapsules", 5));
    }

    #[test]
    fn rate_limiter_sliding_window_prevents_boundary_burst() {
        let mut limiter = ManagementRateLimiter::new();
        // Fill 5 requests
        for _ in 0..5 {
            assert!(limiter.check("ReloadCapsules", 5));
        }

        // Move only 3 of the 5 timestamps to the past (beyond 60s window).
        // This simulates partial window expiry - only 3 slots should free up.
        if let Some(timestamps) = limiter.buckets.get_mut("ReloadCapsules") {
            let past = Instant::now() - std::time::Duration::from_secs(61);
            for ts in timestamps.iter_mut().take(3) {
                *ts = past;
            }
        }

        // Should allow exactly 3 more (the evicted slots), not 5
        for _ in 0..3 {
            assert!(limiter.check("ReloadCapsules", 5));
        }
        assert!(!limiter.check("ReloadCapsules", 5));
    }

    #[test]
    fn rate_limit_for_request_returns_correct_limits() {
        let (name, limit) = rate_limit_for_request(&KernelRequest::ReloadCapsules);
        assert_eq!(name, "ReloadCapsules");
        assert_eq!(limit, Some(5));

        let (name, limit) = rate_limit_for_request(&KernelRequest::ListCapsules);
        assert_eq!(name, "ListCapsules");
        assert_eq!(limit, None);
    }
}
