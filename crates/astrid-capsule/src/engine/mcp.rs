use async_trait::async_trait;
use std::path::PathBuf;
use tracing::info;

use super::ExecutionEngine;
use crate::context::CapsuleContext;
use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::{CapsuleManifest, McpServerDef};

use astrid_mcp::McpClient;

/// Executes Legacy Host MCP servers via `stdio`.
///
/// This engine requires the `host_process` capability. It securely spawns
/// the host command (e.g. `npx` or `python`) and manages its stdio pipes,
/// forwarding the JSON-RPC traffic to the Astrid IPC bus.
pub struct McpHostEngine {
    manifest: CapsuleManifest,
    server_def: McpServerDef,
    capsule_dir: PathBuf,
    mcp_client: McpClient,
}

impl McpHostEngine {
    pub fn new(manifest: CapsuleManifest, server_def: McpServerDef, capsule_dir: PathBuf, mcp_client: McpClient) -> Self {
        Self {
            manifest,
            server_def,
            capsule_dir,
            mcp_client,
        }
    }
}

#[async_trait]
impl ExecutionEngine for McpHostEngine {
    async fn load(&mut self, _ctx: &CapsuleContext) -> CapsuleResult<()> {
        let command_str = self.server_def.command.as_ref().ok_or_else(|| {
            CapsuleError::UnsupportedEntryPoint("MCP server requires a 'command' field".into())
        })?;

        info!(
            capsule = %self.manifest.package.name,
            command = %command_str,
            "Registering legacy MCP host process dynamically (Airlock Override)"
        );

        let server_id = format!("capsule:{}", self.manifest.package.name);

        let config = astrid_mcp::ServerConfig {
            name: server_id.clone(),
            command: Some(command_str.clone()),
            args: self.server_def.args.clone(),
            env: std::collections::HashMap::new(), // In Phase 6/7, inject [env] vars here
            cwd: Some(self.capsule_dir.clone()),
            restart_policy: astrid_mcp::RestartPolicy::Always, // Host engines should restart on crash
            ..Default::default()
        };

        // We use the `astrid-mcp` dynamic connection feature to spawn the `Command`
        // and attach its `stdio` directly to the `McpClient`.
        self.mcp_client.connect_dynamic(&server_id, config).await.map_err(|e| {
            CapsuleError::UnsupportedEntryPoint(format!("Failed to connect MCP host engine: {e}"))
        })?;

        Ok(())
    }

    async fn unload(&mut self) -> CapsuleResult<()> {
        info!(
            capsule = %self.manifest.package.name,
            "Shutting down MCP host process"
        );
        let server_id = format!("capsule:{}", self.manifest.package.name);
        
        let _ = self.mcp_client.disconnect(&server_id).await;
        
        // Let astrid-mcp drop the Child process and `Stdio` streams.
        Ok(())
    }
}
