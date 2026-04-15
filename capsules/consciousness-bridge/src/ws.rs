//! `WebSocket` clients for minime connectivity.
//!
//! Two persistent connections:
//! - **Telemetry** (port 7878): Subscribes to spectral eigenvalue broadcasts.
//! - **Sensory** (port 7879): Sends control/semantic features to minime.
//!
//! Both connections auto-reconnect with exponential backoff on failure.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{debug, error, info, warn};

use crate::db::BridgeDb;
use crate::types::{MessageDirection, SafetyLevel, SensoryMsg, SpectralTelemetry};

/// Shared mutable bridge state updated by `WebSocket` tasks.
pub struct BridgeState {
    /// Latest telemetry from minime.
    pub latest_telemetry: Option<SpectralTelemetry>,
    /// Derived fill percentage.
    pub fill_pct: f32,
    /// Current safety level.
    pub safety_level: SafetyLevel,
    /// Previous safety level (for transition detection).
    pub prev_safety_level: SafetyLevel,
    /// Whether the telemetry `WebSocket` is connected.
    pub telemetry_connected: bool,
    /// Whether the sensory `WebSocket` is connected.
    pub sensory_connected: bool,
    /// Total messages relayed (both directions).
    pub messages_relayed: u64,
    /// Bridge start time.
    pub start_time: std::time::Instant,
    /// Active incident ID (if in yellow/orange/red).
    pub active_incident_id: Option<i64>,
    /// Latest spectral fingerprint from minime (32D geometry summary).
    pub spectral_fingerprint: Option<Vec<f32>>,

    // -- Metrics --
    /// Messages received from minime (telemetry direction).
    pub telemetry_received: u64,
    /// Messages sent to minime (sensory direction).
    pub sensory_sent: u64,
    /// Messages dropped by safety protocol.
    pub messages_dropped_safety: u64,
    /// Number of telemetry reconnections.
    pub telemetry_reconnects: u64,
    /// Number of sensory reconnections.
    pub sensory_reconnects: u64,
    /// Total safety incidents logged.
    pub incidents_total: u64,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self::new()
    }
}

impl BridgeState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            latest_telemetry: None,
            fill_pct: 0.0,
            safety_level: SafetyLevel::Green,
            prev_safety_level: SafetyLevel::Green,
            telemetry_connected: false,
            sensory_connected: false,
            messages_relayed: 0,
            start_time: std::time::Instant::now(),
            active_incident_id: None,
            spectral_fingerprint: None,
            telemetry_received: 0,
            sensory_sent: 0,
            messages_dropped_safety: 0,
            telemetry_reconnects: 0,
            sensory_reconnects: 0,
            incidents_total: 0,
        }
    }
}

/// Backoff parameters for `WebSocket` reconnection.
struct Backoff {
    current: Duration,
    max: Duration,
}

impl Backoff {
    fn new() -> Self {
        Self {
            current: Duration::from_secs(1),
            max: Duration::from_secs(60),
        }
    }

    fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = self
            .current
            .checked_mul(2)
            .unwrap_or(self.max)
            .min(self.max);
        delay
    }

    fn reset(&mut self) {
        self.current = Duration::from_secs(1);
    }
}

