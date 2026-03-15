# astrid-prelude

[![Crates.io](https://img.shields.io/crates/v/astrid-prelude)](https://crates.io/crates/astrid-prelude)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Unified prelude for the Astrid secure agent runtime.

Astrid is deliberately split into isolated, domain-specific crates to enforce strict security and
capability boundaries. `astrid-prelude` aggregates the public API surface of all nine foundational
crates into a single glob import, giving high-level consumers (daemons, CLI frontends, integration
tests) ergonomic access without compromising the underlying isolation. The crate contains zero
runtime logic; it is a pure re-export layer with no overhead.

## Core Features

- **Single import, full surface**: one `use astrid_prelude::*` brings in types from audit,
  capabilities, core, crypto, events, hooks, MCP, telemetry, and workspace.
- **Zero overhead**: no logic, no allocations, no initialization - the compiler erases the
  indirection entirely.
- **Granular fallback**: use individual crate preludes (`astrid_core::prelude::*`) when you need
  minimal dependency footprints in lower-level crates.
- **Strict lint enforcement**: `deny(unsafe_code)`, `deny(missing_docs)`,
  `deny(clippy::unwrap_used)`, and `deny(unreachable_pub)` are all enforced at the crate level.

## Quick Start

Add to your crate's `Cargo.toml` using workspace inheritance:

```toml
[dependencies]
astrid-prelude = { workspace = true }
```

Then bring the entire ecosystem into scope:

```rust
use astrid_prelude::*;

// astrid-core:         AgentId, SessionId, Permission, RiskLevel, UplinkDescriptor, ...
// astrid-crypto:       KeyPair, PublicKey, Signature, ContentHash, ...
// astrid-capabilities: CapabilityToken, CapabilityStore, ResourcePattern, TokenScope, ...
// astrid-audit:        AuditLog, AuditAction, AuditEntry, AuthorizationProof, ...
// astrid-mcp:          McpClient, SecureMcpClient, ServerConfig, ToolDefinition, ...
// astrid-events:       EventBus, EventReceiver, AstridEvent, EventMetadata
// astrid-hooks:        Hook, HookEvent, HookHandler, HookResult
// astrid-telemetry:    LogConfig, LogFormat, setup_logging, RequestContext, ...
// astrid-workspace:    SandboxCommand
```

If you only need types from one domain, prefer its individual prelude to keep your dependency tree
narrow:

```rust
use astrid_crypto::prelude::*;
use astrid_capabilities::prelude::*;
```

## API Reference

`astrid-prelude` exposes no types of its own. All symbols come from the preludes of the following
crates, re-exported verbatim.

### `astrid_audit`

| Symbol | Description |
|---|---|
| `AuditLog` | Append-only, hash-chained audit log (in-memory or persistent) |
| `AuditAction` | Enum of recordable runtime actions |
| `AuditEntry` | A single signed log entry |
| `AuditEntryId` | Unique identifier for a log entry |
| `AuditOutcome` | Success or failure result attached to an entry |
| `AuthorizationProof` | Cryptographic or system-level proof attached to each action |
| `ApprovalScope` | Scope of an approval gate |
| `ChainVerificationResult`, `ChainIssue` | Output of `AuditLog::verify_chain` |
| `AuditError`, `AuditResult` | Error and result types |

### `astrid_capabilities`

| Symbol | Description |
|---|---|
| `CapabilityToken` | Signed, scoped permission token |
| `CapabilityStore` | Runtime store for active tokens |
| `CapabilityValidator` | Validates tokens against requested permissions |
| `AuthorizationResult` | Outcome of a capability check |
| `ResourcePattern` | URI pattern matched against capability grants |
| `TokenScope` | Lifetime scope of a token (e.g., `Session`) |
| `CapabilityError`, `CapabilityResult` | Error and result types |

### `astrid_core`

| Symbol | Description |
|---|---|
| `AgentId`, `SessionId`, `TokenId` | Domain-specific identifiers |
| `Permission` | Permission enum used across capability and audit systems |
| `RiskLevel` | Risk classification for approval gates |
| `Timestamp` | Canonical timestamp type |
| `ApprovalDecision`, `ApprovalOption`, `ApprovalRequest` | Approval gate types |
| `RetryConfig` | Retry policy configuration |
| `UplinkDescriptor`, `UplinkProfile`, `UplinkCapabilities`, `UplinkSource`, `UplinkId` | Uplink configuration |
| `InboundMessage`, `UplinkError`, `UplinkResult` | Uplink runtime types |
| `ElicitationRequest`, `ElicitationResponse`, `ElicitationSchema`, `ElicitationAction` | MCP server-initiated input |
| `SelectOption`, `UrlElicitationRequest`, `UrlElicitationResponse`, `UrlElicitationType` | Elicitation detail types |

### `astrid_crypto`

| Symbol | Description |
|---|---|
| `KeyPair` | Ed25519 signing key pair with generate/sign/verify |
| `PublicKey` | Ed25519 public key |
| `Signature` | Ed25519 signature |
| `ContentHash` | Blake3 content hash |
| `CryptoError`, `CryptoResult` | Error and result types |

### `astrid_events`

| Symbol | Description |
|---|---|
| `EventBus` | Broadcast channel for runtime-wide events |
| `EventReceiver` | Subscription handle with lag detection |
| `AstridEvent` | Enum of all first-party runtime events |
| `EventMetadata` | Common metadata attached to every event |

### `astrid_hooks`

| Symbol | Description |
|---|---|
| `Hook` | A lifecycle hook binding an event to a handler |
| `HookEvent` | Lifecycle point where the hook fires (e.g., `PreToolCall`) |
| `HookHandler` | Handler definition (e.g., shell command) |
| `HookResult` | Result type for hook execution |

### `astrid_mcp`

| Symbol | Description |
|---|---|
| `McpClient` | Multi-server MCP client |
| `SecureMcpClient` | Capability-gated MCP client wrapper |
| `ServerManager` | Manages MCP server process lifecycles |
| `ToolAuthorization` | Authorization decision for a tool invocation |
| `ServerConfig`, `ServersConfig` | Server and multi-server configuration |
| `ToolDefinition`, `ToolContent`, `ToolResult` | Tool descriptor and invocation result |
| `McpError`, `McpResult` | Error and result types |

### `astrid_telemetry`

| Symbol | Description |
|---|---|
| `LogConfig` | Structured logging configuration |
| `LogFormat` | Output format (`Pretty`, `Json`) |
| `LogTarget` | Log destination |
| `setup_logging` | Initialize the global tracing subscriber |
| `RequestContext` | Per-request tracing span context |
| `TelemetryError`, `TelemetryResult` | Error and result types |

### `astrid_workspace`

| Symbol | Description |
|---|---|
| `SandboxCommand` | Command builder with workspace-scoped filesystem constraints |

## Development

This crate contains no logic. Correctness is enforced by the compiler: if a re-exported symbol
becomes private or is removed upstream, this crate fails to compile.

```bash
# Verify the prelude compiles cleanly
cargo check -p astrid-prelude

# Inspect the aggregated docs
cargo doc -p astrid-prelude --no-deps --open

# Run the full workspace test suite
cargo test --workspace -- --quiet
```

## License

Licensed under either the [MIT License](../../LICENSE-MIT) or the
[Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
