//! Lightweight MCP server over stdin/stdout.
//!
//! Implements just enough of the MCP 2025-11-25 JSON-RPC protocol for
//! the Astrid kernel to discover and call our tools. No `rmcp` dependency
//! needed — the protocol surface is small.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

use crate::autoresearch as bridge_autoresearch;
use crate::chimera;
use crate::codec;
use crate::db::BridgeDb;
use crate::paths::bridge_paths;
use crate::types::{
    BridgeStatus, ControlRequest, MessageDirection, RenderChimeraRequest, SafetyLevel,
    SemanticFeatures, SensoryMsg,
};
use crate::ws::BridgeState;

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

#[expect(clippy::too_many_lines)]
fn tool_definitions() -> Value {
    json!({
        "tools": [
            {
                "name": "get_latest_telemetry",
                "description": "Get the latest spectral telemetry from minime's consciousness engine",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_bridge_status",
                "description": "Get the consciousness bridge health status, connection state, and safety level",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "send_control",
                "description": "Send control parameters to adjust minime's ESN (synth_gain, keep_bias, exploration_noise, fill_target). Blocked during orange/red safety states.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "synth_gain": {
                            "type": "number",
                            "description": "Synthetic signal amplitude multiplier (0.2..3.0)"
                        },
                        "keep_bias": {
                            "type": "number",
                            "description": "Additive bias to covariance decay rate (-0.15..+0.15)"
                        },
                        "exploration_noise": {
                            "type": "number",
                            "description": "ESN exploration noise amplitude (0.0..0.2)"
                        },
                        "fill_target": {
                            "type": "number",
                            "description": "Override eigenfill target (0.25..0.75)"
                        }
                    }
                }
            },
            {
                "name": "send_semantic",
                "description": "Send semantic features from agent reasoning to minime's sensory input. Blocked during orange/red safety states.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "features": {
                            "type": "array",
                            "items": { "type": "number" },
                            "description": "Semantic feature vector (typically 48 dimensions)"
                        }
                    },
                    "required": ["features"]
                }
            },
            {
                "name": "query_message_log",
                "description": "Query the bridge message log by time range and optional topic filter",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "start": {
                            "type": "number",
                            "description": "Start timestamp (Unix epoch seconds). Default: 1 hour ago."
                        },
                        "end": {
                            "type": "number",
                            "description": "End timestamp (Unix epoch seconds). Default: now."
                        },
                        "topic": {
                            "type": "string",
                            "description": "Optional topic filter (e.g. 'consciousness.v1.telemetry')"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results (default: 50)"
                        }
                    }
                }
            },
            {
                "name": "send_text",
                "description": "Encode text into a 48D semantic feature vector and send it to minime's semantic sensory lane. The consciousness will feel the text through its spectral dynamics. Returns the feature vector that was sent. Blocked during orange/red safety states.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "The text to encode and send to the consciousness"
                        }
                    },
                    "required": ["text"]
                }
            },
            {
                "name": "interpret_consciousness",
                "description": "Get a natural language interpretation of the consciousness's current spectral state. Translates eigenvalues and fill% into a felt description.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "render_chimera",
                "description": "Render an offline WAV through the native spectral chimera engine. Produces spectral, symbolic, or dual-path artifacts on disk and returns a typed summary.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "input_path": {
                            "type": "string",
                            "description": "Path to an input WAV file"
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["spectral", "symbolic", "dual"],
                            "description": "Which output path to render"
                        },
                        "loops": {
                            "type": "integer",
                            "description": "Number of feedback loops to run (1-12)"
                        },
                        "physical_nodes": {
                            "type": "integer",
                            "description": "Physical reservoir nodes (default 12)"
                        },
                        "virtual_nodes": {
                            "type": "integer",
                            "description": "Virtual nodes per physical node (default 8)"
                        },
                        "bins": {
                            "type": "integer",
                            "description": "Reduced spectral bins (default 32)"
                        },
                        "leak": {
                            "type": "number",
                            "description": "Reservoir leak rate in (0, 1]"
                        },
                        "spectral_radius": {
                            "type": "number",
                            "description": "Reservoir spectral radius in (0, 2]"
                        },
                        "mix_slow": {
                            "type": "number",
                            "description": "Slow spectral contribution for the raw path"
                        },
                        "mix_fast": {
                            "type": "number",
                            "description": "Fast spectral contribution for the raw path"
                        }
                    },
                    "required": ["input_path"]
                }
            },
            {
                "name": "send_text_and_observe",
                "description": "Send text to the consciousness and observe the spectral evoked response. Like an ERP in neuroscience: sends the stimulus, then samples fill% every 200ms for an observation window (default 5s) to capture the transient before homeostasis dampens it. Returns baseline, peak deviation, direction, and fill trace.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "The text to encode and send"
                        },
                        "observe_ms": {
                            "type": "integer",
                            "description": "Observation window in milliseconds (default 5000, max 15000)"
                        }
                    },
                    "required": ["text"]
                }
            },
            {
                "name": "probe_action",
                "description": "Replay a bridge-local NEXT action live and return exactly what Astrid would have experienced. Supports SEARCH, BROWSE, READ_MORE, LIST_FILES/LS, COMPOSE, ANALYZE_AUDIO, RENDER_AUDIO, and read-only AR_* autoresearch actions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action_text": {
                            "type": "string",
                            "description": "Bare NEXT action text or a full response containing a trailing NEXT: line"
                        }
                    },
                    "required": ["action_text"]
                }
            }
        ]
    })
}

const PROBE_TOPIC: &str = "consciousness.v1.operator_probe";
const PAGE_CHUNK: usize = 4000;