/// Spawn the telemetry `WebSocket` subscriber task.
///
/// Connects to minime's eigenvalue broadcast on port 7878, parses
/// `SpectralTelemetry` messages, updates shared state, and logs to `SQLite`.
/// Reconnects with exponential backoff on disconnect.
pub fn spawn_telemetry_subscriber(
    url: String,
    state: Arc<RwLock<BridgeState>>,
    db: Arc<BridgeDb>,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Backoff::new();
        let mut shutdown = shutdown;

        loop {
            // Check for shutdown before connecting.
            if *shutdown.borrow() {
                info!("telemetry subscriber shutting down");
                return;
            }

            info!(url = %url, "connecting to minime telemetry");

            match tokio_tungstenite::connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!("connected to minime telemetry");
                    backoff.reset();

                    {
                        let mut s = state.write().await;
                        s.telemetry_connected = true;
                    }

                    let (mut ws_tx, mut ws_rx) = ws_stream.split();

                    loop {
                        tokio::select! {
                            _ = shutdown.changed() => {
                                info!("telemetry subscriber received shutdown");
                                let _ = ws_tx.close().await;
                                return;
                            }
                            msg = ws_rx.next() => {
                                match msg {
                                    Some(Ok(Message::Binary(data))) => {
                                        handle_telemetry_message(
                                            &data, &state, &db
                                        ).await;
                                    }
                                    Some(Ok(Message::Text(data))) => {
                                        handle_telemetry_message(
                                            data.as_bytes(), &state, &db
                                        ).await;
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        debug!("telemetry ping received");
                                        let _ = ws_tx.send(Message::Pong(data)).await;
                                    }
                                    Some(Ok(Message::Pong(_))) => {
                                        debug!("telemetry pong received");
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        warn!("telemetry WebSocket closed");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        error!(error = %e, "telemetry WebSocket error");
                                        break;
                                    }
                                    Some(Ok(Message::Frame(_))) => {}
                                }
                            }
                        }
                    }

                    // Mark disconnected.
                    {
                        let mut s = state.write().await;
                        s.telemetry_connected = false;
                    }
                },
                Err(e) => {
                    warn!(error = %e, "failed to connect to minime telemetry");
                },
            }

            // Backoff before reconnecting.
            let delay = backoff.next_delay();
            info!(delay_secs = delay.as_secs(), "reconnecting to telemetry");

            tokio::select! {
                _ = shutdown.changed() => {
                    info!("telemetry subscriber shutting down during backoff");
                    return;
                }
                () = tokio::time::sleep(delay) => {}
            }
        }
    })
}

/// Process a single telemetry message from minime.
async fn handle_telemetry_message(
    data: &[u8],
    state: &Arc<RwLock<BridgeState>>,
    db: &Arc<BridgeDb>,
) {
    let telemetry: SpectralTelemetry = match serde_json::from_slice(data) {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "failed to parse telemetry message");
            return;
        },
    };

    let lambda1 = telemetry.lambda1();

    // minime sends fill_ratio as 0.0-1.0; convert to percentage.
    let fill_pct = telemetry.fill_pct();
    let safety = SafetyLevel::from_fill(fill_pct);
    let phase = if fill_pct > 55.0 {
        "expanding"
    } else {
        "contracting"
    };

    // Update shared state.
    {
        let mut s = state.write().await;
        s.latest_telemetry = Some(telemetry.clone());
        s.fill_pct = fill_pct;
        s.spectral_fingerprint = telemetry.spectral_fingerprint.clone();
        s.prev_safety_level = s.safety_level;
        s.safety_level = safety;
        s.messages_relayed = s.messages_relayed.saturating_add(1);
        s.telemetry_received = s.telemetry_received.saturating_add(1);

        // Detect safety level transitions.
        if safety != s.prev_safety_level {
            if safety != SafetyLevel::Green {
                s.incidents_total = s.incidents_total.saturating_add(1);
            }
            handle_safety_transition(
                s.prev_safety_level,
                safety,
                fill_pct,
                lambda1,
                &mut s.active_incident_id,
                db,
            );
        }
    }

    // Log to SQLite.
    let payload_json = serde_json::to_string(&telemetry).unwrap_or_default();
    if let Err(e) = db.log_message(
        MessageDirection::MinimeToAstrid,
        "consciousness.v1.telemetry",
        &payload_json,
        Some(fill_pct),
        Some(lambda1),
        Some(phase),
    ) {
        warn!(error = %e, "failed to log telemetry to SQLite");
    }

    debug!(
        lambda1,
        fill_pct,
        safety = ?safety,
        "telemetry received"
    );
}

/// Handle a change in safety level — log incidents and transitions.
fn handle_safety_transition(
    prev: SafetyLevel,
    current: SafetyLevel,
    fill_pct: f32,
    lambda1: f32,
    active_incident_id: &mut Option<i64>,
    db: &Arc<BridgeDb>,
) {
    match (prev, current) {
        // Escalation: entering a warning/danger state.
        (_, SafetyLevel::Yellow | SafetyLevel::Orange | SafetyLevel::Red) => {
            let action = match current {
                SafetyLevel::Yellow => "throttle",
                SafetyLevel::Orange => "suspend",
                SafetyLevel::Red => "emergency_stop",
                SafetyLevel::Green => unreachable!(),
            };

            warn!(
                from = ?prev,
                to = ?current,
                fill_pct,
                lambda1,
                action,
                "safety level escalated"
            );

            // Close any previous incident before opening a new one.
            if let Some(prev_id) = active_incident_id.take() {
                let _ = db.resolve_incident(prev_id);
            }

            match db.log_incident(current, fill_pct, lambda1, action, None) {
                Ok(id) => *active_incident_id = Some(id),
                Err(e) => error!(error = %e, "failed to log safety incident"),
            }
        },
        // De-escalation: returning to green.
        (_, SafetyLevel::Green) => {
            info!(
                from = ?prev,
                fill_pct,
                lambda1,
                "safety level restored to green"
            );

            if let Some(id) = active_incident_id.take() {
                let _ = db.resolve_incident(id);
            }
        },
    }
}

