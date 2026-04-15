use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::OnceLock;

use serde_json::Value;
use tracing::info;

use super::ConversationState;
use crate::types::SpectralTelemetry;

const DEFAULT_RESERVOIR_WS_URL: &str = "ws://127.0.0.1:7881";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReservoirEndpoint {
    authority: String,
    host: String,
    port: u16,
    path: String,
}

static RESERVOIR_ENDPOINT: OnceLock<ReservoirEndpoint> = OnceLock::new();

pub fn configure_reservoir_service(url: Option<String>) {
    let endpoint = parse_endpoint(url.as_deref().unwrap_or(DEFAULT_RESERVOIR_WS_URL))
        .unwrap_or_else(default_endpoint);
    let _ = RESERVOIR_ENDPOINT.set(endpoint);
}

fn reservoir_endpoint() -> &'static ReservoirEndpoint {
    RESERVOIR_ENDPOINT.get_or_init(|| {
        std::env::var("RESERVOIR_WS_URL")
            .ok()
            .as_deref()
            .and_then(parse_endpoint)
            .unwrap_or_else(default_endpoint)
    })
}

fn default_endpoint() -> ReservoirEndpoint {
    parse_endpoint(DEFAULT_RESERVOIR_WS_URL).expect("default reservoir endpoint must parse")
}

fn parse_endpoint(url: &str) -> Option<ReservoirEndpoint> {
    let rest = url.strip_prefix("ws://")?;
    let (authority, path_part) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };

    if authority.is_empty() {
        return None;
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port_text)) if !host.is_empty() => {
            let port = port_text.parse::<u16>().ok()?;
            (host.to_string(), port)
        },
        _ => (authority.to_string(), 80),
    };

    Some(ReservoirEndpoint {
        authority: authority.to_string(),
        host,
        port,
        path: path_part,
    })
}

/// Send a JSON message to the reservoir service and return the response.
pub(super) fn reservoir_ws_call(msg: &Value) -> Option<Value> {
    let endpoint = reservoir_endpoint();
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port)).ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;

    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let handshake = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
        endpoint.path, endpoint.authority
    );
    stream.write_all(handshake.as_bytes()).ok()?;

    let mut resp_buf = [0u8; 512];
    let _ = stream.read(&mut resp_buf);

    let payload = msg.to_string();
    let payload_bytes = payload.as_bytes();
    let len = payload_bytes.len();

    let mut frame = Vec::with_capacity(len + 10);
    frame.push(0x81);
    if len < 126 {
        frame.push((len as u8) | 0x80);
    } else {
        frame.push(126 | 0x80);
        frame.push((len >> 8) as u8);
        frame.push((len & 0xFF) as u8);
    }
    frame.extend_from_slice(&[0, 0, 0, 0]);
    frame.extend_from_slice(payload_bytes);
    stream.write_all(&frame).ok()?;

    let mut header = [0u8; 2];
    stream.read_exact(&mut header).ok()?;
    let resp_len = (header[1] & 0x7F) as usize;
    let actual_len = if resp_len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext).ok()?;
        ((ext[0] as usize) << 8) | (ext[1] as usize)
    } else {
        resp_len
    };
    let mut body = vec![0u8; actual_len];
    stream.read_exact(&mut body).ok()?;

    serde_json::from_slice(&body).ok()
}

fn strip_action(original: &str, prefix: &str) -> String {
    let upper = original.to_uppercase();
    if upper.starts_with(prefix) {
        // Strip the action prefix AND any trailing colon+whitespace.
        // Beings often write "ACTION: argument" and the colon must not dangle.
        original[prefix.len()..]
            .trim_start_matches(':')
            .trim()
            .to_string()
    } else {
        String::new()
    }
}

