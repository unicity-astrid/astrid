# astrid-mcp

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The syscall boundary for external tools. Device drivers for the OS.**

Astrid capsules live inside WASM. External tool servers live outside it. This crate bridges the gap: it manages MCP server processes, verifies their binaries before spawning them, gates every tool call through the capability system, and writes audit entries for every invocation. It is the sole path through which the kernel reaches external tools.

Wraps the `rmcp` SDK. Implements the MCP 2025-11-25 spec (sampling, roots, elicitation, URL elicitation).

## Why this exists

An agent calling `mcp://filesystem:read_file` is analogous to a process calling `read(2)`. The kernel must verify authorization, log the call, and manage the server process that handles it. Without this crate, every tool call would bypass the security model.

## Two client layers

**`McpClient`** manages server processes, caches tools, and dispatches calls without authorization checks. It is the raw interface for kernel-internal use.

**`SecureMcpClient`** wraps `McpClient` with the full security chain: capability token validation, single-use token consumption (atomic, blocks replay), issuer verification against the runtime's ed25519 public key, and audit logging of every call with a BLAKE3 hash of the arguments. This is what the kernel's tool dispatch path uses.

Both clients are cheaply cloneable. All clones share the same `ServerManager`, tools cache, and capability state via `Arc`.

## Server lifecycle

`ServerManager` owns server processes. It starts them, monitors health via `RunningService::is_closed()`, enforces restart policies (`Never`, `OnFailure { max_retries }`, `Always`), and tracks restart counts for backoff. `try_reconnect` checks the restart policy and reconnects atomically to avoid TOCTOU races.

Binary hash verification runs before any process spawns. If a `binary_hash` is configured, the server binary is read, BLAKE3-hashed, and compared. Mismatch aborts with `McpError::BinaryHashMismatch`. No fallback, no override.

Untrusted servers (the default) are wrapped in an OS-level sandbox via `astrid-workspace`. Trusted servers (`trusted = true`) run natively.

## Reactive tool cache

When a server sends `notifications/tools/list_changed`, a background listener refreshes the in-memory tool cache without polling. The notice channel also handles uplink registrations for Tier 2 capsule processes.

## Server name validation

Server names are validated against a strict ASCII allowlist: alphanumeric, hyphens, underscores, colons, dots (not leading). Path separators, null bytes, shell metacharacters, and Unicode lookalikes are rejected. Display names in error messages are truncated to 40 characters to prevent log poisoning from attacker-controlled input.

## Usage

```toml
[dependencies]
astrid-mcp = { workspace = true }
```

```rust
use astrid_mcp::{McpClient, ServersConfig, ServerConfig};

let mut config = ServersConfig::default();
config.add(
    ServerConfig::stdio("filesystem", "npx")
        .with_args(["-y", "@anthropics/mcp-server-filesystem", "/tmp"])
        .auto_start(),
)?;

let client = McpClient::with_config(config);
client.connect("filesystem").await?;

let tools = client.list_tools().await?;
let result = client.call_tool(
    "filesystem", "read_file",
    serde_json::json!({"path": "/tmp/hello.txt"})
).await?;
```

## Development

```bash
cargo test -p astrid-mcp --all-features
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
