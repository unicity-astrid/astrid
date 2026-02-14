//! Test WASM guest plugin for end-to-end integration testing.
//!
//! Exercises all 7 Astralis host functions through 6 tools:
//!
//! | Tool             | Host Functions Used                        |
//! |------------------|--------------------------------------------|
//! | `test-log`       | `astralis_log`                             |
//! | `test-config`    | `astralis_get_config`                      |
//! | `test-kv`        | `astralis_kv_set`, `astralis_kv_get`       |
//! | `test-file-write`| `astralis_write_file`                      |
//! | `test-file-read` | `astralis_read_file`                       |
//! | `test-roundtrip` | `astralis_kv_set`, `astralis_kv_get`       |
//!
//! Built as a `cdylib` targeting `wasm32-unknown-unknown` for Extism.
//!
//! Export names use hyphens (`describe-tools`, `execute-tool`, `run-hook`)
//! to match the Astralis plugin ABI convention. Since `#[plugin_fn]` only
//! exports with Rust identifier names (underscores), we use `#[export_name]`
//! on raw `extern "C"` functions that call into the Extism input/output API.

use extism_pdk::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Host function imports — must match the signatures registered by
// `astralis_plugins::wasm::host_functions::register_host_functions`
// ---------------------------------------------------------------------------

#[host_fn]
extern "ExtismHost" {
    fn astralis_log(level: String, message: String);
    fn astralis_get_config(key: String) -> String;
    fn astralis_kv_get(key: String) -> String;
    fn astralis_kv_set(key: String, value: String);
    fn astralis_read_file(path: String) -> String;
    fn astralis_write_file(path: String, content: String);
    fn astralis_http_request(request_json: String) -> String;
}

// ---------------------------------------------------------------------------
// ABI types — mirrors `astralis_core::plugin_abi`
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
// We use `#[export_name]` to produce the exact export names the Astralis
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
        }
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
        }
    };

    let result = match tool_input.name.as_str() {
        "test-log" => handle_test_log(&args),
        "test-config" => handle_test_config(&args),
        "test-kv" => handle_test_kv(&args),
        "test-file-write" => handle_test_file_write(&args),
        "test-file-read" => handle_test_file_read(&args),
        "test-roundtrip" => handle_test_roundtrip(&args),
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
        astralis_log("debug".into(), format!("debug: {message}"))?;
        astralis_log("info".into(), format!("info: {message}"))?;
        astralis_log("warn".into(), format!("warn: {message}"))?;
        astralis_log("error".into(), format!("error: {message}"))?;
    }

    Ok(ToolOutput {
        content: format!("logged at all levels: {message}"),
        is_error: false,
    })
}

fn handle_test_config(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let key = args["key"].as_str().unwrap_or("");

    let value = unsafe { astralis_get_config(key.into())? };

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
        astralis_kv_set(key.into(), value.into())?;
    }
    let read_back = unsafe { astralis_kv_get(key.into())? };

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
        astralis_write_file(path.into(), content.into())?;
    }

    let result = serde_json::json!({ "written": true, "path": path });
    Ok(ToolOutput {
        content: serde_json::to_string(&result)?,
        is_error: false,
    })
}

fn handle_test_file_read(args: &serde_json::Value) -> Result<ToolOutput, Error> {
    let path = args["path"].as_str().unwrap_or("");

    let content = unsafe { astralis_read_file(path.into())? };

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
        astralis_kv_set("roundtrip-test".into(), serialized.clone())?;
    }
    let read_back = unsafe { astralis_kv_get("roundtrip-test".into())? };
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
