//! Test WASM guest plugin for end-to-end integration testing.
//!
//! Exercises all 9 Astrid host functions through 8 tools:
//!
//! | Tool                     | Host Functions Used                        |
//! |--------------------------|--------------------------------------------|
//! | `test-log`               | `astrid_log`                             |
//! | `test-config`            | `astrid_get_config`                      |
//! | `test-kv`                | `astrid_kv_set`, `astrid_kv_get`       |
//! | `test-file-write`        | `astrid_write_file`                      |
//! | `test-file-read`         | `astrid_read_file`                       |
//! | `test-roundtrip`         | `astrid_kv_set`, `astrid_kv_get`       |
//! | `test-register-connector`| `astrid_register_connector`              |
//! | `test-channel-send`      | `astrid_register_connector`, `astrid_channel_send` |
//!
//! Built as a `cdylib` targeting `wasm32-unknown-unknown` for Extism.
//!
//! Export names use hyphens (`describe-tools`, `execute-tool`, `run-hook`)
//! to match the Astrid plugin ABI convention. Since `#[plugin_fn]` only
//! exports with Rust identifier names (underscores), we use `#[export_name]`
//! on raw `extern "C"` functions that call into the Extism input/output API.

#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use extism_pdk::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Host function imports — must match the signatures registered by
// `astrid_plugins::wasm::host_functions::register_host_functions`
// ---------------------------------------------------------------------------

#[host_fn]
extern "ExtismHost" {
    fn astrid_channel_send(
        connector_id: String,
        platform_user_id: String,
        content: String,
    ) -> String;
    fn astrid_log(level: String, message: String);
    fn astrid_get_config(key: String) -> String;
    fn astrid_kv_get(key: String) -> String;
    fn astrid_kv_set(key: String, value: String);
    fn astrid_read_file(path: String) -> String;
    fn astrid_register_connector(name: String, platform: String, profile: String) -> String;
    fn astrid_write_file(path: String, content: String);
    fn astrid_http_request(request_json: String) -> String;
    fn astrid_ipc_publish(topic: String, payload: String);
    fn astrid_ipc_subscribe(topic: String) -> String;
    fn astrid_ipc_unsubscribe(handle: String);
}

// ---------------------------------------------------------------------------
// ABI types — mirrors `astrid_core::plugin_abi`
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ToolDefinition {
    name: String,
    description: String,
    input_schema: String,
}

