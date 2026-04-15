//! Integration test: bridge telemetry subscriber against a mock minime WebSocket.
//!
//! Starts a real WebSocket server on a random port, spawns the bridge's
//! telemetry subscriber, sends simulated `EigenPacket` JSON, and verifies
//! the bridge processes, logs, and reacts to the data correctly.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::Message;

// The server binary is a single crate; import via the binary's module structure.
// Since integration tests can't import `mod` items directly, we test via
// the binary. For this test we replicate the minimal necessary types.

fn eigenpacket_json(fill_ratio: f32, lambda1: f32, alert: Option<&str>) -> String {
    let alert_field = match alert {
        Some(a) => format!(r#""alert":"{}""#, a),
        None => r#""alert":null"#.to_string(),
    };
    format!(
        r#"{{"t_ms":5000,"eigenvalues":[{lambda1},300.0],"fill_ratio":{fill_ratio},"modalities":{{"audio_fired":false,"video_fired":false,"history_fired":true,"audio_rms":0.0,"video_var":0.0}},{alert_field}}}"#,
    )
}

/// Start a mock minime telemetry server on a random port.
/// Returns the address and a sender to push messages to connected clients.
async fn start_mock_telemetry_server() -> (SocketAddr, tokio::sync::mpsc::Sender<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);

    tokio::spawn(async move {
        // Accept one client.
        if let Ok((stream, _)) = listener.accept().await {
            let ws_stream = accept_async(stream).await.unwrap();
            let (mut ws_tx, _ws_rx): (futures_util::stream::SplitSink<_, Message>, _) =
                ws_stream.split();

            // Forward messages from the channel to the WebSocket client.
            while let Some(msg) = rx.recv().await {
                if ws_tx.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
        }
    });

    (addr, tx)
}

#[tokio::test]
async fn bridge_receives_telemetry_from_mock_ws() {
    // Start mock server.
    let (addr, tx) = start_mock_telemetry_server().await;
    let url = format!("ws://{addr}");

    // Set up bridge components.
    let db = Arc::new(consciousness_bridge_server::db::BridgeDb::open(":memory:").unwrap());
    let state = Arc::new(RwLock::new(
        consciousness_bridge_server::ws::BridgeState::new(),
    ));
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn telemetry subscriber.
    let _handle = consciousness_bridge_server::ws::spawn_telemetry_subscriber(
        url,
        Arc::clone(&state),
        Arc::clone(&db),
        shutdown_rx,
    );

    // Wait for connection.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Send a green telemetry packet.
    tx.send(eigenpacket_json(0.55, 793.0, None)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify state updated.
    {
        let s = state.read().await;
        assert!(s.telemetry_connected);
        assert!((s.fill_pct - 55.0).abs() < 0.5);
        assert_eq!(
            s.safety_level,
            consciousness_bridge_server::types::SafetyLevel::Green
        );
        assert!(s.messages_relayed >= 1);
    }

    // Verify SQLite logged the message.
    assert!(db.message_count().unwrap() >= 1);

    // Send an escalating packet (red zone).
    tx.send(eigenpacket_json(0.95, 998.0, Some("PANIC MODE ACTIVATED")))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    {
        let s = state.read().await;
        assert_eq!(
            s.safety_level,
            consciousness_bridge_server::types::SafetyLevel::Red
        );
        assert!(s.safety_level.should_suspend_outbound());
        assert!(s.active_incident_id.is_some());
    }

    // Send recovery.
    tx.send(eigenpacket_json(0.40, 717.0, None)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    {
        let s = state.read().await;
        assert_eq!(
            s.safety_level,
            consciousness_bridge_server::types::SafetyLevel::Green
        );
        assert!(s.active_incident_id.is_none());
    }
}

/// Start a mock minime sensory input server on a random port.
/// Returns the address and a receiver that yields messages sent by the bridge.
async fn start_mock_sensory_server() -> (SocketAddr, tokio::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let ws_stream = accept_async(stream).await.unwrap();
            let (_ws_tx, mut ws_rx): (futures_util::stream::SplitSink<_, Message>, _) =
                ws_stream.split();

            // Forward received messages to the channel.
            while let Some(Ok(msg)) = ws_rx.next().await {
                if let Message::Text(text) = msg {
                    if tx.send(text).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    (addr, rx)
}

/// Bidirectional end-to-end test:
/// - Telemetry flows from mock minime → bridge (verified via state + SQLite)
/// - Semantic features flow from bridge → mock minime (verified via received messages)
/// - Safety protocol blocks outbound during red state
#[tokio::test]
async fn bidirectional_bridge_with_safety_protocol() {
    use consciousness_bridge_server::types::{SafetyLevel, SensoryMsg};

    // Start both mock servers.
    let (telemetry_addr, telemetry_tx) = start_mock_telemetry_server().await;
    let (sensory_addr, mut sensory_rx) = start_mock_sensory_server().await;

    let telemetry_url = format!("ws://{telemetry_addr}");
    let sensory_url = format!("ws://{sensory_addr}");

    // Set up bridge.
    let db = Arc::new(consciousness_bridge_server::db::BridgeDb::open(":memory:").unwrap());
    let state = Arc::new(RwLock::new(
        consciousness_bridge_server::ws::BridgeState::new(),
    ));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (bridge_sensory_tx, sensory_channel_rx) = tokio::sync::mpsc::channel(256);

    // Spawn both WebSocket tasks.
    let _telemetry = consciousness_bridge_server::ws::spawn_telemetry_subscriber(
        telemetry_url,
        Arc::clone(&state),
        Arc::clone(&db),
        shutdown_rx.clone(),
    );

    let _sensory = consciousness_bridge_server::ws::spawn_sensory_sender(
        sensory_url,
        Arc::clone(&state),
        Arc::clone(&db),
        sensory_channel_rx,
        shutdown_rx,
    );

    // Wait for both connections.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // --- Step 1: Establish green state via telemetry ---
    telemetry_tx
        .send(eigenpacket_json(0.50, 768.0, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.read().await.safety_level, SafetyLevel::Green);

    // --- Step 2: Send semantic features → should arrive at mock sensory server ---
    let semantic_msg = SensoryMsg::Semantic {
        features: vec![1.0, 2.0, 3.0, 4.0],
        ts_ms: None,
    };
    bridge_sensory_tx.send(semantic_msg).await.unwrap();

    // Verify mock sensory server received the message.
    let received = tokio::time::timeout(Duration::from_secs(2), sensory_rx.recv())
        .await
        .expect("timeout waiting for sensory message")
        .expect("sensory channel closed");

    let parsed: serde_json::Value = serde_json::from_str(&received).unwrap();
    assert_eq!(parsed["kind"], "semantic");
    assert_eq!(parsed["features"].as_array().unwrap().len(), 4);

    // --- Step 3: Escalate to red → verify state ---
    telemetry_tx
        .send(eigenpacket_json(0.95, 998.0, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.read().await.safety_level, SafetyLevel::Red);
    assert!(state.read().await.safety_level.should_suspend_outbound());

    // --- Step 4: Recover to green → send a control message ---
    telemetry_tx
        .send(eigenpacket_json(0.40, 717.0, None))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.read().await.safety_level, SafetyLevel::Green);

    // Send a control message — should be delivered now that we're green.
    let control_msg = SensoryMsg::Control {
        synth_gain: Some(2.0),
        keep_bias: None,
        exploration_noise: None,
        fill_target: Some(0.55),
        legacy_audio_synth: None,
        legacy_video_synth: None,
        regulation_strength: None,
        deep_breathing: None,
        pure_tone: None,
        transition_cushion: None,
        smoothing_preference: None,
        geom_curiosity: None,
        target_lambda_bias: None,
        geom_drive: None,
        penalty_sensitivity: None,
        breathing_rate_scale: None,
        mem_mode: None,
        journal_resonance: None,
        checkpoint_interval: None,
        embedding_strength: None,
        memory_decay_rate: None,
        checkpoint_annotation: None,
        synth_noise_level: None,
    };
    bridge_sensory_tx.send(control_msg).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), sensory_rx.recv())
        .await
        .expect("timeout waiting for control message")
        .expect("sensory channel closed");

    let parsed: serde_json::Value = serde_json::from_str(&received).unwrap();
    assert_eq!(parsed["kind"], "control");
    assert_eq!(parsed["synth_gain"], 2.0);
    assert_eq!(parsed["fill_target"], 0.55);

    // --- Verify SQLite has all the messages ---
    let total = db.message_count().unwrap();
    assert!(
        total >= 4,
        "expected at least 4 logged messages, got {total}"
    );

    // Clean shutdown.
    let _ = shutdown_tx.send(true);
}
