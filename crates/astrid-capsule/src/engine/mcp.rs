use std::path::PathBuf;
use async_trait::async_trait;
use tracing::info;
use tokio::process::{Child, Command};

use crate::error::{CapsuleResult, CapsuleError};
use crate::manifest::{CapsuleManifest, McpServerDef};
use super::ExecutionEngine;

/// Executes Legacy Host MCP servers via `stdio`.
///
/// This engine requires the `host_process` capability. It securely spawns
/// the host command (e.g. `npx` or `python`) and manages its stdio pipes,
/// forwarding the JSON-RPC traffic to the Astrid IPC bus.
pub struct McpHostEngine {
    manifest: CapsuleManifest,
    server_def: McpServerDef,
    capsule_dir: PathBuf,
    process: Option<Child>,
}

impl McpHostEngine {
    pub fn new(manifest: CapsuleManifest, server_def: McpServerDef, capsule_dir: PathBuf) -> Self {
        Self {
            manifest,
            server_def,
            capsule_dir,
            process: None,
        }
    }
}

#[async_trait]
impl ExecutionEngine for McpHostEngine {
    async fn load(&mut self) -> CapsuleResult<()> {
        // Build the command from the manifest definition
        let command_str = self.server_def.command.as_ref().ok_or_else(|| {
            CapsuleError::UnsupportedEntryPoint("MCP server requires a 'command' field".into())
        })?;

        info!(
            capsule = %self.manifest.package.name,
            command = %command_str,
            "Spawning legacy MCP host process (Airlock Override)"
        );

        let mut cmd = Command::new(command_str);
        cmd.args(&self.server_def.args);
        cmd.current_dir(&self.capsule_dir);

        // TODO: In a full implementation, pipe stdin/stdout and connect to astrid-mcp.
        // For Phase 4 scaffolding, we just prove the process starts.

        let child = cmd.spawn().map_err(|e| {
            CapsuleError::UnsupportedEntryPoint(format!("Failed to spawn host process: {}", e))
        })?;

        self.process = Some(child);
        Ok(())
    }

    async fn unload(&mut self) -> CapsuleResult<()> {
        if let Some(mut child) = self.process.take() {
            info!(
                capsule = %self.manifest.package.name,
                "Shutting down MCP host process"
            );
            // Send SIGTERM / kill the child process gracefully
            let _ = child.kill().await;
        }
        Ok(())
    }
}