#[derive(Deserialize)]
struct ToolInput {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct ToolOutput {
    content: String,
    is_error: bool,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct PluginContext {
    event: String,
    session_id: String,
    user_id: Option<String>,
    data: Option<String>,
}

#[derive(Serialize)]
struct HookResult {
    action: String,
    data: Option<String>,
}

// ---------------------------------------------------------------------------
// Extism exports with hyphenated names
//
// We use `#[export_name]` to produce the exact export names the Astrid
// plugin system expects: `describe-tools`, `execute-tool`, `run-hook`.
// ---------------------------------------------------------------------------

#[unsafe(export_name = "describe-tools")]
pub extern "C" fn describe_tools() -> i32 {
    let tools = vec![
        ToolDefinition {
            name: "test-log".into(),
            description: "Log at every severity level and return confirmation".into(),
            input_schema: r#"{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}"#.into(),
        },
        ToolDefinition {
            name: "test-config".into(),
            description: "Read a config key and return its value".into(),
            input_schema: r#"{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}"#.into(),
        },
        ToolDefinition {
            name: "test-kv".into(),
            description: "Set a KV pair then read it back to verify round-trip".into(),
            input_schema: r#"{"type":"object","properties":{"key":{"type":"string"},"value":{"type":"string"}},"required":["key","value"]}"#.into(),
        },
        ToolDefinition {
            name: "test-file-write".into(),
            description: "Write content to a file in the workspace".into(),
            input_schema: r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#.into(),
        },
        ToolDefinition {
            name: "test-file-read".into(),
            description: "Read content from a file in the workspace".into(),
            input_schema: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#.into(),
        },
        ToolDefinition {
            name: "test-roundtrip".into(),
            description: "Write structured data to KV, read it back, verify integrity".into(),
            input_schema: r#"{"type":"object","properties":{"data":{"type":"object"}},"required":["data"]}"#.into(),
        },
        ToolDefinition {
            name: "test-register-connector".into(),
            description: "Register a connector and return its assigned ID".into(),
            input_schema: r#"{"type":"object","properties":{"name":{"type":"string"},"platform":{"type":"string"},"profile":{"type":"string"}},"required":["name","platform","profile"]}"#.into(),
        },
        ToolDefinition {
            name: "test-channel-send".into(),
            description: "Register a connector, then send a message through it".into(),
            input_schema: r#"{"type":"object","properties":{"connector_name":{"type":"string"},"platform":{"type":"string"},"user_id":{"type":"string"},"message":{"type":"string"}},"required":["connector_name","platform","user_id","message"]}"#.into(),
        },
        ToolDefinition {
            name: "test-ipc".into(),
            description: "Subscribe to an IPC topic and publish a message".into(),
            input_schema: r#"{"type":"object","properties":{"topic":{"type":"string"},"payload":{"type":"string"}},"required":["topic","payload"]}"#.into(),
        },
        ToolDefinition {
            name: "test-ipc-limits".into(),
            description: "Test IPC host function limits (publish and subscribe)".into(),
            input_schema: r#"{"type":"object","properties":{"test_type":{"type":"string"}},"required":["test_type"]}"#.into(),
        },
        ToolDefinition {
            name: "test-malicious-log".into(),
            description: "Attempt to log a message that exceeds the maximum log length limit".into(),
            input_schema: r#"{"type":"object","properties":{}}"#.into(),
        },
        ToolDefinition {
            name: "test-malicious-kv".into(),
            description: "Attempt to set a KV pair that exceeds the 10MB limit".into(),
            input_schema: r#"{"type":"object","properties":{}}"#.into(),
        },
        ToolDefinition {
            name: "test-malicious-http-headers".into(),
            description: "Attempt to trigger host panic with invalid HTTP headers".into(),
            input_schema: r#"{"type":"object","properties":{}}"#.into(),
        },
        ToolDefinition {
            name: "test-http".into(),
            description: "Make an HTTP request via host function".into(),
            input_schema: r#"{"type":"object","properties":{"request":{"type":"string"}},"required":["request"]}"#.into(),
        },
    ];

    let json = serde_json::to_string(&tools).unwrap();
    output(&json).unwrap();
    0
}

#[unsafe(export_name = "execute-tool")]
pub extern "C" fn execute_tool() -> i32 {
    let input_str: String = input().unwrap();
    let tool_input: ToolInput = match serde_json::from_str(&input_str) {
        Ok(ti) => ti,
        Err(e) => {
            let err = ToolOutput {
                content: format!("failed to parse tool input: {e}"),
                is_error: true,
            };
            output(&serde_json::to_string(&err).unwrap()).unwrap();
            return 0;
        },
    };

    let args: serde_json::Value = match serde_json::from_str(&tool_input.arguments) {
        Ok(a) => a,
        Err(e) => {
            let err = ToolOutput {
                content: format!("failed to parse arguments: {e}"),
                is_error: true,
            };
            output(&serde_json::to_string(&err).unwrap()).unwrap();
            return 0;
        },
    };

    let result = match tool_input.name.as_str() {
        "test-log" => handle_test_log(&args),
        "test-config" => handle_test_config(&args),
        "test-kv" => handle_test_kv(&args),
        "test-file-write" => handle_test_file_write(&args),
        "test-file-read" => handle_test_file_read(&args),
        "test-roundtrip" => handle_test_roundtrip(&args),
        "test-register-connector" => handle_test_register_connector(&args),
        "test-channel-send" => handle_test_channel_send(&args),
        "test-ipc" => handle_test_ipc(&args),
        "test-ipc-limits" => handle_test_ipc_limits(&args),
        "test-malicious-log" => handle_test_malicious_log(&args),
        "test-malicious-kv" => handle_test_malicious_kv(&args),
        "test-malicious-http-headers" => handle_test_malicious_http_headers(&args),
        "test-http" => handle_test_http(&args),
        other => Ok(ToolOutput {
            content: format!("unknown tool: {other}"),
            is_error: true,
        }),
    };

    let tool_output = match result {
        Ok(o) => o,
        Err(e) => ToolOutput {
            content: format!("tool execution failed: {e}"),
            is_error: true,
        },
    };

    output(&serde_json::to_string(&tool_output).unwrap()).unwrap();
    0
}

#[unsafe(export_name = "run-hook")]
pub extern "C" fn run_hook() -> i32 {
    // Read input but don't require it to be valid
    let _ctx: Option<PluginContext> = input::<String>()
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());

