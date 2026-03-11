# astrid-core

[![Crates.io](https://img.shields.io/crates/v/astrid-core)](https://crates.io/crates/astrid-core)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Foundation types and security primitives for the Astralis secure agent runtime.

`astrid-core` is the bedrock of the Astralis OS workspace. It provides the essential abstractions and types that enforce security, type safety, and context isolation across all system components.

## Core Features

- **Unified Identity Management**: Maps transient, platform-specific user accounts (Discord, CLI, Web) to a canonical, cryptographic internal identity (`AstridUserId`).
- **Context Isolation**: `ContextIdentifier` enforces per-user, per-environment isolation for secure capability boundaries and approval history.
- **Uplink Types**: Descriptors, capabilities, and message types for capsule-to-runtime communication.
- **Frontend Types**: Approval requests, elicitation flows, and user input types used by capsule frontends.
- **Hook Events**: Lifecycle event types for the plugin hook system.

## Core Concepts

### 1. Cross-Frontend Identity Management

Astralis serves users across multiple platforms simultaneously. `astrid-core` solves identity fragmentation through a two-layer architecture:

- **Canonical Identity (`AstridUserId`)**: A UUID-based internal identifier, optionally bound to an ed25519 public key. Single source of truth for "who is this person" across the entire system.
- **Frontend Type (`FrontendType`)**: Identifies which platform a user is interacting from (Discord, CLI, Web, etc.).

### 2. Context Isolation

Astralis isolates context per-user and per-environment via `ContextIdentifier`, which combines the frontend type, the specific channel or group, and the resolved user identity. This forms the foundation for secure capability boundaries and approval history.

### 3. Uplinks

Frontends are capsule uplinks - they connect to the runtime via `UplinkDescriptor` with declared capabilities (`UplinkCapabilities`). Messages flow through `InboundMessage` and `OutboundMessage` types.

## Quick Start

`astrid-core` is designed to be consumed by other crates within the Astralis workspace.

```toml
[dependencies]
astrid-core = { workspace = true }
```

## API Reference

### Key Types

- **Identity**: `AstridUserId`, `FrontendType`
- **Input**: `MessageId`, `ContextIdentifier`
- **Frontend**: `ApprovalRequest`, `ElicitationRequest`, `FrontendContext`, `UserInput`
- **Uplink**: `UplinkDescriptor`, `UplinkCapabilities`, `InboundMessage`, `OutboundMessage`
- **Primitives**: `AgentId`, `SessionId`, `TokenId`, `RiskLevel`, `Permission`

## Development

```bash
cargo test -p astrid-core
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
