# astrid-sys

[![Crates.io](https://img.shields.io/crates/v/astrid-sys)](https://crates.io/crates/astrid-sys)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.93](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)

Raw FFI bindings defining the absolute lowest-level boundary of the Astralis OS Microkernel. 

If Astralis is a vast station in orbit, `astrid-sys` defines the physical airlocks connecting isolated WASM capsules to the core infrastructure. It represents a mathematically pure Application Binary Interface (ABI) where every parameter and return type crossing the WebAssembly boundary is reduced to raw bytes (`Vec<u8>`). This crate operates strictly at the system call level, providing unopinionated, unvarnished access to the host kernel.

## Core Features

The architectural mandate of `astrid-sys` is **zero host-side abstraction**. By reducing the entire API surface to contiguous byte arrays, we achieve true OS-level primitiveness:

* **Encoding Agnosticism:** File paths can contain non-UTF-8 sequences and IPC topics can be raw binary hashes.
* **Maximum Performance:** The core kernel never wastes a single CPU cycle validating string encodings or deserializing complex structs across the WASM boundary.
* **Decoupling:** All ergonomic serialization, typing, and safety abstractions are intentionally pushed up into User-Space, specifically handled by the `astrid-sdk` crate.

## The API Surface

The airlocks expose four primary hardware-level domains to the WASM environment, implemented via `extism-pdk` host functions.

### Virtual File System (VFS)
Raw block-level access to the sandboxed file system provided to the capsule.
* `astrid_fs_exists`, `astrid_fs_stat`, `astrid_fs_mkdir`, `astrid_fs_readdir`, `astrid_fs_unlink`
* `astrid_read_file`, `astrid_write_file`

### Inter-Process Communication (IPC)
The central nervous system for inter-capsule communication and direct frontend uplinks.
* **Message Bus:** `astrid_ipc_publish`, `astrid_ipc_subscribe`, `astrid_ipc_unsubscribe`, `astrid_ipc_poll`
* **Direct Uplinks:** `astrid_uplink_register`, `astrid_uplink_send`

### Storage & Configuration
Persistent state and system-level environment queries.
* **Key-Value Store:** `astrid_kv_get`, `astrid_kv_set`
* **Configuration:** `astrid_get_config`

### General System Services
Core background services, networking, and chronometry.
* `astrid_http_request` - Outbound HTTP requests
* `astrid_log` - Appending raw bytes to the OS journal
* `astrid_cron_schedule`, `astrid_cron_cancel` - Dynamic scheduling triggers

## Quick Start

As a capsule developer, **you should almost never depend on `astrid-sys` directly**. 

This crate is the raw transmission layer. Writing code against it is akin to writing assembly; it is powerful but inherently unsafe and verbose. Instead, user-space capsules depend on `astrid-sdk`, which wraps these raw `Vec<u8>` FFI calls in safe, strongly-typed, and ergonomic Rust futures and structs.

For systems engineers modifying the kernel boundary, a typical host function definition in this crate looks like this:

```rust
use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    /// Poll for the next message on an IPC subscription handle.
    pub fn astrid_ipc_poll(handle: Vec<u8>) -> Vec<u8>;
}
```

The host (the Astralis runtime) implements the exact inverse of this interface. It receives the raw bytes through the airlock, executes the system operation according to the capsule's capability sandbox, and returns the response bytes back across the void.

## Development

```bash
cargo test -p astrid-sys
```

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the [Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
