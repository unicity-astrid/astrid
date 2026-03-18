# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Changelog tracking starts with 0.2.0. Prior versions were not tracked.

## [Unreleased]

### Added

- `astrid_net_read` now uses a self-describing `NetReadStatus` wire format: every response is prefixed with a discriminant byte (`0x00` = data, `0x01` = closed, `0x02` = pending), replacing the previous single-byte sentinel hack
- Headless mode: `astrid -p "prompt"` for non-interactive single-prompt execution with stdin piping support
- Post-install onboarding: `astrid capsule install` now prompts for `[env]` fields immediately after install
- Shared `astrid_telemetry::log_config_from()` behind `config` feature flag — replaces duplicate config bridge code

### Fixed

- `net_write` no longer causes a WASM trap on broken pipe / connection reset when a headless client disconnects; write errors are logged at debug level and the dead stream is cleaned up on the next read
- `net_read` returns a `NET_STREAM_CLOSED` sentinel byte instead of trapping on peer EOF/disconnect, allowing the CLI capsule run loop to remove dead streams gracefully
- Also fixes a variable name mismatch (`capsule` vs `plugin`) in `approval.rs` that caused a compile error
- `~/.astrid/shared/` directory now created on boot, eliminating `global:// VFS not mounted` warning on fresh installs
- Capsule reinstall now preserves existing `.env.json` rather than overwriting it with an empty file
- WASM execution timeout bumped from 30s to 5 minutes to prevent premature cancellation on slow operations
- IPC event dispatcher now delivers events to each capsule in publish order via per-capsule mpsc queues, fixing out-of-order stream text assembly in the ReAct capsule
- `IpcMessage` gains a monotonic `seq` field assigned at publish time for ordering and diagnostics
- KV host function double-encoding: `kv_get_impl` returned `serde_json::to_vec` of raw bytes instead of raw bytes directly
- Config host function double-encoding: `get_config_impl` wrapped string values in JSON quotes, breaking URLs and other string config
- React capsule LLM topic validation: `active_llm_topic()` could produce topics with empty segments causing IPC publish failures

### Changed

- Ephemeral daemon now shuts down immediately when the last client disconnects (idle timeout 0, 1s check interval) instead of waiting 5 minutes
- Renamed `plugin` → `capsule` in the WASM host layer and audit log fields for consistency with project terminology
- Split `astrid-build` 1166-line `build.rs` into focused modules: `rust.rs`, `openclaw.rs`, `mcp.rs`

## [0.4.0] - 2026-03-17

### Added

- `astrid-daemon` crate — standalone kernel daemon binary with `--ephemeral` flag for CLI-spawned instances vs persistent multi-frontend mode
- `astrid-build` crate — standalone capsule compiler and packager (Rust, OpenClaw, MCP). Invoked by CLI via subprocess.
- `astrid start` command — spawn a persistent daemon (detached, no TUI)
- `astrid status` command — query daemon PID, uptime, connected clients, loaded capsules
- `astrid stop` command — graceful daemon shutdown via management API
- `KernelRequest::Shutdown`, `KernelRequest::GetStatus`, and `DaemonStatus` types in `astrid-types`
- `Kernel::boot_time` field for uptime tracking
- Streaming HTTP airlock: `astrid_http_stream_start`, `astrid_http_stream_read`, `astrid_http_stream_close` host functions for real-time SSE consumption (`astrid-capsule`)

### Changed

- CLI no longer embeds the kernel — spawns `astrid-daemon` as a companion binary
- CLI no longer compiles capsules — delegates to `astrid-build` as a companion binary
- CLI reads `IpcMessage` directly from socket instead of wrapping in `AstridEvent::Ipc`
- IPC type imports in CLI now use `astrid-types` directly instead of going through `astrid-events` re-exports
- Package renamed from `astrid-cli` to `astrid` (`cargo install astrid`)

### Removed

- `astrid-kernel` dependency from CLI
- `astrid-openclaw`, `extism`, `cargo_metadata`, `toml_edit` dependencies from CLI
- `Commands::Daemon` and `Commands::WizerInternal` from CLI (moved to `astrid-daemon` and `astrid-build`)

## [0.3.0] - 2026-03-17

### Added

- `astrid-types` crate — shared IPC payload, LLM, and kernel API types with minimal deps (serde, uuid, chrono). WASM-compatible. Both `astrid-events` and the user-space SDK depend on this.
- `yolo` as an alias for `autonomous` workspace mode (`astrid-config`, `astrid-workspace`)

### Changed

- `astrid-events` now re-exports types from `astrid-types` instead of defining them inline. All existing import paths remain valid.
- `astrid-events` `runtime` feature removed — all functionality is now always available. Consumers no longer need `features = ["runtime"]`.

### Removed

- `astrid-sdk`, `astrid-sdk-macros`, `astrid-sys` extracted to standalone repo ([sdk-rust](https://github.com/unicity-astrid/sdk-rust))

## [0.2.0] - 2026-03-15

Initial tracked release. See the [repository history](https://github.com/unicity-astrid/astrid/commits/v0.2.0)
for changes included in this version.

[Unreleased]: https://github.com/unicity-astrid/astrid/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/unicity-astrid/astrid/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/unicity-astrid/astrid/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/unicity-astrid/astrid/releases/tag/v0.2.0
