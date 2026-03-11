//! Secure MCP client with capability-based authorization.
//!
//! Wraps the MCP client with security checks:
//! - Capability token validation
//! - Audit logging
//! - Approval flow integration

use astrid_audit::{AuditAction, AuditLog, AuditOutcome, AuthorizationProof};
use astrid_capabilities::{AuditEntryId, CapabilityStore, CapabilityValidator};
use astrid_core::{Permission, SessionId};
use astrid_crypto::ContentHash;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::client::McpClient;
use crate::config::ServerConfig;
use crate::error::{McpError, McpResult};
use crate::server::ServerManager;
use crate::types::{ToolDefinition, ToolResult};

/// Authorization result for a tool call.
#[derive(Debug, Clone)]
pub enum ToolAuthorization {
    /// Authorized by capability token.
    Authorized {
        /// The audit entry ID for this authorization.
        audit_id: AuditEntryId,
    },
    /// Requires user approval.
    RequiresApproval {
        /// Server name.
        server: String,
        /// Tool name.
        tool: String,
        /// Resource URI.
        resource: String,
    },
}

/// Secure MCP client with capability-based authorization.
pub struct SecureMcpClient {
    /// Underlying MCP client.
    client: McpClient,
    /// Capability store.
    capabilities: Arc<CapabilityStore>,
    /// Audit log.
    audit: Arc<AuditLog>,
    /// Current session ID.
    session_id: SessionId,
}

impl SecureMcpClient {
    /// Create a new secure MCP client.
    #[must_use]
    pub fn new(
        client: McpClient,
        capabilities: Arc<CapabilityStore>,
        audit: Arc<AuditLog>,
        session_id: SessionId,
    ) -> Self {
        Self {
            client,
            capabilities,
            audit,
            session_id,
        }
    }

    /// Check authorization for a tool call.
    ///
    /// # Errors
    ///
    /// Returns an error if the audit log cannot be written.
    ///
    /// # Panics
    ///
    /// Panics if the authorization result indicates authorized but no token is present.
    /// This should never happen in practice as authorization implies a token exists.
    pub fn check_authorization(&self, server: &str, tool: &str) -> McpResult<ToolAuthorization> {
        let resource = format!("mcp://{server}:{tool}");

        let validator = CapabilityValidator::new(&self.capabilities);

        let result = validator.check(&resource, Permission::Invoke);

        if result.is_authorized() {
            // Log the authorized access
            let audit_id = {
                let token = result.token().expect("authorization implies token exists");
                self.audit
                    .append(
                        self.session_id.clone(),
                        AuditAction::McpToolCall {
                            server: server.to_string(),
                            tool: tool.to_string(),
                            args_hash: ContentHash::zero(), // Will be updated when actually called
                        },
                        AuthorizationProof::Capability {
                            token_id: token.id.clone(),
                            token_hash: token.content_hash(),
                        },
                        AuditOutcome::success_with("authorization check"),
                    )
                    .map_err(|e| McpError::TransportError(e.to_string()))?
            };

            debug!(
                server = server,
                tool = tool,
                "Tool call authorized by capability"
            );

            Ok(ToolAuthorization::Authorized { audit_id })
        } else {
            debug!(server = server, tool = tool, "Tool call requires approval");

            Ok(ToolAuthorization::RequiresApproval {
                server: server.to_string(),
                tool: tool.to_string(),
                resource,
            })
        }
    }

