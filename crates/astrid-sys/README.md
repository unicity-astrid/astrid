# astrid-sys

[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/astrid-sys)](https://crates.io/crates/astrid-sys)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

Raw FFI bindings (the Airlocks) for the Astrid OS Microkernel's WASM boundary.

This crate defines the absolute lowest-level, mathematically pure ABI between WASM capsules and the
Astrid host runtime. Every parameter and return type crossing the WebAssembly boundary is raw bytes
(`Vec<u8>`). No string validation. No deserialization. No abstraction. The host implements the exact
inverse of each declaration; the kernel enforces capability sandboxing on its side of the airlock.
All ergonomic typing and safety live in `astrid-sdk`, one layer up.

## Core Features

- **Encoding agnosticism** - file paths may contain non-UTF-8 sequences; IPC topics can be binary
  hashes; the kernel never validates encodings across the boundary
- **Zero host-side abstraction** - the entire API surface is `Vec<u8>` in, `Vec<u8>` out; no
  complex structs cross the WASM boundary
- **Complete syscall surface** - VFS, IPC message bus, uplinks, KV store, HTTP, Unix sockets,
  scheduling, logging, approval gates, identity, lifecycle elicitation, and background processes
- **Blocking and non-blocking IPC** - both `astrid_ipc_poll` (non-blocking) and `astrid_ipc_recv`
  (blocking with timeout) are exposed
- **Extism 1.x host functions** - declared via `#[host_fn] extern "ExtismHost"` using
  `extism-pdk 1.4`

## Quick Start

As a capsule developer you should almost never depend on `astrid-sys` directly. Depend on
`astrid-sdk` instead, which wraps these raw FFI calls in safe, strongly-typed Rust.

If you are working on the kernel boundary itself, add the crate as follows:

```toml
[dependencies]
astrid-sys = "0.2"
```

A host function declaration looks like this:

```rust
use extism_pdk::*;

#[host_fn]
extern "ExtismHost" {
    pub fn astrid_ipc_recv(handle: Vec<u8>, timeout_ms: Vec<u8>) -> Vec<u8>;
}
```

The host receives raw bytes through the airlock, executes the operation inside the capsule's
capability sandbox, and returns a raw byte response. The SDK layer is responsible for serializing
and deserializing the JSON envelopes on both sides.

## API Reference

All functions are declared in `src/lib.rs` under a single `#[host_fn] extern "ExtismHost"` block.

### Virtual File System (VFS)

| Function | Description |
|---|---|
| `astrid_fs_exists(path)` | Check whether a VFS path exists |
| `astrid_fs_mkdir(path)` | Create a directory |
| `astrid_fs_readdir(path)` | List directory contents |
| `astrid_fs_stat(path)` | Get path metadata |
| `astrid_fs_unlink(path)` | Delete a file or directory |
| `astrid_read_file(path)` | Read a file's raw bytes |
| `astrid_write_file(path, content)` | Write raw bytes to a file |

### Inter-Process Communication

| Function | Description |
|---|---|
| `astrid_ipc_publish(topic, payload)` | Publish a message to the OS event bus |
| `astrid_ipc_subscribe(topic)` | Subscribe to a topic; returns a handle |
| `astrid_ipc_unsubscribe(handle)` | Release a subscription handle |
| `astrid_ipc_poll(handle)` | Non-blocking: return next message or empty |
| `astrid_ipc_recv(handle, timeout_ms)` | Blocking: wait for a message or timeout |
| `astrid_uplink_register(name, platform, profile)` | Register a direct frontend uplink |
| `astrid_uplink_send(uplink_id, platform_user_id, content)` | Send via a registered uplink |

### Storage and Configuration

| Function | Description |
|---|---|
| `astrid_kv_get(key)` | Read a value from the KV store |
| `astrid_kv_set(key, value)` | Write a value |
| `astrid_kv_delete(key)` | Delete a key |
| `astrid_kv_list_keys(prefix)` | List keys matching a prefix (returns JSON array) |
| `astrid_kv_clear_prefix(prefix)` | Delete all keys under a prefix (returns JSON count) |
| `astrid_get_config(key)` | Read a system configuration string |
| `astrid_get_caller()` | Get the invoking user ID and session ID |

### Network

| Function | Description |
|---|---|
| `astrid_http_request(request_bytes)` | Issue an HTTP request |
| `astrid_net_bind_unix(path)` | Bind a Unix Domain Socket; returns a listener handle |
| `astrid_net_accept(listener_handle)` | Block until a connection arrives; returns a stream handle |
| `astrid_net_poll_accept(listener_handle)` | Non-blocking accept; empty bytes if no connection pending |
| `astrid_net_read(stream_handle)` | Read bytes from a stream |
| `astrid_net_write(stream_handle, data)` | Write bytes to a stream |
| `astrid_net_close_stream(stream_handle)` | Close a stream and release host resources |

### Scheduling and System Services

| Function | Description |
|---|---|
| `astrid_log(level, message)` | Append a message to the OS journal |
| `astrid_cron_schedule(name, schedule, payload)` | Register a dynamic cron trigger |
| `astrid_cron_cancel(name)` | Cancel a previously registered cron trigger |
| `astrid_trigger_hook(event_bytes)` | Fire a hook event and wait for its synchronous result |
| `astrid_clock_ms()` | Get wall-clock time as milliseconds since the UNIX epoch |

### Lifecycle and Approval

| Function | Description |
|---|---|
| `astrid_elicit(request)` | Prompt the user for input during install or upgrade |
| `astrid_has_secret(request)` | Check whether a secret is configured without reading it |
| `astrid_request_approval(request)` | Block until a human approves or denies a sensitive action |
| `astrid_signal_ready()` | Signal that the capsule run loop is active and subscriptions are live |
| `astrid_get_interceptor_handles()` | Query auto-subscribed interceptor handle mappings |

### Identity

| Function | Description |
|---|---|
| `astrid_identity_resolve(request)` | Resolve a platform user to an Astrid user |
| `astrid_identity_link(request)` | Link a platform identity to an Astrid user |
| `astrid_identity_unlink(request)` | Unlink a platform identity |
| `astrid_identity_create_user(request)` | Create a new Astrid user |
| `astrid_identity_list_links(request)` | List all platform links for a user |

### Capability Checks and Host Processes

| Function | Description |
|---|---|
| `astrid_check_capsule_capability(request)` | Check whether a capsule has a specific manifest capability |
| `astrid_spawn_host(cmd_and_args_json)` | Spawn a native host process (requires `host_process` capability) |
| `astrid_spawn_background_host(request)` | Spawn a background host process with piped stdout/stderr |
| `astrid_read_process_logs_host(request)` | Read buffered output from a background process |
| `astrid_kill_process_host(request)` | Terminate a background process and clean up resources |

## JSON Envelope Conventions

Several host functions use JSON-encoded requests and responses over the raw byte channel. The SDK
layer handles this encoding. Key patterns:

- `astrid_elicit` returns `{"ok":true}` for secrets, `{"value":"..."}` for text or select,
  `{"values":["..."]}` for arrays, or an Extism error on cancellation.
- `astrid_has_secret` takes `{"key":"..."}` and returns `{"exists":true/false}`.
- `astrid_request_approval` takes `{"action":"...","resource":"...","risk_level":"..."}` and
  returns `{"approved":true/false,"decision":"..."}`.
- `astrid_identity_resolve` takes `{"platform":"...","platform_user_id":"..."}` and returns
  `{"found":true/false,"user_id":"...","display_name":"..."}`.
- `astrid_check_capsule_capability` takes `{"source_uuid":"...","capability":"..."}` and returns
  `{"allowed":true/false}`.

## Development

```bash
cargo test -p astrid-sys
```

Because this crate declares only `extern "ExtismHost"` host functions (no Rust-side logic), the
test surface here is minimal. Behavioral tests live in `astrid-sdk` and `astrid-integration-tests`.

## License

This project is dual-licensed under either the [MIT License](../../LICENSE-MIT) or the
[Apache License, Version 2.0](../../LICENSE-APACHE), at your option.
