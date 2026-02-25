# astrid-crypto

[![Crates.io](https://img.shields.io/crates/v/astrid-crypto)](https://crates.io/crates/astrid-crypto)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Cryptographic primitives for the Astralis secure agent runtime.

In the Astralis architecture, authorization is never delegated to the LLM. Instead, it relies strictly on verifiable cryptography. `astrid-crypto` provides the foundational primitives that make this possible: Ed25519 key management, BLAKE3 content hashing, and signature verification. This crate acts as the security boundary for the runtime, ensuring that capability tokens, audit logs, and agent identities are mathematically enforced and protected against common attack vectors.

## Core Features

* **Secure Key Management**: Ed25519 keypairs with automatic `ZeroizeOnDrop` memory clearing for sensitive material.
* **Hardened Filesystem Operations**: Atomic key file creation with `0o600` permissions (on Unix) and protection against symlink attacks.
* **High-Performance Hashing**: BLAKE3-backed `ContentHash` for rapid audit chain linking and domain-separated data integrity.
* **Trusted Verification Registry**: `SignatureVerifier` manages trusted public keys by short `KeyId` for centralized and efficient authorization checks.
* **Serialization Ready**: Seamless Base64 and Hex encoding/decoding via Serde integration.

## Quick Start

```rust
use astrid_crypto::{KeyPair, ContentHash};

// Generate an ephemeral runtime key
let keypair = KeyPair::generate();

// Hash and sign a payload
let message = b"system_instruction:terminate";
let hash = ContentHash::hash(message);
let signature = keypair.sign(hash.as_bytes());

// Verify the cryptographic authorization
assert!(keypair.verify(hash.as_bytes(), &signature).is_ok());
```

## Architecture and Security Model

`astrid-crypto` is designed defensively to protect against misconfigurations and system-level attacks.

### Memory and Storage Isolation
The `KeyPair` struct strictly guards the private `SigningKey`. It does not implement `Clone`, forcing developers to pass it by reference or load separate instances via `load_or_generate_pair`. All file read buffers are wrapped in `Zeroizing` containers to ensure sensitive material is scrubbed from memory immediately after parsing.

### File-System Attack Mitigation
The `load_or_generate` functions eliminate Time-of-Check to Time-of-Use (TOCTOU) vulnerabilities by utilizing `O_CREAT | O_EXCL` operations for atomic file creation. Before any key file is read, the crate inspects file metadata to explicitly reject symlinks, preventing attackers from redirecting key reads or writes to unauthorized locations.

### Domain-Separated Hashing
Astralis relies on `ContentHash` (backed by BLAKE3) for cryptographic links in audit trails and capability identifiers. The `hash_with_domain` function derives a specific hasher key based on a contextual domain string. This guarantees that a hash generated for one subsystem is mathematically distinct from identical data hashed in another subsystem.

### Centralized Verification
Instead of passing raw public keys through the system, Astralis components utilize `SignatureVerifier`. This registry allows components to refer to trusted keys by an 8-byte `KeyId` (the first 8 bytes of the public key). This abstracts key distribution away from the authorization logic, allowing the runtime to evaluate "Is this signature trusted?" rather than "Is this signature from this specific key?".

## API Reference

The crate exposes a minimal, focused public API:
* `KeyPair`: Generation, secure storage, and signing.
* `PublicKey`: Serialization-friendly verification key.
* `ContentHash`: 32-byte BLAKE3 hash with zero-hash and domain support.
* `Signature`: Strongly typed wrapper around Ed25519 signatures.
* `SignatureVerifier`: Collection of trusted keys mapped by `KeyId`.
* `CryptoError`: Unified `thiserror` enumeration for cryptographic and IO failures.

## Development

To run the unit tests, including the filesystem security checks:

```bash
cargo test -p astrid-crypto
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