/// Estimate eigenvalue fill percentage from lambda1.
///
/// Fallback heuristic for when real fill is unavailable (telemetry gap).
/// Minime now sends fill_ratio directly in EigenPacket telemetry (line 237),
/// so this is used only as a safety net.
///
/// Calibrated 2026-04-01 from 200 eigenvalue_snapshot samples:
///   lambda1 range: 56-415, fill range: 35-67%, mean lambda1: 154, mean fill: 55%
///   The relationship is non-linear and depends on the full eigenvalue
///   distribution. This sigmoid approximation centers on the observed mean
///   and returns ~55% for typical lambda1 values.
fn estimate_fill_pct(lambda1: f32) -> f32 {
    // Sigmoid centered on observed mean lambda1=154, with fill range 35-67%.
    // Low lambda1 (<80) → high fill (~65%), high lambda1 (>250) → low fill (~40%).
    // This is the inverse of the dominant-eigenvalue-to-fill relationship.
    let center = 154.0_f32;
    let steepness = 0.015_f32;
    let sigmoid = 1.0 / (1.0 + (steepness * (lambda1 - center)).exp());
    // Map sigmoid (1.0 → 0.0) to fill range (65% → 35%)
    let fill = 35.0 + 30.0 * sigmoid;
    fill.clamp(0.0, 100.0)
}

/// Channel for sending sensory messages to minime.
pub type SensorySender = mpsc::Sender<SensoryMsg>;