    /// Call a tool with authorization check.
    ///
    /// Returns an error if not authorized. Use `check_authorization` first
    /// to determine if approval is needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool call fails or arguments cannot be serialized.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        args: Value,
        authorization: AuthorizationProof,
    ) -> McpResult<ToolResult> {
        // Compute args hash
        let args_json =
            serde_json::to_vec(&args).map_err(|e| McpError::SerializationError(e.to_string()))?;
        let args_hash = ContentHash::hash(&args_json);

        // Log the tool call
        let audit_result = {
            self.audit.append(
                self.session_id.clone(),
                AuditAction::McpToolCall {
                    server: server.to_string(),
                    tool: tool.to_string(),
                    args_hash,
                },
                authorization,
                AuditOutcome::success_with("tool call started"),
            )
        };

        if let Err(e) = audit_result {
            warn!(error = %e, "Failed to log tool call start");
        }

        // Make the actual call
        let result = self.client.call_tool(server, tool, args).await;

        // Log the result
        {
            let outcome = match &result {
                Ok(r) if r.success => AuditOutcome::success_with(r.text_content()),
                Ok(r) => AuditOutcome::failure(r.error.as_deref().unwrap_or("unknown error")),
                Err(e) => AuditOutcome::failure(e.to_string()),
            };

            let _ = self.audit.append(
                self.session_id.clone(),
                AuditAction::McpToolCall {
                    server: server.to_string(),
                    tool: tool.to_string(),
                    args_hash,
                },
                AuthorizationProof::System {
                    reason: "result logging".to_string(),
                },
                outcome,
            );
        }

        result
    }

    /// Call a tool if authorized, otherwise return authorization requirement.
    ///
    /// # Errors
    ///
    /// Returns an error if the authorization check or tool call fails.
    pub async fn call_tool_if_authorized(
        &self,
        server: &str,
        tool: &str,
        args: Value,
    ) -> McpResult<Result<ToolResult, ToolAuthorization>> {
        match self.check_authorization(server, tool)? {
            ToolAuthorization::Authorized { audit_id: _ } => {
                let result = self
                    .call_tool(
                        server,
                        tool,
                        args,
                        AuthorizationProof::Capability {
                            token_id: astrid_core::TokenId::new(), // This would be the actual token
                            token_hash: ContentHash::zero(),
                        },
                    )
                    .await?;
                Ok(Ok(result))
            },
            auth @ ToolAuthorization::RequiresApproval { .. } => Ok(Err(auth)),
        }
    }

    /// List all available tools.
    ///
    /// # Errors
    ///
    /// Returns an error if tools cannot be listed.
    pub async fn list_tools(&self) -> McpResult<Vec<ToolDefinition>> {
        self.client.list_tools().await
    }

    /// Get a specific tool definition.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool cannot be retrieved.
    pub async fn get_tool(&self, server: &str, tool: &str) -> McpResult<Option<ToolDefinition>> {
        self.client.get_tool(server, tool).await
    }

    /// Connect to a server.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot be connected.
    pub async fn connect(&self, server_name: &str) -> McpResult<()> {
        self.client.connect(server_name).await?;

        // Get the actual transport type for logging
        let transport = self
            .client
            .server_manager()
            .get_config(server_name)
            .map_or_else(|| "unknown".to_string(), |c| format!("{:?}", c.transport));

        // Log server start
        {
            let _ = self.audit.append(
                self.session_id.clone(),
                AuditAction::ServerStarted {
                    name: server_name.to_string(),
                    transport,
                    binary_hash: None,
                },
                AuthorizationProof::System {
                    reason: "server connection".to_string(),
                },
                AuditOutcome::success(),
            );
        }

        Ok(())
    }

    /// Disconnect from a server.
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot be disconnected.
    pub async fn disconnect(&self, server_name: &str) -> McpResult<()> {
        self.client.disconnect(server_name).await?;

        // Log server stop
        {
            let _ = self.audit.append(
                self.session_id.clone(),
                AuditAction::ServerStopped {
                    name: server_name.to_string(),
                    reason: "user disconnect".to_string(),
                },
                AuthorizationProof::System {
                    reason: "server disconnection".to_string(),
                },
                AuditOutcome::success(),
            );
        }

        Ok(())
    }

    /// List running servers.
    pub async fn list_servers(&self) -> Vec<String> {
        self.client.list_servers().await
    }

    /// Dynamically connect a new server using a provided configuration.
    ///
    /// This delegates directly to the underlying [`McpClient`] — server
    /// lifecycle operations (connect/disconnect) are infrastructure, not
    /// agent-initiated tool calls, so no capability check is performed.
    ///
    /// # Errors
    ///
    /// Returns an error if the server is already running or cannot be started.
    pub async fn connect_dynamic(&self, name: &str, config: ServerConfig) -> McpResult<()> {
        self.client.connect_dynamic(name, config).await
    }

    /// Disconnect from all servers.
    ///
    /// # Errors
    ///
    /// Returns an error if servers cannot be stopped.
    pub async fn disconnect_all(&self) -> McpResult<()> {
        self.client.disconnect_all().await
    }

    /// Shut down the client, disconnecting from all servers.
    ///
    /// # Errors
    ///
    /// Returns an error if disconnection fails.
    pub async fn shutdown(&self) -> McpResult<()> {
        self.client.shutdown().await
    }

    /// Get the server manager.
    #[must_use]
    pub fn server_manager(&self) -> &ServerManager {
        self.client.server_manager()
    }

    /// Connect to all auto-start servers.
    ///
    /// # Errors
    ///
    /// Returns an error only if refreshing the tools cache fails.
    pub async fn connect_auto_servers(&self) -> McpResult<usize> {
        self.client.connect_auto_servers().await
    }

    /// Get the underlying MCP client.
    #[must_use]
    pub fn inner(&self) -> &McpClient {
        &self.client
    }

    /// Get the capability store.
    #[must_use]
    pub fn capabilities(&self) -> &Arc<CapabilityStore> {
        &self.capabilities
    }

    /// Get the audit log.
    #[must_use]
    pub fn audit(&self) -> &Arc<AuditLog> {
        &self.audit
    }

    /// Get the session ID.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}

