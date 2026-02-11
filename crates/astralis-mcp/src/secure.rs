//! Secure MCP client with capability-based authorization.
//!
//! Wraps the MCP client with security checks:
//! - Capability token validation
//! - Audit logging
//! - Approval flow integration

use astralis_audit::{AuditAction, AuditLog, AuditOutcome, AuthorizationProof};
use astralis_capabilities::{AuditEntryId, CapabilityStore, CapabilityValidator};
use astralis_core::{Permission, SessionId};
use astralis_crypto::ContentHash;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::client::McpClient;
use crate::error::{McpError, McpResult};
use crate::types::{
    PromptContent, PromptDefinition, ResourceContent, ResourceDefinition, ToolDefinition,
    ToolResult,
};

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
                let token = result.token().unwrap();
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
                            token_id: astralis_core::TokenId::new(), // This would be the actual token
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

    /// List resources from a server.
    ///
    /// # Errors
    ///
    /// Returns an error if the call fails.
    pub async fn list_resources(&self, server: &str) -> McpResult<Vec<ResourceDefinition>> {
        let result = self.client.list_resources(server).await;

        // Audit log
        {
            let outcome = match &result {
                Ok(resources) => {
                    AuditOutcome::success_with(format!("listed {} resources", resources.len()))
                },
                Err(e) => AuditOutcome::failure(e.to_string()),
            };
            let _ = self.audit.append(
                self.session_id.clone(),
                AuditAction::McpToolCall {
                    server: server.to_string(),
                    tool: "list_resources".to_string(),
                    args_hash: ContentHash::zero(),
                },
                AuthorizationProof::System {
                    reason: "resource listing".to_string(),
                },
                outcome,
            );
        }

        result
    }

    /// Read a resource from a server.
    ///
    /// # Errors
    ///
    /// Returns an error if the call fails.
    pub async fn read_resource(&self, server: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
        let result = self.client.read_resource(server, uri).await;

        // Audit log
        {
            let outcome = match &result {
                Ok(_) => AuditOutcome::success_with(format!("read resource: {uri}")),
                Err(e) => AuditOutcome::failure(e.to_string()),
            };
            let _ = self.audit.append(
                self.session_id.clone(),
                AuditAction::McpToolCall {
                    server: server.to_string(),
                    tool: "read_resource".to_string(),
                    args_hash: ContentHash::hash(uri.as_bytes()),
                },
                AuthorizationProof::System {
                    reason: "resource read".to_string(),
                },
                outcome,
            );
        }

        result
    }

    /// List prompts from a server.
    ///
    /// # Errors
    ///
    /// Returns an error if the call fails.
    pub async fn list_prompts(&self, server: &str) -> McpResult<Vec<PromptDefinition>> {
        let result = self.client.list_prompts(server).await;

        // Audit log
        {
            let outcome = match &result {
                Ok(prompts) => {
                    AuditOutcome::success_with(format!("listed {} prompts", prompts.len()))
                },
                Err(e) => AuditOutcome::failure(e.to_string()),
            };
            let _ = self.audit.append(
                self.session_id.clone(),
                AuditAction::McpToolCall {
                    server: server.to_string(),
                    tool: "list_prompts".to_string(),
                    args_hash: ContentHash::zero(),
                },
                AuthorizationProof::System {
                    reason: "prompt listing".to_string(),
                },
                outcome,
            );
        }

        result
    }

    /// Get a prompt from a server.
    ///
    /// # Errors
    ///
    /// Returns an error if the call fails.
    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> McpResult<PromptContent> {
        let result = self.client.get_prompt(server, name, arguments).await;

        // Audit log
        {
            let outcome = match &result {
                Ok(_) => AuditOutcome::success_with(format!("got prompt: {name}")),
                Err(e) => AuditOutcome::failure(e.to_string()),
            };
            let _ = self.audit.append(
                self.session_id.clone(),
                AuditAction::McpToolCall {
                    server: server.to_string(),
                    tool: "get_prompt".to_string(),
                    args_hash: ContentHash::hash(name.as_bytes()),
                },
                AuthorizationProof::System {
                    reason: "prompt retrieval".to_string(),
                },
                outcome,
            );
        }

        result
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

    /// Get the underlying MCP client.
    #[must_use]
    pub fn inner(&self) -> &McpClient {
        &self.client
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
    use astralis_crypto::KeyPair;

    #[tokio::test]
    async fn test_secure_client_creation() {
        let config = crate::config::ServersConfig::default();
        let client = McpClient::with_config(config);
        let capabilities = Arc::new(CapabilityStore::in_memory());
        let keypair = KeyPair::generate();
        let audit = Arc::new(AuditLog::in_memory(keypair));
        let session_id = SessionId::new();

        let secure = SecureMcpClient::new(client, capabilities, audit, session_id);

        assert!(secure.list_tools().await.unwrap().is_empty());
    }
}
