# astrid-sdk

[![Crates.io](https://img.shields.io/crates/v/astrid-sdk)](https://crates.io/crates/astrid-sdk)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

The standard library and safe ABI wrapper for Astralis OS User-Space Capsules.

`astrid-sdk` bridges the gap between the raw, unsafe WebAssembly FFI boundary defined by `astrid-sys` and the developer-friendly Rust environment expected by capsule authors. It transforms low-level memory pointer manipulation into safe, idiomatic Rust functions with built-in, zero-friction serialization. If you are building a native Rust capsule for the Astralis microkernel, this is the primary crate you will depend on.

## The FFI Boundary Problem

The Astralis microkernel utilizes Extism to execute User-Space Capsules within an isolated WebAssembly sandbox. At the ABI level, the system interfaces exclusively through raw memory pointers and byte vectors (`Vec<u8>`). 

Calling the kernel directly through `astrid-sys` requires `unsafe` blocks, manual pointer management, and explicit serialization of data types before they can traverse the boundary. 

`astrid-sdk` abstracts this complexity entirely by providing:
* **Zero-Cost Safety**: Encapsulates all `unsafe` FFI calls behind a deterministic and safe Rust API.
* **Transparent Serialization**: Automatically encodes and decodes complex data structures to JSON, MessagePack, or Borsh during boundary traversal.
* **Ergonomic Capability Modules**: Organizes system calls into logical namespaces (known as "Airlocks") functionally analogous to `std::fs` or `std::env`.
* **Entrypoint Generation**: Re-exports the `#[capsule]` attribute macro to eliminate initialization boilerplate when defining WebAssembly entry functions.

## Quick Start

The fastest way to initialize a capsule is to rely on the `prelude` module and the `#[capsule]` macro. The SDK abstracts the complexities of kernel communication so you can focus on system logic.

```rust
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct GreetingPayload {
    message: String,
}

#[capsule]
fn run() -> Result<(), SysError> {
    // 1. Retrieve configuration data from the host kernel
    let name = sys::get_config_string("user_name")
        .unwrap_or_else(|_| "Astrid".to_string());
    
    // 2. Dispatch logs natively to the Astralis system logger
    sys::log("info", format!("Generating greeting for {}", name))?;
    
    // 3. Publish an event via IPC (auto-serialized to JSON across the ABI)
    let payload = GreetingPayload { message: format!("Hello, {}!", name) };
    ipc::publish_json("greetings.new", &payload)?;
    
    // 4. Persist state within the key-value store
    kv::set_json("last_greeting", &payload)?;

    Ok(())
}
```

## API Surface: The Airlocks

The SDK organizes system capabilities into distinct modules that mirror the security capability definitions within the kernel. 

* **`fs`** (Virtual File System): Isolated file operations within the capsule's sandbox bounds. Provides `exists`, `read_bytes`, `write_string`, `mkdir`, `stat`, and `unlink`.
* **`ipc`** (Inter-Process Communication): Publish and subscribe access to the internal Astralis Event Bus. Supports raw bytes, JSON, and MessagePack formats natively (`publish_json`, `publish_msgpack`, `subscribe`, `poll_bytes`).
* **`kv`** (Persistent Storage): Key-Value store operations equipped with transparent serialization logic. Includes `get_bytes`, `set_json`, `get_borsh`, and more.
* **`uplink`** (Frontend Messaging): Mediated communication with connected platforms (e.g., Telegram, CLI) via `register` and `send_bytes`.
* **`http`** (Network): Outbound HTTP request execution via `request_bytes`.
* **`cron`** (Scheduling): Dynamic background job scheduling mechanisms utilizing `schedule` and `cancel`.
* **`sys`** (System): Core operational routines including logging (`log`) and configuration retrieval (`get_config_string`, `get_config_bytes`).

## Serialization Flexibility

Because different capsules face varying memory limitations and performance constraints, the SDK abstains from forcing a unified data format. `astrid-sdk` integrates multiple serialization libraries specifically designed for cross-ABI communication. The `kv` and `ipc` modules, for example, natively expose:

* `set_bytes` / `publish_bytes`: Direct, unadulterated byte access with minimal overhead.
* `set_json` / `publish_json`: Powered by `serde_json`. Ideal for human-readable state, web payloads, or system debugging.
* `set_borsh` / `get_borsh`: Powered by `borsh`. Optimized for high-performance, strictly structured, dense binary state.
* `publish_msgpack`: Powered by `rmp-serde`. A lightweight binary alternative to JSON.

## Error Handling Architecture

Every system call returns a deterministic `Result<T, SysError>`. The `SysError` type unifies underlying failures to provide clear visibility into ABI-related faults. 

It specifically maps:
* Host execution and WebAssembly faults (`extism_pdk::Error`)
* Serialization boundary failures (`serde_json`, `rmp_serde`, `borsh`)
* Logical API boundary errors

This unified architecture enables the use of the `?` operator for clean, idiomatic execution paths without the need to continuously map underlying framework errors.

## Feature Flags

* **`derive`** (default): Enables the `#[capsule]` macro for entry point generation, provided internally by `astrid-sdk-macros`.
* **`default`**: Inherits the `derive` feature flag.

## Development

This crate acts as the primary developer interface for Astralis capsules. Run tests via the workspace:

```bash
cargo test -p astrid-sdk
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.