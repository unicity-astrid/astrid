# astralis-core

Foundation types and traits for the Astralis secure agent runtime SDK.

## Overview

This crate provides the core abstractions that all other Astralis crates build upon. It defines the fundamental types for security operations, identity management, and frontend integration.

## Features

- **Error Types**: `SecurityError` and `SecurityResult` for consistent error handling across the SDK
- **Input Classification**: `TaggedMessage` for attributing messages to their source with context
- **Identity Management**: `AstralisUserId` for unified user identity across frontends (CLI, Discord, Web, etc.)
- **Frontend Trait**: Abstract interface for UI implementations
- **Common Types**: `SessionId`, `Permission`, `RiskLevel`, `AgentId`, `TokenId`, `Timestamp`
- **Version Management**: `Version` and `Versioned` traits for state migrations
- **Retry Utilities**: `RetryConfig` with exponential backoff for resilient operations

## The Frontend Trait

The `Frontend` trait is the primary abstraction for UI implementations. All frontends (CLI, Discord, Web, etc.) implement this trait to handle:

- **Elicitation**: MCP servers requesting user input
- **URL Elicitation**: OAuth flows, credential collection, payment confirmations
- **Approval Requests**: Sensitive operation authorization with options (Allow Once, Allow Always, Allow Session, Deny)
- **Status/Error Display**: User feedback mechanisms
- **Input Reception**: Receiving user commands and responses

```rust
use astralis_core::{Frontend, ApprovalRequest, ApprovalDecision};

// Implement Frontend for your UI
struct MyFrontend;

#[async_trait::async_trait]
impl Frontend for MyFrontend {
    // ... implement required methods
}
```

## Key Exports

```rust
// Errors
pub use SecurityError, SecurityResult;

// Frontend
pub use Frontend, ApprovalRequest, ApprovalDecision, ApprovalOption;
pub use ElicitationRequest, ElicitationResponse, UrlElicitationRequest;

// Identity
pub use AstralisUserId, FrontendType, FrontendLink;

// Input
pub use TaggedMessage, MessageId, ContextIdentifier;

// Common types
pub use SessionId, Permission, RiskLevel, AgentId, TokenId, Timestamp;

// Utilities
pub use RetryConfig, RetryOutcome, Version, Versioned;
```

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
astralis-core = { path = "../astralis-core" }
```

## License

This crate is licensed under the MIT license.
