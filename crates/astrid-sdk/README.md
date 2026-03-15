# astrid-sdk

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The system library for Astrid OS user space.**

In the OS model, `astrid-sys` is the raw syscall table and this crate is libc. It wraps 48 `unsafe` FFI calls into typed, safe Rust modules that mirror `std`. Capsule authors depend on `astrid-sdk` and `serde`. Nothing else.

Every `unsafe` block in the entire user-space stack lives here (45 call sites). Capsule code is fully safe Rust.

## Where it fits

```text
Capsule code (safe Rust)
    |
  astrid-sdk       typed modules: fs, net, ipc, kv, http, ...
    |
  astrid-sys       raw Vec<u8> FFI (the syscall table)
    |
  Kernel           capability checks, VFS, IPC bus, audit
```

The SDK does not talk to the network, the filesystem, or any external service. Every operation is a request to the kernel through the WASM host ABI. The kernel decides whether to allow it.

## Module layout

| Module | std equivalent | Purpose |
|---|---|---|
| `fs` | `std::fs` | Virtual filesystem reads, writes, stat, mkdir, readdir |
| `net` | `std::net` | Unix domain sockets: bind, accept, read, write |
| `process` | `std::process` | Spawn host processes (foreground and background) |
| `env` | `std::env` | Capsule configuration via `astrid_get_config` |
| `time` | `std::time` | Wall-clock milliseconds via `astrid_clock_ms` |
| `log` | `log` crate | Structured logging to the kernel journal |
| `runtime` | N/A | Readiness signaling, caller context |
| `ipc` | N/A | Publish/subscribe event bus with blocking receive |
| `kv` | N/A | Persistent key-value storage with versioned envelopes |
| `http` | N/A | Outbound HTTP requests |
| `cron` | N/A | Dynamic cron job scheduling |
| `uplink` | N/A | Direct frontend messaging |
| `hooks` | N/A | Trigger kernel hook events |
| `elicit` | N/A | Interactive prompts during install/upgrade |
| `identity` | N/A | Platform user resolution and linking |
| `approval` | N/A | Block until a human approves or denies |
| `capabilities` | N/A | Cross-capsule capability checks |
| `interceptors` | N/A | Auto-subscribed interceptor handle queries |

## Quick start

```toml
[dependencies]
astrid-sdk = "0.2"
serde = { version = "1", features = ["derive"] }
```

```rust
use astrid_sdk::prelude::*;

struct MyCapsule;

#[capsule]
impl MyCapsule {
    #[astrid::tool]
    fn greet(&self, name: String) -> Result<String, SysError> {
        log::info(format!("greeting {name}"))?;
        kv::set_json("last_name", &name)?;
        Ok(format!("Hello, {name}!"))
    }
}
```

The `#[capsule]` macro (from `astrid-sdk-macros`, re-exported here) generates all WASM ABI exports. See that crate's README for the full attribute reference.

## Versioned KV storage

`kv::set_versioned` / `kv::get_versioned` wrap values in a version envelope. Reading data written at a newer schema version returns an explicit error, not silent corruption. `kv::get_versioned_or_migrate` takes a migration closure for backward-compatible upgrades.

## Three serialization formats

Every relevant KV and IPC operation supports JSON (`serde_json`), MessagePack (`rmp-serde`), and Borsh. Pick based on size constraints and interop needs.

## Feature flags

| Flag | Default | Effect |
|---|---|---|
| `derive` | yes | Enables the `#[capsule]` proc macro via `astrid-sdk-macros` |

## Development

```bash
cargo test -p astrid-sdk
```

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
