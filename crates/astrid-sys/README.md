# astrid-sys

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](../../LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The syscall table for Astrid OS.**

In the OS model, this crate is the boundary between user space and kernel space. It declares the 48 host functions that a WASM capsule can invoke, and nothing else. No logic, no validation, no deserialization. Every parameter crosses the boundary as `Vec<u8>`. The kernel enforces capability checks on its side of each call.

Capsule authors should never depend on this crate directly. Use `astrid-sdk`, which wraps every syscall in a typed, safe Rust function. This crate exists for the same reason `/usr/include/asm/unistd.h` exists: to define the raw ABI contract between user space and kernel.

## The ABI

All 48 functions live in a single `#[host_fn] extern "ExtismHost"` block. Declared via `extism-pdk 1.4`.

**Filesystem (7)** `astrid_fs_exists`, `astrid_read_file`, `astrid_write_file`, `astrid_fs_mkdir`, `astrid_fs_readdir`, `astrid_fs_stat`, `astrid_fs_unlink`

**IPC (7)** `astrid_ipc_publish`, `astrid_ipc_subscribe`, `astrid_ipc_unsubscribe`, `astrid_ipc_poll`, `astrid_ipc_recv`, `astrid_uplink_register`, `astrid_uplink_send`

**Storage (7)** `astrid_kv_get`, `astrid_kv_set`, `astrid_kv_delete`, `astrid_kv_list_keys`, `astrid_kv_clear_prefix`, `astrid_get_config`, `astrid_get_caller`

**Network (7)** `astrid_http_request`, `astrid_net_bind_unix`, `astrid_net_accept`, `astrid_net_poll_accept`, `astrid_net_read`, `astrid_net_write`, `astrid_net_close_stream`

**Identity (5)** `astrid_identity_resolve`, `astrid_identity_link`, `astrid_identity_unlink`, `astrid_identity_create_user`, `astrid_identity_list_links`

**Process (4)** `astrid_spawn_host`, `astrid_spawn_background_host`, `astrid_read_process_logs_host`, `astrid_kill_process_host`

**Lifecycle (5)** `astrid_elicit`, `astrid_has_secret`, `astrid_signal_ready`, `astrid_get_interceptor_handles`, `astrid_check_capsule_capability`

**System (6)** `astrid_log`, `astrid_cron_schedule`, `astrid_cron_cancel`, `astrid_trigger_hook`, `astrid_clock_ms`, `astrid_request_approval`

## Design decisions

**Everything is `Vec<u8>`.** File paths can contain non-UTF-8 sequences. IPC topics can be binary hashes. The kernel never validates encodings at the ABI layer. This is deliberate: the boundary is a data pipe, not a schema validator.

**No return-code convention.** Success and failure semantics are defined per-syscall. Some return JSON objects, some return raw bytes, some return nothing. The SDK layer imposes uniform `Result<T, SysError>` semantics on top.

**Single extern block.** All 48 declarations live in one `#[host_fn] extern "ExtismHost"` block. No module hierarchy. The SDK layer provides the module structure.

## Development

```bash
cargo test -p astrid-sys
```

This crate contains zero Rust-side logic. It declares `extern` functions only. Behavioral tests live in `astrid-sdk` and `astrid-integration-tests`.

## License

Dual MIT/Apache-2.0. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).
