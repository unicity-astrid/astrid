//! Workspace-scoped MCP server registry.
//!
//! Manages per-workspace [`McpClient`] instances with reference-counted
//! lifecycle. Global server names take priority — workspace configs that
//! collide with a global server name are rejected (logged, not fatal).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use astralis_mcp::{McpClient, ServersConfig};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Per-workspace MCP state.
struct WorkspaceMcpState {
    /// The MCP client for this workspace's servers.
    client: McpClient,
    /// Number of active sessions referencing this workspace.
    ref_count: usize,
    /// Server names from this workspace's `.mcp.json`.
    server_names: Vec<String>,
}

/// Registry that manages workspace-scoped MCP server lifecycles.
///
/// Each workspace with a `.mcp.json` gets its own [`McpClient`]. When the
/// first session in a workspace calls [`acquire`](Self::acquire), the registry
/// loads the config, creates a client, and connects auto-start servers.
/// Subsequent sessions in the same workspace share the client (ref-counted).
/// When the last session ends, [`release`](Self::release) shuts down the
/// workspace servers.
pub struct WorkspaceMcpRegistry {
    /// Per-workspace state, keyed by canonicalized workspace path.
    workspaces: RwLock<HashMap<PathBuf, WorkspaceMcpState>>,
    /// Names of globally-configured MCP servers (used for collision detection).
    global_server_names: HashSet<String>,
}

impl WorkspaceMcpRegistry {
    /// Create a new registry.
    ///
    /// `global_server_names` lists the names of servers configured globally
    /// (from `~/.astralis/servers.toml`). Workspace servers that collide
    /// with these names are rejected.
    #[must_use]
    pub fn new(global_server_names: HashSet<String>) -> Self {
        Self {
            workspaces: RwLock::new(HashMap::new()),
            global_server_names,
        }
    }

    /// Acquire the workspace MCP client for the given workspace path.
    ///
    /// On the first call for a workspace:
    /// 1. Loads `.mcp.json` (or `.astralis/mcp.json`)
    /// 2. Rejects server names that collide with global servers
    /// 3. Creates an `McpClient` and connects auto-start servers
    ///
    /// Subsequent calls increment the ref count and return a clone of the
    /// existing client.
    ///
    /// Returns `Ok(None)` if the workspace has no `.mcp.json`.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file is malformed, oversized, or
    /// server startup fails.
    pub async fn acquire(
        &self,
        workspace: &Path,
    ) -> Result<Option<McpClient>, astralis_mcp::McpError> {
        let key = workspace.to_path_buf();

        // Fast path: already loaded — increment ref count.
        {
            let mut map = self.workspaces.write().await;
            if let Some(state) = map.get_mut(&key) {
                state.ref_count += 1;
                info!(
                    workspace = %workspace.display(),
                    ref_count = state.ref_count,
                    "Workspace MCP client re-acquired"
                );
                return Ok(Some(state.client.clone()));
            }
        }

        // Slow path: load config from disk.
        let Some(mut ws_config) = ServersConfig::load_workspace_mcp(workspace)? else {
            return Ok(None);
        };

        // Reject server names that collide with global servers.
        let mut rejected = Vec::new();
        for name in ws_config.server_names() {
            if self.global_server_names.contains(&name) {
                warn!(
                    server = %name,
                    workspace = %workspace.display(),
                    "Workspace MCP server name collides with global server — skipping"
                );
                rejected.push(name);
            }
        }
        for name in &rejected {
            ws_config.remove(name);
        }

        if ws_config.servers.is_empty() {
            return Ok(None);
        }

        let server_names = ws_config.server_names();

        let client = McpClient::with_config(ws_config);
        match client.connect_auto_servers().await {
            Ok(n) => info!(
                workspace = %workspace.display(),
                count = n,
                "Connected workspace MCP servers"
            ),
            Err(e) => warn!(
                workspace = %workspace.display(),
                error = %e,
                "Error connecting workspace MCP servers"
            ),
        }

        let client_clone = client.clone();

        {
            let mut map = self.workspaces.write().await;
            map.insert(
                key,
                WorkspaceMcpState {
                    client,
                    ref_count: 1,
                    server_names,
                },
            );
        }

        Ok(Some(client_clone))
    }

    /// Release a workspace MCP client reference.
    ///
    /// Decrements the ref count. When it reaches zero, shuts down the
    /// workspace's MCP servers and removes the entry.
    pub async fn release(&self, workspace: &Path) {
        let key = workspace.to_path_buf();
        let mut map = self.workspaces.write().await;

        let should_remove = if let Some(state) = map.get_mut(&key) {
            state.ref_count = state.ref_count.saturating_sub(1);
            info!(
                workspace = %workspace.display(),
                ref_count = state.ref_count,
                "Workspace MCP client released"
            );
            state.ref_count == 0
        } else {
            false
        };

        if should_remove && let Some(state) = map.remove(&key) {
            if let Err(e) = state.client.shutdown().await {
                warn!(
                    workspace = %workspace.display(),
                    error = %e,
                    "Error shutting down workspace MCP servers"
                );
            }
            info!(
                workspace = %workspace.display(),
                servers = ?state.server_names,
                "Workspace MCP servers shut down"
            );
        }
    }

