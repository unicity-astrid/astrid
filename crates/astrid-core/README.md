# astrid-core

[![Crates.io](https://img.shields.io/crates/v/astrid-core)](https://crates.io/crates/astrid-core)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Foundation types and security primitives for the Astralis secure agent runtime.

`astrid-core` is the bedrock of the Astralis OS workspace. It provides the essential abstractions and types that enforce security, type safety, and context isolation across all system components.

## Core Features

- **Context Isolation**: `ContextIdentifier` enforces per-user, per-environment isolation for secure capability boundaries and approval history.
- **Uplink Types**: Descriptors, capabilities, and message types for capsule-to-runtime communication.
- **Platform Types**: Approval requests, elicitation flows, and user input types used by capsule platforms.
- **Hook Events**: Lifecycle event types for the hook system.

## Core Concepts

### 1. Context Isolation

Astralis isolates context per-user and per-environment via `ContextIdentifier`, which combines the platform type, the specific channel or group, and the resolved user identity. This forms the foundation for secure capability boundaries and approval history.

### 2. Uplinks

Platforms are capsule uplinks - they connect to the runtime via `UplinkDescriptor` with declared capabilities (`UplinkCapabilities`). Messages flow through `InboundMessage`.

## Quick Start

`astrid-core` is designed to be consumed by other crates within the Astralis workspace.

```toml
[dependencies]
astrid-core = { workspace = true }
```

## API Reference

### Key Types

- **Input**: `MessageId`, `ContextIdentifier`, `FrontendType`
- **Platform**: `ApprovalRequest`, `ElicitationRequest`, `FrontendContext`, `UserInput`
- **Uplink**: `UplinkDescriptor`, `UplinkCapabilities`, `InboundMessage`
- **Primitives**: `AgentId`, `SessionId`, `TokenId`, `RiskLevel`, `Permission`

## Development

```bash
cargo test -p astrid-core
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
