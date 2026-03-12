use astrid_events::ipc::{IpcMessage, IpcPayload};
use astrid_events::kernel_api::{KernelRequest, KernelResponse};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Spawns background tasks for the kernel management API and connection tracking.
///
/// Two listeners:
/// 1. `kernel.request.*` - handles management commands (list capsules, reload, etc.)
/// 2. `client.disconnect` - decrements the active connection counter on graceful disconnect.
///
/// Connection *increment* happens when the WASM proxy capsule accepts a socket
/// connection (it publishes a `client.connected` event). For ungraceful disconnects,
/// the idle monitor uses `EventBus::subscriber_count()` as a secondary signal.
#[must_use]
pub(crate) fn spawn_kernel_router(kernel: Arc<crate::Kernel>) -> tokio::task::JoinHandle<()> {
    // Spawn the connection tracker as a sibling task.
    drop(spawn_connection_tracker(Arc::clone(&kernel)));

    let mut receiver = kernel.event_bus.subscribe_topic("kernel.request.*");

    tokio::spawn(async move {
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
/// Listens on `client.*` topics:
/// - `client.connected` - a new socket connection was accepted.
/// - `client.disconnect` - a client sent a graceful disconnect.
fn spawn_connection_tracker(kernel: Arc<crate::Kernel>) -> tokio::task::JoinHandle<()> {
    let mut receiver = kernel.event_bus.subscribe_topic("client.*");

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
                IpcPayload::RawJson(val) => {
                    // client.connected events from the proxy capsule.
                    if message.topic == "client.connected"
                        && val.get("status").and_then(|v| v.as_str()) == Some("connected")
                    {
                        kernel.connection_opened();
                        debug!("New client connection accepted");
                    }
                },
                _ => {},
            }
        }
    })
}

#[expect(clippy::too_many_lines)]
async fn handle_request(kernel: &Arc<crate::Kernel>, topic: String, req: KernelRequest) {
    let response_topic = topic.replace("kernel.request.", "kernel.response.");

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

    // Publish response back to the bus
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
