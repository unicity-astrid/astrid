//! Prelude module - commonly used types for convenient import.
//!
//! Use `use astrid_mcp::prelude::*;` to import all essential types.
//!
//! # Example
//!
//! ```rust,no_run
//! use astrid_mcp::prelude::*;
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
pub use crate::{ServerConfig, ServersConfig};

// Tool types
pub use crate::{ToolContent, ToolDefinition, ToolResult};

// Canonical elicitation types from astrid-core
pub use crate::{
    ElicitationRequest, ElicitationResponse, ElicitationSchema, UrlElicitationRequest,
    UrlElicitationResponse, UrlElicitationType,
};
