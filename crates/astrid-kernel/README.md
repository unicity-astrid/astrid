# astrid-kernel

[![Crates.io](https://img.shields.io/crates/v/astrid-kernel)](https://crates.io/crates/astrid-kernel)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

The micro-kernel for the Astrid secure agent runtime: boots WASM capsules, routes IPC between them, and manages session lifetime.

`astrid-kernel` is a pure WASM runner with no business logic and no network servers. It instantiates an `EventBus`, loads `.capsule` files into the Extism sandbox, routes IPC bytes between capsules, and manages the system-wide security surfaces (capability store, audit log, overlay VFS, Unix domain socket) for a single agent session. All frontends - CLI, Discord, web - drive the same kernel instance over that Unix socket.

## Core Features

- **Capsule lifecycle management**: Discovers capsules from `~/.astrid/capsules/`, topologically sorts by dependency, loads uplinks before non-uplinks, awaits readiness signals, and injects tool schemas into every capsule's KV namespace after load.
- **Kernel management API**: Listens on `astrid.v1.request.*` topics on the `EventBus` and handles `ListCapsules`, `GetCommands`, `GetCapsuleMetadata`, `ReloadCapsules`, and `InstallCapsule` requests. Mutating operations are rate-limited with a sliding-window limiter (e.g. `ReloadCapsules` is capped at 5/min).
- **Overlay VFS**: Mounts a copy-on-write filesystem over the workspace root. Writes land in a session-scoped `TempDir` that is discarded on shutdown; the lower layer (the real workspace) is never modified unless the caller explicitly commits.
- **Chain-linked audit log**: Opens `~/.astrid/audit.db` at boot, verifies the ed25519-signed chain of every historical session, and logs violations at `error!` before continuing (fail-open for availability, loud for integrity).
- **Unix socket + session token**: Binds `~/.astrid/sessions/system.sock` (0o700 parent directory) before any capsule loads. Generates a random `SessionToken`, writes it to `~/.astrid/sessions/system.token` (0o600), and clears it on shutdown so the secret does not persist.
- **Idle auto-shutdown**: Dual-signal idle monitor tracks the explicit `active_connections` counter and the `EventBus` subscriber count. Takes the minimum of both to handle ungraceful CLI exits. Shuts down after `ASTRID_IDLE_TIMEOUT_SECS` (default 300) of zero effective connections and no daemon/cron capsules running.
- **Capsule restart**: `restart_capsule` unregisters and explicitly unloads the old instance (preventing orphaned MCP child processes), reloads from disk, then dispatches `handle_lifecycle_restart` to the new instance.
- **Identity bootstrap**: Creates a KV-backed identity store at boot and registers the CLI root user idempotently. Identity links from config are applied before capsules load.

## Quick Start

```toml
[dependencies]
astrid-kernel = "0.2"
```

```rust
use astrid_kernel::Kernel;
use astrid_core::SessionId;
use std::path::PathBuf;

// Requires a multi-threaded tokio runtime - block_in_place panics on current_thread.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session_id = SessionId::new();
    let workspace = PathBuf::from("/path/to/workspace");

    // Boot: opens KV store, audit log, VFS, socket, token, spawns background tasks.
    let kernel = Kernel::new(session_id, workspace).await?;

    // Discover and load all capsules from ~/.astrid/capsules/.
    kernel.load_all_capsules().await;

    // Write the readiness sentinel so the CLI knows the daemon is accepting connections.
    astrid_kernel::socket::write_readiness_file()?;

    // Run until OS signal or idle timeout triggers shutdown.
    kernel.shutdown(None).await;
    Ok(())
}
```

## API Reference

### Key Types

#### `Kernel`

The central kernel struct. All fields are `pub` so frontends and capsule loaders can read them directly.

| Field | Type | Purpose |
|---|---|---|
| `session_id` | `SessionId` | Unique identifier for this boot session. |
| `event_bus` | `Arc<EventBus>` | Global IPC message bus shared by all capsules. |
| `capsules` | `Arc<RwLock<CapsuleRegistry>>` | Registry of all loaded WASM capsules. |
| `mcp` | `SecureMcpClient` | Capability-gated MCP client with audit logging. |
| `capabilities` | `Arc<CapabilityStore>` | Persistent, KV-backed capability store. |
| `vfs` | `Arc<dyn Vfs>` | Overlay VFS (copy-on-write over workspace root). |
| `overlay_vfs` | `Arc<OverlayVfs>` | Concrete overlay handle for commit/rollback. |
| `vfs_root_handle` | `DirHandle` | cap-std physical security boundary for VFS access. |
| `workspace_root` | `PathBuf` | Physical path the VFS is mounted to. |
| `global_root` | `Option<PathBuf>` | `~/.astrid/shared/` - readable by capsules declaring `fs_read = ["global://"]`. |
| `kv` | `Arc<SurrealKvStore>` | Shared persistent KV store backing all capsule namespaces. |
| `audit_log` | `Arc<AuditLog>` | Chain-linked cryptographic audit log. |
| `allowance_store` | `Arc<AllowanceStore>` | Capsule-level approval allowances (session and always scopes). |
| `session_token` | `Arc<SessionToken>` | Random token generated at boot for CLI socket authentication. |
| `active_connections` | `AtomicUsize` | Number of active CLI sessions. |

#### Key Methods

```rust
// Boot a new kernel session.
Kernel::new(session_id: SessionId, workspace_root: PathBuf) -> Result<Arc<Self>, io::Error>

// Auto-discover and load all capsules in dependency order.
kernel.load_all_capsules().await

// Gracefully shut down: broadcasts KernelShutdown, drains capsules, flushes KV,
// removes socket and token files.
kernel.shutdown(reason: Option<String>).await

// Load a single capsule from a directory containing Capsule.toml.
kernel.load_capsule(dir: PathBuf) -> Result<(), anyhow::Error>  // pub(crate)

// Restart a capsule: unload old, reload from disk, send lifecycle event.
kernel.restart_capsule(id: &CapsuleId) -> Result<(), anyhow::Error>  // pub(crate)

// Connection tracking (called by KernelRouter on IPC events).
kernel.connection_opened()
kernel.connection_closed()  // clears session allowances when count reaches 0
kernel.connection_count() -> usize
```

### `kernel_router` module

`spawn_kernel_router` is called internally during `Kernel::new`. It subscribes to `astrid.v1.request.*` on the event bus and dispatches `KernelRequest` variants. Responses are published on `astrid.v1.response.<suffix>`.

Handled requests:

| Request | Rate limit | Behavior |
|---|---|---|
| `ListCapsules` | None | Returns the list of registered capsule IDs. |
| `GetCommands` | None | Returns all commands from all loaded capsule manifests. |
| `GetCapsuleMetadata` | None | Returns name, LLM providers, and interceptor events per capsule. |
| `ReloadCapsules` | 5/min | Drops capsules in `Failed` state, then calls `load_all_capsules`. |
| `InstallCapsule` | 10/min | Not yet implemented; returns an error response. |
| `ApproveCapability` | 10/min | Not yet implemented; returns an error response. |

### `socket` module

Unix socket and readiness file management. Key public items:

```rust
// Path to the kernel socket (~/.astrid/sessions/system.sock).
socket::kernel_socket_path() -> PathBuf

// Path to the readiness sentinel (~/.astrid/sessions/system.ready).
socket::readiness_path() -> PathBuf

// Write the sentinel file (0o600) signaling the daemon is ready for connections.
// Call this after load_all_capsules() completes.
socket::write_readiness_file() -> Result<(), io::Error>

// Remove the sentinel file (best-effort, silent on error).
socket::remove_readiness_file()
```

`bind_session_socket` (crate-private) validates the socket path length against the platform `sun_path` limit (104 bytes on macOS/FreeBSD/OpenBSD, 108 on Linux), removes stale sockets, and rejects paths where another kernel is already listening.

## Development

```bash
cargo test -p astrid-kernel -- --quiet
```

The rate limiter and socket path validation have unit tests in their respective modules. Integration tests that boot a full `Kernel` require a multi-threaded runtime and a writable `$ASTRID_HOME` or `$HOME`.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
