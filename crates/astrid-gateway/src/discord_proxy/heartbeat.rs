//! Heartbeat task for Discord Gateway zombie connection detection.
//!
//! Runs as a concurrent task alongside the `WebSocket` reader. Sends
//! periodic heartbeats and detects zombie connections when ACKs are
//! not received.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tracing::{debug, trace, warn};

use super::protocol::{self, GatewayPayload};

/// Tracks heartbeat health for zombie connection detection.
pub(crate) struct HeartbeatState {
    /// Whether we received an ACK for the last heartbeat we sent.
    pub last_ack_received: bool,
}

impl HeartbeatState {
    /// Create a new heartbeat state, starting with ACK received
    /// (no heartbeat sent yet).
    pub(super) fn new() -> Self {
        Self {
            last_ack_received: true,
        }
    }

    /// Record that a heartbeat ACK was received.
    pub(super) fn ack_received(&mut self) {
        self.last_ack_received = true;
        trace!("Heartbeat ACK received");
    }
}

/// Runs the heartbeat loop.
///
/// # Arguments
///
/// * `interval_ms` — Heartbeat interval from the Hello payload.
/// * `sequence` — Shared sequence number (updated by the event loop).
/// * `heartbeat_state` — Shared ACK tracking state.
/// * `ws_tx` — Channel to send outbound payloads to the writer task.
/// * `zombie_tx` — Oneshot to signal zombie detection to the event loop.
/// * `shutdown_rx` — Daemon shutdown signal.
///
/// # Lifecycle
///
/// The first heartbeat is sent after `interval_ms * jitter` (random
/// 0.0..1.0) to prevent thundering herd. Subsequent heartbeats are
/// sent at exactly `interval_ms`.
///
/// If the previous heartbeat's ACK has not been received when it's
/// time to send the next, the connection is considered a zombie.
/// The `zombie_tx` oneshot fires to signal the event loop to
/// reconnect.
pub(crate) async fn run_heartbeat(
    interval_ms: u64,
    sequence: Arc<Mutex<Option<u64>>>,
    heartbeat_state: Arc<Mutex<HeartbeatState>>,
    ws_tx: mpsc::Sender<GatewayPayload>,
    zombie_tx: oneshot::Sender<()>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    // First beat: jitter to prevent thundering herd.
    let jitter_factor = f64::from(fastrand::u32(0..1000)) / 1000.0;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let first_delay_ms = (interval_ms as f64 * jitter_factor) as u64;
    let first_delay = Duration::from_millis(first_delay_ms);

    debug!(interval_ms, first_delay_ms, "Heartbeat task started");

    tokio::select! {
        biased;
        _ = shutdown_rx.recv() => return,
        () = tokio::time::sleep(first_delay) => {},
    }

    // Send first heartbeat.
    if send_heartbeat_if_healthy(&sequence, &heartbeat_state, &ws_tx)
        .await
        .is_err()
    {
        let _ = zombie_tx.send(());
        return;
    }

    let interval = Duration::from_millis(interval_ms);
    loop {
        tokio::select! {
            biased;
            _ = shutdown_rx.recv() => {
                debug!("Heartbeat task shutting down");
                return;
            }
            () = tokio::time::sleep(interval) => {
                if send_heartbeat_if_healthy(
                    &sequence,
                    &heartbeat_state,
                    &ws_tx,
                )
                .await
                .is_err()
                {
                    // Zombie detected — signal the event loop.
                    warn!(
                        "Heartbeat ACK missed — \
                         zombie connection detected"
                    );
                    let _ = zombie_tx.send(());
                    return;
                }
            }
        }
    }
}

/// Check ACK status and send a heartbeat if healthy.
///
/// Returns `Err(())` if the previous ACK was not received (zombie).
async fn send_heartbeat_if_healthy(
    sequence: &Arc<Mutex<Option<u64>>>,
    heartbeat_state: &Arc<Mutex<HeartbeatState>>,
    ws_tx: &mpsc::Sender<GatewayPayload>,
) -> Result<(), ()> {
    let mut state = heartbeat_state.lock().await;

    if !state.last_ack_received {
        return Err(());
    }

    let seq = *sequence.lock().await;
    let payload = protocol::build_heartbeat(seq);

    debug!(seq = ?seq, "Sending heartbeat");
    state.last_ack_received = false;
    drop(state);

    // If the send channel is closed, the writer task exited — treat
    // as connection lost (the outer loop will handle reconnection).
    if ws_tx.send(payload).await.is_err() {
        return Err(());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_state_initial() {
        let state = HeartbeatState::new();
        assert!(state.last_ack_received);
    }

    #[test]
    fn heartbeat_state_ack_cycle() {
        let mut state = HeartbeatState::new();
        // Simulate: we send a heartbeat, mark as not received.
        state.last_ack_received = false;
        assert!(!state.last_ack_received);
        // ACK arrives.
        state.ack_received();
        assert!(state.last_ack_received);
    }
}
