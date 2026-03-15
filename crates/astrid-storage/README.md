# astrid-storage

[![Crates.io](https://img.shields.io/crates/v/astrid-storage)](https://crates.io/crates/astrid-storage)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Unified dual-tier persistence layer for the Astrid runtime.

`astrid-storage` provides the foundational storage infrastructure for Astrid's secure agent runtime. It implements a strictly separated two-tier architecture: a low-level, ACID-compliant key-value store with hard namespace isolation for WASM capsule storage, and a full SurrealDB query engine for system-level state. Both tiers scale from a single-agent embedded binary to a distributed multi-node deployment through configuration alone — no application code changes required.

## Core Features

- **Namespace isolation**: `ScopedKvStore` enforces rigid per-namespace boundaries. Guests operate on plain `get`/`set`/`delete` calls without ever seeing the underlying key structure. Empty keys, null bytes, and injection characters are rejected at the validation boundary.
- **Prefix operations**: `list_keys_with_prefix` and `clear_prefix` efficiently scope range operations within a namespace. Both `MemoryKvStore` and `SurrealKvStore` have native implementations.
- **Typed JSON access**: `ScopedKvStore::get_json` and `set_json` handle serde round-trips, eliminating boilerplate for struct storage.
- **In-memory store**: `MemoryKvStore` is always available without feature flags, making unit tests zero-overhead.
- **Persistent KV store**: `SurrealKvStore` (`kv` feature) wraps SurrealKV's MVCC LSM-tree with transactional reads and atomic batch deletes.
- **SurrealDB query engine**: `Database` (`db` feature) connects to SurrealDB in embedded (`surrealkv://`), in-memory (`mem://`), or distributed (`tikv://`) mode. Same API, different backing store.
- **Secret storage**: `SecretStore` trait with `KvSecretStore` (KV-backed, always available) and `KeychainSecretStore` (`keychain` feature, OS keychain via the `keyring` crate). `FallbackSecretStore` probes the keychain at construction time and commits to a single backend for the lifetime of the store, preventing split-brain.
- **Identity store**: `IdentityStore` trait with `KvIdentityStore` for managing users and cross-platform identity links. Platform names are normalized and key-path injection characters (`/`, `\0`) are rejected.
- **No unsafe code**: the crate is compiled with `#![deny(unsafe_code)]`.

## Quick Start

This is an internal workspace crate. Add it to your `Cargo.toml` using workspace inheritance and specify the storage tiers you need.

```toml
[dependencies]
astrid-storage = { workspace = true, features = ["full"] }
```

Available features:

| Feature | Enables |
|---------|---------|
| `kv` | `SurrealKvStore` (persistent, embedded) |
| `db` | `Database` (SurrealDB query engine) |
| `keychain` | `KeychainSecretStore` + `FallbackSecretStore` (OS keychain) |
| `full` | `kv` + `db` |

## API Reference

### Key Types

#### `KvStore` trait (`kv` module)

The core async trait for namespaced byte storage. All implementations share this interface.

```rust
pub trait KvStore: Send + Sync {
    async fn get(&self, namespace: &str, key: &str) -> StorageResult<Option<Vec<u8>>>;
    async fn set(&self, namespace: &str, key: &str, value: Vec<u8>) -> StorageResult<()>;
    async fn delete(&self, namespace: &str, key: &str) -> StorageResult<bool>;
    async fn exists(&self, namespace: &str, key: &str) -> StorageResult<bool>;
    async fn list_keys(&self, namespace: &str) -> StorageResult<Vec<String>>;
    async fn list_keys_with_prefix(&self, namespace: &str, prefix: &str) -> StorageResult<Vec<String>>;
    async fn clear_namespace(&self, namespace: &str) -> StorageResult<u64>;
    async fn clear_prefix(&self, namespace: &str, prefix: &str) -> StorageResult<u64>;
}
```

#### `MemoryKvStore`

In-memory `KvStore` backed by a `HashMap`. Always available without feature flags. Primary use: unit tests and ephemeral state.

```rust
let store = Arc::new(MemoryKvStore::new());
```

#### `SurrealKvStore` (feature: `kv`)

Persistent `KvStore` backed by SurrealKV's embedded LSM-tree. Namespace isolation is implemented via a null-byte composite key scheme (`namespace\0key`). All writes use explicit transactions; `clear_namespace` and `clear_prefix` collect keys and delete atomically in a single commit.

```rust
let store = SurrealKvStore::open("./data/kv")?;
// or, with custom options:
let store = SurrealKvStore::open_with_options(opts)?;
store.close().await?;
```

#### `ScopedKvStore`

A namespace-pre-bound wrapper around any `Arc<dyn KvStore>`. This is the API surface WASM guests interact with — they receive a `ScopedKvStore` and never handle namespaces directly.

```rust
use std::sync::Arc;
use astrid_storage::kv::{MemoryKvStore, ScopedKvStore};

let store = Arc::new(MemoryKvStore::new());
let scoped = ScopedKvStore::new(store, "wasm:my-plugin")?;

scoped.set("config", b"{}".to_vec()).await?;
let bytes = scoped.get("config").await?;

// Typed JSON round-trip
scoped.set_json("config", &my_struct).await?;
let loaded: MyStruct = scoped.get_json("config").await?.unwrap();

// Prefix operations
let keys = scoped.list_keys_with_prefix("session.").await?;
let cleared = scoped.clear_prefix("session.").await?;
```

#### `Database` (feature: `db`)

Wraps a SurrealDB connection and exposes the raw client for direct SurrealQL queries.

```rust
use astrid_storage::Database;

// Embedded persistent store (development / production single-node)
let db = Database::connect_embedded("./data/system.db").await?;

// In-memory (tests)
let db = Database::connect_memory().await?;

// Access the SurrealDB client directly
let client = db.client();
let results: Vec<Record> = client.query("SELECT * FROM capability").await?.take(0)?;
```

Connection string formats supported by `Database`:

| Mode | Connection string |
|------|-------------------|
| Embedded persistent | `surrealkv://path/to/data` |
| In-memory | `mem://` |
| Distributed (TiKV) | `tikv://pd0:2379` |

#### `SecretStore` trait (`secret` module)

Synchronous trait for capsule credential storage. All implementations are `Send + Sync`.

- `KvSecretStore`: stores secrets in the `ScopedKvStore` under a `__secret:` prefix. Suitable for headless and CI environments.
- `KeychainSecretStore` (feature: `keychain`): delegates to the OS keychain. Service name is scoped to `astrid:{capsule_id}`.
- `FallbackSecretStore` (feature: `keychain`): probes the OS keychain once at construction and commits to either keychain or KV for the lifetime of the store.
- `build_secret_store`: convenience constructor that returns the best available implementation as `Arc<dyn SecretStore>`.

```rust
use astrid_storage::secret::build_secret_store;

let store = build_secret_store("my-capsule", scoped_kv, runtime_handle);
store.set("api_key", "sk-12345")?;
let exists = store.exists("api_key")?;
```

#### `IdentityStore` trait (`identity` module)

Async trait for managing Astrid users and cross-platform identity links. `KvIdentityStore` persists to a `ScopedKvStore` (typically namespace `system:identity`).

```rust
use astrid_storage::{IdentityStore, KvIdentityStore};

let identity = KvIdentityStore::new(scoped_kv);

let user = identity.create_user(Some("Alice")).await?;
identity.link("discord", "123456789", user.id, "admin").await?;

// Resolve a Discord user to an Astrid identity
let resolved = identity.resolve("discord", "123456789").await?;
```

Platform names are normalized to lowercase/trimmed before storage. Input containing `/` or `\0` is rejected to prevent key-path injection.

#### `StorageError`

All storage operations return `StorageResult<T>`, a type alias for `Result<T, StorageError>`.

```rust
pub enum StorageError {
    NotFound(String),
    Internal(String),
    Connection(String),
    Serialization(String),
    InvalidKey(String),
}
```

## Development

```bash
# Run tests for the in-memory implementations (no feature flags required)
cargo test -p astrid-storage

# Run tests including the SurrealKV persistent backend
cargo test -p astrid-storage --features kv

# Run all tests across all backends
cargo test -p astrid-storage --all-features
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
