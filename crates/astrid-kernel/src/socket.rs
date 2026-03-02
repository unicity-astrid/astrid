use std::path::PathBuf;
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn, error};
use std::sync::Arc;
use astrid_events::EventBus;

/// Path to the local Unix Domain Socket for the daemon.
#[must_use]
pub fn daemon_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".astrid/daemon.sock")
}

/// Spawns a background task that listens for local IPC connections via Unix Domain Sockets.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn spawn_socket_server(event_bus: Arc<EventBus>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let path = daemon_socket_path();
        
        // Remove stale socket file if it exists
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                error!(error = %e, "Failed to bind to Unix socket");
                return;
            }
        };

        info!(path = %path.display(), "Listening on local Unix Domain Socket");

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let bus = Arc::clone(&event_bus);
                    tokio::spawn(async move {
                        handle_client(stream, bus).await;
                    });
                }
                Err(e) => {
                    warn!(error = %e, "Failed to accept Unix socket connection");
                }
            }
        }
    })
}

#[allow(clippy::cast_possible_truncation)]
async fn handle_client(stream: UnixStream, event_bus: Arc<EventBus>) {
    // 1. A client connects. We subscribe them to the global event bus.
    let mut receiver = event_bus.subscribe();
    
    // We need to read from the stream (client sending us events)
    // AND write to the stream (forwarding bus events to the client).
    let (mut read_half, mut write_half) = stream.into_split();
    
    // Forwarding loop: EventBus -> Client
    let forward_task = tokio::spawn(async move {
        while let Some(msg) = receiver.recv().await {
                if let Ok(bytes) = serde_json::to_vec(&msg) {
                        // Protocol: 4 byte length prefix, then JSON payload
                        let len = bytes.len() as u32;
                        if write_half.write_all(&len.to_be_bytes()).await.is_err() {
                            break;
                        }
                        if write_half.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                }
    });

    // Reading loop: Client -> EventBus
    loop {
        let mut len_buf = [0u8; 4];
        if read_half.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        
        // Prevent massive memory allocations (max 10MB per message)
        if len > 10 * 1024 * 1024 {
            break; 
        }

        let mut payload = vec![0u8; len];
        if read_half.read_exact(&mut payload).await.is_err() {
            break;
        }

        // Deserialize and publish to the Event Bus!
        if let Ok(msg) = serde_json::from_slice::<astrid_events::ipc::IpcMessage>(&payload) {
            let _ = event_bus.publish(astrid_events::AstridEvent::Ipc { metadata: astrid_events::EventMetadata::new("unix_socket_client"), message: msg });
        }
    }
    
    forward_task.abort();
}