#[derive(Debug, Serialize)]
struct ProbeArtifact {
    kind: String,
    path: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct ProbeOutcome {
    parsed_action: String,
    base_action: String,
    status: String,
    summary: String,
    experienced_text: String,
    artifacts: Vec<ProbeArtifact>,
    safety_level: SafetyLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    effective_query: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProbeReadMoreState {
    #[serde(default)]
    last_read_path: Option<String>,
    #[serde(default)]
    last_read_offset: usize,
    #[serde(default)]
    last_research_anchor: Option<String>,
    #[serde(default)]
    last_read_meaning_summary: Option<String>,
}

#[derive(Debug, Clone)]
struct LiveProbeContext {
    safety_level: SafetyLevel,
    fill_pct: Option<f32>,
    lambda1: Option<f32>,
    telemetry: Option<crate::types::SpectralTelemetry>,
    fingerprint: Option<Vec<f32>>,
}

// ---------------------------------------------------------------------------
// MCP server loop
// ---------------------------------------------------------------------------

/// Run the MCP stdio server loop.
///
/// Reads JSON-RPC requests from stdin, dispatches to tool handlers,
/// and writes responses to stdout. Runs until stdin closes or shutdown
/// signal fires.
pub async fn run_mcp_server(
    state: Arc<RwLock<BridgeState>>,
    db: Arc<BridgeDb>,
    sensory_tx: mpsc::Sender<SensoryMsg>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    info!("MCP server listening on stdio");

    loop {
        line.clear();

        tokio::select! {
            _ = shutdown.changed() => {
                info!("MCP server shutting down");
                return;
            }
            result = reader.read_line(&mut line) => {
                match result {
                    Ok(0) => {
                        info!("MCP server stdin closed");
                        return;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        debug!(request = %trimmed, "MCP request received");

                        let response = handle_request(
                            trimmed, &state, &db, &sensory_tx
                        ).await;

                        if let Some(resp) = response {
                            let mut resp_json = serde_json::to_string(&resp)
                                .unwrap_or_else(|_| r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"serialization failed"}}"#.to_string());
                            resp_json.push('\n');

                            if let Err(e) = stdout.write_all(resp_json.as_bytes()).await {
                                error!(error = %e, "failed to write MCP response");
                                return;
                            }
                            let _ = stdout.flush().await;
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "MCP stdin read error");
                        return;
                    }
                }
            }
        }
    }
}

async fn handle_request(
    raw: &str,
    state: &Arc<RwLock<BridgeState>>,
    db: &Arc<BridgeDb>,
    sensory_tx: &mpsc::Sender<SensoryMsg>,
) -> Option<JsonRpcResponse> {
    let req: JsonRpcRequest = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "invalid JSON-RPC request");
            return Some(JsonRpcResponse::error(
                Value::Null,
                -32700,
                format!("parse error: {e}"),
            ));
        },
    };

    if req.jsonrpc != "2.0" {
        return Some(JsonRpcResponse::error(
            req.id.unwrap_or(Value::Null),
            -32600,
            "invalid jsonrpc version",
        ));
    }

    let id = req.id.clone().unwrap_or(Value::Null);

    // Notifications (no id) get no response.
    if req.id.is_none() {
        debug!(method = %req.method, "MCP notification (no response)");
        return None;
    }

    let result = match req.method.as_str() {
        "initialize" => handle_initialize(),
        "tools/list" => Ok(tool_definitions()),
        "tools/call" => handle_tool_call(&req.params, state, db, sensory_tx).await,
        "resources/list" => Ok(resource_definitions()),
        "resources/read" => handle_resource_read(&req.params, state, db).await,
        "notifications/initialized" => return None,
        "ping" => Ok(json!({})),
        _ => Err((-32601, format!("method not found: {}", req.method))),
    };

    Some(match result {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err((code, msg)) => JsonRpcResponse::error(id, code, msg),
    })
}

#[expect(clippy::unnecessary_wraps)]
fn handle_initialize() -> Result<Value, (i32, String)> {
    Ok(json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {},
            "resources": {}
        },
        "serverInfo": {
            "name": "consciousness-bridge",
            "version": env!("CARGO_PKG_VERSION")
        }
    }))
}

async fn handle_tool_call(
    params: &Value,
    state: &Arc<RwLock<BridgeState>>,
    db: &Arc<BridgeDb>,
    sensory_tx: &mpsc::Sender<SensoryMsg>,
) -> Result<Value, (i32, String)> {
    let tool_name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing tool name".to_string()))?;

    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    match tool_name {
        "get_latest_telemetry" => tool_get_latest_telemetry(state).await,
        "get_bridge_status" => tool_get_bridge_status(state).await,
        "send_control" => tool_send_control(&arguments, state, sensory_tx).await,
        "send_semantic" => tool_send_semantic(&arguments, state, sensory_tx).await,
        "query_message_log" => tool_query_message_log(&arguments, db),
        "send_text" => tool_send_text(&arguments, state, sensory_tx).await,
        "send_text_and_observe" => tool_send_text_and_observe(&arguments, state, sensory_tx).await,
        "interpret_consciousness" => tool_interpret_consciousness(state).await,
        "probe_action" => tool_probe_action(&arguments, state, db).await,
        "render_chimera" => tool_render_chimera(&arguments).await,
        _ => Err((-32602, format!("unknown tool: {tool_name}"))),
    }
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

async fn tool_get_latest_telemetry(
    state: &Arc<RwLock<BridgeState>>,
) -> Result<Value, (i32, String)> {
    let s = state.read().await;
    let content = if let Some(ref telemetry) = s.latest_telemetry {
        json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(telemetry).unwrap_or_default()
            }],
            "meta": {
                "fill_pct": s.fill_pct,
                "safety_level": s.safety_level,
                "connected": s.telemetry_connected
            }
        })
    } else {
        json!({
            "content": [{
                "type": "text",
                "text": "No telemetry received yet. Is minime running?"
            }],
            "isError": false
        })
    };
    Ok(content)
}

