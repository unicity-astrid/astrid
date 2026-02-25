# astrid-mcp

[![Crates.io](https://img.shields.io/crates/v/astrid-mcp)](https://crates.io/crates/astrid-mcp)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Robust MCP client integration and server lifecycle management for the Astralis OS.

`astrid-mcp` provides the essential bridge between the Astralis intelligence kernel and external tool ecosystems. By wrapping the official `rmcp` SDK (Model Context Protocol), this crate transforms standard MCP interactions into secure, audited, and capability-driven operations native to Astralis. It manages the full lifecycle of MCP servers, rigorously verifies executables before launch, and enforces strict authorization boundaries on every tool invocation.

## Core Features

* **Server Lifecycle Management**: Native process control to start, stop, restart, and monitor MCP server instances directly from TOML configurations.
* **Cryptographic Binary Verification**: Prevents supply-chain attacks by hashing and verifying server binaries (SHA-256 or BLAKE3) before execution.
* **Capability-Based Authorization**: Zero-trust tool invocation using the Astralis capabilities system and strict, centralized audit logging.
* **Nov 2025 MCP Spec Native**:
  * **Sampling**: Support for server-initiated LLM calls.
  * **Roots**: Dynamic server boundary inquiries.
  * **Elicitation**: Safe human-in-the-loop requests and URL credential collection.
  * **Tasks**: Tracking for long-running operations.
* **Robust Rate Limiting**: Built-in, configurable limits for high-volume tool execution to prevent resource exhaustion or quota breaches.
* **Dynamic Configuration**: Hot-pluggable configuration allowing servers to be dynamically added or removed at runtime.

## Architecture

`astrid-mcp` is designed in layers to isolate process management from protocol logic and authorization. This separation of concerns ensures that a compromised external tool cannot bypass Astralis security boundaries.

* **`ServerManager`**: Handles spawning child processes, capturing standard I/O for MCP transport, tracking process health, and enforcing restart policies.
* **`BinaryVerifier`**: Intercepts server startup to ensure the target executable matches an expected cryptographic hash, falling back to cached verifications for rapid subsequent startups.
* **`McpClient`**: The primary interface for caching tools, handling server notices (e.g., tool list refreshes), and orchestrating standard MCP communications over the underlying `rmcp` transport.
* **`SecureMcpClient`**: A secure wrapper requiring `SessionId`, `CapabilityStore`, and `AuditLog` instances. It mandates that every tool invocation holds a valid capability token and logs all operations (both success and failure) to the centralized Astralis audit system.

## Quick Start

Add `astrid-mcp` to your `Cargo.toml`:

```toml
[dependencies]
astrid-mcp = { workspace = true }
```

Instantiate a client, configure a server, and execute a tool:

```rust
use astrid_mcp::{McpClient, ServersConfig, ServerConfig};

#[tokio::main]
async fn main() -> Result<(), astrid_mcp::McpError> {
    let mut config = ServersConfig::default();
    
    // Configure an npx-based MCP server
    config.add(
        ServerConfig::stdio("filesystem", "npx")
            .with_args(["-y", "@anthropics/mcp-server-filesystem", "/tmp"])
            .auto_start()
    );

    let client = McpClient::with_config(config);
    client.connect("filesystem").await?;

    let result = client.call_tool(
        "filesystem",
        "read_file",
        serde_json::json!({"path": "/tmp/test.txt"})
    ).await?;

    println!("Tool output: {}", result.text_content());
    Ok(())
}
```

### Secure Usage

For production environments within Astralis, the `SecureMcpClient` enforces capability requirements and records immutable audit trails. It requires tight integration with `astrid-core`, `astrid-audit`, and `astrid-capabilities`.

```rust
use astrid_mcp::{McpClient, SecureMcpClient, ServersConfig};
use astrid_core::SessionId;
use std::sync::Arc;

// Assume capabilities_store and audit_log are initialized by the Astralis runtime
let base_client = McpClient::with_config(config);
let secure_client = SecureMcpClient::new(
    base_client,
    capabilities_store,
    audit_log,
    SessionId::new()
);

// Connect and automatically audit the connection event
secure_client.connect("filesystem").await?;

// Invocation requires prior capability issuance
// If unauthorized, this returns a requirement for approval rather than failing blindly
let result = secure_client.call_tool_if_authorized(
    "filesystem",
    "write_file",
    serde_json::json!({"path": "/tmp/out.txt", "content": "hello"})
).await?;
```

## Development

To run tests for this specific crate within the workspace:

```bash
cargo test -p astrid-mcp --all-features
```

When contributing to protocol-level features, ensure you reference the November 2025 MCP specification and implement the corresponding handlers in the `capabilities` module.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
