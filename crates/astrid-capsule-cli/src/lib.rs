use astrid_sdk::net::{accept, bind_unix, read, write};
use astrid_sdk::prelude::*;

use extism_pdk::FnResult;

#[plugin_fn]
pub fn run() -> FnResult<()> {
    // 1. Subscribe to TUI-relevant IPC topics only.
    // IMPORTANT: If a new event topic is consumed by the TUI, add it here.
    // Internal pipeline events (LLM requests, tool dispatch, identity builds)
    // must NOT be forwarded to the CLI socket.
    let topics = [
        "agent.v1.response",
        "agent.v1.stream.delta",
        "astrid.v1.onboarding.required",
        "astrid.v1.elicit.*",
        "astrid.v1.response.*",
        "astrid.v1.capsules_loaded",
        "registry.v1.response.*",
        "registry.v1.active_model_changed",
        "registry.v1.selection.*",
    ];
    let sub_handles: Vec<_> = topics
        .iter()
        .map(|t| ipc::subscribe(t).map_err(|e| extism_pdk::Error::msg(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?;

    // Signal readiness so the kernel can proceed with loading dependent capsules.
    // Best-effort: failure means the host mutex is poisoned (unrecoverable).
    let _ = sys::signal_ready();

    // 2. Resolve the socket path from the kernel-injected config.
    // bind_unix is a no-op on the host side (the kernel pre-binds the socket),
    // but the path is used for logging and future diagnostics.
    let path = sys::socket_path()
        .map_err(|e| extism_pdk::Error::msg(format!("Failed to resolve socket path: {e}")))?;

    let _ = sys::log(
        "info",
        format!("CLI Proxy: accepting connections on {path}"),
    );
    let listener = bind_unix(&path).map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // 4. Enter the blocking accept loop.
    // NOTE: This is a single-client design — only one CLI connection is
    // serviced at a time. A second `astrid chat` invocation will block at
    // accept() until the first disconnects. Spawning a task per connection
    // requires WASM threading or an async runtime, which is out of scope.
    loop {
        let stream = match accept(&listener) {
            Ok(s) => s,
            Err(e) => {
                let _ = sys::log("warn", format!("Accept error: {e:?}, backing off"));
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            },
        };
        let _ = sys::log("info", "CLI client connected to proxy");

        // Inner loop to read messages from the client
        'inner: loop {
            // 1. Read from socket (has 50ms timeout on the host side)
            match read(&stream) {
                Ok(bytes) => {
                    if !bytes.is_empty() {
                        // Parse the incoming JSON into an IpcMessage
                        if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                            if let (Some(topic), Some(payload)) = (
                                msg.get("topic").and_then(|t| t.as_str()),
                                msg.get("payload"),
                            ) {
                                // Ingress topic allowlist: only publish to topics the
                                // CLI legitimately needs. Prevents an authenticated
                                // client from injecting into internal pipeline topics.
                                // IMPORTANT: Update this list when adding new
                                // CLI-originated topics.
                                if is_allowed_ingress_topic(topic) {
                                    if let Err(e) = ipc::publish_json(topic, payload) {
                                        let _ = sys::log(
                                            "error",
                                            format!("Failed to publish IPC: {:?}", e),
                                        );
                                    }
                                } else {
                                    let _ = sys::log(
                                        "warn",
                                        format!(
                                            "Dropped ingress message to blocked topic: {topic}"
                                        ),
                                    );
                                }
                            }
                        } else {
                            let _ = sys::log("warn", "Received malformed IPC payload from socket");
                        }
                    }
                },
                Err(e) => {
                    let _ = sys::log("error", format!("Socket read error: {:?}", e));
                    break;
                },
            }

            // 2. Poll Event Bus — check each topic subscription and forward
            //    individual IpcMessages to the CLI socket.
            for handle in &sub_handles {
                match ipc::poll_bytes(handle) {
                    Ok(bytes) => {
                        if !bytes.is_empty()
                            && let Err(()) = forward_poll_messages(&stream, &bytes)
                        {
                            break 'inner;
                        }
                    },
                    Err(_) => {
                        break 'inner;
                    },
                }
            }
        }
    }
}

/// Parse the poll envelope `{"messages": [...], "dropped": N}` and write
/// each `IpcMessage` individually to the CLI socket.
fn forward_poll_messages(
    stream: &astrid_sdk::net::StreamHandle,
    poll_bytes: &[u8],
) -> Result<(), ()> {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => {
            let _ = sys::log("warn", "Failed to parse poll envelope");
            return Ok(());
        },
    };

    // Warn if the event bus reports dropped messages — a dropped
    // AgentResponse with is_final=true would leave the TUI stuck in Streaming.
    if let Some(dropped) = envelope.get("dropped").and_then(|d| d.as_u64())
        && dropped > 0
    {
        let _ = sys::log(
            "warn",
            format!("Event bus dropped {dropped} messages — TUI may be stale"),
        );
    }

    let messages = match envelope.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return Ok(()),
    };

    for msg in messages {
        let msg_bytes = match serde_json::to_vec(msg) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Err(e) = write(stream, &msg_bytes) {
            let _ = sys::log("error", format!("Socket write error: {e:?}"));
            return Err(());
        }
    }

    Ok(())
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
    "registry.v1.selection.",
];

fn is_allowed_ingress_topic(topic: &str) -> bool {
    ALLOWED_INGRESS_EXACT.contains(&topic)
        || ALLOWED_INGRESS_PREFIXES
            .iter()
            .any(|p| topic.starts_with(p))
}