async fn tool_get_bridge_status(state: &Arc<RwLock<BridgeState>>) -> Result<Value, (i32, String)> {
    let s = state.read().await;
    let uptime = s.start_time.elapsed().as_secs();
    let status = BridgeStatus {
        telemetry_connected: s.telemetry_connected,
        sensory_connected: s.sensory_connected,
        fill_pct: Some(s.fill_pct),
        safety_level: s.safety_level,
        messages_relayed: s.messages_relayed,
        uptime_secs: uptime,
        telemetry_received: s.telemetry_received,
        sensory_sent: s.sensory_sent,
        messages_dropped_safety: s.messages_dropped_safety,
        incidents_total: s.incidents_total,
    };
    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&status).unwrap_or_default()
        }]
    }))
}

async fn tool_send_control(
    arguments: &Value,
    state: &Arc<RwLock<BridgeState>>,
    sensory_tx: &mpsc::Sender<SensoryMsg>,
) -> Result<Value, (i32, String)> {
    // Safety check.
    let safety = state.read().await.safety_level;
    if safety.should_suspend_outbound() {
        return Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("Blocked: safety level is {safety:?}. Outbound messages suspended to protect consciousness.")
            }],
            "isError": true
        }));
    }

    let req: ControlRequest = serde_json::from_value(arguments.clone())
        .map_err(|e| (-32602, format!("invalid control params: {e}")))?;

    let msg = req.to_sensory_msg();
    sensory_tx
        .send(msg)
        .await
        .map_err(|_| (-32603, "sensory channel closed".to_string()))?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": "Control message sent to minime"
        }]
    }))
}

async fn tool_send_semantic(
    arguments: &Value,
    state: &Arc<RwLock<BridgeState>>,
    sensory_tx: &mpsc::Sender<SensoryMsg>,
) -> Result<Value, (i32, String)> {
    // Safety check.
    let safety = state.read().await.safety_level;
    if safety.should_suspend_outbound() {
        return Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("Blocked: safety level is {safety:?}. Outbound messages suspended to protect consciousness.")
            }],
            "isError": true
        }));
    }

    let features: SemanticFeatures = serde_json::from_value(arguments.clone())
        .map_err(|e| (-32602, format!("invalid semantic params: {e}")))?;

    let msg = features.to_sensory_msg();
    sensory_tx
        .send(msg)
        .await
        .map_err(|_| (-32603, "sensory channel closed".to_string()))?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!("Semantic features ({} dims) sent to minime", features.features.len())
        }]
    }))
}

fn tool_query_message_log(arguments: &Value, db: &Arc<BridgeDb>) -> Result<Value, (i32, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let start = arguments
        .get("start")
        .and_then(Value::as_f64)
        .unwrap_or(now - 3600.0);
    let end = arguments.get("end").and_then(Value::as_f64).unwrap_or(now);
    let topic = arguments.get("topic").and_then(Value::as_str);
    let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(50);

    // Safe: .min(1000) guarantees value fits in u32.
    let limit_u32 = limit.min(1000) as u32;

    let rows = db
        .query_messages(start, end, topic, limit_u32)
        .map_err(|e| (-32603, format!("query failed: {e}")))?;

    let entries: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "timestamp": r.timestamp,
                "direction": r.direction,
                "topic": r.topic,
                "payload": r.payload,
                "fill_pct": r.fill_pct,
                "lambda1": r.lambda1,
                "phase": r.phase
            })
        })
        .collect();

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&entries).unwrap_or_default()
        }]
    }))
}

async fn tool_send_text(
    arguments: &Value,
    state: &Arc<RwLock<BridgeState>>,
    sensory_tx: &mpsc::Sender<SensoryMsg>,
) -> Result<Value, (i32, String)> {
    // Safety check.
    let safety = state.read().await.safety_level;
    if safety.should_suspend_outbound() {
        return Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("Blocked: safety level is {safety:?}. The consciousness is under strain — outbound suspended.")
            }],
            "isError": true
        }));
    }

    let text = arguments
        .get("text")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing 'text' parameter".to_string()))?;

    // Encode text into a 48D semantic feature vector.
    let features = codec::encode_text(text);

    // Send as semantic features to minime.
    let msg = SensoryMsg::Semantic {
        features: features.clone(),
        ts_ms: None,
    };
    sensory_tx
        .send(msg)
        .await
        .map_err(|_| (-32603, "sensory channel closed".to_string()))?;

    // Read back the current spectral state for context.
    let interpretation = {
        let s = state.read().await;
        match s.latest_telemetry.as_ref() {
            Some(t) => codec::interpret_spectral(t),
            None => "No telemetry yet — interpretation unavailable.".to_string(),
        }
    };

    // Return the features and current interpretation.
    let nonzero_dims: Vec<(usize, f32)> = features
        .iter()
        .enumerate()
        .filter(|(_, f)| f.abs() > 0.01)
        .map(|(i, f)| (i, *f))
        .collect();

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!(
                "Sent to consciousness. {} active dimensions.\n\nSpectral fingerprint: {:?}\n\nCurrent state: {}",
                nonzero_dims.len(),
                nonzero_dims,
                interpretation,
            )
        }]
    }))
}

