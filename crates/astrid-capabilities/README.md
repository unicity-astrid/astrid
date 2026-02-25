# astrid-capabilities

[![Crates.io](https://img.shields.io/crates/v/astrid-capabilities)](https://crates.io/crates/astrid-capabilities)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Cryptographically signed, glob-scoped authorization tokens for the Astralis runtime.

This crate implements the core capability-based security model for Astralis. It ensures that every action taken by an agent—whether invoking a Model Context Protocol (MCP) tool or accessing the local filesystem—is explicitly authorized by a verifiable token. Rather than relying on implicit permissions or ambient authority, `astrid-capabilities` requires a cryptographically signed token linked directly to a user approval event. This guarantees a verifiable chain of authorization that cannot be forged, bypassed, or escalated.

## Core Features

* **Cryptographic Verification**: The runtime signs all capability tokens using Ed25519 keys, preventing tampering or forgery by compromised agents.
* **Precise Resource Scoping**: Supports exact URI matches and glob patterns (e.g., `mcp://filesystem:*`, `file:///home/user/**`) with strict, built-in path traversal prevention.
* **Audit Linkage**: Every token immutably references the specific `AuditEntryId` of the user approval that generated it, maintaining a perfect forensic trail.
* **Flexible Storage Backends**: Unified interface for ephemeral session tokens (in-memory) and durable persistent tokens (backed by SurrealKV).
* **Replay & Revocation Protection**: Built-in support for single-use tokens, used-token tracking, and explicit revocation lists.
* **Time-Bounded Access**: Expiration enforcement with configurable clock-skew tolerance.

## Architecture & Security Model

The security model assumes that agent runtimes are untrusted and must prove they have the authority to execute specific actions. 

When a user approves an action, the system generates a `CapabilityToken`. This token contains the exact resource scope, the granted permissions, the expiration time, and the UUID of the approval audit entry. The system then hashes this data and signs it using the runtime's private key. 

At the point of resource access, the `CapabilityValidator` intercepts the request, retrieves the relevant tokens from the `CapabilityStore`, verifies the Ed25519 signatures, checks for expiration or revocation, and confirms the resource requested matches the token's allowed glob pattern.

## Quick Start

The following example demonstrates creating a token, persisting it to memory, and validating a subsequent access request.

```rust
use astrid_capabilities::{
    CapabilityToken, CapabilityStore, ResourcePattern, TokenScope, AuditEntryId,
    TokenBuilder, CapabilityValidator
};
use astrid_core::Permission;
use astrid_crypto::KeyPair;
use std::time::Duration;

// 1. Initialize storage and the runtime cryptographic identity
let store = CapabilityStore::in_memory();
let runtime_key = KeyPair::generate();

// 2. Define the exact scope of the capability
let pattern = ResourcePattern::new("mcp://filesystem:*").expect("valid pattern");

// 3. Generate a signed token linked to an audit event
let token = TokenBuilder::new(pattern)
    .permission(Permission::Invoke)
    .session()
    .ttl(chrono::Duration::hours(1))
    .build(runtime_key.key_id(), AuditEntryId::new(), &runtime_key);

// 4. Store the authorized capability
store.add(token).unwrap();

// 5. Validate authorization at the point of use
let validator = CapabilityValidator::new(&store);
let auth_result = validator.check("mcp://filesystem:read_file", Permission::Invoke);

assert!(auth_result.is_authorized());
```

## Resource Patterns

`astrid-capabilities` uses a custom URI scheme to define access boundaries. Resources follow either an MCP tool format (`mcp://server:tool`) or a filesystem format (`file://path`). 

The `ResourcePattern` type parses these URIs and supports glob matching:

* `mcp://filesystem:read_file` - Exact match for a single tool.
* `mcp://filesystem:*` - Matches any tool hosted by the `filesystem` server.
* `mcp://*:read_*` - Matches any tool starting with `read_` across all servers.
* `file:///home/user/**` - Matches any file or directory under `/home/user`.

### Path Traversal Protection

To prevent directory traversal attacks, the `ResourcePattern` constructor and its matching engine actively reject any pattern or resource URI containing `..` segments. Even if a glob pattern like `file:///home/user/**` is authorized, a subsequent check against `file:///home/user/../../etc/passwd` will deterministically fail.

## Storage & Persistence

The `CapabilityStore` abstracts token persistence across two tiers:

1. **Session Scope**: Ephemeral tokens stored in memory. These are destroyed when the application restarts or when `clear_session` is invoked.
2. **Persistent Scope**: Durable tokens persisted to disk using the `astrid-storage` crate (backed by SurrealKV). 

Regardless of the storage tier, the store maintains synchronized state for revoked tokens and single-use token consumption, preventing replay attacks across both memory and disk boundaries.

## Development

To run the test suite for this specific crate:

```bash
cargo test -p astrid-capabilities
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
