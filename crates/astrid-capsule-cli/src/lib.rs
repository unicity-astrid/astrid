use astrid_sdk::net::{accept, bind_unix, read, write};
use astrid_sdk::prelude::*;

use extism_pdk::FnResult;

#[plugin_fn]
pub fn run() -> FnResult<()> {
    // 1. Subscribe to all IPC events
    let sub_handle = ipc::subscribe("*").map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // 2. Determine the physical socket path dynamically
    // The Kernel defines this as AstridHome::resolve()?.sessions_dir().join("system.sock")
    // For now we will use the standard fallback path but ensure we handle errors cleanly.
    // In the future this should be injected via `wasm_config` by the HostState.
    let path = "/tmp/.astrid/sessions/system.sock";

    // 3. Bind the Unix Domain Socket using the SDK Airlock
    let _ = sys::log("info", format!("CLI Proxy binding to socket: {path}"));
    let listener = bind_unix(path).map_err(|e| extism_pdk::Error::msg(e.to_string()))?;

    // 4. Enter the blocking accept loop
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
        loop {
            // 1. Read from socket (has 50ms timeout on the host side)
            match read(&stream) {
                Ok(bytes) => {
                    if !bytes.is_empty() {
                        // Parse the incoming JSON into an IpcMessage
                        if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                            if let (Some(topic), Some(payload)) = (
                                msg.get("topic").and_then(|t| t.as_str()),
                                msg.get("payload"),
                            ) && let Err(e) = ipc::publish_json(topic, payload)
                            {
                                let _ =
                                    sys::log("error", format!("Failed to publish IPC: {:?}", e));
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

            // 2. Poll Event Bus — extract individual IpcMessages from the poll
            //    envelope and forward each one to the CLI socket as a standalone
            //    IpcMessage (the CLI client deserializes IpcMessage directly).
            match ipc::poll_bytes(&sub_handle) {
                Ok(bytes) => {
                    if !bytes.is_empty()
                        && let Err(()) = forward_poll_messages(&stream, &bytes)
                    {
                        break;
                    }
                },
                Err(_) => {
                    // Polling error or closed channel
                    break;
                },
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
