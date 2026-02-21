//! Event subscription, shutdown, and tool listing RPC method implementations.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::{PendingSubscriptionSink, SubscriptionMessage};
use tokio::sync::broadcast;
use tracing::{info, warn};

use super::RpcImpl;
use crate::rpc::{ToolInfo, error_codes};

impl RpcImpl {
    pub(super) async fn subscribe_events_impl(
        &self,
        pending: PendingSubscriptionSink,
        session_id: astrid_core::SessionId,
    ) -> jsonrpsee::core::SubscriptionResult {
        // Look up the session handle (brief read lock).
        let handle = {
            let sessions = self.sessions.read().await;
            let h = sessions.get(&session_id).cloned().ok_or_else(|| {
                jsonrpsee::core::StringError::from(format!("Session not found: {session_id}"))
            })?;

            if h.is_connector() {
                return Err(jsonrpsee::core::StringError::from(
                    "session is managed by the inbound router and its events cannot be subscribed to via RPC",
                ));
            }
            h
        };

        let mut event_rx = handle.event_tx.subscribe();

        let sink = pending.accept().await?;

        // Track this connection for ephemeral shutdown monitoring.
        let connections = Arc::clone(&self.active_connections);
        connections.fetch_add(1, Ordering::Relaxed);

        // Spawn a task to forward events from the broadcast channel to the subscription.
        tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let msg = SubscriptionMessage::from_json(&event);
                        match msg {
                            Ok(msg) => {
                                if sink.send(msg).await.is_err() {
                                    break; // Client disconnected.
                                }
                            },
                            Err(e) => {
                                warn!("Failed to serialize event: {e}");
                            },
                        }
                    },
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "Event subscriber lagged");
                    },
                    Err(broadcast::error::RecvError::Closed) => {
                        break; // Channel closed (session ended).
                    },
                }
            }

            // Client disconnected -- decrement connection count.
            connections.fetch_sub(1, Ordering::Relaxed);
        });

        Ok(())
    }

    pub(super) fn shutdown_impl(&self) {
        let _ = self.shutdown_tx.send(());
        info!("Shutdown requested via RPC");
    }

    pub(super) async fn list_tools_impl(&self) -> Result<Vec<ToolInfo>, ErrorObjectOwned> {
        let mut result: Vec<ToolInfo> = Vec::new();

        // MCP server tools.
        let tools = self.runtime.mcp().list_tools().await.map_err(|e| {
            ErrorObjectOwned::owned(
                error_codes::INTERNAL_ERROR,
                format!("Failed to list tools: {e}"),
                None::<()>,
            )
        })?;
        result.extend(tools.into_iter().map(|t| ToolInfo {
            name: t.name,
            server: t.server,
            description: t.description,
        }));

        // Plugin tools.
        let registry = self.plugin_registry.read().await;
        for td in registry.all_tool_definitions() {
            // Qualified name is "plugin:{plugin_id}:{tool_name}".
            // Extract the "plugin:{plugin_id}" prefix as the server field.
            let server = td
                .name
                .rsplit_once(':')
                .map_or_else(|| td.name.clone(), |(prefix, _)| prefix.to_string());
            result.push(ToolInfo {
                name: td.name,
                server,
                description: Some(td.description),
            });
        }

        Ok(result)
    }
}