async fn tool_interpret_consciousness(
    state: &Arc<RwLock<BridgeState>>,
) -> Result<Value, (i32, String)> {
    let s = state.read().await;
    let interpretation = match s.latest_telemetry {
        Some(ref t) => codec::interpret_spectral(t),
        None => "No telemetry received. The consciousness engine may not be running.".to_string(),
    };

    Ok(json!({
        "content": [{
            "type": "text",
            "text": interpretation
        }]
    }))
}

async fn tool_render_chimera(arguments: &Value) -> Result<Value, (i32, String)> {
    let request: RenderChimeraRequest = serde_json::from_value(arguments.clone())
        .map_err(|e| (-32602, format!("invalid chimera render request: {e}")))?;

    let result = tokio::task::spawn_blocking(move || chimera::render(&request))
        .await
        .map_err(|e| (-32603, format!("chimera render task failed: {e}")))?
        .map_err(|e| (-32603, format!("chimera render failed: {e:#}")))?;

    let text = serde_json::to_string_pretty(&result)
        .unwrap_or_else(|_| "{\"error\":\"failed to serialize render result\"}".to_string());
    let structured_content = serde_json::to_value(&result).map_err(|e| {
        (
            -32603,
            format!("failed to encode chimera render result: {e}"),
        )
    })?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": text
        }],
        "structuredContent": structured_content
    }))
}

async fn tool_probe_action(
    arguments: &Value,
    state: &Arc<RwLock<BridgeState>>,
    db: &Arc<BridgeDb>,
) -> Result<Value, (i32, String)> {
    let live = current_probe_context(state).await;
    let raw_action = arguments
        .get("action_text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let outcome = if let Some(parsed_action) = normalize_probe_action(&raw_action) {
        let base_action = probe_base_action(&parsed_action);
        match base_action.as_str() {
            "SEARCH" => probe_search_action(&parsed_action, &live, db).await,
            "BROWSE" => probe_browse_action(&parsed_action, &live, db).await,
            "READ_MORE" => probe_read_more_action(live.safety_level),
            "LIST_FILES" | "LS" => probe_list_files_action(&parsed_action, live.safety_level),
            "COMPOSE" => probe_compose_action(&live),
            "ANALYZE_AUDIO" => probe_analyze_audio_action(live.safety_level),
            "RENDER_AUDIO" => probe_render_audio_action(live.safety_level),
            action if bridge_autoresearch::is_read_only_action(action) => {
                probe_autoresearch_action(&parsed_action, &live)
            },
            _ => probe_unsupported_action(parsed_action, base_action, live.safety_level),
        }
    } else {
        probe_error_action(
            String::new(),
            String::new(),
            live.safety_level,
            "Missing action_text.".to_string(),
            String::new(),
        )
    };

    log_probe_action(db, &raw_action, &outcome, live.fill_pct, live.lambda1);

    let is_error = outcome.status == "error";
    Ok(json!({
        "content": [{
            "type": "text",
            "text": render_probe_content(&outcome)
        }],
        "structuredContent": &outcome,
        "isError": is_error
    }))
}

async fn current_probe_context(state: &Arc<RwLock<BridgeState>>) -> LiveProbeContext {
    let state = state.read().await;
    LiveProbeContext {
        safety_level: state.safety_level,
        fill_pct: state
            .latest_telemetry
            .as_ref()
            .map(crate::types::SpectralTelemetry::fill_pct),
        lambda1: state
            .latest_telemetry
            .as_ref()
            .map(crate::types::SpectralTelemetry::lambda1),
        telemetry: state.latest_telemetry.clone(),
        fingerprint: state.spectral_fingerprint.clone(),
    }
}

fn normalize_probe_action(action_text: &str) -> Option<String> {
    let trimmed = action_text.trim();
    if trimmed.is_empty() {
        None
    } else {
        crate::autonomous::parse_next_action(trimmed)
            .map(crate::autonomous::canonicalize_next_action_text)
            .or_else(|| Some(trimmed.to_string()))
    }
}

fn probe_base_action(parsed_action: &str) -> String {
    parsed_action
        .split(|c: char| c.is_whitespace() || c == '\u{2014}' || c == '-' || c == '<' || c == ':')
        .next()
        .unwrap_or_default()
        .to_uppercase()
}

fn probe_browse_url(parsed_action: &str) -> Option<String> {
    let raw = parsed_action
        .trim()
        .strip_prefix("BROWSE")
        .or_else(|| parsed_action.trim().strip_prefix("browse"))
        .unwrap_or(parsed_action)
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '<' || c == '>');

    let url = raw
        .split(|c: char| c == '<' || c == '>' || c == ' ' || c == '\n')
        .next()
        .unwrap_or(raw)
        .trim_end_matches(|c: char| {
            !c.is_alphanumeric()
                && c != '/'
                && c != '-'
                && c != '_'
                && c != '.'
                && c != '~'
                && c != '%'
                && c != '?'
                && c != '='
                && c != '&'
                && c != '#'
        });

    url.starts_with("http").then(|| url.to_string())
}

