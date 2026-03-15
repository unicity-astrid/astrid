# astrid-audit

[![Crates.io](https://img.shields.io/crates/v/astrid-audit)](https://crates.io/crates/astrid-audit)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Chain-linked cryptographic audit logging for Astrid.

Every security-relevant event in the Astrid runtime - MCP tool calls, file writes, capability issuance, user approvals, sub-agent spawns - is recorded as an Ed25519-signed entry that links to the hash of the entry before it. Any modification to a historical entry breaks the chain and is detectable. This crate does not make policy decisions; it is the incorruptible observer that makes everything else auditable.

## Core Features

- **Chain-linked integrity**: Every entry embeds a BLAKE3 hash of the preceding entry, creating a tamper-evident chain per session from genesis.
- **Ed25519 signatures**: The runtime signs each entry at creation time. The signing public key is embedded in the entry itself, so verification works across key rotations.
- **Tamper detection**: `verify_chain` checks three invariants - valid genesis, valid signatures, and unbroken hash links. Each failure is reported as a typed `ChainIssue`.
- **Rich action coverage**: 25+ auditable action variants covering MCP tool calls, file I/O, capability lifecycle, approvals, LLM requests, session events, elicitation, and security violations.
- **Privacy-preserving**: Tool call arguments are stored as BLAKE3 hashes, not raw content.
- **SurrealKV persistence**: Durable, session-indexed storage via `astrid-storage`. An in-memory backend is available for tests.

## Quick Start

```toml
[dependencies]
astrid-audit = "0.2.0"
```

```rust
use astrid_audit::{AuditLog, AuditAction, AuditOutcome, AuthorizationProof};
use astrid_core::SessionId;
use astrid_crypto::KeyPair;

let runtime_key = KeyPair::generate();
let log = AuditLog::in_memory(runtime_key);
let session_id = SessionId::new();

// Record an action
let entry_id = log.append(
    session_id.clone(),
    AuditAction::McpToolCall {
        server: "filesystem".to_string(),
        tool: "read_file".to_string(),
        args_hash: astrid_crypto::ContentHash::hash(b"..."),
    },
    AuthorizationProof::Capability {
        token_id: my_token_id,
        token_hash: my_token_hash,
    },
    AuditOutcome::success(),
)?;

// Verify chain integrity
let result = log.verify_chain(&session_id)?;
assert!(result.valid);
```

## API Reference

### Key Types

- `AuditLog` - main entry point; wraps storage and the runtime signing key. Open with `AuditLog::open(path, key)` for persistence or `AuditLog::in_memory(key)` for tests.
- `AuditEntry` - a single record: action, authorization proof, outcome, previous hash, embedded public key, and Ed25519 signature.
- `AuditAction` - enum of 25+ auditable event variants (`McpToolCall`, `FileWrite`, `CapabilityCreated`, `ApprovalGranted`, `SubAgentSpawned`, `SecurityViolation`, and more).
- `AuthorizationProof` - how the action was authorized: `User`, `Capability`, `UserApproval`, `NotRequired`, `System`, or `Denied`.
- `AuditOutcome` - `Success` or `Failure` with optional detail message.
- `ApprovalScope` - `Once`, `Session`, `Workspace`, or `Always`.
- `ChainVerificationResult` / `ChainIssue` - result of `verify_chain` or `verify_all`; issues are typed as `InvalidGenesis`, `InvalidSignature`, or `BrokenLink`.

### `AuditLog` Methods

| Method | Description |
|---|---|
| `append(session, action, proof, outcome)` | Sign and persist a new entry, updating the chain head. |
| `verify_chain(session_id)` | Verify all three chain invariants for a session. |
| `verify_all()` | Verify every session in the log. |
| `get(id)` | Retrieve a single entry by ID. |
| `get_session_entries(session_id)` | All entries for a session in order. |
| `list_sessions()` | All session IDs with recorded entries. |
| `count()` / `count_session(id)` | Entry counts across the whole log or one session. |
| `flush()` | Explicit flush (no-op for SurrealKV, which commits per write). |

## Development

```bash
cargo test -p astrid-audit -- --quiet
```

Changes to `AuditEntry::signing_data` or `AuditAction` serialization alter BLAKE3 outputs and invalidate all existing chain signatures. Treat those surfaces as stable API.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
