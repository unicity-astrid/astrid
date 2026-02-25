# astrid-core

[![Crates.io](https://img.shields.io/crates/v/astrid-core)](https://crates.io/crates/astrid-core)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Foundation types and security primitives for the Astralis secure agent runtime.

`astrid-core` is the bedrock of the Astralis OS workspace. It provides the essential abstractions, types, and traits that enforce security, type safety, and context isolation across all system components. Rather than reimplementing identity, input parsing, or error handling in every module, `astrid-core` establishes a unified interface that ensures verifiable execution from input reception to tool execution.

## Core Features

- **Unified Identity Management**: Maps transient, platform-specific user accounts (Discord, CLI, Telegram) to a canonical, cryptographic internal identity (`AstridUserId`).
- **Verifiable Input Attribution**: Every piece of input is encapsulated in a `TaggedMessage`, ensuring the runtime always knows exactly who initiated a request and in what context.
- **Universal Frontend Interface**: The `Frontend` trait standardizes how the system requests user approvals, performs elicitation, and relays status updates across any UI platform.
- **Standardized Security Errors**: The `SecurityError` enum provides a consistent, semantic error model for cryptographic failures, capability violations, and sandbox boundaries.
- **Connector Adapters**: Built-in adapter traits for bridging inbound and outbound messages between frontends and the core runtime.

## Core Concepts

### 1. Cross-Frontend Identity Management

Astralis is designed to serve users across multiple platforms simultaneously. `astrid-core` solves the identity fragmentation problem through a two-layer architecture:

- **Canonical Identity (`AstridUserId`)**: A UUID-based internal identifier, optionally bound to an ed25519 public key. This is the single source of truth for "who is this person" across the entire system.
- **Platform Links (`FrontendLink`)**: Binds a specific platform account to an `AstridUserId`. This enables memory continuity and unified audit trails regardless of where the user interacts from.

### 2. Input Classification and Context Isolation

Astralis isolates context per-user and per-environment. This isolation is enforced through `ContextIdentifier`, which combines the frontend type, the specific channel or group, and the resolved user identity. When wrapped in a `TaggedMessage`, this forms the foundation for secure capability boundaries and approval history.

### 3. The Frontend Trait

All UI implementations (CLI, Discord, Web) implement the `Frontend` trait. This defines the contract between the Astralis core and the user interface, standardizing critical flows:

- **Elicitation**: Used when an MCP server requires configuration, API keys, or credentials from the user.
- **Approvals**: Intercepting sensitive operations and requesting explicit authorization (e.g., Allow Once, Allow Always, Deny).
- **Verification**: Dispatching verification requests to users depending on the risk level and context.

## Quick Start

`astrid-core` is designed to be consumed by other crates within the Astralis workspace.

Add the dependency to your `Cargo.toml`:

```toml
[dependencies]
astrid-core = { workspace = true }
```

### The Frontend Trait Example
```rust
use astrid_core::{Frontend, FrontendContext, ApprovalRequest, ApprovalDecision};
use async_trait::async_trait;

struct MyCustomFrontend;

#[async_trait]
impl Frontend for MyCustomFrontend {
    fn get_context(&self) -> FrontendContext {
        // Return context boundaries
        unimplemented!()
    }

    async fn request_approval(&self, _request: ApprovalRequest) -> Result<ApprovalDecision, astrid_core::error::SecurityError> {
        // Render approval UI and await user decision
        unimplemented!()
    }
    
    // ... implement remaining required methods
}
```

## API Reference

The crate exposes its primary types through top-level exports for ergonomic imports.

### Key Types

- **Identity**: `AstridUserId`, `FrontendType`, `FrontendLink`
- **Input**: `TaggedMessage`, `MessageId`, `ContextIdentifier`
- **Frontend**: `Frontend`, `ApprovalRequest`, `ElicitationRequest`, `UserInput`
- **Errors**: `SecurityError`, `SecurityResult`
- **Primitives**: `AgentId`, `SessionId`, `TokenId`, `RiskLevel`, `Permission`

## Development

To run the test suite for this specific crate:

```bash
cargo test -p astrid-core
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
