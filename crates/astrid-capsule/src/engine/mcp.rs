use async_trait::async_trait;
use std::path::PathBuf;
use tracing::{info, warn};

use super::ExecutionEngine;
use crate::context::CapsuleContext;
use crate::error::{CapsuleError, CapsuleResult};
use crate::manifest::{CapsuleManifest, McpServerDef};

use astrid_mcp::SecureMcpClient;

/// Executes Legacy Host MCP servers via `stdio`.
///
/// This engine requires the `host_process` capability. It securely spawns
/// the host command (e.g. `npx` or `python`) and manages its stdio pipes,
/// forwarding the JSON-RPC traffic to the Astrid IPC bus.
///
/// Server lifecycle ops (`connect_dynamic`, `disconnect`) go through
/// `SecureMcpClient` so they are audit-logged. Internal interceptor hook
/// calls use the bare inner `McpClient` with `AuthorizationProof::System`
/// semantics - they are kernel infrastructure, not user-facing tool calls.
pub struct McpHostEngine {
    manifest: CapsuleManifest,
    server_def: McpServerDef,
    capsule_dir: PathBuf,
    mcp_client: SecureMcpClient,
}

impl McpHostEngine {
    /// Create a new MCP host engine.
    pub fn new(
        manifest: CapsuleManifest,
        server_def: McpServerDef,
        capsule_dir: PathBuf,
        mcp_client: SecureMcpClient,
    ) -> Self {
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
    async fn load(&mut self, ctx: &CapsuleContext) -> CapsuleResult<()> {
        let original_command_str = self
            .server_def
            .command
            .as_ref()
            .ok_or_else(|| {
                CapsuleError::UnsupportedEntryPoint("MCP server requires a 'command' field".into())
            })?
            .clone();

        // 1. Explicitly verify if the host_process capability was granted in the manifest.
        // We check against the *original* command name *before* any path resolution occurs.
        // This prevents malicious capsules from bypassing checks by naming directories
        // with substrings of allowed commands (e.g., `./bin/npx-compat/`).
        let is_granted = self.manifest.capabilities.host_process.iter().any(|cmd| {
            original_command_str == *cmd || original_command_str.starts_with(&format!("{cmd} "))
        });

        if !is_granted {
            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                "Security Check Failed: host_process capability for '{}' was not declared in the manifest.",
                original_command_str
            )));
        }

        let mut command_str = original_command_str.clone();

        // 2. Fat Binary Resolution:
        // If the command is a relative path (e.g. "./bin/my-tool") that exists locally within
        // the capsule directory, check if it's a directory. If it is, append the host's target triple.
        let local_cmd_path = self.capsule_dir.join(&command_str);

        // Prevent path traversal outside the capsule directory
        if let Ok(canonical_cmd) = local_cmd_path.canonicalize()
            && let Ok(canonical_capsule_dir) = self.capsule_dir.canonicalize()
        {
            if !canonical_cmd.starts_with(&canonical_capsule_dir) {
                return Err(CapsuleError::UnsupportedEntryPoint(format!(
                    "Path traversal detected: command '{}' escapes the capsule directory.",
                    command_str
                )));
            }

            if canonical_cmd.is_dir() {
                let host_triple = env!("TARGET"); // Injected by cargo at build time
                let arch_slice = canonical_cmd.join(host_triple);

                // Ensure it is a regular file and canonicalize to check symlinks don't escape
                if arch_slice.is_file() {
                    if let Ok(canon_slice) = arch_slice.canonicalize() {
                        if !canon_slice.starts_with(&canonical_capsule_dir) {
                            return Err(CapsuleError::UnsupportedEntryPoint(format!(
                                "Fat binary slice '{}' resolves outside the capsule boundary.",
                                host_triple
                            )));
                        }
                        info!(
                            "Fat binary resolved: using {} slice for {}",
                            host_triple, command_str
                        );
                        command_str = canon_slice.to_string_lossy().to_string();
                    } else {
                        return Err(CapsuleError::UnsupportedEntryPoint(format!(
                            "Failed to resolve fat binary slice for the current architecture: {}",
                            host_triple
                        )));
                    }
                } else {
                    return Err(CapsuleError::UnsupportedEntryPoint(format!(
                        "Fat binary directory '{}' does not contain a valid slice for the current architecture: {}",
                        command_str, host_triple
                    )));
                }
            } else if canonical_cmd.is_file() {
                // It's a local file, just use the absolute path directly to be safe
                command_str = canonical_cmd.to_string_lossy().to_string();
            }
        }

        info!(
            capsule = %self.manifest.package.name,
            original_command = %original_command_str,
            resolved_command = %command_str,
            "Registering legacy MCP host process dynamically (Airlock Override)"
        );

        // Resolve [env] from KV store / defaults / onboarding before spawning.
        let resolved_env = super::resolve_env(&self.manifest, ctx, &[], "mcp_host_engine").await?;

        let server_id = format!("capsule:{}", self.manifest.package.name);

        let config = astrid_mcp::ServerConfig {
            name: server_id.clone(),
            command: Some(command_str),
            args: self.server_def.args.clone(),
            env: resolved_env,
            cwd: Some(self.capsule_dir.clone()),
            restart_policy: astrid_mcp::RestartPolicy::Always, // Host engines should restart on crash
            ..Default::default()
        };

        // We use the `astrid-mcp` dynamic connection feature to spawn the `Command`
        // and attach its `stdio` directly to the `McpClient`.
        self.mcp_client
            .connect_dynamic(&server_id, config)
            .await
            .map_err(|e| {
                CapsuleError::UnsupportedEntryPoint(format!(
                    "Failed to connect MCP host engine: {e}"
                ))
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

    fn check_health(&self) -> crate::capsule::CapsuleState {
        let server_id = format!("capsule:{}", self.manifest.package.name);
        // Safety: `block_in_place` is safe here because the health monitor
        // calls `check_health()` from a multi-threaded tokio runtime context
        // (the kernel task), not from within a `LocalSet` or single-threaded
        // runtime. The inner future is a short lock check, not a long I/O op.
        let is_running = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { self.mcp_client.inner().is_server_running(&server_id).await })
        });
        if is_running {
            crate::capsule::CapsuleState::Ready
        } else {
            crate::capsule::CapsuleState::Failed(format!(
                "MCP server '{server_id}' is no longer running"
            ))
        }
    }

    fn invoke_interceptor(&self, action: &str, payload: &[u8]) -> CapsuleResult<Vec<u8>> {
        let server_id = format!("capsule:{}", self.manifest.package.name);

        let params: serde_json::Value = serde_json::from_slice(payload).map_err(|e| {
            CapsuleError::ExecutionFailed(format!("failed to deserialize interceptor payload: {e}"))
        })?;

        // Use call_tool with the `astrid_hook_intercept` tool so we get
        // a response back (request-response interceptor pattern).
        let tool_args = serde_json::json!({
            "hook": action,
            "payload": params,
        });

        // Interceptor hooks are kernel-internal infrastructure, not
        // user-facing tool calls. Use the bare inner McpClient directly -
        // no capability check needed for the kernel invoking its own hooks.
        let client = self.mcp_client.inner().clone();

        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                client
                    .call_tool(&server_id, "astrid_hook_intercept", tool_args)
                    .await
            })
        });

        match result {
            Ok(tool_result) => {
                // Extract text content from the MCP ToolResult
                let text = tool_result
                    .content
                    .iter()
                    .filter_map(|c| {
                        if let astrid_mcp::ToolContent::Text { text } = c {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("");

                if text.is_empty() || text == "null" {
                    Ok(Vec::new())
                } else {
                    Ok(text.into_bytes())
                }
            },
            Err(e) => {
                warn!(
                    capsule = %self.manifest.package.name,
                    hook = %action,
                    error = %e,
                    "Failed to invoke hook interceptor on MCP capsule"
                );
                Ok(Vec::new())
            },
        }
    }
}
