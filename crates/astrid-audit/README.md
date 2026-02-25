# astrid-audit

[![Crates.io](https://img.shields.io/crates/v/astrid-audit)](https://crates.io/crates/astrid-audit)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Cryptographically verified, chain-linked audit logging for the Astralis OS.

In a multi-agent operating system, trust is derived from transparency. `astrid-audit` is the system of record for Astralis OS. It provides an immutable, cryptographically signed, and chain-linked audit log of every security-relevant event—from LLM tool calls and file system operations to user approvals and capability issuance. 

By weaving a continuous hash chain over Ed25519-signed entries, it guarantees tamper-evident historical integrity. If a process, sub-agent, or external actor attempts to modify the log, the chain breaks. This crate does not execute tasks; it serves as the universal, incorruptible observer.

## Core Features

* **Chain-Linked Integrity**: Every entry contains a BLAKE3 content hash of the previous entry in the session, creating an unbroken chain from genesis.
* **Cryptographic Signatures**: The Astralis runtime signs every entry using its Ed25519 key, ensuring non-repudiation and proof of origin.
* **Tamper Evidence**: Built-in verification mechanisms immediately detect broken links, invalid signatures, or unauthorized genesis entries.
* **Persistent Storage**: Integrates with `SurrealKV` for durable, session-indexed storage and rapid retrieval.
* **Rich Context Logging**: Captures granular telemetry including authorization proofs, action outcomes, and session lineage without exposing raw sensitive data (utilizing argument hashing).

## Architecture

`astrid-audit` operates as a foundational gear within the Astralis runtime.

1. **The Entry**: Defined by `AuditEntry`, each record combines session metadata, the specific `AuditAction` (e.g., `McpToolCall`, `CapabilityCreated`), the `AuthorizationProof` (why the action was allowed), and the `AuditOutcome`.
2. **The Chain**: Modeled as a directed acyclic graph grouped by `SessionId`. When an entry is appended via `AuditLog::append`, the system retrieves the current chain head from storage, computes the previous entry's BLAKE3 hash, signs the new entry payload, and updates the head.
3. **The Storage**: Interacts with the workspace `astrid-storage` crate, using a specialized `SurrealKvAuditStorage` implementation to map `AuditEntryId` to serialized entries and maintain session indexes.

## Quick Start

The easiest way to interact with the audit log during testing or internal development is via the `AuditBuilder`.

```rust
use astrid_audit::{AuditLog, AuditBuilder, AuditAction, AuthorizationProof};
use astrid_core::SessionId;
use astrid_crypto::KeyPair;

// Initialize the log with the runtime's cryptographic key
let runtime_key = KeyPair::generate();
let log = AuditLog::in_memory(runtime_key);
let session_id = SessionId::new();

// Record a successful system action
let entry_id = AuditBuilder::new(&log, session_id)
    .action(AuditAction::ServerStarted {
        name: "filesystem-mcp".to_string(),
        transport: "stdio".to_string(),
        binary_hash: None,
    })
    .authorization(AuthorizationProof::System {
        reason: "system initialization".to_string(),
    })
    .success()
    .unwrap();
```

### Recording Events

Most events are recorded via the `AuditBuilder` or by calling `AuditLog::append` directly. Every action must be accompanied by an `AuthorizationProof` and an `AuditOutcome`.

```rust
use astrid_audit::{AuditLog, AuditAction, AuditOutcome, AuthorizationProof};
use astrid_core::SessionId;

// Assuming `log` and `session_id` are already in scope...
let entry_id = log.append(
    session_id.clone(),
    AuditAction::ContextSummarized {
        evicted_count: 50,
        tokens_freed: 12000,
    },
    AuthorizationProof::System {
        reason: "context window optimization".to_string(),
    },
    AuditOutcome::success(),
).unwrap();
```

### Verifying Chain Integrity

The audit log's primary value is its provable integrity. You can verify a specific session's chain or the entire system log to ensure no historical tampering has occurred.

```rust
// Verify a single session's integrity
let verification = log.verify_chain(&session_id).unwrap();

if verification.valid {
    println!("Chain intact. Verified {} entries.", verification.entries_verified);
} else {
    for issue in verification.issues {
        eprintln!("Integrity violation: {}", issue);
    }
}
```

Verification enforces three rigid invariants:
1. The genesis entry has a zeroed previous hash.
2. Every signature is mathematically valid against the runtime's public key.
3. Every entry's previous hash exactly matches the computed BLAKE3 content hash of the preceding entry in the sequence.

## Development

To test this crate locally:

```bash
cargo test -p astrid-audit --all-features
```

As a core security component, changes to `astrid-audit` require strict scrutiny. Any modifications to `AuditEntry::signing_data` or `AuditAction` serialization must be backwards compatible or accompanied by a migration strategy, as changes will alter BLAKE3 outputs and invalidate historical chain signatures.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.