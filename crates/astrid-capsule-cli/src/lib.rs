use astrid_sdk::prelude::*;
use astrid_sdk::net::{bind_unix, accept, read};

use extism_pdk::FnResult;

#[plugin_fn]
pub fn run() -> FnResult<()> {
    // 1. Fetch the caller context to determine our Session ID
    let caller = sys::get_caller().expect("Failed to get caller context");
    let session_id = caller.session_id.unwrap_or_else(|| "default".to_string());
    
    // 2. Determine the physical socket path
    let path = format!("/tmp/.astrid/sessions/{}/ipc.sock", session_id);
    
    // 3. Bind the Unix Domain Socket using the SDK Airlock
    sys::log("info", format!("CLI Proxy binding to socket: {}", path)).unwrap();
    let listener = bind_unix(&path).expect("Failed to bind to Unix Socket");
    
    // 4. Enter the blocking accept loop
    loop {
        if let Ok(stream) = accept(&listener) {
            sys::log("info", "CLI client connected to proxy").unwrap();
            
            // Spawn a loop to read messages from the client
            loop {
                match read(&stream) {
                    Ok(bytes) => {
                        if bytes.is_empty() {
                            sys::log("info", "CLI client disconnected").unwrap();
                            break; 
                        }
                        
                        // Parse the incoming JSON into an IpcMessage
                        if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                            if let (Some(topic), Some(payload)) = (msg.get("topic").and_then(|t| t.as_str()), msg.get("payload")) {
                                astrid_sdk::ipc::publish_json(topic, payload).expect("Failed to publish IPC");
                            }
                        } else {
                            sys::log("warn", "Received malformed IPC payload from socket").unwrap();
                        }
                    },
                    Err(e) => {
                        sys::log("error", format!("Socket read error: {:?}", e)).unwrap();
                        break;
                    }
                }
            }
        }
    }
}