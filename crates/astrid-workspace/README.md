# astrid-workspace

[![Crates.io](https://img.shields.io/crates/v/astrid-workspace)](https://crates.io/crates/astrid-workspace)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Operational workspace boundaries and host-level process sandboxing for the Astrid secure agent runtime.

This crate defines the semantic, physical boundaries of where an agent is authorized to operate on
the host machine, and provides OS-native process sandboxing (Linux `bwrap`, macOS Seatbelt) to
enforce those boundaries on spawned shell processes. It works alongside the WASM execution sandbox
(Capsules) rather than replacing it: WASM isolation is inescapable by design; workspace boundaries
are escapable with user approval.

## Core Features

- **Path boundary enforcement**: Pre-compiled glob matching, canonical path resolution, and symlink
  evaluation to prevent directory traversal and unauthorized host access.
- **Escape/airlock protocol**: Structured `EscapeRequest` and `EscapeDecision` flows with
  session-scoped and permanent approval memory.
- **Three operational modes**: `Safe` (always ask), `Guided` (smart defaults), and `Autonomous`
  (unrestricted). Default is `Safe`.
- **Built-in profiles**: Predefined configurations for `safe`, `power_user`, `autonomous`, and `ci`
  environments.
- **OS-native process sandboxing**: `SandboxCommand` and `ProcessSandboxConfig` wrap spawned
  processes in `bwrap` (Linux) or `sandbox-exec`/Seatbelt (macOS), restricting filesystem writes
  to the worktree and overlaying hidden paths with empty tmpfs.
- **Injection-safe profile generation**: All paths are validated for absolute form, valid UTF-8, and
  absence of SBPL-dangerous characters (`"`, `\`, `\0`) before being interpolated into sandbox
  profiles.
- **Session-scoped Git worktrees**: `ActiveWorktree` creates a per-session branch and worktree,
  auto-commits any WIP on drop, then removes the physical directory.

## Architecture: Boundary vs. Sandbox

| Aspect | Workspace Boundary | Execution Sandbox (WASM/Capsules) |
|--------|-------------------|-----------------------------------|
| **Domain** | Semantic filesystem access and host operations | CPU instructions, memory, panic isolation |
| **Escapable** | Yes, via approval airlock | No, inescapable by design |
| **Control** | User-defined per profile | Hardcoded by Astrid kernel architecture |
| **Purpose** | Prevents unauthorized host modification | Prevents malicious code from exploiting the runtime |

## Quick Start

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
astrid-workspace = "0.2"
```

### Wrapping a shell process in the OS sandbox

`SandboxCommand` is the simplest path: pass it a `std::process::Command` and the path the process
may write to.

```rust,ignore
use astrid_workspace::SandboxCommand;
use std::process::Command;

let inner = Command::new("npm");
// inner.args(["install"]).current_dir("/home/user/project");

// On Linux this prepends bwrap; on macOS it prepends sandbox-exec.
let sandboxed = SandboxCommand::wrap(inner, "/home/user/project".as_ref())?;
```

### Building a configurable sandbox prefix

Use `ProcessSandboxConfig` when you need a different `Command` type (e.g., `tokio::process::Command`)
or want fine-grained control over extra read/write paths and hidden paths.

```rust,ignore
use astrid_workspace::ProcessSandboxConfig;

let config = ProcessSandboxConfig::new("/home/user/project")
    .with_network(true)
    .with_extra_read("/usr/local/share/myapp")
    .with_hidden("/home/user/.astrid");

if let Some(prefix) = config.sandbox_prefix()? {
    let mut cmd = tokio::process::Command::new(&prefix.program);
    cmd.args(&prefix.args);
    // Append the real program and its args after the sandbox prefix.
    cmd.arg("npx").arg("@anthropic/mcp-server-filesystem");
}
```

## API Reference

### Public Types

Only the sandbox module is part of the public API. Boundary checking, escape handling, and worktree
management are internal to the crate and consumed by higher-level Astrid crates.

#### `SandboxCommand`

A one-shot helper that wraps a `std::process::Command` in the OS-native sandbox and returns the
wrapped `Command`.

```rust,ignore
pub fn wrap(inner_cmd: Command, worktree_path: &Path) -> io::Result<Command>
```

Returns `Err` if `worktree_path` is relative, non-UTF-8, or contains `"`, `\`, or `\0`.

#### `ProcessSandboxConfig`

A builder for data-oriented sandbox configuration. Produces a `SandboxPrefix` (program + args
vector) rather than wrapping a `Command` directly, allowing use with any async or alternate
`Command` type.

| Method | Description |
|--------|-------------|
| `new(writable_root)` | Create config with a single writable root |
| `with_network(bool)` | Allow or deny network access (default: allow) |
| `with_extra_read(path)` | Add a read-only path beyond OS defaults |
| `with_extra_write(path)` | Add an additional writable path |
| `with_hidden(path)` | Overlay a path with empty tmpfs (Linux) or deny-rule (macOS) |
| `sandbox_prefix()` | Build and return `Some(SandboxPrefix)` on Linux/macOS, `None` elsewhere |

#### `SandboxPrefix`

The raw program and args vector produced by `ProcessSandboxConfig::sandbox_prefix()`. Caller appends
the inner command after these args.

```rust,ignore
pub struct SandboxPrefix {
    pub program: OsString,  // e.g. "bwrap" or "sandbox-exec"
    pub args: Vec<OsString>,
}
```

### Internal Modules (not public API)

These modules exist but are only accessible within the crate and to higher-level Astrid crates that
depend on it:

| Module | Description |
|--------|-------------|
| `boundaries` | `WorkspaceBoundary` and `PathCheck` - path evaluation against workspace config |
| `config` | `WorkspaceConfig`, `WorkspaceMode`, `EscapePolicy`, `AutoAllowPaths` |
| `escape` | `EscapeRequest`, `EscapeDecision`, `EscapeHandler`, `EscapeFlow` |
| `profiles` | `WorkspaceProfile` and built-in profiles (`safe`, `power_user`, `autonomous`, `ci`) |
| `worktree` | `ActiveWorktree` - RAII Git worktree per agent session |

## Development

```bash
cargo test -p astrid-workspace
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the
[Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
