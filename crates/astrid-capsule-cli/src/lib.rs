use astrid_sdk::net::{StreamHandle, accept, bind_unix, close, poll_accept, read, write};
use astrid_sdk::prelude::*;

#[derive(Default)]
struct CliProxy;

#[capsule]
impl CliProxy {
    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        // 1. Subscribe to TUI-relevant IPC topics only.
        // IMPORTANT: If a new event topic is consumed by the TUI, add it here.
        // Internal pipeline events (LLM requests, tool dispatch, identity builds)
        // must NOT be forwarded to the CLI socket.
        let topics = [
            "agent.v1.response",
            "agent.v1.stream.delta",
            "astrid.v1.onboarding.required",
            "astrid.v1.elicit.*",
            "astrid.v1.approval",
            "astrid.v1.response.*",
            "astrid.v1.capsules_loaded",
            "registry.v1.response.*",
            "registry.v1.active_model_changed",
            "registry.v1.selection.*",
            "session.v1.response.*",
        ];
        let sub_handles: Vec<_> = topics
            .iter()
            .map(|t| ipc::subscribe(t).map_err(|e| SysError::ApiError(e.to_string())))
            .collect::<Result<Vec<_>, _>>()?;

        // Signal readiness so the kernel can proceed with loading dependent capsules.
        // Best-effort: failure means the host mutex is poisoned (unrecoverable).
        let _ = runtime::signal_ready();

        // 2. Resolve the socket path from the kernel-injected config.
        // bind_unix is a no-op on the host side (the kernel pre-binds the socket),
        // but the path is used for logging and future diagnostics.
        let path = runtime::socket_path()
            .map_err(|e| SysError::ApiError(format!("Failed to resolve socket path: {e}")))?;

        let _ = log::log(
            "info",
            format!("CLI Proxy: accepting connections on {path}"),
        );
        let listener = bind_unix(&path).map_err(|e| SysError::ApiError(e.to_string()))?;

        // 3. Multi-connection accept loop.
        // Supports up to 8 concurrent CLI clients (enforced at host level).
        // IPC events are broadcast to all connected clients. Any authenticated
        // client can send prompts - the daemon is a single agent.
        let mut streams: Vec<StreamHandle> = Vec::new();

        loop {
            // Phase A: block until at least one client is connected.
            if streams.is_empty() {
                let stream = match accept(&listener) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = log::warn(format!("Accept error: {e:?}, backing off"));
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    },
                };
                let _ = log::info("CLI client connected to proxy");
                streams.push(stream);
            }

            // Phase B: poll for one additional connection (non-blocking).
            // Max one per iteration to bound handshake stall to ~5s worst case.
            if let Ok(Some(new_stream)) = poll_accept(&listener) {
                let _ = log::info("Additional CLI client connected to proxy");
                streams.push(new_stream);
            }

            // Phase C: read from all streams.
            // NOTE: 50ms timeout per stream = linear scaling (N*50ms per iteration).
            // Acceptable for CLI use (2-3 typical, 8 max = 400ms worst case).
            let mut dead_indices: Vec<usize> = Vec::new();
            for (i, stream) in streams.iter().enumerate() {
                match read(stream) {
                    Ok(bytes) => {
                        if !bytes.is_empty() {
                            handle_ingress(&bytes);
                        }
                    },
                    Err(e) => {
                        let _ = log::error(format!("Socket read error: {e:?}"));
                        dead_indices.push(i);
                    },
                }
            }

            // Remove dead streams in reverse order to preserve indices.
            for &i in dead_indices.iter().rev() {
                let dead = streams.remove(i);
                let _ = close(&dead);
                let _ = log::info("CLI client disconnected from proxy");
            }

            // Phase D: poll IPC subscriptions and broadcast to all live streams.
            let mut broadcast_dead: Vec<usize> = Vec::new();
            for handle in &sub_handles {
                match ipc::poll_bytes(handle) {
                    Ok(bytes) => {
                        if !bytes.is_empty() {
                            broadcast_poll_messages(&streams, &bytes, &mut broadcast_dead);
                        }
                    },
                    Err(_) => {
                        // Subscription error is fatal for the proxy.
                        break;
                    },
                }
            }

            // Remove streams that failed during broadcast.
            broadcast_dead.sort_unstable();
            broadcast_dead.dedup();
            for &i in broadcast_dead.iter().rev() {
                let dead = streams.remove(i);
                let _ = close(&dead);
                let _ = log::info("CLI client disconnected during broadcast");
            }
        }
    }
}

/// Parse an incoming client message and publish it to the IPC bus if the
/// topic passes the ingress allowlist.
fn handle_ingress(bytes: &[u8]) {
    let msg = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => v,
        Err(_) => {
            let _ = log::warn("Received malformed IPC payload from socket");
            return;
        },
    };

    let (Some(topic), Some(payload)) = (
        msg.get("topic").and_then(|t| t.as_str()),
        msg.get("payload"),
    ) else {
        return;
    };

    if is_allowed_ingress_topic(topic) {
        if let Err(e) = ipc::publish_json(topic, payload) {
            let _ = log::error(format!("Failed to publish IPC: {e:?}"));
        }
    } else {
        let _ = log::warn(format!("Dropped ingress message to blocked topic: {topic}"));
    }
}

/// Parse the poll envelope once, then broadcast each individual `IpcMessage`
/// to every connected stream. Tracks failed stream indices in `dead`.
fn broadcast_poll_messages(streams: &[StreamHandle], poll_bytes: &[u8], dead: &mut Vec<usize>) {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => {
            let _ = log::warn("Failed to parse poll envelope");
            return;
        },
    };

    if let Some(dropped) = envelope.get("dropped").and_then(|d| d.as_u64())
        && dropped > 0
    {
        let _ = log::warn(format!(
            "Event bus dropped {dropped} messages - TUI may be stale"
        ));
    }

    let Some(messages) = envelope.get("messages").and_then(|m| m.as_array()) else {
        return;
    };

    // Pre-serialize each message once, then write to all streams.
    let serialized: Vec<Vec<u8>> = messages
        .iter()
        .filter_map(|msg| serde_json::to_vec(msg).ok())
        .collect();

    for (i, stream) in streams.iter().enumerate() {
        for msg_bytes in &serialized {
            if let Err(e) = write(stream, msg_bytes) {
                let _ = log::error(format!("Socket write error: {e:?}"));
                dead.push(i);
                break; // Skip remaining messages for this dead stream.
            }
        }
    }
}

/// Exact topics the CLI is allowed to publish to the internal IPC bus.
const ALLOWED_INGRESS_EXACT: &[&str] = &[
    "user.v1.prompt",
    "client.v1.disconnect",
    "cli.v1.command.execute",
];

/// Topic prefixes the CLI is allowed to publish (suffix-routed topics).
/// IMPORTANT: Update this list when adding new CLI-originated topic prefixes.
const ALLOWED_INGRESS_PREFIXES: &[&str] = &[
    "astrid.v1.request.",
    "astrid.v1.elicit.response.",
    "astrid.v1.approval.response.",
    "registry.v1.selection.",
    "session.v1.request.",
];

fn is_allowed_ingress_topic(topic: &str) -> bool {
    ALLOWED_INGRESS_EXACT.contains(&topic)
        || ALLOWED_INGRESS_PREFIXES
            .iter()
            .any(|p| topic.starts_with(p))
}
