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
                                    ipc::publish_json(topic, payload).expect("Failed to publish IPC");
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

                // 2. Poll Event Bus
                match ipc::poll_bytes(&sub_handle) {
                    Ok(bytes) => {
                        if !bytes.is_empty()
                            && let Err(e) = write(&stream, &bytes)
                        {
                            sys::log("error", format!("Socket write error: {:?}", e))?;
                            break;
                        }
                    },
                    Err(_) => {
                        // Polling error or closed channel
                        break;
                    }
                }
            }
        }
    }
}