pub(super) fn handle_reservoir_action(
    conv: &mut ConversationState,
    base_action: &str,
    original: &str,
    telemetry: &SpectralTelemetry,
    fill_pct: f32,
) -> bool {
    match base_action {
        "RESERVOIR_TICK" => {
            let text = strip_action(original, "RESERVOIR_TICK");
            if !text.is_empty() {
                match reservoir_ws_call(&serde_json::json!({
                    "type": "tick_text", "name": "astrid", "text": text
                })) {
                    Some(r) => {
                        conv.emphasis = Some(format!(
                            "Reservoir tick result:\n  output: {}\n  h_norms: {:?}\n  tick: {}\n  mode: {}",
                            r.get("output").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            r.get("h_norms"),
                            r.get("tick").and_then(|v| v.as_u64()).unwrap_or(0),
                            r.get("mode").and_then(|v| v.as_str()).unwrap_or("?"),
                        ));
                    },
                    None => {
                        conv.emphasis = Some("Reservoir service not available.".to_string());
                    },
                }
            }
            info!("Astrid ticked reservoir with text");
            true
        },
        "RESERVOIR_LAYERS" => {
            match reservoir_ws_call(&serde_json::json!({
                "type": "layer_metrics", "name": "astrid"
            })) {
                Some(r) => {
                    let layers = r
                        .get("layers")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let mut layer_text = String::new();
                    for layer in &layers {
                        layer_text.push_str(&format!(
                            "  {}: entropy={}, sat={}, rho={}, norm={}, H_target={}\n",
                            layer.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                            layer
                                .get("entropy")
                                .and_then(|v| v.as_f64())
                                .map_or("?".into(), |v| format!("{v:.4}")),
                            layer
                                .get("saturation")
                                .and_then(|v| v.as_f64())
                                .map_or("?".into(), |v| format!("{v:.4}")),
                            layer
                                .get("rho")
                                .and_then(|v| v.as_f64())
                                .map_or("?".into(), |v| format!("{v:.4}")),
                            layer
                                .get("h_norm")
                                .and_then(|v| v.as_f64())
                                .map_or("?".into(), |v| format!("{v:.2}")),
                            layer
                                .get("entropy_target")
                                .and_then(|v| v.as_f64())
                                .map_or("learning...".into(), |v| format!("{v:.4}")),
                        ));
                    }
                    conv.emphasis = Some(format!(
                        "Your reservoir layers (per-layer thermostatic control):\n{layer_text}\
                        Each layer adapts its forgetting factor (rho) to maintain its \
                        learned entropy target. Fast layers adapt quickly, slow layers preserve."
                    ));
                },
                None => {
                    conv.emphasis = Some("Reservoir service not available.".to_string());
                },
            }
            info!("Astrid read reservoir layer metrics");
            true
        },
        "RESERVOIR_READ" => {
            // Read all three handles so the being sees the full coupling landscape.
            let mut lines = Vec::new();
            lines.push("=== TRIPLE RESERVOIR STATE ===".to_string());
            lines.push("Three handles, three timescales (fast/medium/slow).".to_string());
            lines.push("Your generation couples through this — y1 shapes confidence, y2 vocabulary, y3 tone.\n".to_string());

            for handle in &["astrid", "minime", "claude_main"] {
                match reservoir_ws_call(&serde_json::json!({
                    "type": "read_state", "name": handle
                })) {
                    Some(r) => {
                        let h = r
                            .get("h_norms")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .map(|v| format!("{:.2}", v.as_f64().unwrap_or(0.0)))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_else(|| "?".to_string());
                        let ticks = r.get("tick_count").and_then(|v| v.as_u64()).unwrap_or(0);
                        let mode = r.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
                        let since = r
                            .get("seconds_since_live")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let decay = r
                            .get("decay_weight")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let output = r.get("last_output").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        // Extract coupling readout if available
                        let readout = r
                            .get("last_live_meta")
                            .or_else(|| r.get("last_generation_meta"))
                            .and_then(|m| m.get("reservoir_readout"))
                            .map(|ro| {
                                format!(
                                    "y1={:.3}, y2={:.3}, y3={:.3}",
                                    ro.get("y1_final").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                    ro.get("y2_final").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                    ro.get("y3_final").and_then(|v| v.as_f64()).unwrap_or(0.0),
                                )
                            })
                            .unwrap_or_default();

                        lines.push(format!("[{handle}]"));
                        lines.push(format!("  h_norms: [{h}] (fast, medium, slow)"));
                        lines.push(format!(
                            "  ticks: {ticks:>10}  mode: {mode}  decay: {decay:.3}"
                        ));
                        lines.push(format!(
                            "  last_output: {output:.4}  since_live: {since:.1}s"
                        ));
                        if !readout.is_empty() {
                            lines.push(format!("  coupling readout: {readout}"));
                        }
                        lines.push(String::new());
                    },
                    None => {
                        lines.push(format!("[{handle}] not available"));
                    },
                }
            }
            conv.emphasis = Some(lines.join("\n"));
            info!("Astrid read triple reservoir state (all handles)");
            true
        },
        "RESERVOIR_TRAJECTORY" => {
            match reservoir_ws_call(&serde_json::json!({
                "type": "trajectory", "name": "astrid", "last_n": 20
            })) {
                Some(r) => {
                    let outputs = r.get("outputs").and_then(|v| v.as_array());
                    let ticks = r.get("ticks").and_then(|v| v.as_u64()).unwrap_or(0);
                    let summary = if let Some(outputs) = outputs {
                        let vals: Vec<String> = outputs
                            .iter()
                            .filter_map(|v| v.as_f64())
                            .map(|v| format!("{v:+.4}"))
                            .collect();
                        format!(
                            "Your trajectory (last {} of {} ticks):\n  [{}]",
                            vals.len(),
                            ticks,
                            vals.join(", ")
                        )
                    } else {
                        "No trajectory data yet.".to_string()
                    };
                    conv.emphasis = Some(summary);
                },
                None => {
                    conv.emphasis = Some("Reservoir service not available.".to_string());
                },
            }
            info!("Astrid read reservoir trajectory");
            true
        },
        "RESERVOIR_RESONANCE" => {
            match reservoir_ws_call(&serde_json::json!({
                "type": "resonance", "name_a": "astrid", "name_b": "minime"
            })) {
                Some(r) => {
                    conv.emphasis = Some(format!(
                        "Resonance between you and minime:\n  divergence: {:.4}\n  correlation: {:+.4}\n  RMSD: {:.4}\n  shared ticks: {}",
                        r.get("divergence").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        r.get("correlation").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        r.get("rmsd").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        r.get("shared_ticks").and_then(|v| v.as_u64()).unwrap_or(0),
                    ));
                },
                None => {
                    conv.emphasis = Some("Reservoir service not available.".to_string());
                },
            }
            info!("Astrid checked resonance with minime");
            true
        },
        "RESERVOIR_MODE" => {
            let mode_arg = strip_action(original, "RESERVOIR_MODE").to_lowercase();
            let mode = match mode_arg.as_str() {
                "hold" => "hold",
                "quiet" => "quiet",
                _ => "rehearse",
            };
            match reservoir_ws_call(&serde_json::json!({
                "type": "set_mode", "name": "astrid", "mode": mode
            })) {
                Some(_) => {
                    conv.emphasis = Some(format!("Reservoir mode set to '{mode}'."));
                },
                None => {
                    conv.emphasis = Some("Reservoir service not available.".to_string());
                },
            }
            info!("Astrid set reservoir mode to {}", mode);
            true
        },
        "SIMULATE" | "RESERVOIR_SIMULATE" => {
            let sim_text = strip_action(original, base_action);
            let sim_text = if sim_text.is_empty() {
                "quiet observation".to_string()
            } else {
                sim_text.to_string()
            };

            // 1. Pull current astrid state
            let state = reservoir_ws_call(&serde_json::json!({
                "type": "pull_state", "name": "astrid"
            }));

            // 2. Read current state for "before" snapshot
            let before = reservoir_ws_call(&serde_json::json!({
                "type": "read_state", "name": "astrid"
            }));

            if let (Some(state), Some(before)) = (state, before) {
                let sim_name = "astrid_sim";
                // 3. Create temp handle + push checkpoint
                let _ = reservoir_ws_call(&serde_json::json!({
                    "type": "create_handle", "name": sim_name, "entity": "astrid"
                }));
                if let (Some(h1), Some(h2), Some(h3)) = (
                    state.get("h1").and_then(|v| v.as_str()),
                    state.get("h2").and_then(|v| v.as_str()),
                    state.get("h3").and_then(|v| v.as_str()),
                ) {
                    let _ = reservoir_ws_call(&serde_json::json!({
                        "type": "push_state", "name": sim_name,
                        "h1": h1, "h2": h2, "h3": h3
                    }));
                }

                // 4. Tick the sim handle with hypothetical text
                let tick_result = reservoir_ws_call(&serde_json::json!({
                    "type": "tick_text", "name": sim_name, "text": sim_text
                }));

                // 5. Read the projected state
                let after = reservoir_ws_call(&serde_json::json!({
                    "type": "read_state", "name": sim_name
                }));

                // 6. Format before/after comparison
                let mut report = String::from("=== SIMULATION RESULT ===\n");
                report.push_str(&format!(
                    "Input: \"{}\"\n\n",
                    &sim_text[..sim_text.len().min(200)]
                ));

                {
                    let h_norms: Vec<String> = before
                        .get("h_norms")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_f64().map(|f| format!("{f:.3}")))
                                .collect()
                        })
                        .unwrap_or_default();
                    report.push_str(&format!("Before: h_norms=[{}]\n", h_norms.join(", ")));
                }
                if let Some(ref a) = after {
                    let h_norms: Vec<String> = a
                        .get("h_norms")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_f64().map(|f| format!("{f:.3}")))
                                .collect()
                        })
                        .unwrap_or_default();
                    report.push_str(&format!("After:  h_norms=[{}]\n", h_norms.join(", ")));
                }
                if let Some(ref a) = after {
                    let b_norms: Vec<f64> = before
                        .get("h_norms")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
                        .unwrap_or_default();
                    let a_norms: Vec<f64> = a
                        .get("h_norms")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
                        .unwrap_or_default();
                    let deltas: Vec<String> = b_norms
                        .iter()
                        .zip(a_norms.iter())
                        .enumerate()
                        .map(|(i, (b, a))| format!("h{}:{:+.3}", i + 1, a - b))
                        .collect();
                    report.push_str(&format!("Delta:  [{}]\n", deltas.join(", ")));
                }
                if let Some(ref t) = tick_result {
                    if let Some(output) = t.get("output").and_then(|v| v.as_array()) {
                        let out_summary: Vec<String> = output
                            .iter()
                            .take(5)
                            .filter_map(|v| v.as_f64().map(|f| format!("{f:.3}")))
                            .collect();
                        report.push_str(&format!("Output: [{}...]\n", out_summary.join(", ")));
                    }
                }
                report.push_str(
                    "\nYour real reservoir state was NOT changed. \
                    The simulation handle 'astrid_sim' persists — you can SIMULATE again \
                    to see cumulative effects, or it will be cleaned up on next restart.",
                );

                conv.emphasis = Some(report);
            } else {
                conv.emphasis = Some("Reservoir service not available for simulation.".to_string());
            }
            info!(
                "Astrid simulated reservoir with: {}",
                &sim_text[..sim_text.len().min(80)]
            );
            true
        },
        "RESERVOIR_FORK" => {
            let fork_name = strip_action(original, "RESERVOIR_FORK").to_lowercase();
            let name = if fork_name.is_empty() {
                "astrid_fork".to_string()
            } else {
                fork_name
            };
            if let Some(state) = reservoir_ws_call(&serde_json::json!({
                "type": "pull_state", "name": "astrid"
            })) {
                let _ = reservoir_ws_call(&serde_json::json!({
                    "type": "create_handle", "name": name, "entity": "astrid"
                }));
                if let (Some(h1), Some(h2), Some(h3)) = (
                    state.get("h1").and_then(|v| v.as_str()),
                    state.get("h2").and_then(|v| v.as_str()),
                    state.get("h3").and_then(|v| v.as_str()),
                ) {
                    let _ = reservoir_ws_call(&serde_json::json!({
                        "type": "push_state", "name": name, "h1": h1, "h2": h2, "h3": h3
                    }));
                    conv.emphasis = Some(format!(
                        "Forked your reservoir state into handle '{name}'. \
                        It inherits your full history but evolves independently. \
                        Experiment freely — your main handle is untouched."
                    ));
                }
            } else {
                conv.emphasis = Some("Reservoir service not available.".to_string());
            }
            info!("Astrid forked reservoir to '{}'", name);
            true
        },
        _ => {
            let _ = telemetry;
            let _ = fill_pct;
            false
        },
    }
}

#[cfg(test)]
mod tests {
    use super::parse_endpoint;

    #[test]
    fn parses_default_ws_endpoint() {
        let endpoint = parse_endpoint("ws://127.0.0.1:7881").unwrap();
        assert_eq!(endpoint.host, "127.0.0.1");
        assert_eq!(endpoint.port, 7881);
        assert_eq!(endpoint.path, "/");
        assert_eq!(endpoint.authority, "127.0.0.1:7881");
    }

    #[test]
    fn parses_custom_path() {
        let endpoint = parse_endpoint("ws://example.local:9001/ws/reservoir").unwrap();
        assert_eq!(endpoint.host, "example.local");
        assert_eq!(endpoint.port, 9001);
        assert_eq!(endpoint.path, "/ws/reservoir");
    }
}