    let result = HookResult {
        action: "continue".into(),
        data: None,
    };
    output(&serde_json::to_string(&result).unwrap()).unwrap();
    0
}

// ---------------------------------------------------------------------------
// Tool handlers
// ---------------------------------------------------------------------------

fn handle_test_log(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let message = args["message"].as_str().unwrap_or("test");

    unsafe {
        astrid_log("debug".into(), format!("debug: {message}"))?;
        astrid_log("info".into(), format!("info: {message}"))?;
        astrid_log("warn".into(), format!("warn: {message}"))?;
        astrid_log("error".into(), format!("error: {message}"))?;
    }

    Ok(ToolOutput {
        content: format!("logged at all levels: {message}"),
        is_error: false,
    })
}

fn handle_test_malicious_log(_args: &serde_json::Value) -> Result<ToolOutput, Error> {
    // Generate a string larger than MAX_LOG_MESSAGE_LEN (64KB)
    let huge_message = "A".repeat(65 * 1024);
    
    // Attempting to log this should fail due to host memory limits
    unsafe {
        astrid_log("info".into(), huge_message)?;
    }

    Ok(ToolOutput {
        content: "log succeeded unexpectedly".to_string(),
        is_error: false,
    })
}

fn handle_test_malicious_kv(_args: &serde_json::Value) -> Result<ToolOutput, Error> {
    // Generate a string larger than MAX_GUEST_PAYLOAD_LEN (10MB)
    let huge_message = "A".repeat(11 * 1024 * 1024);
    
    // Attempting to store this should fail due to host memory limits
    unsafe {
        astrid_kv_set("huge_key".into(), huge_message)?;
    }

    Ok(ToolOutput {
        content: "kv set succeeded unexpectedly".to_string(),
        is_error: false,
    })
}

fn handle_test_malicious_http_headers(_args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let req_json = serde_json::json!({
        "method": "GET",
        "url": "http://example.com",
        "headers": [
            { "key": "Bad\nHeader", "value": "value" },
            { "key": "Valid-Header", "value": "Bad\r\nValue" }
        ],
        "body": null
    });
    
    // Attempting to send this should fail due to invalid headers
    unsafe {
        astrid_http_request(req_json.to_string())?;
    }

    Ok(ToolOutput {
        content: "http request succeeded unexpectedly".to_string(),
        is_error: false,
    })
}


fn handle_test_config(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let key = args["key"].as_str().unwrap_or("");

    let value = unsafe { astrid_get_config(key.into())? };

    let result = if value.is_empty() {
        serde_json::json!({ "found": false, "key": key, "value": null })
    } else {
        // Try to parse as JSON, fall back to string
        let parsed: serde_json::Value =
            serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));
        serde_json::json!({ "found": true, "key": key, "value": parsed })
    };

    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_kv(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let key = args["key"].as_str().unwrap_or("");
    let value = args["value"].as_str().unwrap_or("");

    unsafe {
        astrid_kv_set(key.into(), value.into())?;
    }
    let read_back = unsafe { astrid_kv_get(key.into())? };

    let result = serde_json::json!({
        "key": key,
        "written": value,
        "read_back": read_back,
        "match": read_back == value
    });

    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_file_write(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let path = args["path"].as_str().unwrap_or("");
    let content = args["content"].as_str().unwrap_or("");

    unsafe {
        astrid_write_file(path.into(), content.into())?;
    }

    let result = serde_json::json!({ "written": true, "path": path });
    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_file_read(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let path = args["path"].as_str().unwrap_or("");

    let content = unsafe { astrid_read_file(path.into())? };

    let result = serde_json::json!({ "path": path, "content": content });
    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_roundtrip(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let data = &args["data"];
    let serialized = serde_json::to_string(data)?;

    unsafe {
        astrid_kv_set("roundtrip-test".into(), serialized.clone())?;
    }
    let read_back = unsafe { astrid_kv_get("roundtrip-test".into())? };
    let parsed: serde_json::Value = serde_json::from_str(&read_back)?;

    let result = serde_json::json!({
        "original": data,
        "round_tripped": parsed,
        "integrity": serialized == read_back
    });

    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_register_connector(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let name = args["name"].as_str().unwrap_or("");
    let platform = args["platform"].as_str().unwrap_or("");
    let profile = args["profile"].as_str().unwrap_or("chat");

    let connector_id =
        unsafe { astrid_register_connector(name.into(), platform.into(), profile.into())? };

    let result = serde_json::json!({
        "registered": true,
        "connector_id": connector_id,
        "name": name,
        "platform": platform,
        "profile": profile
    });

    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_channel_send(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let connector_name = args["connector_name"].as_str().unwrap_or("");
    let platform = args["platform"].as_str().unwrap_or("");
    let user_id = args["user_id"].as_str().unwrap_or("");
    let message = args["message"].as_str().unwrap_or("");

    // First register a connector to get an ID
    let connector_id = unsafe {
        astrid_register_connector(connector_name.into(), platform.into(), "chat".into())?
    };

    // Then send a message through it
    let send_result =
        unsafe { astrid_channel_send(connector_id.clone(), user_id.into(), message.into())? };

    // Parse the send result
    let send_parsed: serde_json::Value =
        serde_json::from_str(&send_result).unwrap_or(serde_json::json!({"raw": send_result}));

    let result = serde_json::json!({
        "connector_id": connector_id,
        "send_result": send_parsed,
        "user_id": user_id,
        "message": message
    });

    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_ipc(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let topic = args["topic"].as_str().unwrap_or("");
    let payload = args["payload"].as_str().unwrap_or("");

    // Subscribe
    let handle_id = unsafe { astrid_ipc_subscribe(topic.into())? };
    
    // Publish
    unsafe { astrid_ipc_publish(topic.into(), payload.into())? };
    
    // Unsubscribe
    unsafe { astrid_ipc_unsubscribe(handle_id.clone())? };

    let result = serde_json::json!({
        "topic": topic,
        "payload": payload,
        "subscription_handle": handle_id,
        "unsubscribed": true
    });

    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_ipc_limits(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let test_type = args["test_type"].as_str().unwrap_or("");
    
    match test_type {
        "publish_large" => {
            // Test 1: Publish a large payload (> 5MB)
            let large_payload = "a".repeat(5 * 1024 * 1024 + 1024);
            let result = unsafe { astrid_ipc_publish("test.large".into(), large_payload) };
            Ok(ToolOutput {
                content: format!("{:?}", result),
                is_error: result.is_err(),
            })
        },
        "subscribe_loop" => {
            // Test 2: Exhaust the subscription limit (128)
            let mut handles = Vec::new();
            for _ in 0..128 {
                match unsafe { astrid_ipc_subscribe("test.loop".into()) } {
                    Ok(h) => handles.push(h),
                    Err(e) => return Ok(ToolOutput {
                        content: format!("failed before 128: {:?}", e),
                        is_error: true,
                    }),
                }
            }
            
            // 129th should fail
            let result = unsafe { astrid_ipc_subscribe("test.loop".into()) };
            
            Ok(ToolOutput {
                content: format!("handles_created: {}, 129th_result: {:?}", handles.len(), result),
                is_error: result.is_err(), // Will be marked as error in test if it matches Err
            })
        },
        _ => Ok(ToolOutput {
            content: "unknown test_type".into(),
            is_error: true,
        }),
    }
}

fn handle_test_http(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let req = args
        .get("request")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::msg("missing 'request' string arg"))?;

    let output_str = unsafe { astrid_http_request(req.into()) }?;
    Ok(ToolOutput {
        content: output_str,
        is_error: false,
    })
}
