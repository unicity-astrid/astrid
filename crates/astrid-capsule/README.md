# astrid-capsule

[![Crates.io](https://img.shields.io/crates/v/astrid-capsule)](https://crates.io/crates/astrid-capsule)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Core runtime management, manifest routing, and composite execution engines for Astralis OS user-space capsules.

`astrid-capsule` is the engine room of the Astralis extension ecosystem. Implementing the Phase 4 Manifest-First architecture, it acts as the definitive boundary between the core OS and user-provided code. Rather than forcing developers into a single execution paradigm, this crate utilizes a Composite Architecture. It parses declarative manifests (`Capsule.toml`) to seamlessly orchestrate WebAssembly sandboxes, legacy Model Context Protocol (MCP) host processes, and static context under a unified, secure lifecycle.

If Astralis is the ship, `astrid-capsule` provides the standardized docking bays, lifecycle routing, and security airlocks for every extension that comes aboard.

## Core Features

- **Manifest-First Routing**: Parses `Capsule.toml` manifests to automatically wire up capabilities, LLM tools, cron jobs, environment variables, and OS interceptors before a single line of user code executes.
- **Composite Architecture**: A single capsule can encompass multiple execution engines. Run a secure WASM component alongside an "airlock override" legacy Node/Python MCP process, managed entirely as one logical unit.
- **Pluggable Execution Engines**: Native support for `WasmEngine` (via Extism), `McpHostEngine` (legacy `stdio` binaries), and `StaticEngine` (declarative-only capsules).
- **Hardened Security Gates**: Intercepts host calls (HTTP, File I/O, Connector Registration) via the `CapsuleSecurityGate` trait, integrating natively with `astrid-approval` for human-in-the-loop permission budgets.
- **Zero-Friction Hot Reloading**: A built-in daemon (`watcher.rs`) monitors capsule source directories, debounces file events, `blake3`-hashes source trees, and emits precise invalidation events.

## Architecture: The Composite Model

Because a single manifest can define multiple distinct execution units, the OS uses an additive model rather than polymorphic variants. 

The `CapsuleLoader` acts as the router. Upon reading a manifest, it instantiates a `CompositeCapsule` packed with the requested `ExecutionEngine` implementations. When the OS commands the capsule to `.load()`, the composite iterates through its engines—initializing WASM memory, spawning child processes, or parsing static context. If any single engine fails, the entire capsule safely rolls back its state.

### The Execution Engines

1. **WasmEngine**: Loads and executes compiled WebAssembly (or OpenClaw scripts) within a strictly sandboxed Extism runtime.
2. **McpHostEngine**: The "airlock override." Spawns a native child process (e.g., `npx`, `python`) to communicate via standard MCP JSON-RPC over `stdio`.
3. **StaticEngine**: Always attached. Handles the injection of context files, static commands, and predefined skills directly into OS memory without booting any VMs or secondary processes.

## The Manifest (`Capsule.toml`)

The manifest is the absolute source of truth. `astrid-capsule` translates this declarative configuration into strongly-typed structures (`CapsuleManifest`) to orchestrate the environment.

```toml
[package]
name = "github-agent"
version = "1.0.0"
astrid-version = ">=0.1.0"

# 1. The primary WASM logic
[component]
entrypoint = "bin/github_agent.wasm"

# 2. OS Capabilities requested
[capabilities]
net = ["api.github.com"]
fs_read = ["/home/user/.gitconfig"]

# 3. Environment Variables elicited during docking
[env.GITHUB_TOKEN]
type = "secret"
request = "Please provide a GitHub Personal Access Token"

# 4. The Airlock Override: Legacy MCP stdio server
[[mcp_server]]
id = "local-git"
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-git"]

# 5. Scheduled Background Tasks (Cron)
[[cron]]
name = "sync-issues"
schedule = "0 * * * *"
action = "astrid.github.sync"

# 6. eBPF-style Lifecycle Hooks
[[interceptor]]
event = "BeforeToolCall"

# 7. LLM Provider (Agent Brain)
[[llm_provider]]
id = "claude-3-5-sonnet"
description = "Anthropic Claude 3.5 Sonnet Provider"
capabilities = ["text", "vision", "tools"]
```

## Quick Start

For developers integrating this crate into the wider Astralis OS routing layer, loading a capsule from a manifest requires the `CapsuleLoader` and discovery system:

```rust
use std::path::PathBuf;
use astrid_capsule::discovery::discover_manifests;
use astrid_capsule::loader::CapsuleLoader;
use astrid_capsule::registry::CapsuleRegistry;
use astrid_mcp::McpClient;

async fn init_capsules(mcp_client: McpClient) {
    let loader = CapsuleLoader::new(mcp_client);
    let mut registry = CapsuleRegistry::new();
    
    // 1. Discover manifests in `.astrid/plugins/`
    let found = discover_manifests(None);
    
    // 2. Route and build Composite Capsules
    for (manifest, dir) in found {
        match loader.create_capsule(manifest, dir) {
            Ok(mut capsule) => {
                // 3. Register and Load
                registry.register(capsule);
            }
            Err(e) => tracing::error!("Failed to build capsule: {e}"),
        }
    }
}
```

## Security Integration

All engines are strictly bound by the `CapsuleSecurityGate` trait. By default, when `astrid-capsule` is compiled with the `approval` feature, this integrates directly with `astrid-approval`'s `SecurityInterceptor`.

This ensures that any filesystem reads/writes and HTTP requests originating from within a capsule (regardless of the underlying engine) respect the system's global budget and policy constraints. Test implementations (`AllowAllGate`, `DenyAllGate`) are provided for isolated unit testing.

## Development

This crate is a critical load-bearing component of Astralis OS. When adding new fields to `Capsule.toml`, ensure you update both the `CapsuleManifest` struct in `src/manifest.rs` and the routing logic in `src/loader.rs`. Any new execution paradigm must implement the `ExecutionEngine` trait in `src/engine/mod.rs`.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.