# astrid-prelude

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The unified import for Astrid OS kernel-side types.**

In the OS model, kernel crates (`astrid-kernel`, `astrid-approval`, `astrid-cli`) need types from across the kernel surface: crypto primitives, capability tokens, audit entries, event bus handles, MCP definitions, hook types, telemetry config, and workspace boundaries. Importing nine crate preludes individually is tedious and produces import blocks that obscure the actual code.

This crate re-exports the public prelude of every foundational kernel crate into a single `use astrid_prelude::*`. It contains zero runtime logic. The compiler erases the indirection entirely.

## What it re-exports

| Source crate | What you get |
|---|---|
| `astrid-core` | `AgentId`, `SessionId`, `Permission`, `RiskLevel`, `ApprovalDecision`, `ElicitationRequest` |
| `astrid-crypto` | `KeyPair`, `PublicKey`, `Signature`, `ContentHash` |
| `astrid-capabilities` | `CapabilityToken`, `CapabilityStore`, `CapabilityValidator`, `ResourcePattern`, `TokenScope` |
| `astrid-audit` | `AuditLog`, `AuditEntry`, `AuditAction`, `AuthorizationProof`, `ChainVerificationResult` |
| `astrid-events` | `EventBus`, `EventReceiver`, `AstridEvent`, `EventMetadata` |
| `astrid-hooks` | `Hook`, `HookEvent`, `HookHandler`, `HookResult` |
| `astrid-mcp` | `McpClient`, `SecureMcpClient`, `ServerManager`, `ServerConfig`, `ToolDefinition` |
| `astrid-telemetry` | `LogConfig`, `LogFormat`, `setup_logging`, `RequestContext` |
| `astrid-workspace` | `SandboxCommand` |

## When to use it

Use `astrid-prelude` in crates that already depend on most of these transitively: the kernel, the CLI, integration tests. It saves import noise without adding new dependencies.

Do not use it in leaf crates that only need one or two upstream types. Import `astrid_core::prelude::*` or `astrid_crypto::prelude::*` directly to keep dependency trees narrow.

## Quick start

```toml
[dependencies]
astrid-prelude = { workspace = true }
```

```rust
use astrid_prelude::*;

// Full kernel type surface available:
// KeyPair, CapabilityToken, AuditEntry, EventBus, McpClient, ...
```

## Lint policy

Enforces `deny(unsafe_code)`, `deny(missing_docs)`, `deny(clippy::unwrap_used)`, `deny(unreachable_pub)`. This crate is pure re-exports, so these lints protect against accidental logic creep.

## Development

```bash
cargo test -p astrid-prelude
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
