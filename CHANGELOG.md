# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Changelog tracking starts with 0.2.0. Prior versions were not tracked.

## [Unreleased]

### Added

- `astrid-daemon` crate — standalone kernel daemon binary with `--ephemeral` flag for CLI-spawned instances vs persistent multi-frontend mode
- `astrid-build` crate — standalone capsule compiler and packager (Rust, OpenClaw, MCP). Invoked by CLI via subprocess.
- `astrid start` command — spawn a persistent daemon (detached, no TUI)
- `astrid status` command — query daemon PID, uptime, connected clients, loaded capsules
- `astrid stop` command — graceful daemon shutdown via management API
- `KernelRequest::Shutdown`, `KernelRequest::GetStatus`, and `DaemonStatus` types in `astrid-types`
- `Kernel::boot_time` field for uptime tracking

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

[Unreleased]: https://github.com/unicity-astrid/astrid/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/unicity-astrid/astrid/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/unicity-astrid/astrid/releases/tag/v0.2.0