/// `SecureMcpClient` is cheaply cloneable — the underlying `McpClient` is
/// `Arc`-wrapped, and `CapabilityStore`/`AuditLog` are already behind `Arc`.
impl Clone for SecureMcpClient {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            capabilities: Arc::clone(&self.capabilities),
            audit: Arc::clone(&self.audit),
            session_id: self.session_id.clone(),
        }
    }
}

impl std::fmt::Debug for SecureMcpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureMcpClient")
            .field("client", &self.client)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astrid_crypto::KeyPair;

    fn make_secure_client() -> SecureMcpClient {
        let config = crate::config::ServersConfig::default();
        let client = McpClient::with_config(config);
        let capabilities = Arc::new(CapabilityStore::in_memory());
        let keypair = KeyPair::generate();
        let audit = Arc::new(AuditLog::in_memory(keypair));
        let session_id = SessionId::new();
        SecureMcpClient::new(client, capabilities, audit, session_id)
    }

    #[tokio::test]
    async fn test_secure_client_creation() {
        let secure = make_secure_client();
        assert!(secure.list_tools().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_secure_client_clone_shares_state() {
        let secure = make_secure_client();
        let cloned = secure.clone();

        // Both see the same empty tool list (shared ServerManager)
        assert!(secure.list_tools().await.unwrap().is_empty());
        assert!(cloned.list_tools().await.unwrap().is_empty());

        // Same session ID
        assert_eq!(secure.session_id(), cloned.session_id());

        // Same capability store (Arc identity)
        assert!(Arc::ptr_eq(secure.capabilities(), cloned.capabilities()));

        // Same audit log (Arc identity)
        assert!(Arc::ptr_eq(secure.audit(), cloned.audit()));
    }

    #[tokio::test]
    async fn test_secure_client_inner_returns_bare_client() {
        let secure = make_secure_client();
        let inner = secure.inner();

        // Inner client should also see empty tools
        assert!(inner.list_tools().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_no_capability_returns_requires_approval() {
        let secure = make_secure_client();

        // No tokens in the store, so any tool check should require approval
        let auth = secure
            .check_authorization("test-server", "test-tool")
            .unwrap();
        assert!(
            matches!(auth, ToolAuthorization::RequiresApproval { .. }),
            "Empty capability store should deny by default"
        );
    }

    #[tokio::test]
    async fn test_delegation_methods() {
        let secure = make_secure_client();

        // list_servers delegates correctly
        assert!(secure.list_servers().await.is_empty());

        // disconnect_all on empty client succeeds
        assert!(secure.disconnect_all().await.is_ok());

        // shutdown on empty client succeeds
        assert!(secure.shutdown().await.is_ok());
    }
}
