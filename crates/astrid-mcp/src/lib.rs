//! Astrid MCP - MCP client with server lifecycle management.
//!
//! This crate provides:
//! - MCP server configuration and lifecycle management
//! - MCP client for tool calling
//! - Secure client with capability-based authorization
//!
//! # Architecture
//!
//! The MCP layer wraps the official `rmcp` SDK with:
//! - Server configuration from TOML files
//! - Process lifecycle management (start/stop/restart)
//! - Binary hash verification before execution
//! - Integration with capability-based authorization
//!
//! # Example
//!
//! ```rust,no_run
//! use astrid_mcp::{McpClient, ServersConfig, ServerConfig};
//!
//! # async fn example() -> Result<(), astrid_mcp::McpError> {
//! // Create configuration
//! let mut config = ServersConfig::default();
//! config.add(
//!     ServerConfig::stdio("filesystem", "npx")
//!         .with_args(["-y", "@anthropics/mcp-server-filesystem", "/tmp"])
//!         .auto_start()
//! );
//!
//! // Create client
//! let client = McpClient::with_config(config);
//!
//! // Connect to server
//! client.connect("filesystem").await?;
//!
//! // List available tools
//! let tools = client.list_tools().await?;
//! for tool in tools {
//!     println!("Tool: {}:{}", tool.server, tool.name);
//! }
//!
//! // Call a tool
//! let result = client.call_tool(
//!     "filesystem",
//!     "read_file",
//!     serde_json::json!({"path": "/tmp/test.txt"})
//! ).await?;
//!
//! println!("Result: {}", result.text_content());
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]

pub mod prelude;

mod client;
mod config;
mod error;
mod secure;
mod server;
mod types;

// Nov 2025 MCP spec modules
pub mod capabilities;
pub mod rate_limit;
pub mod tasks;
pub mod verification;

pub use client::McpClient;
pub use config::{RestartPolicy, ServerConfig, ServersConfig, Transport};
pub use error::{McpError, McpResult};
pub use secure::{SecureMcpClient, ToolAuthorization};
pub use server::{McpServerStatus, ServerManager};
pub use types::{
    PromptArgument, PromptContent, PromptDefinition, PromptMessage, ResourceContent,
    ResourceDefinition, ServerCapabilities, ServerInfo, ToolContent, ToolDefinition, ToolResult,
};

// Re-exports from new modules
pub use capabilities::{
    AstridClientHandler, BridgeChannelCapabilities, BridgeChannelDefinition, BridgeChannelInfo,
    CapabilitiesHandler, ElicitationHandler, RootsHandler, RootsRequest, RootsResponse,
    SamplingHandler, SamplingRequest, SamplingResponse, ServerNotice, UrlElicitationHandler,
};

// Re-export canonical elicitation types from astrid-core for convenience.
// These are the single source of truth — no duplicates in astrid-mcp.
pub use astrid_core::{
    ElicitationRequest, ElicitationResponse, ElicitationSchema, UrlElicitationRequest,
    UrlElicitationResponse, UrlElicitationType,
};
pub use rate_limit::{PendingGuard, RateLimit, RateLimitResult, RateLimiter, RateLimits};
pub use tasks::{Task, TaskManager, TaskState};
pub use verification::{
    BinaryVerifier, VerificationResult, verify_binary_hash, verify_command_hash,
};
