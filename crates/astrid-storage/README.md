# astrid-storage

[![Crates.io](https://img.shields.io/crates/v/astrid-storage)](https://crates.io/crates/astrid-storage)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The unified dual-tier persistence layer for the Astralis runtime.

`astrid-storage` provides the foundational memory and persistence infrastructure for the Astralis OS. It implements a strictly separated two-tier architecture: a low-level, high-performance Key-Value store for isolating untrusted WASM capsules, and a high-level document-graph query engine for managing the system's core capabilities, audit chains, and approval routing.

By centralizing storage around the SurrealDB ecosystem, the runtime scales from a single-agent embedded binary to a distributed multi-node deployment backed by TiKV without altering a single line of application code.

## Core Features
- **Namespace Isolation**: `ScopedKvStore` enforces rigid boundaries for multi-tenant or multi-plugin execution environments.
- **Typed Operations**: Built-in `get_json` and `set_json` convenience methods for ergonomic struct serialization within scoped boundaries.
- **Memory Fallback**: A fully compliant `MemoryKvStore` implementation for ephemeral sessions, fast unit testing, and temporary state.
- **Unified Scaling**: Transition from local `surrealkv://` embedded stores to distributed `tikv://` clusters through configuration alone.

## Architecture

### Tier 1: Raw Key-Value (`KvStore`)
Designed for strict WASM guest isolation. Powered by `SurrealKV`, an embedded, versioned, ACID-compliant LSM-tree. 

Untrusted capsules never receive direct access to the file system or global state. Instead, the runtime provisions a `ScopedKvStore` bound to a specific namespace (e.g., `wasm:{plugin_id}`). The guest code executes `get`, `set`, and `delete` operations without visibility into the underlying host key structure, enforcing a cryptographic airlock around guest memory.

### Tier 2: Query Engine (`Database`)
Designed for the Astralis system core. Powered by `SurrealDB` and `SurrealQL`.

System operations require relational integrity, graph traversal, and complex querying. The Tier 2 engine manages the cryptographic audit log, capability tokens, budget tracking, and persistent agent memory. It uses the exact same underlying `SurrealKV` storage engine when running in embedded mode, ensuring atomic consistency across the entire OS.

## Quick Start

This is an internal workspace crate. Add it to your `Cargo.toml` using the workspace inheritance and specify the storage engines you require.

```toml
[dependencies]
astrid-storage = { workspace = true, features = ["full"] }
```

### Provisioning a WASM Capsule Storage Airlock

When docking a new extension, the runtime provisions an isolated storage view:

```rust
use std::sync::Arc;
use astrid_storage::kv::{MemoryKvStore, ScopedKvStore};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct ExtensionConfig {
    retries: u32,
    endpoint: String,
}

// Host provisions the global memory store
let global_store = Arc::new(MemoryKvStore::new());

// Host binds a restricted namespace for the extension
let extension_storage = ScopedKvStore::new(global_store, "wasm:data-processor").unwrap();

// Extension executes typed reads and writes safely contained in its orbit
let config = ExtensionConfig {
    retries: 3,
    endpoint: "api.internal.net".into(),
};

extension_storage.set_json("agent_config", &config).await.unwrap();
```

### Initializing the System Database

The `Database` struct wraps the connection logic for the main OS state:

```rust
use astrid_storage::Database;

// Connect to a local embedded database for development
let db = Database::connect_embedded("./data/system.db").await.unwrap();

// Or initialize an ephemeral memory database for tests
let test_db = Database::connect_memory().await.unwrap();

// Access the underlying SurrealDB client for direct SurrealQL queries
let client = db.client();
```

## Development

```bash
# Run tests for the in-memory implementations
cargo test -p astrid-storage

# Run tests requiring the SurrealKV feature
cargo test -p astrid-storage --features kv

# Run tests requiring all storage backends
cargo test -p astrid-storage --all-features
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
