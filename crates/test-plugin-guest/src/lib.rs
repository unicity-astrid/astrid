//! Test WASM guest plugin for end-to-end integration testing.
//!
//! Exercises all 9 Astrid host functions through 8 tools.
//! Built using the new `astrid-sdk` and `#[capsule]` macro.

#![allow(unsafe_code)]
#![allow(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub struct TestCapsule;

#[derive(Serialize)]
struct ToolOutput {
    content: String,
    is_error: bool,
}

#[derive(Deserialize, Default)]
struct TestLogArgs {
    message: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestConfigArgs {
    key: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestKvArgs {
    key: Option<String>,
    value: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestFileWriteArgs {
    path: Option<String>,
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestFileReadArgs {
    path: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestRoundtripArgs {
    data: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct TestRegisterConnectorArgs {
    name: Option<String>,
    platform: Option<String>,
    profile: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestChannelSendArgs {
    connector_name: Option<String>,
    platform: Option<String>,
    user_id: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestIpcArgs {
    topic: Option<String>,
    payload: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestIpcLimitsArgs {
    test_type: Option<String>,
}

#[derive(Deserialize, Default)]
struct TestHttpArgs {
    request: Option<String>,
}

#[derive(Deserialize, Default)]
struct EmptyArgs {}

#[capsule]
impl TestCapsule {
    #[astrid::tool("test-log")]
    fn handle_test_log(&self, args: TestLogArgs) -> Result<ToolOutput, SysError> {
        let message = args.message.unwrap_or_else(|| "test".to_string());

        sys::log("debug", format!("debug: {message}"))?;
        sys::log("info", format!("info: {message}"))?;
        sys::log("warn", format!("warn: {message}"))?;
        sys::log("error", format!("error: {message}"))?;

        Ok(ToolOutput {
            content: format!("logged at all levels: {message}"),
            is_error: false,
        })
    }

    #[astrid::tool("test-malicious-log")]
    fn handle_test_malicious_log(&self, _args: EmptyArgs) -> Result<ToolOutput, SysError> {
        let huge_message = "A".repeat(65 * 1024);
        sys::log("info", huge_message)?;
        Ok(ToolOutput {
            content: "log succeeded unexpectedly".to_string(),
            is_error: false,
        })
    }

    #[astrid::tool("test-malicious-kv")]
    fn handle_test_malicious_kv(&self, _args: EmptyArgs) -> Result<ToolOutput, SysError> {
        let huge_message = "A".repeat(11 * 1024 * 1024);
        kv::set_bytes("huge_key", huge_message.as_bytes())?;
        Ok(ToolOutput {
            content: "kv set succeeded unexpectedly".to_string(),
            is_error: false,
        })
    }

    #[astrid::tool("test-malicious-http-headers")]
    fn handle_test_malicious_http_headers(&self, _args: EmptyArgs) -> Result<ToolOutput, SysError> {
        let req_json = serde_json::json!({
            "method": "GET",
            "url": "http://example.com",
            "headers": {
                "Bad\nHeader": "value",
                "Valid-Header": "Bad\r\nValue"
            },
            "body": null
        });
        
        http::request_bytes(req_json.to_string().as_bytes())?;

        Ok(ToolOutput {
            content: "http request succeeded unexpectedly".to_string(),
            is_error: false,
        })
    }

    #[astrid::tool("test-config")]
    fn handle_test_config(&self, args: TestConfigArgs) -> Result<ToolOutput, SysError> {
        let key = args.key.unwrap_or_default();
        let value = sys::get_config_string(&key)?;

        let result = if value.is_empty() {
            serde_json::json!({ "found": false, "key": key, "value": null })
        } else {
            let parsed: serde_json::Value =
                serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));
            serde_json::json!({ "found": true, "key": key, "value": parsed })
        };

        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    #[astrid::tool("test-kv")]
    fn handle_test_kv(&self, args: TestKvArgs) -> Result<ToolOutput, SysError> {
        let key = args.key.unwrap_or_default();
        let value = args.value.unwrap_or_default();

        kv::set_bytes(&key, value.as_bytes())?;
        let read_back_bytes = kv::get_bytes(&key)?;
        let read_back = String::from_utf8_lossy(&read_back_bytes).to_string();

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

    #[astrid::tool("test-file-write")]
    fn handle_test_file_write(&self, args: TestFileWriteArgs) -> Result<ToolOutput, SysError> {
        let path = args.path.unwrap_or_default();
        let content = args.content.unwrap_or_default();

        fs::write_string(&path, &content)?;

        let result = serde_json::json!({ "written": true, "path": path });
        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    #[astrid::tool("test-file-read")]
    fn handle_test_file_read(&self, args: TestFileReadArgs) -> Result<ToolOutput, SysError> {
        let path = args.path.unwrap_or_default();

        let content = fs::read_string(&path)?;

        let result = serde_json::json!({ "path": path, "content": content });
        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    #[astrid::tool("test-roundtrip")]
    fn handle_test_roundtrip(&self, args: TestRoundtripArgs) -> Result<ToolOutput, SysError> {
        kv::set_json("roundtrip-test", &args.data)?;
        let read_back: serde_json::Value = kv::get_json("roundtrip-test")?;

        let result = serde_json::json!({
            "original": args.data,
            "round_tripped": read_back,
            "integrity": args.data == read_back
        });

        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    #[astrid::tool("test-register-connector")]
    fn handle_test_register_connector(&self, args: TestRegisterConnectorArgs) -> Result<ToolOutput, SysError> {
        let name = args.name.unwrap_or_default();
        let platform = args.platform.unwrap_or_default();
        let profile = args.profile.unwrap_or_else(|| "chat".to_string());

        let connector_id_bytes = uplink::register(&name, &platform, &profile)?;
        let connector_id = String::from_utf8_lossy(&connector_id_bytes).to_string();

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

    #[astrid::tool("test-channel-send")]
    fn handle_test_channel_send(&self, args: TestChannelSendArgs) -> Result<ToolOutput, SysError> {
        let connector_name = args.connector_name.unwrap_or_default();
        let platform = args.platform.unwrap_or_default();
        let user_id = args.user_id.unwrap_or_default();
        let message = args.message.unwrap_or_default();

        let connector_id_bytes = uplink::register(&connector_name, &platform, "chat")?;

        let send_result_bytes = uplink::send_bytes(&connector_id_bytes, user_id.as_bytes(), message.as_bytes())?;
        let send_result = String::from_utf8_lossy(&send_result_bytes).to_string();

        let send_parsed: serde_json::Value =
            serde_json::from_str(&send_result).unwrap_or(serde_json::json!({"raw": send_result}));

        let result = serde_json::json!({
            "connector_id": String::from_utf8_lossy(&connector_id_bytes).to_string(),
            "send_result": send_parsed,
            "user_id": user_id,
            "message": message
        });

        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    #[astrid::tool("test-ipc")]
    fn handle_test_ipc(&self, args: TestIpcArgs) -> Result<ToolOutput, SysError> {
        let topic = args.topic.unwrap_or_default();
        let payload = args.payload.unwrap_or_default();

        let handle_bytes = ipc::subscribe(&topic)?;
        
        ipc::publish_bytes(&topic, payload.as_bytes())?;
        
        ipc::unsubscribe(&handle_bytes)?;

        let result = serde_json::json!({
            "topic": topic,
            "payload": payload,
            "subscription_handle": String::from_utf8_lossy(&handle_bytes).to_string(),
            "unsubscribed": true
        });

        Ok(ToolOutput {
            content: serde_json::to_string(&result)?,
            is_error: false,
        })
    }

    #[astrid::tool("test-ipc-limits")]
    fn handle_test_ipc_limits(&self, args: TestIpcLimitsArgs) -> Result<ToolOutput, SysError> {
        let test_type = args.test_type.unwrap_or_default();
        
        match test_type.as_str() {
            "publish_large" => {
                let large_payload = "a".repeat(5 * 1024 * 1024 + 1024);
                let _ = ipc::publish_bytes("test.large", large_payload.as_bytes());
                
                Ok(ToolOutput {
                    content: "did not trap".into(),
                    is_error: true,
                })
            },
            "subscribe_loop" => {
                let mut handles = Vec::new();
                for _ in 0..128 {
                    match ipc::subscribe("test.loop") {
                        Ok(h) => handles.push(h),
                        Err(e) => return Ok(ToolOutput {
                            content: format!("failed before 128: {:?}", e),
                            is_error: true,
                        }),
                    }
                }
                
                let result = ipc::subscribe("test.loop");
                
                Ok(ToolOutput {
                    content: format!("handles_created: {}, 129th_result: {:?}", handles.len(), result),
                    is_error: result.is_err(),
                })
            },
            _ => Ok(ToolOutput {
                content: "unknown test_type".into(),
                is_error: true,
            }),
        }
    }

    #[astrid::tool("test-http")]
    fn handle_test_http(&self, args: TestHttpArgs) -> Result<ToolOutput, SysError> {
        let req = args.request.unwrap_or_default();

        let output_bytes = http::request_bytes(req.as_bytes())?;
        let output_str = String::from_utf8_lossy(&output_bytes).to_string();

        Ok(ToolOutput {
            content: output_str,
            is_error: false,
        })
    }

    #[astrid::interceptor("run-hook")]
    fn run_hook(&self, _args: EmptyArgs) -> Result<serde_json::Value, SysError> {
        Ok(serde_json::json!({
            "action": "continue",
            "data": null
        }))
    }
}
