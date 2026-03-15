# astrid-capabilities

[![Crates.io](https://img.shields.io/crates/v/astrid-capabilities)](https://crates.io/crates/astrid-capabilities)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Cryptographically signed, glob-scoped authorization tokens for the Astrid secure agent runtime.

This crate implements the core capability-based security model for Astrid. Every action an agent takes - whether invoking an MCP tool or accessing the filesystem - must be backed by a signed `CapabilityToken` linked to the specific user approval event that created it. Authorization cannot be forged, escalated, or replayed.

## Core Features

- **Ed25519 signatures**: Every token is signed by the runtime's key pair. Post-issuance tampering is detectable and rejected.
- **Glob-scoped resources**: Patterns like `mcp://filesystem:*` or `file:///home/user/**` use compiled glob matching. Exact URIs skip glob compilation entirely.
- **Path traversal rejection**: `..` segments are rejected at pattern creation and at match time, even when a glob would otherwise match.
- **Audit linkage**: Every token carries an `AuditEntryId` that references the approval event that authorized it.
- **Dual-tier storage**: Session tokens live in memory; persistent tokens survive restarts via SurrealKV. Both tiers share the same revocation and single-use tracking.
- **Replay protection**: Single-use tokens are marked consumed atomically on first use. Used-token state survives process restart.
- **Revocation**: Tokens can be revoked at any time. Revocation is persisted before in-memory state is updated, so it survives crashes.
- **Clock-skew tolerance**: Expiration checks accept a configurable skew window (default 30 seconds).

## Quick Start

```toml
[dependencies]
astrid-capabilities = "0.2"
```

```rust
use astrid_capabilities::{
    CapabilityToken, CapabilityStore, ResourcePattern, TokenScope,
    AuditEntryId, CapabilityValidator,
};
use astrid_core::Permission;
use astrid_crypto::KeyPair;

// Runtime signing key
let runtime_key = KeyPair::generate();

// Scope the capability to all tools on the filesystem MCP server
let pattern = ResourcePattern::new("mcp://filesystem:*").unwrap();

// Create a signed session token linked to an approval audit entry
let token = CapabilityToken::create(
    pattern,
    vec![Permission::Invoke],
    TokenScope::Session,
    runtime_key.key_id(),
    AuditEntryId::new(),
    &runtime_key,
    None, // no TTL - valid for the duration of the session
);

// Store and check
let store = CapabilityStore::in_memory();
store.add(token).unwrap();

let validator = CapabilityValidator::new(&store);
let result = validator.check("mcp://filesystem:read_file", Permission::Invoke);
assert!(result.is_authorized());
```

## API Reference

### Key Types

- **`CapabilityToken`** - A signed authorization token. Fields include `resource`, `permissions`, `scope`, `issued_at`, `expires_at`, `approval_audit_id`, and `single_use`. Call `validate()` to verify expiration and signature together.
- **`ResourcePattern`** - A URI pattern with glob support. Constructors: `new` (auto-detects glob), `exact`, `mcp_tool`, `mcp_server`, `file_dir`, `file_exact`. All constructors reject `..` path segments.
- **`CapabilityStore`** - Thread-safe token store. `in_memory()` for session use; `with_persistence(path)` or `with_kv_store(arc)` for durable storage. Core methods: `add`, `get`, `revoke`, `mark_used`, `use_token`, `has_capability`, `find_capability`, `list_tokens`, `cleanup_expired`.
- **`CapabilityValidator`** - Wraps a store and enforces issuer trust. `check(resource, permission)` returns `AuthorizationResult::Authorized { token }` or `RequiresApproval { resource, permission }`. Chain `trust_issuer(public_key)` to restrict accepted issuers.
- **`AuthorizationResult`** - The result of a `CapabilityValidator::check` call. `is_authorized()` and `token()` are the primary access points.
- **`AuditEntryId`** - A UUID wrapper linking a token to the approval event that created it.
- **`TokenScope`** - `Session` (in-memory only) or `Persistent` (written to KV store).
- **`DirHandle` / `FileHandle`** - UUID-based opaque handles for VFS directory and file references. Used to prevent guests from forging arbitrary paths.

### Resource Pattern Examples

| Pattern | Matches |
|---|---|
| `mcp://filesystem:read_file` | Exactly that one tool |
| `mcp://filesystem:*` | Any tool on the `filesystem` server |
| `mcp://*:read_*` | Any tool starting with `read_` on any server |
| `file:///home/user/**` | Any file or directory under `/home/user` |

### Persistence Behavior

Persistent tokens are re-validated (expiry + signature) on every read from the KV store as a defense-in-depth measure against disk tampering. Revocation and single-use markers are written to the KV store before in-memory state is updated, so both survive a crash-restart cycle.

## Development

```bash
cargo test -p astrid-capabilities
```

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