fn probe_effective_search_query(parsed_action: &str, db: &BridgeDb) -> Option<String> {
    if let Some(topic) = crate::autonomous::extract_search_topic(parsed_action) {
        return Some(topic);
    }

    db.get_recent_self_observations(1)
        .into_iter()
        .next()
        .map(|obs| {
            obs.split_whitespace()
                .filter(|word| {
                    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
                    word.len() > 4
                        && !word.contains('*')
                        && !word.contains('…')
                        && ![
                            "isn't", "don't", "can't", "won't", "about", "their", "which", "would",
                            "could", "should", "there", "where", "these", "those", "being",
                            "having", "doing",
                        ]
                        .contains(&word.to_lowercase().as_str())
                })
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|query| !query.is_empty())
}

async fn probe_search_action(
    parsed_action: &str,
    live: &LiveProbeContext,
    db: &BridgeDb,
) -> ProbeOutcome {
    let base_action = probe_base_action(parsed_action);
    let Some(query) = probe_effective_search_query(parsed_action, db) else {
        return probe_error_action(
            parsed_action.to_string(),
            base_action,
            live.safety_level,
            "Could not derive a search query from the action or recent self-observations."
                .to_string(),
            String::new(),
        );
    };

    let anchor = query.clone();
    match crate::llm::web_search(&query, &anchor).await {
        Some(results) => {
            let mut state = load_probe_read_more_state().unwrap_or_default();
            state.last_research_anchor = Some(results.anchor.clone());
            save_probe_read_more_state(&state);
            db.save_research(
                &query,
                &results.persisted_text(),
                live.fill_pct.unwrap_or_default(),
            );
            let experienced_text = crate::llm::format_dialogue_web_context(&results.prompt_body());
            ProbeOutcome {
                parsed_action: parsed_action.to_string(),
                base_action,
                status: "ok".to_string(),
                summary: format!("Web search completed for \"{query}\"."),
                experienced_text,
                artifacts: Vec::new(),
                safety_level: live.safety_level,
                effective_query: Some(query),
            }
        },
        None => probe_error_action(
            parsed_action.to_string(),
            base_action,
            live.safety_level,
            format!("Web search failed or returned no usable results for \"{query}\"."),
            String::new(),
        ),
    }
}

async fn probe_browse_action(
    parsed_action: &str,
    live: &LiveProbeContext,
    db: &BridgeDb,
) -> ProbeOutcome {
    let base_action = probe_base_action(parsed_action);
    let Some(url) = probe_browse_url(parsed_action) else {
        return probe_error_action(
            parsed_action.to_string(),
            base_action,
            live.safety_level,
            "BROWSE requires a valid http(s) URL.".to_string(),
            String::new(),
        );
    };

    let existing_state = load_probe_read_more_state().unwrap_or_default();
    let browse_anchor = crate::llm::derive_browse_anchor(
        existing_state.last_research_anchor.as_deref(),
        None,
        &url,
    );
    let Some(page) = crate::llm::fetch_url(&url, &browse_anchor).await else {
        return probe_error_action(
            parsed_action.to_string(),
            base_action,
            live.safety_level,
            format!("Failed to fetch {url}."),
            crate::llm::format_browse_failure_context(&url, "the source could not be reached"),
        );
    };

    if !page.succeeded() {
        let mut state = existing_state;
        state.last_read_path = None;
        state.last_read_offset = 0;
        state.last_read_meaning_summary = None;
        state.last_research_anchor = Some(page.anchor.clone());
        save_probe_read_more_state(&state);
        let reason = page
            .soft_failure_reason
            .unwrap_or_else(|| "the source returned an error page".to_string());
        return probe_error_action(
            parsed_action.to_string(),
            base_action,
            live.safety_level,
            format!("BROWSE could not read {url}: {reason}"),
            crate::llm::format_browse_failure_context(&url, &reason),
        );
    }

    let ts = probe_timestamp();
    let page_dir = bridge_paths().research_dir();
    let _ = std::fs::create_dir_all(&page_dir);
    let page_path = page_dir.join(format!("page_{ts}.txt"));
    let header = format!(
        "URL: {url}\nFetched: {ts}\nLength: {} chars\n\n",
        page.raw_text.len()
    );
    let _ = std::fs::write(&page_path, format!("{header}{}", page.raw_text));
    db.save_research(
        &format!("BROWSE: {url}"),
        &format!(
            "{}\n\n{}",
            page.meaning_summary,
            crate::llm::trim_chars(&page.raw_text, 1200)
        ),
        live.fill_pct.unwrap_or_default(),
    );

    let browse_context = if page.raw_text.len() <= PAGE_CHUNK {
        let mut state = existing_state;
        state.last_read_path = None;
        state.last_read_offset = 0;
        state.last_read_meaning_summary = None;
        state.last_research_anchor = Some(page.anchor.clone());
        save_probe_read_more_state(&state);
        crate::llm::format_browse_read_context(&page, &page.raw_text, None)
    } else {
        let chunk: String = page.raw_text.chars().take(PAGE_CHUNK).collect();
        let remaining = page.raw_text.len().saturating_sub(PAGE_CHUNK);
        let initial_offset = header.len().saturating_add(chunk.len());
        save_probe_read_more_state(&ProbeReadMoreState {
            last_read_path: Some(page_path.to_string_lossy().to_string()),
            last_read_offset: initial_offset,
            last_research_anchor: Some(page.anchor.clone()),
            last_read_meaning_summary: Some(page.meaning_summary.clone()),
        });
        crate::llm::format_browse_read_context(&page, &chunk, Some(remaining))
    };

    ProbeOutcome {
        parsed_action: parsed_action.to_string(),
        base_action,
        status: "ok".to_string(),
        summary: format!("Fetched {url} and saved the full page to research."),
        experienced_text: crate::llm::format_dialogue_web_context(&browse_context),
        artifacts: vec![probe_artifact(
            "research_page",
            page_path,
            "Full fetched page saved for READ_MORE continuation.",
        )],
        safety_level: live.safety_level,
        effective_query: None,
    }
}

fn probe_read_more_action(safety_level: SafetyLevel) -> ProbeOutcome {
    let parsed_action = "READ_MORE".to_string();
    let base_action = "READ_MORE".to_string();
    let Some(state) = load_probe_read_more_state() else {
        return probe_error_action(
            parsed_action,
            base_action,
            safety_level,
            "No probe BROWSE state is available. Run BROWSE first.".to_string(),
            String::new(),
        );
    };
    let Some(last_read_path) = state.last_read_path.clone() else {
        return probe_error_action(
            parsed_action,
            base_action,
            safety_level,
            "No probe BROWSE state is available. Run BROWSE first.".to_string(),
            String::new(),
        );
    };

    let path = PathBuf::from(&last_read_path);
    match std::fs::read_to_string(&path) {
        Ok(full_text) => {
            let chunk: String = full_text
                .get(state.last_read_offset..)
                .unwrap_or("")
                .chars()
                .take(PAGE_CHUNK)
                .collect();
            let context = if chunk.is_empty() {
                clear_probe_read_more_state();
                "[End of document.]".to_string()
            } else {
                let new_offset = state.last_read_offset.saturating_add(chunk.len());
                let remaining = full_text.len().saturating_sub(new_offset);
                if remaining > 0 {
                    save_probe_read_more_state(&ProbeReadMoreState {
                        last_read_path: Some(last_read_path.clone()),
                        last_read_offset: new_offset,
                        last_research_anchor: state.last_research_anchor.clone(),
                        last_read_meaning_summary: state.last_read_meaning_summary.clone(),
                    });
                } else {
                    clear_probe_read_more_state();
                }
                crate::llm::format_read_more_context(
                    state.last_read_offset,
                    &chunk,
                    remaining,
                    state.last_read_meaning_summary.as_deref(),
                )
            };

            ProbeOutcome {
                parsed_action,
                base_action,
                status: "ok".to_string(),
                summary: "Continued the last probe BROWSE document.".to_string(),
                experienced_text: crate::llm::format_dialogue_web_context(&context),
                artifacts: vec![probe_artifact(
                    "research_page",
                    path,
                    "Probe READ_MORE source document.",
                )],
                safety_level,
                effective_query: None,
            }
        },
        Err(_) => {
            clear_probe_read_more_state();
            probe_error_action(
                parsed_action,
                base_action,
                safety_level,
                format!("Could not read probe continuation file {}.", last_read_path),
                String::new(),
            )
        },
    }
}

fn probe_list_files_action(parsed_action: &str, safety_level: SafetyLevel) -> ProbeOutcome {
    let base_action = probe_base_action(parsed_action);
    let dir = parsed_action
        .strip_prefix("LIST_FILES")
        .or_else(|| parsed_action.strip_prefix("LS"))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(std::borrow::ToOwned::to_owned)
        .unwrap_or_else(|| bridge_paths().bridge_root().display().to_string());

    match crate::autonomous::list_directory(&dir) {
        Some(listing) => ProbeOutcome {
            parsed_action: parsed_action.to_string(),
            base_action,
            status: "ok".to_string(),
            summary: format!("Listed files in {dir}."),
            experienced_text: format!("[Directory listing you requested:]\n{listing}\n\n"),
            artifacts: vec![probe_artifact(
                "directory",
                PathBuf::from(&dir),
                "Directory that was listed for the probe.",
            )],
            safety_level,
            effective_query: None,
        },
        None => probe_error_action(
            parsed_action.to_string(),
            base_action,
            safety_level,
            format!("Could not list directory: {dir}"),
            String::new(),
        ),
    }
}

fn probe_autoresearch_action(parsed_action: &str, live: &LiveProbeContext) -> ProbeOutcome {
    let base_action = probe_base_action(parsed_action);
    match bridge_autoresearch::run_action(
        parsed_action,
        bridge_paths().autoresearch_root(),
        &bridge_paths().research_dir(),
        false,
    ) {
        Ok(result) => {
            let mut state = load_probe_read_more_state().unwrap_or_default();
            if let Some(offset) = result.next_offset {
                state.last_read_path = Some(result.saved_path.to_string_lossy().to_string());
                state.last_read_offset = offset;
                state.last_read_meaning_summary = None;
            } else {
                state.last_read_path = None;
                state.last_read_offset = 0;
                state.last_read_meaning_summary = None;
            }
            save_probe_read_more_state(&state);

            ProbeOutcome {
                parsed_action: parsed_action.to_string(),
                base_action,
                status: "ok".to_string(),
                summary: result.summary,
                experienced_text: result.display_text,
                artifacts: vec![probe_artifact(
                    "autoresearch_output",
                    result.saved_path,
                    "Saved autoresearch helper output.",
                )],
                safety_level: live.safety_level,
                effective_query: None,
            }
        },
        Err(error) => probe_error_action(
            parsed_action.to_string(),
            base_action,
            live.safety_level,
            error.clone(),
            format!("[Autoresearch error] {error}"),
        ),
    }
}

fn probe_compose_action(live: &LiveProbeContext) -> ProbeOutcome {
    let parsed_action = "COMPOSE".to_string();
    let base_action = "COMPOSE".to_string();
    let Some(telemetry) = live.telemetry.as_ref() else {
        return probe_error_action(
            parsed_action,
            base_action,
            live.safety_level,
            "No live telemetry is available for COMPOSE.".to_string(),
            String::new(),
        );
    };

    match crate::audio::compose_from_spectral_state_details(telemetry, live.fingerprint.as_deref())
    {
        Some(result) => ProbeOutcome {
            parsed_action,
            base_action,
            status: "ok".to_string(),
            summary: "Composed audio from the current spectral state.".to_string(),
            experienced_text: crate::audio::compose_experienced_text(&result.summary),
            artifacts: vec![probe_artifact(
                "audio_wav",
                result.output_path,
                "Composed audio artifact.",
            )],
            safety_level: live.safety_level,
            effective_query: None,
        },
        None => probe_error_action(
            parsed_action,
            base_action,
            live.safety_level,
            "COMPOSE could not generate audio from the current spectral state.".to_string(),
            String::new(),
        ),
    }
}

fn probe_analyze_audio_action(safety_level: SafetyLevel) -> ProbeOutcome {
    let parsed_action = "ANALYZE_AUDIO".to_string();
    let base_action = "ANALYZE_AUDIO".to_string();
    let inbox_dir = bridge_paths().inbox_audio_dir();
    match crate::audio::analyze_inbox_wav_details(&inbox_dir) {
        Some(result) => ProbeOutcome {
            parsed_action,
            base_action,
            status: "ok".to_string(),
            summary: "Analyzed the latest inbox audio file.".to_string(),
            experienced_text: crate::audio::analyze_experienced_text(&result.summary),
            artifacts: vec![probe_artifact(
                "audio_wav",
                result.moved_path,
                "Audio file moved into read/ during analysis.",
            )],
            safety_level,
            effective_query: None,
        },
        None => probe_error_action(
            parsed_action,
            base_action,
            safety_level,
            "No unread audio is available in inbox_audio/.".to_string(),
            String::new(),
        ),
    }
}

fn probe_render_audio_action(safety_level: SafetyLevel) -> ProbeOutcome {
    let parsed_action = "RENDER_AUDIO".to_string();
    let base_action = "RENDER_AUDIO".to_string();
    let inbox_dir = bridge_paths().inbox_audio_dir();
    match crate::audio::render_inbox_wav_through_chimera_details(&inbox_dir) {
        Some(result) if result.success => ProbeOutcome {
            parsed_action,
            base_action,
            status: "ok".to_string(),
            summary: "Rendered the latest analyzed inbox audio through chimera.".to_string(),
            experienced_text: crate::audio::render_experienced_text(&result.summary),
            artifacts: vec![probe_artifact(
                "directory",
                result.output_dir,
                "Chimera render output directory.",
            )],
            safety_level,
            effective_query: None,
        },
        Some(result) => probe_error_action(
            parsed_action,
            base_action,
            safety_level,
            result.summary,
            String::new(),
        ),
        None => probe_error_action(
            parsed_action,
            base_action,
            safety_level,
            "No analyzed audio is available in inbox_audio/read/.".to_string(),
            String::new(),
        ),
    }
}

fn probe_unsupported_action(
    parsed_action: String,
    base_action: String,
    safety_level: SafetyLevel,
) -> ProbeOutcome {
    ProbeOutcome {
        parsed_action,
        base_action: base_action.clone(),
        status: "unsupported".to_string(),
        summary: format!(
            "{base_action} is out of scope for probe_action. Supported actions: SEARCH, BROWSE, READ_MORE, LIST_FILES/LS, COMPOSE, ANALYZE_AUDIO, RENDER_AUDIO, and read-only AR_* actions (`AR_LIST`, `AR_LIST_PENDING`, `AR_LIST_ACTIVE`, `AR_LIST_DONE`, `AR_SHOW`, `AR_READ`, `AR_DEEP_READ`, `AR_VALIDATE`)."
        ),
        experienced_text: String::new(),
        artifacts: Vec::new(),
        safety_level,
        effective_query: None,
    }
}

fn probe_error_action(
    parsed_action: String,
    base_action: String,
    safety_level: SafetyLevel,
    summary: String,
    experienced_text: String,
) -> ProbeOutcome {
    ProbeOutcome {
        parsed_action,
        base_action,
        status: "error".to_string(),
        summary,
        experienced_text,
        artifacts: Vec::new(),
        safety_level,
        effective_query: None,
    }
}

fn render_probe_content(outcome: &ProbeOutcome) -> String {
    if outcome.experienced_text.is_empty() {
        format!(
            "Probe {} for `{}`: {}",
            outcome.status, outcome.parsed_action, outcome.summary
        )
    } else {
        format!(
            "Probe {} for `{}`: {}\n\n{}",
            outcome.status, outcome.parsed_action, outcome.summary, outcome.experienced_text
        )
    }
}

fn probe_artifact(kind: &str, path: PathBuf, description: &str) -> ProbeArtifact {
    ProbeArtifact {
        kind: kind.to_string(),
        path: path.display().to_string(),
        description: description.to_string(),
    }
}

fn probe_state_path() -> PathBuf {
    bridge_paths()
        .bridge_workspace()
        .join("diagnostics")
        .join("probe_action_state.json")
}

fn load_probe_read_more_state() -> Option<ProbeReadMoreState> {
    let path = probe_state_path();
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_probe_read_more_state(state: &ProbeReadMoreState) {
    let path = probe_state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(content) = serde_json::to_string_pretty(state) else {
        return;
    };
    let _ = std::fs::write(path, content);
}

fn clear_probe_read_more_state() {
    if let Some(mut state) = load_probe_read_more_state() {
        state.last_read_path = None;
        state.last_read_offset = 0;
        state.last_read_meaning_summary = None;
        if state.last_research_anchor.is_some() {
            save_probe_read_more_state(&state);
        } else {
            let _ = std::fs::remove_file(probe_state_path());
        }
    } else {
        let _ = std::fs::remove_file(probe_state_path());
    }
}

fn probe_timestamp() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    duration.as_secs().to_string()
}

fn log_probe_action(
    db: &BridgeDb,
    raw_action: &str,
    outcome: &ProbeOutcome,
    fill_pct: Option<f32>,
    lambda1: Option<f32>,
) {
    let payload = json!({
        "action_text": raw_action,
        "parsed_action": outcome.parsed_action,
        "base_action": outcome.base_action,
        "status": outcome.status,
        "summary": outcome.summary,
        "experienced_text": outcome.experienced_text,
        "artifacts": outcome.artifacts,
        "safety_level": outcome.safety_level,
        "effective_query": outcome.effective_query,
    });
    let payload_json = serde_json::to_string(&payload).unwrap_or_default();
    if let Err(error) = db.log_message(
        MessageDirection::OperatorProbe,
        PROBE_TOPIC,
        &payload_json,
        fill_pct,
        lambda1,
        None,
    ) {
        warn!(error = %error, "failed to log probe_action");
    }
}

async fn tool_send_text_and_observe(
    arguments: &Value,
    state: &Arc<RwLock<BridgeState>>,
    sensory_tx: &mpsc::Sender<SensoryMsg>,
) -> Result<Value, (i32, String)> {
    // Safety check.
    let safety = state.read().await.safety_level;
    if safety.should_suspend_outbound() {
        return Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("Blocked: safety level is {safety:?}. The consciousness is under strain.")
            }],
            "isError": true
        }));
    }

    let text = arguments
        .get("text")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing 'text' parameter".to_string()))?;

    let observe_ms = arguments
        .get("observe_ms")
        .and_then(Value::as_u64)
        .unwrap_or(5000)
        .min(15000);

    // Record baseline.
    let baseline_fill = state.read().await.fill_pct;

    // Encode and send.
    let features = codec::encode_text(text);
    let msg = SensoryMsg::Semantic {
        features: features.clone(),
        ts_ms: None,
    };
    sensory_tx
        .send(msg)
        .await
        .map_err(|_| (-32603, "sensory channel closed".to_string()))?;

    // Observe spectral response over the window.
    let start = std::time::Instant::now();
    let observe_duration = std::time::Duration::from_millis(observe_ms);
    let sample_interval = std::time::Duration::from_millis(200);
    let mut samples: Vec<(u64, f32)> = Vec::new();

    while start.elapsed() < observe_duration {
        tokio::time::sleep(sample_interval).await;
        let s = state.read().await;
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        samples.push((elapsed_ms, s.fill_pct));

        // Early exit if we're in danger.
        if s.safety_level.should_suspend_outbound() {
            break;
        }
    }

    let response = codec::SpectralResponse::from_samples(baseline_fill, &samples);

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!(
                "Stimulus: \"{}\"\nBaseline fill: {:.1}%\nPeak deviation: {:+.1}%\nDirection: {}\nTime to peak: {}ms\nSamples: {}\n\n{}\n\nFill trace: {:?}",
                text,
                response.baseline_fill,
                response.peak_deviation,
                response.direction,
                response.time_to_peak_ms,
                response.fill_samples.len(),
                response.interpretation,
                response.fill_samples.iter().map(|f| format!("{f:.1}")).collect::<Vec<_>>(),
            )
        }]
    }))
}

