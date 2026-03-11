//! Test helpers for constructing [`SecureMcpClient`] in test contexts.
//!
//! Gated behind the `test-support` feature. Add to your `dev-dependencies`:
//!
//! ```toml
//! astrid-mcp = { workspace = true, features = ["test-support"] }
//! ```

use std::sync::Arc;

use astrid_audit::AuditLog;
use astrid_capabilities::CapabilityStore;
use astrid_core::SessionId;
use astrid_crypto::KeyPair;

use crate::client::McpClient;
use crate::config::ServersConfig;
use crate::secure::SecureMcpClient;

/// Create a [`SecureMcpClient`] backed by in-memory stores with an empty
/// server configuration. Suitable for unit and integration tests that need
/// a valid `SecureMcpClient` but do not connect to real MCP servers.
#[must_use]
pub fn test_secure_mcp_client() -> SecureMcpClient {
    let client = McpClient::with_config(ServersConfig::default());
    let capabilities = Arc::new(CapabilityStore::in_memory());
    let audit = Arc::new(AuditLog::in_memory(KeyPair::generate()));
    SecureMcpClient::new(client, capabilities, audit, SessionId::new())
}
