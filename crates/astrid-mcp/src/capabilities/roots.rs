//! Roots capability types and handler trait.
//!
//! Implements the MCP Nov 2025 roots capability: server inquiries about
//! operational boundaries (directories/URIs the client controls).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request for operational boundaries (roots).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootsRequest {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// Server making the request.
    pub server: String,
}

/// Response to a roots request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootsResponse {
    /// Request ID for correlation.
    pub request_id: Uuid,
    /// List of root directories/URIs the server can access.
    pub roots: Vec<Root>,
}

/// A root directory or URI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Root {
    /// URI of the root (e.g., `file:///home/user/project`).
    pub uri: String,
    /// Human-readable name.
    pub name: Option<String>,
}

/// Handler for server inquiries about operational boundaries.
#[async_trait]
pub trait RootsHandler: Send + Sync {
    /// Handle a roots request from a server.
    ///
    /// Returns the list of roots (directories, URIs) that the server
    /// is allowed to access.
    async fn handle_roots(&self, request: RootsRequest) -> RootsResponse;
}