    /// Shut down all workspace MCP servers (daemon shutdown).
    pub async fn shutdown_all(&self) {
        let mut map = self.workspaces.write().await;
        for (workspace, state) in map.drain() {
            if let Err(e) = state.client.shutdown().await {
                warn!(
                    workspace = %workspace.display(),
                    error = %e,
                    "Error shutting down workspace MCP servers during global shutdown"
                );
            }
        }
        info!("All workspace MCP servers shut down");
    }
}

impl std::fmt::Debug for WorkspaceMcpRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceMcpRegistry")
            .field("global_server_names", &self.global_server_names)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_no_mcp_json() {
        let dir = tempfile::tempdir().unwrap();
        let registry = WorkspaceMcpRegistry::new(HashSet::new());

        let result = registry.acquire(dir.path()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_acquire_with_mcp_json() {
        let dir = tempfile::tempdir().unwrap();
        let mcp_json = r#"{
            "mcpServers": {
                "test-server": {
                    "command": "echo",
                    "args": ["hello"]
                }
            }
        }"#;
        std::fs::write(dir.path().join(".mcp.json"), mcp_json).unwrap();

        let registry = WorkspaceMcpRegistry::new(HashSet::new());
        let client = registry.acquire(dir.path()).await.unwrap();
        assert!(client.is_some());

        // Second acquire should return the same client (ref count = 2).
        let client2 = registry.acquire(dir.path()).await.unwrap();
        assert!(client2.is_some());

        // Verify ref count is 2.
        let map = registry.workspaces.read().await;
        assert_eq!(map[&dir.path().to_path_buf()].ref_count, 2);
    }

    #[tokio::test]
    async fn test_collision_detection() {
        let dir = tempfile::tempdir().unwrap();
        let mcp_json = r#"{
            "mcpServers": {
                "global-server": { "command": "echo" },
                "ws-only": { "command": "echo" }
            }
        }"#;
        std::fs::write(dir.path().join(".mcp.json"), mcp_json).unwrap();

        let globals: HashSet<String> = ["global-server".to_string()].into_iter().collect();
        let registry = WorkspaceMcpRegistry::new(globals);

        let client = registry.acquire(dir.path()).await.unwrap();
        assert!(client.is_some());

        // Only ws-only should be present (global-server was rejected).
        let map = registry.workspaces.read().await;
        let state = &map[&dir.path().to_path_buf()];
        assert_eq!(state.server_names, vec!["ws-only"]);
    }

    #[tokio::test]
    async fn test_all_collisions_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mcp_json = r#"{
            "mcpServers": {
                "global-only": { "command": "echo" }
            }
        }"#;
        std::fs::write(dir.path().join(".mcp.json"), mcp_json).unwrap();

        let globals: HashSet<String> = ["global-only".to_string()].into_iter().collect();
        let registry = WorkspaceMcpRegistry::new(globals);

        let client = registry.acquire(dir.path()).await.unwrap();
        assert!(client.is_none());
    }

    #[tokio::test]
    async fn test_release_decrements() {
        let dir = tempfile::tempdir().unwrap();
        let mcp_json = r#"{ "mcpServers": { "s": { "command": "echo" } } }"#;
        std::fs::write(dir.path().join(".mcp.json"), mcp_json).unwrap();

        let registry = WorkspaceMcpRegistry::new(HashSet::new());
        let _ = registry.acquire(dir.path()).await.unwrap();
        let _ = registry.acquire(dir.path()).await.unwrap();

        // Release once — ref count should go to 1.
        registry.release(dir.path()).await;
        {
            let map = registry.workspaces.read().await;
            assert_eq!(map[&dir.path().to_path_buf()].ref_count, 1);
        }

        // Release again — entry should be removed.
        registry.release(dir.path()).await;
        {
            let map = registry.workspaces.read().await;
            assert!(!map.contains_key(&dir.path().to_path_buf()));
        }
    }

    #[tokio::test]
    async fn test_shutdown_all() {
        let dir = tempfile::tempdir().unwrap();
        let mcp_json = r#"{ "mcpServers": { "s": { "command": "echo" } } }"#;
        std::fs::write(dir.path().join(".mcp.json"), mcp_json).unwrap();

        let registry = WorkspaceMcpRegistry::new(HashSet::new());
        let _ = registry.acquire(dir.path()).await.unwrap();

        registry.shutdown_all().await;

        let map = registry.workspaces.read().await;
        assert!(map.is_empty());
    }
}
