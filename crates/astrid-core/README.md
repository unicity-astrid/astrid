# astrid-core

[![Crates.io](https://img.shields.io/crates/v/astrid-core)](https://crates.io/crates/astrid-core)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Core types and traits for the Astrid secure agent runtime.

`astrid-core` is the shared vocabulary of the entire Astrid workspace. Every other crate depends on it. It defines the identifiers, approval primitives, uplink abstractions, elicitation protocol types, capsule ABI types, session authentication, directory layout, and the environment variable security policy that all enforcement points must use.

## Core Features

- **Stable identifier types** - `AgentId`, `SessionId`, `TokenId`, `UplinkId`, `AstridUserId`, and `Timestamp` with UUID backing, `Display` formatting, and full serde support.
- **Cross-platform identity** - `AstridUserId` is the canonical user identity across all frontends (CLI, Discord, Telegram, etc.), linked to platform-specific IDs via `FrontendLink`. Optional ed25519 public key field with base64 serde encoding.
- **Approval primitives** - `ApprovalRequest` / `ApprovalDecision` with five-tier `ApprovalOption` (once, session, workspace, always, deny) and `RiskLevel` ordering (`Low` < `Medium` < `High` < `Critical`). `Critical` risk additionally requires DM verification.
- **Uplink abstraction** - `UplinkDescriptor` (builder pattern), `UplinkCapabilities` (flag struct with `full()`, `notify_only()`, `receive_only()` presets), `UplinkProfile` (chat, interactive, notify, bridge), `UplinkSource` (native, WASM, OpenClaw) with validated capsule IDs, and `InboundMessage` (builder pattern). Hard limit of 32 uplinks per capsule via `MAX_UPLINKS_PER_CAPSULE`.
- **Elicitation protocol** - MCP server-initiated user input: `ElicitationRequest` / `ElicitationResponse` with text, secret, select, and confirm schemas. URL-mode elicitation (`UrlElicitationRequest` / `UrlElicitationResponse`) for OAuth and payment flows.
- **Capsule ABI types** - Rust mirrors of the `astrid:capsule@0.1.0` WIT records: `CapsuleAbiContext`, `CapsuleAbiResult`, `ToolInput`, `ToolOutput`, `ToolDefinition`, `HttpResponse`, `LogLevel`, `KeyValuePair`.
- **Session authentication** - `SessionToken`: 256-bit CSPRNG token, hex encode/decode, atomic write-then-rename to a 0o600 file, constant-time comparison via `subtle`. `HandshakeRequest` / `HandshakeResponse` for the Unix socket wire protocol.
- **Directory layout** - `AstridHome` resolves `~/.astrid/` (or `$ASTRID_HOME`) and exposes typed path accessors for every subdirectory and database path. `WorkspaceDir` detects and manages the per-project `.astrid/` directory, including workspace UUID generation.
- **Environment variable security policy** - `is_blocked_spawn_env` enforces a shared blocklist of vars (library injection, proxy interception, CA trust override, language-specific code injection, etc.) that must never be set by untrusted configuration on spawned child processes.
- **Retry configuration** - `RetryConfig` with exponential backoff, configurable jitter, and named presets (`fast()`, `network()`, `api()`).

## Quick Start

`astrid-core` is an internal workspace crate, consumed by other crates in the Astrid workspace.

```toml
[dependencies]
astrid-core = { workspace = true }
```

Import the prelude for convenient access to the most commonly used types:

```rust
use astrid_core::prelude::*;
```

## API Reference

### Key Types

**Identifiers**

| Type | Description |
|------|-------------|
| `AgentId` | UUID-backed agent instance identifier, displayed as `agent:<uuid>` |
| `SessionId` | UUID-backed session identifier; `SessionId::SYSTEM` is the kernel's well-known nil UUID |
| `TokenId` | UUID-backed capability token identifier |
| `UplinkId` | Opaque UUID for a registered uplink |
| `AstridUserId` | Canonical cross-platform user identity with optional ed25519 public key |
| `Timestamp` | `DateTime<Utc>` wrapper with `is_past()`, `is_future()`, and ISO 8601 display |

**Approval and Risk**

| Type | Description |
|------|-------------|
| `RiskLevel` | Ordered enum: `Low`, `Medium`, `High`, `Critical`; `High`+ requires approval |
| `Permission` | `Read`, `Write`, `Execute`, `Delete`, `Invoke`, `List`, `Create` |
| `ApprovalRequest` | Builder-style request with risk level, resource, and options |
| `ApprovalOption` | `AllowOnce`, `AllowSession`, `AllowWorkspace`, `AllowAlways`, `Deny` |
| `ApprovalDecision` | Response to an approval request; `creates_capability()` for `AllowAlways` decisions |

**Uplink**

| Type | Description |
|------|-------------|
| `UplinkDescriptor` | Immutable uplink registration (id, name, platform, source, capabilities, profile, metadata) |
| `UplinkCapabilities` | Capability flags: `can_receive`, `can_send`, `can_approve`, `can_elicit`, `supports_rich_media`, `supports_threads`, `supports_buttons` |
| `UplinkProfile` | `Chat`, `Interactive`, `Notify`, `Bridge` |
| `UplinkSource` | `Native`, `Wasm { capsule_id }`, `OpenClaw { capsule_id }` - validated IDs on `new_wasm()` / `new_openclaw()` |
| `InboundMessage` | Message arriving from a frontend uplink with platform, user, content, context, and optional thread ID |
| `UplinkError` | `NotConnected`, `SendFailed`, `InvalidCapsuleId`, `UnsupportedOperation` |

**Elicitation**

| Type | Description |
|------|-------------|
| `ElicitationRequest` | MCP server-initiated input request (text, secret, select, confirm) |
| `ElicitationSchema` | `Text`, `Secret`, `Select { options, multiple }`, `Confirm { default }` |
| `ElicitationResponse` | `Submit { value }`, `Cancel`, `Dismiss` |
| `UrlElicitationRequest` | Redirect-based flow for OAuth, payments, credentials, generic external |
| `UrlElicitationResponse` | Completion status and optional callback data |

**Capsule ABI**

| Type | Description |
|------|-------------|
| `CapsuleAbiContext` | Hook invocation context (event, session_id, user_id, data payload) |
| `CapsuleAbiResult` | Hook result with action directive and optional data payload |
| `ToolInput` / `ToolOutput` | Tool invocation arguments and results |
| `ToolDefinition` | Tool metadata with name, description, and JSON Schema input spec |
| `HttpResponse` | HTTP response from host (status, headers as `KeyValuePair`, body) |
| `LogLevel` | `Trace`, `Debug`, `Info`, `Warn`, `Error` |

**Session and Auth**

| Type | Description |
|------|-------------|
| `SessionToken` | 256-bit CSPRNG token; atomic 0o600 file write; constant-time `ct_eq`; `Debug` is redacted |
| `HandshakeRequest` / `HandshakeResponse` | Wire protocol types for Unix socket authentication |
| `PROTOCOL_VERSION` | Current wire protocol version constant |

**Directory Layout**

| Type | Description |
|------|-------------|
| `AstridHome` | Resolves and manages `~/.astrid/` with typed accessors for all subdirectories and database paths |
| `WorkspaceDir` | Detects (`.astrid/`, `.git`, `ASTRID.md` sentinel walk) and manages per-project `.astrid/` including workspace UUID |

**Security and Retry**

| Type / Function | Description |
|-----------------|-------------|
| `is_blocked_spawn_env(key)` | Returns `true` if the env var must not be set by untrusted config on spawned processes |
| `RetryConfig` | Exponential backoff with jitter; presets `fast()`, `network()`, `api()` |

**Identity**

| Type / Function | Description |
|-----------------|-------------|
| `FrontendLink` | Maps a platform-specific user ID to an `AstridUserId`; stored as `link/{platform}/{platform_user_id}` |
| `normalize_platform(name)` | Trim and lowercase a platform name - the canonical normalization for all uplinks |

## Development

```bash
cargo test -p astrid-core
```

## License

Dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
