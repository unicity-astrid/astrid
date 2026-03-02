use std::sync::Arc;
use astrid_events::ipc::{IpcMessage, IpcPayload};
use astrid_events::kernel_api::{KernelRequest, KernelResponse};
use tracing::{info, warn};

/// Spawns a background task that listens to the Event Bus for `kernel.request.*` topics.
#[must_use]
pub fn spawn_kernel_router(kernel: Arc<crate::Kernel>) -> tokio::task::JoinHandle<()> {
    let mut receiver = kernel.event_bus.subscribe_topic("kernel.request.*");

    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let astrid_events::AstridEvent::Ipc { message, .. } = &*event else { continue };
            
            // Only process standard IPC messages that contain JSON payloads.
            let IpcPayload::RawJson(val) = &message.payload else { continue };

            match serde_json::from_value::<KernelRequest>(val.clone()) {
                Ok(req) => {
                    handle_request(&kernel, message.topic.clone(), req).await;
                }
                Err(e) => {
                    warn!(error = %e, topic = %message.topic, "Failed to parse KernelRequest from IPC");
                }
            }
        }
    })
}

async fn handle_request(kernel: &Arc<crate::Kernel>, topic: String, req: KernelRequest) {
    let response_topic = topic.replace("kernel.request.", "kernel.response.");
    
    let res = match req {
        KernelRequest::InstallCapsule { source, workspace } => {
            info!(source = %source, workspace, "Kernel received install request");
            // Here the kernel would verify identity, parse the capsule, and potentially
            // return ApprovalRequired if it needs dangerous capabilities!
            KernelResponse::Error("Installation logic not yet implemented in kernel router".to_string())
        }
        KernelRequest::ApproveCapability { request_id, signature: _ } => {
            info!(request_id = %request_id, "Kernel received capability approval");
            KernelResponse::Error("Approval logic not yet implemented in kernel router".to_string())
        }
        KernelRequest::ListSessions => {
            // Because sessions are managed by capsules in the microkernel, the OS 
            // itself might just broadcast a "sessions.request" and gather them,
            // or query the storage layer!
            KernelResponse::Success(serde_json::json!([]))
        }
        KernelRequest::ListCapsules => {
            let reg = kernel.capsules.read().await;
            let mut list = Vec::new();
            for c in reg.list() {
                list.push(c.to_string());
            }
            KernelResponse::Success(serde_json::json!(list))
        }
    };
    
    // Publish response back to the bus
    if let Ok(val) = serde_json::to_value(res) {
        let msg = IpcMessage::new(response_topic, IpcPayload::RawJson(val), astrid_core::types::SessionId::from_uuid(uuid::Uuid::new_v4()).0);
        let _ = kernel.event_bus.publish(astrid_events::AstridEvent::Ipc {
            metadata: astrid_events::EventMetadata::new("kernel_router"),
            message: msg,
        });
    }
}