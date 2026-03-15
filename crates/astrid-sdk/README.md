# astrid-sdk

[![Crates.io](https://img.shields.io/crates/v/astrid-sdk)](https://crates.io/crates/astrid-sdk)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)

The safe Rust SDK for building User-Space Capsules on Astrid OS.

`astrid-sdk` sits between the raw WebAssembly FFI defined by `astrid-sys` and the Rust code you actually want to write. It wraps every `unsafe` host call, serializes data transparently across the ABI boundary, and organizes system capabilities into modules whose names deliberately mirror the Rust standard library. If you are building a Capsule, this is the only crate you need.

## Core Features

- **Safe ABI boundary** - all `unsafe` FFI calls are contained inside the crate; capsule code is fully safe Rust
- **`std`-mirrored module names** - `fs`, `env`, `net`, `time`, `log`, and `process` follow standard library naming so the API is discoverable without reading docs
- **Three serialization formats** - JSON (`serde_json`), MessagePack (`rmp-serde`), and Borsh are available on every relevant operation; choose based on your memory and interop constraints
- **Versioned KV storage** - `kv::set_versioned` / `kv::get_versioned` / `kv::get_versioned_or_migrate` handle schema evolution with a fail-secure envelope format
- **`#[capsule]` macro** - generates the required `extern "C"` WebAssembly exports, dispatches tool/command/interceptor/cron calls to annotated methods, and handles stateful or stateless operation modes
- **Interactive install lifecycle** - `elicit::text`, `elicit::secret`, `elicit::select`, and `elicit::array` let capsules prompt the user during `#[astrid::install]` and `#[astrid::upgrade]` hooks
- **Human approval gates** - `approval::request` blocks the capsule until a user approves or denies a sensitive action, with allowance-store fast-path for pre-approved patterns
- **Platform identity resolution** - `identity::resolve`, `link`, `unlink`, and `list_links` map platform-specific user IDs (Discord, Twitch, etc.) to Astrid-native user UUIDs

## Quick Start

Add the dependency to your capsule's `Cargo.toml`:

```toml
[dependencies]
astrid-sdk = "0.2"
serde = { version = "1", features = ["derive"] }
```

A minimal capsule using the `#[capsule]` macro:

```rust
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

struct MyCapsule;

#[capsule]
impl MyCapsule {
    #[astrid::tool]
    fn greet(&self, name: String) -> Result<String, SysError> {
        log::info(format!("greeting {name}"))?;
        kv::set_json("last_name", &name)?;
        Ok(format!("Hello, {name}!"))
    }

    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        runtime::signal_ready()?;
        // event loop driven by IPC subscriptions
        Ok(())
    }
}
```

## Modules

The SDK mirrors `std` where applicable and adds Astrid-specific modules for everything else.

| Module | `std` equivalent | Purpose |
|---|---|---|
| `fs` | `std::fs` | Virtual filesystem (read, write, create_dir, metadata, remove_file) |
| `env` | `std::env` | Capsule config values injected by the kernel at load time |
| `net` | `std::net` | Unix domain socket bind, accept, read, write, poll_accept |
| `time` | `std::time` | Wall-clock time via host call (`now_ms`) |
| `log` | `log` crate | Structured logging at debug / info / warn / error levels |
| `process` | `std::process` | Spawn foreground and background host processes |
| `ipc` | N/A | Topic-based event bus (publish, subscribe, poll, blocking recv) |
| `kv` | N/A | Persistent key-value store with JSON, Borsh, and versioned helpers |
| `http` | N/A | Outbound HTTP requests via the kernel HTTP Airlock |
| `cron` | N/A | Dynamic background job scheduling (schedule, cancel) |
| `uplink` | N/A | Direct messaging to connected frontends (CLI, Telegram, etc.) |
| `hooks` | N/A | Trigger user-defined middleware from within a capsule |
| `elicit` | N/A | Interactive prompts during install/upgrade lifecycle |
| `identity` | N/A | Platform user identity resolution and linking |
| `approval` | N/A | Human-in-the-loop approval gates for sensitive actions |
| `capabilities` | N/A | Cross-capsule manifest capability queries |
| `interceptors` | N/A | Auto-subscribed interceptor bindings for run-loop capsules |
| `runtime` | N/A | OS signaling (`signal_ready`) and caller context retrieval |

## API Reference

### `SysError`

The unified error type returned by every SDK function:

```rust
pub enum SysError {
    HostError(extism_pdk::Error),
    JsonError(serde_json::Error),
    MsgPackEncodeError(rmp_serde::encode::Error),
    MsgPackDecodeError(rmp_serde::decode::Error),
    BorshError(std::io::Error),
    ApiError(String),
}
```

All variants implement `From` for their underlying error type, so `?` works directly without mapping.

### Key Types

| Type | Module | Description |
|---|---|---|
| `SysError` | root | Unified error type for all SDK operations |
| `SubscriptionHandle` | `ipc` | Typed handle returned by `ipc::subscribe` |
| `UplinkId` | `uplink` | Opaque connection ID returned by `uplink::register` |
| `ListenerHandle` | `net` | Bound Unix socket listener |
| `StreamHandle` | `net` | Open Unix socket stream |
| `Versioned<T>` | `kv` | Enum result of `kv::get_versioned`: `Current`, `NeedsMigration`, `Unversioned`, `NotFound` |
| `CallerContext` | `types` | User ID and session ID for the current capsule execution |
| `ApprovalResult` | `approval` | `approved: bool` and `decision: String` from an approval gate |
| `ResolvedUser` | `identity` | Astrid-native `user_id` and optional `display_name` |
| `Link` | `identity` | Platform-to-Astrid identity link record |
| `ProcessResult` | `process` | `stdout`, `stderr`, `exit_code` from a completed process |
| `BackgroundProcessHandle` | `process` | Opaque handle for a background process |
| `InterceptorBinding` | `interceptors` | Runtime handle and topic for an auto-subscribed interceptor |

### `#[capsule]` Macro

The `capsule` attribute goes on an `impl` block. Methods inside are annotated with `#[astrid::<kind>]`:

| Annotation | Signature requirement | Description |
|---|---|---|
| `#[astrid::tool]` | `fn(&self, args: T) -> Result<R, SysError>` | Exposed as an MCP tool; args deserialized from JSON |
| `#[astrid::command]` | `fn(&self, args: T) -> Result<R, SysError>` | CLI command dispatch |
| `#[astrid::interceptor]` | `fn(&self, payload: T) -> Result<R, SysError>` | IPC event interceptor |
| `#[astrid::cron]` | `fn(&self) -> Result<(), SysError>` | Scheduled background task |
| `#[astrid::install]` | `fn(&self) -> Result<(), SysError>` | One-time install lifecycle hook |
| `#[astrid::upgrade]` | `fn(&self, prev_version: &str) -> Result<(), SysError>` | Upgrade lifecycle hook |
| `#[astrid::run]` | `fn(&self) -> Result<(), SysError>` | Long-running event loop |
| `#[astrid::mutable]` | Combined with tool/command/interceptor/cron | Loads and saves KV state around the call |

Pass `#[capsule(state)]` to enable the stateful mode, which automatically loads the struct from `kv::get_json("__state")` before each dispatch and saves it back after.

### Versioned KV

```rust
// Write with schema version 1
kv::set_versioned("my_key", &my_data, 1)?;

// Read back - returns Current, NeedsMigration, Unversioned, or NotFound
match kv::get_versioned::<MyData>("my_key", 1)? {
    Versioned::Current(data) => { /* use data */ },
    Versioned::NeedsMigration { raw, stored_version } => { /* migrate */ },
    Versioned::Unversioned(raw) => { /* handle legacy */ },
    Versioned::NotFound => { /* key absent */ },
}

// Or let the SDK call your migration function and write back automatically
let data = kv::get_versioned_or_migrate("my_key", 2, |raw, version| {
    // return T at version 2
})?;
```

Stored format is `{"__sv": <u32>, "data": <payload>}`. Reading data at a version newer than `current_version` returns an error rather than silently misinterpreting the schema.

## Feature Flags

| Flag | Default | Description |
|---|---|---|
| `derive` | yes | Enables the `#[capsule]` macro via `astrid-sdk-macros` |

## Development

```bash
cargo test -p astrid-sdk
```

Capsules compile to `wasm32-wasip1`. The SDK itself does not require the WASM target for unit tests - the versioned KV logic and type serialization tests run on the host.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
