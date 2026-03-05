use astrid_sdk::net::{accept, bind_unix, read, write};
use astrid_sdk::prelude::*;

use extism_pdk::FnResult;

#[plugin_fn]
pub fn run() -> FnResult<()> {
    // 1. Fetch the caller context to determine our Session ID
    let caller = sys::get_caller().expect("Failed to get caller context");
    let _session_id = caller.session_id.unwrap_or_else(|| "default".to_string());

    // Subscribe to all IPC events
    let sub_handle = ipc::subscribe("*").expect("Failed to subscribe to IPC");

    // 2. Determine the physical socket path
    let path = "/tmp/.astrid/sessions/system.sock";

    // 3. Bind the Unix Domain Socket using the SDK Airlock
    sys::log("info", format!("CLI Proxy binding to socket: {path}")).unwrap();
    let listener = bind_unix(path).expect("Failed to bind to Unix Socket");

    // 4. Enter the blocking accept loop
    loop {
        if let Ok(stream) = accept(&listener) {
            sys::log("info", "CLI client connected to proxy")?;

            // Spawn a loop to read messages from the client
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
                                ) {
                                    ipc::publish_json(topic, payload)
                                        .expect("Failed to publish IPC");
                                }
                            } else {
                                sys::log("warn", "Received malformed IPC payload from socket")?;
                            }
                        }
                    },
                    Err(e) => {
                        sys::log("error", format!("Socket read error: {:?}", e))?;
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
