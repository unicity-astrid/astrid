# astralis-audit

Chain-linked cryptographic audit logging for the Astralis secure agent runtime.

## Features

- **Cryptographically signed entries** - Every audit entry is signed by the runtime's ed25519 key
- **Chain-linked integrity** - Each entry contains the hash of the previous entry, providing tamper evidence
- **Persistent storage** - Uses SurrealDB (surrealkv) for durable audit trail storage
- **Chain verification** - Detect any modifications to historical entries
- **Session-indexed** - Entries are organized and queryable by session

## Security Model

Every audit entry is:
- Signed by the runtime's ed25519 key
- Linked to the previous entry via content hash
- Timestamped
- Indexed by session

The chain linking provides tamper evidence - any modification to historical entries breaks the chain and is detectable.

## Usage

```rust
use astralis_audit::{AuditLog, AuditAction, AuditOutcome, AuthorizationProof};
use astralis_core::SessionId;
use astralis_crypto::KeyPair;

// Create an in-memory audit log
let runtime_key = KeyPair::generate();
let user_id = runtime_key.key_id();
let log = AuditLog::in_memory(runtime_key).unwrap();

// Start a session
let session_id = SessionId::new();

// Record an action
let entry_id = log.append(
    session_id.clone(),
    AuditAction::SessionStarted {
        user_id,
        frontend: "cli".to_string(),
    },
    AuthorizationProof::System {
        reason: "session start".to_string(),
    },
    AuditOutcome::success(),
).unwrap();

// Verify chain integrity
let result = log.verify_chain(&session_id).unwrap();
assert!(result.valid);
```

## Key Types

- `AuditLog` - Main interface for recording and querying audit entries
- `AuditEntry` - A single signed, chain-linked audit record
- `AuditAction` - The action being audited (tool calls, approvals, etc.)
- `AuditOutcome` - Success or failure result of the action
- `AuthorizationProof` - How the action was authorized
- `AuditBuilder` - Fluent builder for constructing entries
- `ChainVerificationResult` - Result of chain integrity verification

## License

This crate is licensed under the MIT license.
