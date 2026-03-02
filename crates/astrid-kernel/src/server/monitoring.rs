//! Background monitoring loops: health checks, ephemeral shutdown, session cleanup.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use astrid_core::SessionId;
use tracing::{info, warn};

use super::DaemonServer;

/// Guard that aborts a spawned Tokio task when dropped.
///
/// Unlike `JoinHandle::drop`, which does NOT cancel the task, this guard
/// ensures background tasks are cleaned up when their owner is cancelled.
pub(super) struct AbortOnDrop(pub(super) tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl DaemonServer {
    /// Spawn the health monitoring loop.
    ///
    /// Checks server health at the configured interval (from
    /// `gateway.health_interval_secs`, floored at 5 s). Dead servers with a
    /// restart policy are automatically reconnected.
    ///
    /// The loop clones the `McpClient` out of the runtime once at startup
    /// (cheap -- all `Arc` internals) so that health checks and reconnects
    /// never block session-mutating RPCs.
    #[must_use]
    pub fn spawn_health_loop(&self) -> tokio::task::JoinHandle<()> {
        let mcp = self.runtime.mcp().clone();
        let health_interval = self.health_interval;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(health_interval);
            loop {
                interval.tick().await;

                let health = mcp.server_manager().health_check().await;

                for (name, alive) in &health {
                    if !alive {
                        warn!(server = %name, "MCP server is dead");
                        match mcp.try_reconnect(name).await {
                            Ok(true) => {
                                info!(server = %name, "Server restarted by health loop");
                            },
                            Ok(false) => {
                                info!(server = %name, "Restart not allowed by policy");
                            },
                            Err(e) => {
                                warn!(server = %name, error = %e, "Restart failed");
                            },
                        }
                    }
                }
            }
        })
    }

    /// Spawn the ephemeral shutdown monitor.
    ///
    /// Returns `None` if the daemon is in persistent mode. In ephemeral mode,
    /// the monitor waits an initial 10 s for the first client to connect, then
    /// polls `active_connections` every 5 s. When all connections have been
    /// gone for `ephemeral_grace_secs` it sends a shutdown signal.
    #[must_use]
    pub fn spawn_ephemeral_monitor(&self) -> Option<tokio::task::JoinHandle<()>> {
        if !self.ephemeral {
            return None;
        }

        let connections = Arc::clone(&self.active_connections);
        let shutdown_tx = self.shutdown_tx.clone();
        let grace = Duration::from_secs(self.ephemeral_grace_secs);

        Some(tokio::spawn(async move {
            // Give the first client time to connect after auto-start.
            tokio::time::sleep(Duration::from_secs(10)).await;

            let mut idle_since: Option<Instant> = None;

            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;

                let count = connections.load(Ordering::Relaxed);
                if count == 0 {
                    let start = *idle_since.get_or_insert_with(Instant::now);
                    if start.elapsed() >= grace {
                        info!(
                            "Ephemeral daemon idle for {}s — shutting down",
                            grace.as_secs()
                        );
                        let _ = shutdown_tx.send(());
                        return;
                    }
                } else {
                    // Reset whenever at least one client is connected.
                    idle_since = None;
                }
            }
        }))
    }

    /// Spawn the stale-session cleanup loop.
    ///
    /// Periodically sweeps the session map looking for orphaned sessions
    /// (no event subscribers and no active turn). Orphaned sessions are
    /// saved to disk and removed from the in-memory map.
    ///
    /// Also sweeps `connector_sessions` to prune entries whose referenced
    /// session no longer exists in the sessions map (e.g. after a direct
    /// `end_session` RPC call on a connector session).
    #[must_use]
    pub fn spawn_session_cleanup_loop(&self) -> tokio::task::JoinHandle<()> {
        let sessions = Arc::clone(&self.sessions);
        let connector_sessions = Arc::clone(&self.connector_sessions);
        let runtime = Arc::clone(&self.runtime);
        let interval = self.session_cleanup_interval;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;

                let orphaned: Vec<SessionId> = {
                    let map = sessions.read().await;
                    let mut ids = Vec::new();
                    for (id, handle) in map.iter() {
                        // Connector sessions have no CLI subscribers by design —
                        // they are managed by the inbound router and must not be
                        // evicted on idle. Evicting them destroys conversation
                        // continuity between turns for the same user.
                        if handle.user_id.is_some() {
                            continue;
                        }

                        let no_subscribers = handle.event_tx.receiver_count() == 0;

                        // Use try_lock (non-blocking) instead of an async lock to
                        // avoid holding the sessions read-lock across an await —
                        // which violates the module's locking discipline. If the
                        // Mutex is contended (a task is actively clearing it),
                        // treat the session conservatively as having an active turn.
                        let no_active_turn = handle.turn_handle.try_lock().ok().is_some_and(|g| {
                            g.as_ref().is_none_or(tokio::task::JoinHandle::is_finished)
                        });

                        if no_subscribers && no_active_turn {
                            ids.push(id.clone());
                        }
                    }
                    ids
                };

                if !orphaned.is_empty() {
                    let to_save: Vec<_> = {
                        let mut map = sessions.write().await;
                        orphaned
                            .iter()
                            .filter_map(|id| map.remove(id).map(|h| (id.clone(), h)))
                            .collect()
                    };

                    for (id, handle) in to_save {
                        let session = handle.session.lock().await;
                        if let Err(e) = runtime.save_session(&session) {
                            warn!(session_id = %id, error = %e, "Failed to save orphaned session");
                        } else {
                            info!(session_id = %id, "Cleaned up orphaned session");
                        }
                        // Evict plugin KV stores for this session (same as end_session).
                        runtime.cleanup_capsule_kv_stores(&id);
                    }
                }

                // Sweep connector_sessions for entries whose referenced session
                // no longer exists. This handles the case where end_session_impl
                // was called directly on a connector session (which removes it
                // from `sessions` but has no access to `connector_sessions`).
                // Lock ordering: read sessions first, write connector_sessions second.
                {
                    let live = sessions.read().await;
                    connector_sessions
                        .write()
                        .await
                        .retain(|_, sid| live.contains_key(sid));
                }
            }
        })
    }
}