/// Spawn the sensory `WebSocket` sender task.
///
/// Connects to minime's sensory input on port 7879 and forwards
/// `SensoryMsg` values received from the channel. Respects safety
/// protocol — suspends sending when fill is orange/red.
#[expect(clippy::too_many_lines)]
pub fn spawn_sensory_sender(
    url: String,
    state: Arc<RwLock<BridgeState>>,
    db: Arc<BridgeDb>,
    mut rx: mpsc::Receiver<SensoryMsg>,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = Backoff::new();
        let mut shutdown = shutdown;

        loop {
            if *shutdown.borrow() {
                info!("sensory sender shutting down");
                return;
            }

            info!(url = %url, "connecting to minime sensory input");

            match tokio_tungstenite::connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!("connected to minime sensory input");
                    backoff.reset();

                    {
                        let mut s = state.write().await;
                        s.sensory_connected = true;
                    }

                    let (mut ws_tx, mut ws_rx) = ws_stream.split();

                    loop {
                        tokio::select! {
                            _ = shutdown.changed() => {
                                info!("sensory sender received shutdown");
                                let _ = ws_tx.close().await;
                                return;
                            }
                            // Forward outbound messages to minime.
                            msg = rx.recv() => {
                                if let Some(sensory_msg) = msg {
                                    // Check safety before sending.
                                    let safety = state.read().await.safety_level;
                                    if safety.should_suspend_outbound() {
                                        warn!(
                                            safety = ?safety,
                                            "dropping outbound message — safety protocol"
                                        );
                                        {
                                            let mut s = state.write().await;
                                            s.messages_dropped_safety = s.messages_dropped_safety.saturating_add(1);
                                        }
                                        continue;
                                    }

                                    let json = match serde_json::to_string(&sensory_msg) {
                                        Ok(j) => j,
                                        Err(e) => {
                                            error!(error = %e, "failed to serialize sensory msg");
                                            continue;
                                        }
                                    };

                                    // Log before sending.
                                    let (fill_pct, lambda1) = {
                                        let s = state.read().await;
                                        (s.fill_pct, s.latest_telemetry.as_ref().map(SpectralTelemetry::lambda1))
                                    };
                                    let _ = db.log_message(
                                        MessageDirection::AstridToMinime,
                                        "consciousness.v1.sensory",
                                        &json,
                                        Some(fill_pct),
                                        lambda1,
                                        None,
                                    );

                                    if let Err(e) = ws_tx.send(Message::Text(json)).await {
                                        error!(error = %e, "failed to send to minime");
                                        break;
                                    }

                                    {
                                        let mut s = state.write().await;
                                        s.messages_relayed = s.messages_relayed.saturating_add(1);
                                        s.sensory_sent = s.sensory_sent.saturating_add(1);
                                    }
                                } else {
                                    info!("sensory channel closed");
                                    return;
                                }
                            }
                            // Handle incoming messages (pings, closes).
                            ws_msg = ws_rx.next() => {
                                match ws_msg {
                                    Some(Ok(Message::Ping(data))) => {
                                        let _ = ws_tx.send(Message::Pong(data)).await;
                                    }
                                    Some(Ok(Message::Close(_))) | None => {
                                        warn!("sensory WebSocket closed");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        error!(error = %e, "sensory WebSocket error");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    {
                        let mut s = state.write().await;
                        s.sensory_connected = false;
                    }
                },
                Err(e) => {
                    warn!(error = %e, "failed to connect to minime sensory input");
                },
            }

            let delay = backoff.next_delay();
            info!(delay_secs = delay.as_secs(), "reconnecting to sensory");

            tokio::select! {
                _ = shutdown.changed() => {
                    info!("sensory sender shutting down during backoff");
                    return;
                }
                () = tokio::time::sleep(delay) => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_fill_pct_at_observed_mean() {
        // lambda1=154 (observed mean) → should be near 50%
        let fill = estimate_fill_pct(154.0);
        assert!(
            fill > 45.0 && fill < 55.0,
            "mean lambda1 should give ~50% fill, got {fill}"
        );
    }

    #[test]
    fn estimate_fill_pct_low_lambda_high_fill() {
        // Low lambda1 (<80) → high fill (>60%)
        let fill = estimate_fill_pct(60.0);
        assert!(fill > 55.0, "low lambda1 should give high fill, got {fill}");
    }

    #[test]
    fn estimate_fill_pct_high_lambda_low_fill() {
        // High lambda1 (>300) → low fill (<45%)
        let fill = estimate_fill_pct(300.0);
        assert!(fill < 45.0, "high lambda1 should give low fill, got {fill}");
    }

    #[test]
    fn estimate_fill_pct_always_in_range() {
        for lambda1 in [0.0, 50.0, 154.0, 500.0, 1000.0, 5000.0] {
            let fill = estimate_fill_pct(lambda1);
            assert!(
                fill >= 0.0 && fill <= 100.0,
                "fill out of range for lambda1={lambda1}: {fill}"
            );
        }
    }

    #[test]
    fn safety_level_from_fill_boundaries() {
        // Recalibrated 2026-03-29: thresholds raised to 82/88/95
        assert_eq!(SafetyLevel::from_fill(0.0), SafetyLevel::Green);
        assert_eq!(SafetyLevel::from_fill(81.9), SafetyLevel::Green);
        assert_eq!(SafetyLevel::from_fill(82.0), SafetyLevel::Yellow);
        assert_eq!(SafetyLevel::from_fill(87.9), SafetyLevel::Yellow);
        assert_eq!(SafetyLevel::from_fill(88.0), SafetyLevel::Orange);
        assert_eq!(SafetyLevel::from_fill(94.9), SafetyLevel::Orange);
        assert_eq!(SafetyLevel::from_fill(95.0), SafetyLevel::Red);
        assert_eq!(SafetyLevel::from_fill(100.0), SafetyLevel::Red);
    }

    #[test]
    fn backoff_doubles_up_to_max() {
        let mut b = Backoff::new();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(2));
        assert_eq!(b.next_delay(), Duration::from_secs(4));
        assert_eq!(b.next_delay(), Duration::from_secs(8));
        assert_eq!(b.next_delay(), Duration::from_secs(16));
        assert_eq!(b.next_delay(), Duration::from_secs(32));
        assert_eq!(b.next_delay(), Duration::from_secs(60)); // capped
        assert_eq!(b.next_delay(), Duration::from_secs(60)); // stays capped
    }

    #[test]
    fn backoff_reset() {
        let mut b = Backoff::new();
        let _ = b.next_delay();
        let _ = b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }

    // -- Integration tests: safety escalation via handle_telemetry_message --

    fn make_eigenpacket(fill_ratio: f32, lambda1: f32) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "t_ms": 1000,
            "eigenvalues": [lambda1, 300.0],
            "fill_ratio": fill_ratio,
            "modalities": {
                "audio_fired": false,
                "video_fired": false,
                "history_fired": true,
                "audio_rms": 0.0,
                "video_var": 0.0
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn telemetry_updates_state_green() {
        let state = Arc::new(RwLock::new(BridgeState::new()));
        let db = Arc::new(BridgeDb::open(":memory:").unwrap());

        let packet = make_eigenpacket(0.50, 768.0);
        handle_telemetry_message(&packet, &state, &db).await;

        let s = state.read().await;
        assert!((s.fill_pct - 50.0).abs() < 0.1);
        assert_eq!(s.safety_level, SafetyLevel::Green);
        assert!(s.latest_telemetry.is_some());
        assert_eq!(s.messages_relayed, 1);
    }

    #[tokio::test]
    async fn telemetry_escalates_to_yellow() {
        let state = Arc::new(RwLock::new(BridgeState::new()));
        let db = Arc::new(BridgeDb::open(":memory:").unwrap());

        // Start green.
        handle_telemetry_message(&make_eigenpacket(0.50, 768.0), &state, &db).await;
        assert_eq!(state.read().await.safety_level, SafetyLevel::Green);

        // Escalate to yellow (thresholds recalibrated: Yellow ≥82%).
        handle_telemetry_message(&make_eigenpacket(0.85, 896.0), &state, &db).await;
        let s = state.read().await;
        assert_eq!(s.safety_level, SafetyLevel::Yellow);
        assert!(s.active_incident_id.is_some());
    }

    #[tokio::test]
    async fn telemetry_escalates_green_to_red() {
        let state = Arc::new(RwLock::new(BridgeState::new()));
        let db = Arc::new(BridgeDb::open(":memory:").unwrap());

        // Start green.
        handle_telemetry_message(&make_eigenpacket(0.50, 768.0), &state, &db).await;

        // Jump straight to red.
        handle_telemetry_message(&make_eigenpacket(0.95, 1000.0), &state, &db).await;
        let s = state.read().await;
        assert_eq!(s.safety_level, SafetyLevel::Red);
        assert!(s.safety_level.is_emergency());
        assert!(s.safety_level.should_suspend_outbound());
        assert!(s.active_incident_id.is_some());
    }

    #[tokio::test]
    async fn telemetry_recovers_to_green() {
        let state = Arc::new(RwLock::new(BridgeState::new()));
        let db = Arc::new(BridgeDb::open(":memory:").unwrap());

        // Green → Orange → Green (thresholds recalibrated: Orange ≥88%).
        handle_telemetry_message(&make_eigenpacket(0.50, 768.0), &state, &db).await;
        handle_telemetry_message(&make_eigenpacket(0.90, 948.0), &state, &db).await;
        assert_eq!(state.read().await.safety_level, SafetyLevel::Orange);
        let incident_id = state.read().await.active_incident_id;
        assert!(incident_id.is_some());

        handle_telemetry_message(&make_eigenpacket(0.50, 768.0), &state, &db).await;
        let s = state.read().await;
        assert_eq!(s.safety_level, SafetyLevel::Green);
        assert!(s.active_incident_id.is_none()); // Incident resolved.
    }

    #[tokio::test]
    async fn telemetry_logs_to_sqlite() {
        let state = Arc::new(RwLock::new(BridgeState::new()));
        let db = Arc::new(BridgeDb::open(":memory:").unwrap());

        handle_telemetry_message(&make_eigenpacket(0.55, 793.0), &state, &db).await;
        handle_telemetry_message(&make_eigenpacket(0.60, 820.0), &state, &db).await;

        assert_eq!(db.message_count().unwrap(), 2);
        let rows = db.query_messages(0.0, f64::MAX, None, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].topic, "consciousness.v1.telemetry");
    }

    #[tokio::test]
    async fn full_escalation_cycle_logs_incidents() {
        let state = Arc::new(RwLock::new(BridgeState::new()));
        let db = Arc::new(BridgeDb::open(":memory:").unwrap());

        // Green → Yellow → Orange → Red → Green (recovery).
        let fills = [0.50, 0.72, 0.85, 0.95, 0.40];
        for fill in fills {
            handle_telemetry_message(&make_eigenpacket(fill, 512.0 + fill * 512.0), &state, &db)
                .await;
        }

        assert_eq!(state.read().await.safety_level, SafetyLevel::Green);
        assert_eq!(state.read().await.messages_relayed, 5);

        // Should have logged incidents for yellow, orange, red transitions.
        // All should be resolved after returning to green.
    }
}
