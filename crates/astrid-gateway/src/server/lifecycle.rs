//! Daemon shutdown and cleanup logic.

use astrid_plugins::PluginId;
use tracing::{info, warn};

use super::DaemonServer;

impl DaemonServer {
    /// Gracefully unload all registered plugins.
    pub async fn shutdown_plugins(&self) {
        let mut registry = self.plugin_registry.write().await;
        let ids: Vec<PluginId> = registry.list().into_iter().cloned().collect();
        for id in ids {
            if let Some(plugin) = registry.get_mut(&id)
                && let Err(e) = plugin.unload().await
            {
                warn!(plugin_id = %id, error = %e, "Error unloading plugin during shutdown");
            }
        }
        info!("Plugins shut down");
    }

    /// Gracefully shut down all MCP servers.
    pub async fn shutdown_servers(&self) {
        if let Err(e) = self.runtime.mcp().shutdown().await {
            warn!(error = %e, "Error shutting down MCP servers");
        }
        info!("MCP servers shut down");
    }

    /// Clean up daemon files (PID, port, mode) on shutdown.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(self.paths.pid_file());
        let _ = std::fs::remove_file(self.paths.port_file());
        let _ = std::fs::remove_file(self.paths.mode_file());
        info!("Daemon files cleaned up");
    }
}
