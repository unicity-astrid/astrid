# astralis-capabilities

Cryptographically signed authorization tokens for the Astralis secure agent runtime.

## Overview

This crate provides capability-based authorization using ed25519-signed tokens. Every token is cryptographically linked to the approval audit entry that created it, ensuring a verifiable chain of authorization.

## Features

- **Capability Tokens** - Ed25519-signed authorization tokens with audit linkage
- **Resource Patterns** - Glob-based matching for flexible resource scoping
- **Token Storage** - Session (in-memory) and persistent (SurrealDB) storage backends
- **Validation** - Token signature verification and authorization checking
- **Time Bounds** - Optional expiration for time-limited capabilities

## Security Model

Every capability token is:
- Signed by the runtime's ed25519 key
- Linked to the approval audit entry that created it
- Time-bounded (optional expiration)
- Scoped (session or persistent)

## Usage

```rust
use astralis_capabilities::{
    CapabilityToken, CapabilityStore, ResourcePattern, TokenScope, AuditEntryId,
};
use astralis_core::Permission;
use astralis_crypto::KeyPair;

// Create a capability store
let store = CapabilityStore::in_memory();

// Runtime key for signing
let runtime_key = KeyPair::generate();

// Create a capability token
let token = CapabilityToken::create(
    ResourcePattern::new("mcp://filesystem:*").unwrap(),
    vec![Permission::Invoke],
    TokenScope::Session,
    runtime_key.key_id(),
    AuditEntryId::new(),
    &runtime_key,
    None,
);

// Add to store
store.add(token).unwrap();

// Check capability
assert!(store.has_capability("mcp://filesystem:read_file", Permission::Invoke));
```

## Key Types

| Type | Description |
|------|-------------|
| `CapabilityToken` | Signed authorization token with scope and permissions |
| `CapabilityStore` | Storage backend for session and persistent tokens |
| `ResourcePattern` | Glob pattern for matching resource URIs |
| `TokenScope` | Session (memory) or Persistent (SurrealDB) scope |
| `CapabilityValidator` | Token validation and authorization checking |

## License

This crate is licensed under the MIT license.
