# astrid-crypto

[![Crates.io](https://img.shields.io/crates/v/astrid-crypto)](https://crates.io/crates/astrid-crypto)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)

Cryptographic primitives for the Astrid secure agent runtime.

In the Astrid architecture, authorization is never delegated to the LLM. It relies strictly on
verifiable cryptography. `astrid-crypto` provides the foundational primitives that make this
possible: Ed25519 key management, BLAKE3 content hashing, and signature verification. These
primitives back capability tokens, audit chain links, and agent identity throughout the runtime.

## Core Features

- **Ed25519 key generation and signing** via `ed25519-dalek`, seeded from `OsRng`.
- **ZeroizeOnDrop memory protection** - the `SigningKey` is scrubbed from memory on drop; temporary
  buffers used during key import are explicitly zeroized immediately after use.
- **No `Clone` on `KeyPair`** - the private key cannot be silently duplicated; consumers must pass
  it by reference or explicitly export the secret bytes.
- **Leak-safe `Debug`** - `KeyPair` and `Signature` `Debug` impls print only a short hex prefix,
  never raw key or signature material.
- **BLAKE3 `ContentHash`** - 32-byte hash used for audit chain linking and content integrity checks,
  with a sentinel `zero()` value for genesis entries.
- **Consistent encoding** - `PublicKey`, `Signature`, and `ContentHash` all support hex and base64
  round-trips; Serde serializes `PublicKey` and `Signature` as base64 strings, `ContentHash` as hex.
- **`#![deny(unsafe_code)]`** - no unsafe blocks in the crate.

## Quick Start

Add the dependency:

```toml
[dependencies]
astrid-crypto = "0.2"
```

Generate a key pair, sign a message, and verify:

```rust
use astrid_crypto::{KeyPair, ContentHash};

// Generate an ephemeral key pair
let keypair = KeyPair::generate();

// Hash a payload
let message = b"capability:fs.read:/tmp";
let hash = ContentHash::hash(message);

// Sign and verify
let signature = keypair.sign(hash.as_bytes());
assert!(keypair.verify(hash.as_bytes(), &signature).is_ok());

// Export the public key for distribution
let pk = keypair.export_public_key();
println!("key_id={}", pk.key_id_hex()); // first 8 bytes, safe to log
```

Or import everything at once with the prelude:

```rust
use astrid_crypto::prelude::*;
```

## API Reference

### Key Types

| Type | Description |
|------|-------------|
| `KeyPair` | Ed25519 signing keypair. Not `Clone`. Private key is zeroized on drop. |
| `PublicKey` | 32-byte verifying key. `Clone`, `Copy`, `Hash`. Serializes as base64. |
| `Signature` | 64-byte Ed25519 signature. `Clone`, `Copy`. Serializes as base64. |
| `ContentHash` | 32-byte BLAKE3 hash. `Clone`, `Copy`. Serializes as hex. |
| `CryptoError` | `thiserror` enum covering length mismatches, bad encodings, and verification failures. |
| `CryptoResult<T>` | `Result<T, CryptoError>` alias. |

### `KeyPair`

```rust
// Construction
KeyPair::generate() -> KeyPair
KeyPair::from_secret_key(bytes: &[u8]) -> CryptoResult<KeyPair>

// Identity
keypair.public_key_bytes() -> &[u8; 32]
keypair.export_public_key() -> PublicKey
keypair.key_id() -> [u8; 8]          // first 8 bytes of public key
keypair.key_id_hex() -> String        // 16-char hex, safe to log

// Signing
keypair.sign(message: &[u8]) -> Signature
keypair.verify(message: &[u8], sig: &Signature) -> CryptoResult<()>

// Export (handle with care)
keypair.secret_key_bytes() -> [u8; 32]
```

### `PublicKey`

```rust
PublicKey::from_bytes(bytes: [u8; 32]) -> PublicKey
PublicKey::try_from_slice(slice: &[u8]) -> CryptoResult<PublicKey>
PublicKey::from_hex(s: &str) -> CryptoResult<PublicKey>
PublicKey::from_base64(s: &str) -> CryptoResult<PublicKey>
pk.verify(message: &[u8], sig: &Signature) -> CryptoResult<()>
pk.to_hex() -> String
pk.to_base64() -> String
pk.key_id_hex() -> String
```

### `ContentHash`

```rust
ContentHash::hash(data: &[u8]) -> ContentHash
ContentHash::zero() -> ContentHash      // sentinel for genesis/empty entries
ContentHash::from_bytes(bytes: [u8; 32]) -> ContentHash
ContentHash::try_from_slice(slice: &[u8]) -> Option<ContentHash>
ContentHash::from_hex(s: &str) -> Result<ContentHash, hex::FromHexError>
ContentHash::from_base64(s: &str) -> Result<ContentHash, base64::DecodeError>
hash.is_zero() -> bool
hash.to_hex() -> String
hash.to_base64() -> String
```

### `Signature`

```rust
Signature::from_bytes(bytes: [u8; 64]) -> Signature
Signature::try_from_slice(slice: &[u8]) -> CryptoResult<Signature>
Signature::from_hex(s: &str) -> CryptoResult<Signature>
Signature::from_base64(s: &str) -> CryptoResult<Signature>
sig.verify(message: &[u8], public_key: &[u8; 32]) -> CryptoResult<()>
sig.to_hex() -> String
sig.to_base64() -> String
sig.to_dalek() -> ed25519_dalek::Signature
```

## Development

```bash
cargo test -p astrid-crypto
```

The test suite covers key generation uniqueness, secret-key import round-trips, sign/verify
correctness, cross-key and wrong-message rejection, hex/base64 encoding round-trips for all three
types, and Serde JSON serialization for `ContentHash`.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the
[Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
