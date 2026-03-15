# astrid-capsule

[![Crates.io](https://img.shields.io/crates/v/astrid-capsule)](https://crates.io/crates/astrid-capsule)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Core runtime management for User-Space Capsules in Astrid OS.

`astrid-capsule` implements the Manifest-First architecture for Astrid's extension ecosystem. It parses `Capsule.toml` manifests, routes execution to the appropriate runtime environment (WASM sandbox, legacy MCP host process, or static context), enforces capability-based security gates, and manages the full capsule lifecycle under a single unified abstraction.

## Core Features

- **Manifest-first routing**: `CapsuleLoader` reads a `Capsule.toml` and wires up the correct execution engines automatically. No code is required from a capsule author to register tools, interceptors, uplinks, or cron jobs - the manifest is the complete declaration.
- **Composite execution model**: A single capsule can run a WASM component, a legacy stdio MCP process, and static context injection simultaneously, all managed under one `Capsule` lifecycle.
- **Three built-in execution engines**: `WasmEngine` (Extism-based WASM sandbox), `McpHostEngine` (native stdio subprocess, the "airlock override"), and `StaticEngine` (context files, skills, and commands injected without booting any VM).
- **Capability-based security gates**: The `CapsuleSecurityGate` trait intercepts every host call (HTTP, filesystem read/write, host process spawn, socket bind, identity operations). The `ManifestSecurityGate` implementation enforces declared capabilities from the manifest, with path traversal rejection and workspace-confined wildcard matching.
- **Topological boot ordering**: `toposort_manifests` uses Kahn's algorithm to order capsule load sequences by capability dependency (`requires`/`provides`), supporting single-segment wildcard matching (`topic:llm.stream.*`).
- **IPC event dispatch**: `EventDispatcher` subscribes to the `EventBus` and fans out IPC events and lifecycle events to matching capsule interceptors concurrently. Per-capsule semaphores (default 4 permits) bound concurrent interceptor invocations.
- **Manifest validation**: `load_manifest` enforces semver versions, rejects empty IPC topic segments, validates dependency capability prefixes (`topic:`, `tool:`, `llm:`, `uplink:`), rejects wildcards in `provides`, and enforces `astrid-version` compatibility at load time.

## Quick Start

```toml
[dependencies]
astrid-capsule = { workspace = true }
```

Discover and load capsules from `.astrid/capsules/`:

```rust
use std::path::PathBuf;
use astrid_capsule::discovery::discover_manifests;
use astrid_capsule::loader::CapsuleLoader;
use astrid_capsule::registry::CapsuleRegistry;
use astrid_mcp::SecureMcpClient;

async fn init_capsules(mcp_client: SecureMcpClient) {
    let loader = CapsuleLoader::new(mcp_client);
    let mut registry = CapsuleRegistry::new();

    // Scans .astrid/capsules/ for Capsule.toml files
    let found = discover_manifests(None);

    for (manifest, dir) in found {
        match loader.create_capsule(manifest, dir) {
            Ok(capsule) => {
                registry.register(capsule).expect("duplicate capsule id");
            }
            Err(e) => tracing::error!("Failed to build capsule: {e}"),
        }
    }
}
```

## The Manifest (`Capsule.toml`)

Every capsule is fully described by a `Capsule.toml`. The runtime reads this file and provisions the appropriate engines - no capsule author needs to write Rust.

```toml
[package]
name = "my-capsule"
version = "1.0.0"
astrid-version = ">=0.1.0"

# WASM component (optional)
[[component]]
file = "bin/my_capsule.wasm"
hash = "sha256:abc123..."  # optional integrity check

# Capability declarations (fail-closed by default)
[capabilities]
net = ["api.example.com"]
fs_read = ["workspace://src"]
fs_write = ["workspace://out"]
ipc_publish = ["my.v1.events.*"]
ipc_subscribe = ["llm.v1.response.*"]

# Environment variables elicited from the user during install
[env.API_KEY]
type = "secret"
request = "Enter your API key"

# Legacy stdio MCP server (airlock override)
[[mcp_server]]
id = "my-server"
type = "stdio"
command = "npx"
args = ["-y", "@example/mcp-server"]

# eBPF-style IPC interceptors
[[interceptor]]
event = "user.prompt"
action = "handle_prompt"

# Dependency ordering
[dependencies]
provides = ["topic:my.v1.events.ready"]
requires = ["topic:llm.v1.response.*"]

# Scheduled tasks
[[cron]]
name = "daily-sync"
schedule = "0 0 * * *"
action = "my.v1.sync"

# IPC topic API declarations (schema-annotated)
[[topic]]
name = "my.v1.events.ready"
direction = "publish"
description = "Emitted when the capsule is ready"
schema = "schemas/ready.json"
```

## API Reference

### Key Types

- `CapsuleManifest` - Deserialized `Capsule.toml`. The source of truth for all engine provisioning, capability checks, and dependency ordering.
- `Capsule` (trait) - Unified interface for all capsule implementations. Provides `load`, `unload`, `tools`, `invoke_interceptor`, `wait_ready`, and `check_health`.
- `CompositeCapsule` - The concrete implementation. Owns a `Vec<Box<dyn ExecutionEngine>>` and fans out lifecycle calls across all engines.
- `CapsuleLoader` - Factory that translates a manifest into a `CompositeCapsule` populated with the correct engines.
- `CapsuleRegistry` - In-memory store for loaded capsules, indexed by `CapsuleId`. Also tracks uplink descriptors and WASM session UUID mappings for IPC capability checks.
- `CapsuleContext` - Execution context passed to engines during `load`: workspace root, KV store, event bus, CLI socket, registry, and optional identity/approval stores.
- `CapsuleSecurityGate` (trait) - Gate for every sensitive host call. `ManifestSecurityGate` enforces the manifest's declared capabilities. `AllowAllGate` and `DenyAllGate` are provided for tests.
- `EventDispatcher` - Subscribes to the `EventBus` and dispatches IPC events and lifecycle events to registered interceptors concurrently.
- `CapsuleId` - Validated identifier (lowercase alphanumeric and hyphens only).
- `CapsuleError` / `CapsuleResult<T>` - Error type and result alias for all capsule operations.
- `toposort_manifests` - Kahn's algorithm for capability-based load ordering. Returns `CycleError` with the original manifest list on cycle detection.
- `capability_matches` - Wildcard-aware capability matcher used by both toposort and dependency resolution.

### Execution Engines (internal)

- `WasmEngine` - Extism-based WASM sandbox with host functions for IPC, KV, VFS, HTTP, and identity operations.
- `McpHostEngine` - Spawns a native stdio subprocess and bridges it via `SecureMcpClient`. Declared with `type = "stdio"` in the manifest.
- `StaticEngine` - Injects `context_files`, `commands`, and `skills` into OS memory without booting any external process.

## Security Model

All capability checks are fail-closed. A capsule that declares no `net`, `fs_read`, `fs_write`, `ipc_publish`, `ipc_subscribe`, or `identity` capabilities gets none of them. The `ManifestSecurityGate`:

- Rejects paths containing `..` components to prevent traversal attacks
- Confines wildcard (`*`) file access to the canonical workspace root - paths outside (e.g. `~/.astrid/keys/`) are always denied
- Resolves `workspace://` and `global://` scheme prefixes to physical paths at construction time
- Enforces an identity capability hierarchy: `admin > link > resolve`
- Denies `net_bind` and identity operations by default; capsules must explicitly declare them

The `allow_prompt_injection` capability field is `false` by default. Capsules without it cannot modify the LLM system prompt, even if their interceptor returns one.

## Development

```bash
cargo test --workspace -- --quiet
```

When adding fields to `CapsuleManifest` in `src/manifest.rs`, update `src/loader.rs` if the new field affects engine selection, and update `src/toposort.rs` if it affects dependency ordering. New execution paradigms must implement the `ExecutionEngine` trait in `src/engine/mod.rs`.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
