# astrid-workspace

[![Crates.io](https://img.shields.io/crates/v/astrid-workspace)](https://crates.io/crates/astrid-workspace)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Operational boundaries, safe zones, and airlock routing for the Astralis agent runtime.

While the WASM execution sandbox (Capsules) provides inescapable memory and thread safety, `astrid-workspace` defines the semantic, physical boundaries of where an agent is authorized to operate on the host machine. It establishes the "safe interior" of the workspace hull and provides a strict "airlock" protocol for when an agent needs to reach beyond those borders.

## Core Features

* **Boundary Enforcement**: Pre-compiled glob matching, canonical path resolution, and symlink evaluation to prevent directory traversal and unauthorized host system access.
* **Airlock Routing (Escapes)**: Structured `EscapeRequest` and `EscapeDecision` flows to handle agent operations that attempt to reach outside the safe zone.
* **Operational Postures**: Configurable safety modes ranging from strictly `Safe` (always ask before opening the airlock) to fully `Autonomous` (unrestricted deep-space operation).
* **Mission Profiles**: Predefined configurations (`safe`, `power_user`, `autonomous`, `ci`) designed for specific operating environments.
* **Session Memory**: Persistent and session-scoped state tracking via the `EscapeHandler` to remember user-approved airlock decisions seamlessly.

## Architecture: The Boundary vs. The Sandbox

Understanding the distinction between this crate and the WASM sandbox is critical for Astralis developers.

| Aspect | Workspace Boundary (`astrid-workspace`) | Execution Sandbox (WASM/Capsules) |
|--------|-----------------------------------------|-----------------------------------|
| **Domain** | Semantic file system access and host operations. | CPU instructions, memory allocation, panic isolation. |
| **Flexibility** | Escapable via the airlock approval process. | Inescapable by design. |
| **Control** | User-defined per mission profile. | Hardcoded by the Astralis kernel architecture. |
| **Purpose** | Prevents agents from accidentally modifying the host system. | Prevents malicious code from exploiting the host runtime. |

### How Integration Works

`astrid-workspace` acts as the primary navigational chart for the Astralis OS. It does not perform the file operations itself; instead, it provides the deterministic logic required by `astrid-core` and the various execution components (WASM host functions, MCP tools) to decide if an action is permitted.

When an agent attempts a filesystem operation:
1. The requested path is canonicalized to resolve symlinks and relative segments.
2. The `WorkspaceBoundary` evaluates the path against the active `WorkspaceConfig`.
3. If the path falls within the workspace root or an `auto_allow` list, the operation proceeds (`PathCheck::Allowed` or `PathCheck::AutoAllowed`).
4. If the path breaches the perimeter and is not on a `never_allow` list, execution halts. The system generates an `EscapeRequest` and routes it over the `EventBus`.
5. The frontend intercepts the request, prompts the user, and returns an `EscapeDecision` (`AllowOnce`, `AllowSession`, `AllowAlways`, or `Deny`).
6. The `EscapeHandler` records the decision and unblocks the execution thread.

## Quick Start

### Configuring a Mission Profile

You can build a custom boundary using `WorkspaceConfig` or rely on predefined profiles:

```rust
use astrid_workspace::profiles::WorkspaceProfile;

// Load the power-user profile which auto-allows standard developer directories
// like ~/.cargo or /usr/include while protecting the rest of the system.
let profile = WorkspaceProfile::power_user("/home/user/workspace");
let boundary = astrid_workspace::WorkspaceBoundary::new(profile.config);
```

### Managing Airlock Escapes

When a path requires approval, use the `EscapeHandler` to manage the request state over time:

```rust
use std::path::PathBuf;
use astrid_workspace::{EscapeHandler, EscapeRequest, EscapeOperation, EscapeDecision};

let mut handler = EscapeHandler::new();
let path = PathBuf::from("/external/data/metrics.csv");

// 1. Generate the request
let request = EscapeRequest::new(&path, EscapeOperation::Read, "Analyze external metrics")
    .with_tool("read_file")
    .with_server("mcp-filesystem");

// 2. Process the user's decision
handler.process_decision(&request, EscapeDecision::AllowAlways);

// 3. Subsequent checks will pass automatically
assert!(handler.is_allowed(&path));
```

## Development

To build and test the workspace crate specifically:

```bash
cargo test -p astrid-workspace --all-features
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
