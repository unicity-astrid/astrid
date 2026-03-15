# astrid-mcp

[![Crates.io](https://img.shields.io/crates/v/astrid-mcp)](https://crates.io/crates/astrid-mcp)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

MCP client with server lifecycle management for Astrid.

`astrid-mcp` wraps the [`rmcp`](https://crates.io/crates/rmcp) SDK and adds everything an agent runtime needs: process lifecycle management, binary hash verification before execution, TOML-driven configuration, capability-based authorization, and immutable audit logging for every tool invocation. It is the sole entry point through which Astrid frontends and capsules interact with external tool servers.

## Core Features

- **Server lifecycle management** - Start, stop, restart, and health-monitor MCP server processes from TOML configuration. Supports stdio (child process) and SSE transports with configurable restart policies (`Never`, `OnFailure { max_retries }`, `Always`).
- **Binary hash verification** - SHA-256 content hash checked against a pinned `binary_hash` value in config before any process is spawned. Mismatches abort the launch with `McpError::BinaryHashMismatch`.
- **Secure tool invocation** - `SecureMcpClient` wraps every tool call with a capability token check (`astrid-capabilities`), consumes single-use tokens atomically to block replay, verifies the token issuer matches the trusted runtime key, and writes an audit entry to `astrid-audit` before and after each call.
- **MCP Nov 2025 spec support** - Client-side handler traits for sampling (server-initiated LLM calls), roots (server boundary inquiries), elicitation (structured user input), and URL elicitation (OAuth/payment flows). Canonical elicitation types live in `astrid-core` - no duplicates in this crate.
- **Dynamic server management** - Add or remove servers at runtime via `connect_dynamic` / `disconnect` without restarting the client.
- **Reactive tool cache** - Background listener processes `notifications/tools/list_changed` pushed by servers and refreshes the in-memory tools cache without polling.
- **Server name validation** - Names are validated against a strict ASCII allowlist (alphanumeric, `-`, `_`, `:`, `.`) with path-traversal rejection, null-byte rejection, and Unicode lookalike rejection at both load time and `add()` time.
- **`Arc`-cloneable clients** - Both `McpClient` and `SecureMcpClient` are cheaply cloneable; all clones share the same `ServerManager`, tools cache, capability store, and audit log.
- **`test-support` feature** - `testing::test_secure_mcp_client()` constructs a `SecureMcpClient` backed by in-memory stores for unit tests that don't connect to real servers.

## Quick Start

```toml
[dependencies]
astrid-mcp = { workspace = true }
```

```rust
use astrid_mcp::{McpClient, ServersConfig, ServerConfig};

#[tokio::main]
async fn main() -> Result<(), astrid_mcp::McpError> {
    let mut config = ServersConfig::default();
    config.add(
        ServerConfig::stdio("filesystem", "npx")
            .with_args(["-y", "@anthropics/mcp-server-filesystem", "/tmp"])
            .auto_start(),
    )?;

    let client = McpClient::with_config(config);
    client.connect("filesystem").await?;

    let tools = client.list_tools().await?;
    for tool in &tools {
        println!("{}", tool.full_name()); // "filesystem:read_file"
    }

    let result = client
        .call_tool("filesystem", "read_file", serde_json::json!({"path": "/tmp/hello.txt"}))
        .await?;
    println!("{}", result.text_content());
    Ok(())
}
```

### Configuration file (`~/.astrid/servers.toml`)

```toml
[servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@anthropics/mcp-server-filesystem", "/home/user"]
auto_start = true
restart_policy = "on_failure"

[servers.filesystem.restart_policy]
on_failure = { max_retries = 3 }

[servers.remote-api]
transport = "sse"
url = "https://tools.example.com/mcp"

# Untrusted server: sandboxed, network disabled, extra read path
[servers.sandboxed-tool]
command = "my-tool"
trusted = false
allow_network = false
allowed_read_paths = ["/data/shared"]
allowed_write_paths = ["/data/output"]
binary_hash = "sha256:abc123..."
```

Load the default config with `ServersConfig::load_default()` or a specific path with `ServersConfig::load(path)`.

### Secure usage

For production use within Astrid, `SecureMcpClient` requires a capability token for every tool invocation and writes both a pre-call and post-call audit entry:

```rust
use astrid_mcp::{McpClient, SecureMcpClient, ServersConfig, ToolAuthorization};
use std::sync::Arc;

let base = McpClient::with_config(config);
let secure = SecureMcpClient::new(base, capabilities_store, audit_log, session_id);

secure.connect("filesystem").await?;

match secure.call_tool_if_authorized("filesystem", "read_file", args).await? {
    Ok(result) => println!("{}", result.text_content()),
    Err(ToolAuthorization::RequiresApproval { server, tool, resource }) => {
        // surface an approval gate to the user
    }
    Err(_) => unreachable!(),
}
```

## API Reference

### Key Types

| Type | Description |
|---|---|
| `McpClient` | Primary client. Manages server processes, caches tools, and calls tools without authorization checks. |
| `SecureMcpClient` | Wraps `McpClient` with capability token validation, issuer verification, single-use token consumption, and audit logging. |
| `ServerManager` | Low-level process manager. Spawns child processes, tracks health, enforces restart policies. |
| `ServersConfig` | Collection of `ServerConfig` entries. Loads from / saves to TOML. |
| `ServerConfig` | Per-server config: transport, command, args, env, sandbox flags, restart policy, optional binary hash. |
| `Transport` | `Stdio` (child process, default) or `Sse` (HTTP streaming). |
| `RestartPolicy` | `Never` (default), `OnFailure { max_retries }`, or `Always`. |
| `ToolDefinition` | Tool metadata: name, server, description, JSON Schema for inputs. `full_name()` returns `"server:tool"`, `resource_uri()` returns `"mcp://server:tool"`. |
| `ToolResult` | Tool call output: success flag, `Vec<ToolContent>`, and optional error string. `text_content()` joins text content. |
| `ToolContent` | `Text { text }`, `Image { data, mime_type }`, or `Resource { uri, data, mime_type }`. |
| `ToolAuthorization` | `Authorized { proof }` or `RequiresApproval { server, tool, resource }`. Returned by `check_authorization`. |
| `McpError` | Error enum covering server lifecycle, tool calls, authorization, binary hash mismatches, config, transport, and protocol errors. |
| `ElicitationRequest` / `ElicitationResponse` | Re-exported from `astrid-core`. Canonical types for structured user-input requests from MCP servers. |
| `UrlElicitationRequest` / `UrlElicitationResponse` | Re-exported from `astrid-core`. Used for OAuth and payment flows. |

### `McpClient` methods

- `with_config(config)` / `from_default_config()` - construct
- `connect(name)` / `disconnect(name)` / `reconnect(name)` - named server lifecycle
- `connect_dynamic(name, config)` - add and connect a server at runtime
- `connect_auto_servers()` - starts all servers marked `auto_start = true`
- `disconnect_all()` / `shutdown()` - bulk teardown
- `list_tools()` / `get_tool(server, tool)` - tool discovery
- `call_tool(server, tool, args)` - raw tool invocation (no auth check)
- `try_reconnect(name)` - checks restart policy atomically before reconnecting
- `send_notification(server, method, params)` - fire-and-forget JSON-RPC notification
- `list_servers()` / `is_server_running(name)` - runtime status

### `SecureMcpClient` methods

- `check_authorization(server, tool)` - validates capability token; consumes single-use tokens
- `call_tool(server, tool, args, proof)` - invokes tool and writes audit entries
- `call_tool_if_authorized(server, tool, args)` - convenience combining both steps
- All `McpClient` lifecycle methods, each with audit entries for connect/disconnect events

### `ServerConfig` builder

```rust
ServerConfig::stdio("name", "command")
    .with_args(["-y", "pkg"])
    .with_env("KEY", "value")
    .with_hash("sha256:...")
    .with_restart_policy(RestartPolicy::OnFailure { max_retries: 3 })
    .with_network(false)
    .with_read_path("/data")
    .with_write_path("/output")
    .trusted()      // opt out of OS sandbox
    .auto_start()

ServerConfig::sse("name", "https://host/mcp")
```

### Prelude

```rust
use astrid_mcp::prelude::*;
// Imports: McpClient, SecureMcpClient, ServerManager, ServerConfig, ServersConfig,
//          ToolDefinition, ToolResult, ToolContent, ToolAuthorization,
//          ElicitationRequest/Response, UrlElicitationRequest/Response,
//          McpError, McpResult
```

### `test-support` feature

```toml
[dev-dependencies]
astrid-mcp = { workspace = true, features = ["test-support"] }
```

```rust
use astrid_mcp::testing::test_secure_mcp_client;

let client = test_secure_mcp_client();
// Backed by in-memory CapabilityStore and AuditLog - no real MCP servers needed.
```

## Development

```bash
cargo test -p astrid-mcp --all-features
```

## License

Dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
