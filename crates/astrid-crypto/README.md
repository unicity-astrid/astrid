# astrid-crypto

Cryptographic primitives for the Astrid secure agent runtime.

## Overview

This crate provides the cryptographic foundation for Astrid, implementing the
core philosophy: **Cryptography over prompts.** Authorization comes from ed25519
signatures and capability tokens, not from hoping the LLM follows instructions.

## Features

- **Ed25519 Key Pairs** - Asymmetric signing with secure memory handling via `zeroize`
- **Digital Signatures** - Sign and verify capability tokens and audit entries
- **BLAKE3 Content Hashing** - Fast, secure hashing for audit chains and verification
- **Serialization** - Serde support with base64/hex encoding

## Key Exports

- `KeyPair` - Ed25519 signing key pair with secure memory
- `PublicKey` - Ed25519 public key for verification
- `Signature` - Digital signature wrapper
- `ContentHash` - BLAKE3 hash for content verification

## Usage

```rust
use astrid_crypto::{KeyPair, ContentHash};

// Generate a new key pair
let keypair = KeyPair::generate();

// Sign a message
let message = b"important data";
let signature = keypair.sign(message);

// Verify the signature
assert!(keypair.verify(message, &signature).is_ok());

// Hash content
let hash = ContentHash::hash(message);
println!("Hash: {}", hash.to_hex());
```

## Dependencies

- `ed25519-dalek` - Ed25519 signatures
- `blake3` - Content hashing
- `zeroize` - Secure memory clearing
- `serde` - Serialization support

## Security

This crate enforces `#![deny(unsafe_code)]` and uses `zeroize` to clear
sensitive key material from memory when dropped.

## License

This crate is licensed under the MIT license.