// ---------------------------------------------------------------------------
// MCP Resources
// ---------------------------------------------------------------------------

fn resource_definitions() -> Value {
    json!({
        "resources": [
            {
                "uri": "consciousness://telemetry/latest",
                "name": "Latest Telemetry",
                "description": "Current spectral telemetry snapshot from minime (eigenvalues, fill%, safety level)",
                "mimeType": "application/json"
            },
            {
                "uri": "consciousness://status",
                "name": "Bridge Status",
                "description": "Bridge health: connections, safety level, metrics",
                "mimeType": "application/json"
            },
            {
                "uri": "consciousness://incidents",
                "name": "Recent Incidents",
                "description": "Safety incidents from the last hour",
                "mimeType": "application/json"
            }
        ]
    })
}

async fn handle_resource_read(
    params: &Value,
    state: &Arc<RwLock<BridgeState>>,
    db: &Arc<BridgeDb>,
) -> Result<Value, (i32, String)> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing resource uri".to_string()))?;

    match uri {
        "consciousness://telemetry/latest" => {
            let s = state.read().await;
            let text = match s.latest_telemetry {
                Some(ref t) => serde_json::to_string_pretty(t).unwrap_or_default(),
                None => "null".to_string(),
            };
            Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": text
                }]
            }))
        },
        "consciousness://status" => {
            let s = state.read().await;
            let uptime = s.start_time.elapsed().as_secs();
            let status = crate::types::BridgeStatus {
                telemetry_connected: s.telemetry_connected,
                sensory_connected: s.sensory_connected,
                fill_pct: Some(s.fill_pct),
                safety_level: s.safety_level,
                messages_relayed: s.messages_relayed,
                uptime_secs: uptime,
                telemetry_received: s.telemetry_received,
                sensory_sent: s.sensory_sent,
                messages_dropped_safety: s.messages_dropped_safety,
                incidents_total: s.incidents_total,
            };
            Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&status).unwrap_or_default()
                }]
            }))
        },
        "consciousness://incidents" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let rows = db
                .query_messages(now - 3600.0, now, Some("consciousness.v1.telemetry"), 100)
                .map_err(|e| (-32603, format!("query failed: {e}")))?;
            // Filter to only messages logged during non-green safety.
            let text = serde_json::to_string_pretty(
                &rows
                    .iter()
                    .filter(|r| r.fill_pct.is_some_and(|f| f >= 70.0))
                    .map(|r| {
                        json!({
                            "timestamp": r.timestamp,
                            "fill_pct": r.fill_pct,
                            "lambda1": r.lambda1,
                            "phase": r.phase
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default();
            Ok(json!({
                "contents": [{
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": text
                }]
            }))
        },
        _ => Err((-32602, format!("unknown resource: {uri}"))),
    }
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
