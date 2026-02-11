# astralis-mcp

MCP client with server lifecycle management for Astralis.

This crate wraps the official `rmcp` SDK with secure server management,
capability-based authorization, and full Nov 2025 MCP spec support.

## Features

- **Server Configuration** - TOML-based configuration for MCP servers
- **Lifecycle Management** - Start, stop, restart MCP server processes
- **Binary Verification** - Hash verification before server execution
- **Secure Client** - Capability-based authorization layer
- **Rate Limiting** - Configurable rate limits for tool calls
- **Nov 2025 MCP Spec**:
  - Sampling (server-initiated LLM calls)
  - Roots (server boundary inquiries)
  - Elicitation (server requests for user input)
  - URL Elicitation (OAuth flows, credential collection)
  - Tasks (long-running operations)

## Usage

```rust
use astralis_mcp::{McpClient, ServersConfig, ServerConfig};

#[tokio::main]
async fn main() -> Result<(), astralis_mcp::McpError> {
    // Create configuration
    let mut config = ServersConfig::default();
    config.add(
        ServerConfig::stdio("filesystem", "npx")
            .with_args(["-y", "@anthropics/mcp-server-filesystem", "/tmp"])
            .auto_start()
    );

    // Create client
    let client = McpClient::with_config(config);

    // Connect to server
    client.connect("filesystem").await?;

    // List available tools
    let tools = client.list_tools().await?;
    for tool in tools {
        println!("Tool: {}:{}", tool.server, tool.name);
    }

    // Call a tool
    let result = client.call_tool(
        "filesystem",
        "read_file",
        serde_json::json!({"path": "/tmp/test.txt"})
    ).await?;

    println!("Result: {}", result.text_content());
    Ok(())
}
```

## Key Types

| Type | Description |
|------|-------------|
| `McpClient` | Core MCP client for tool calling |
| `SecureMcpClient` | Client with capability-based authorization |
| `ServerConfig` | Single server configuration |
| `ServersConfig` | Collection of server configurations |
| `ServerManager` | Server process lifecycle management |
| `TaskManager` | Long-running task tracking |
| `RateLimiter` | Rate limiting for tool calls |
| `BinaryVerifier` | Binary hash verification |

## License

This crate is licensed under the MIT license.
