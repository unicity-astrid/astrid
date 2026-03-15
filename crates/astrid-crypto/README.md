# astrid-crypto

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The cryptographic foundation that makes authorization math, not hope.**

In the OS model, this is the kernel's cryptographic subsystem. Every capability token, every audit chain link, every agent identity assertion ultimately bottoms out in the primitives this crate provides: Ed25519 signatures via `ed25519-dalek`, BLAKE3 content hashing via `blake3`, and secure key lifecycle via `zeroize`. No other crate in the workspace touches raw key material.

## What this crate provides

**`KeyPair`** generates Ed25519 signing keys from `OsRng`. The `SigningKey` is `ZeroizeOnDrop`, scrubbed from memory when the struct drops. `KeyPair` deliberately does not implement `Clone`. You cannot silently duplicate a private key. Pass by reference or explicitly export the secret bytes. `Debug` prints only the first 8 bytes of the public key as hex, never raw material. `from_secret_key` zeroizes its temporary buffer immediately after constructing the `SigningKey`.

**`PublicKey`** is 32 bytes, `Clone`, `Copy`, `Hash`. Serializes as base64 via serde. Supports hex and base64 round-trips. `key_id_hex()` returns the first 8 bytes as hex for safe logging. Verifies signatures independently of the `KeyPair` that produced them, which is how the audit log verifies entries signed by rotated keys.

**`Signature`** is 64 bytes, `Clone`, `Copy`. Serializes as base64. `Debug` prints only a short hex prefix. Wraps `ed25519-dalek::Signature` with `from_bytes`, `to_dalek`, and standalone `verify(message, public_key_bytes)`.

**`ContentHash`** is a 32-byte BLAKE3 hash. Used for audit chain linking (each entry hashes the previous), capsule source tree verification, and tool argument privacy (arguments stored as hashes, not raw content). `zero()` is the sentinel value for genesis entries. Serializes as hex via serde.

## Who depends on this

`astrid-capabilities` signs and verifies capability tokens. `astrid-audit` signs entries and links them via `ContentHash`. `astrid-approval` passes the runtime `KeyPair` through to both. `astrid-capsule` uses `ContentHash` for BLAKE3 source tree verification before loading WASM binaries. This crate is the root of the trust chain.

## Encoding conventions

`PublicKey` and `Signature` serialize as base64 in JSON (compact for network transport). `ContentHash` serializes as hex (human-readable in logs and audit dumps). All three support both hex and base64 round-trip conversions for interop.

## Usage

```toml
[dependencies]
astrid-crypto = { workspace = true }
```

```rust
use astrid_crypto::{KeyPair, ContentHash};

let keypair = KeyPair::generate();

let message = b"capability:fs.read:/tmp";
let hash = ContentHash::hash(message);

let signature = keypair.sign(hash.as_bytes());
assert!(keypair.verify(hash.as_bytes(), &signature).is_ok());

let pk = keypair.export_public_key();
println!("key_id={}", pk.key_id_hex()); // first 8 bytes, safe to log
```

`#![deny(unsafe_code)]` is enforced crate-wide.

## Development

```bash
cargo test -p astrid-crypto
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
