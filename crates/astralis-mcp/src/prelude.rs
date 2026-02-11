//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astralis_mcp::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,no_run
//! use astralis_mcp::prelude::*;
//!
//! # async fn example() -> McpResult<()> {
//! // Create configuration
//! let mut config = ServersConfig::default();
//! config.add(
//!     ServerConfig::stdio("filesystem", "npx")
//!         .with_args(["-y", "@anthropics/mcp-server-filesystem", "/tmp"])
//!         .auto_start()
//! );
//!
//! // Create client and connect
//! let client = McpClient::with_config(config);
//! client.connect("filesystem").await?;
//!
//! // List tools
//! let tools = client.list_tools().await?;
//! for tool in tools {
//!     println!("Tool: {}:{}", tool.server, tool.name);
//! }
//! # Ok(())
//! # }
//! ```

// Errors
pub use crate::{McpError, McpResult};

// Client types
pub use crate::ServerManager;
pub use crate::{McpClient, SecureMcpClient, ToolAuthorization};
pub use crate::{ServerConfig, ServersConfig, Transport};

// Tool types
pub use crate::{ToolContent, ToolDefinition, ToolResult};

// Resource and prompt definitions
pub use crate::{
    PromptArgument, PromptContent, PromptDefinition, PromptMessage, ResourceContent,
    ResourceDefinition,
};

// Server capabilities and info
pub use crate::{ServerCapabilities, ServerInfo};

// Nov 2025 MCP spec handlers and canonical elicitation types
pub use crate::{
    CapabilitiesHandler, ElicitationHandler, ElicitationRequest, ElicitationResponse,
    ElicitationSchema, RootsHandler, RootsRequest, RootsResponse, SamplingHandler, SamplingRequest,
    SamplingResponse, UrlElicitationHandler, UrlElicitationRequest, UrlElicitationResponse,
    UrlElicitationType,
};

// Rate limiting
pub use crate::{PendingGuard, RateLimit, RateLimitResult, RateLimiter, RateLimits};

// Tasks
pub use crate::{Task, TaskManager, TaskState};

// Verification
pub use crate::{BinaryVerifier, VerificationResult, verify_binary_hash, verify_command_hash};
