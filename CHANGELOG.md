# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Changelog tracking starts with 0.2.0. Prior versions were not tracked.

## [Unreleased]

### Changed

- Append-only artifact store — `bin/` and `wit/` are never deleted on capsule remove. Content-addressed artifacts are the audit trail; deleting them breaks provability. Future `astrid gc` for explicit cleanup.
- Replace `[dependencies]` provides/requires string arrays with `[imports]`/`[exports]` namespaced TOML tables — semver version requirements on imports (`^1.0`), exact versions on exports (`1.0.0`), optional imports, namespace/interface name validation
- **WIT spec:** Rewrite `wit/astrid-capsule.wit` to document all 51 host ABI functions (was 7). Split monolithic `host` interface into 11 domain-specific interfaces (fs, ipc, uplink, kv, net, http, sys, cron, process, elicit, approval, identity). Updated guest exports to reflect actual entry points (`astrid_hook_trigger`, `astrid_tool_call`, `run`, `astrid_install`, `astrid_upgrade`). Bumped package version to `0.2.0`.

### Added

- Distro.lock regeneration on `astrid capsule update` — keeps the lockfile in sync after capsule updates
- Content-addressed WIT storage — capsule install hashes `.wit` files into `~/.astrid/wit/`, capsule remove cleans up unreferenced WIT files, `wit_files` field in `meta.json`
- `astrid capsule tree` command — renders the imports/exports dependency graph of all installed capsules, showing which capsule exports satisfy each import, with unsatisfied imports highlighted in red (`astrid capsule deps` retained as hidden alias)
- `astrid init` with distro-based capsule installation — fetches Distro.toml, multi-select provider groups, shared variable prompts with `{{ var }}` template resolution, progress bars, writes Distro.lock for reproducibility. Supports `--distro` flag for custom distros.
- Distro.toml parser and Distro.lock generator — parse distro manifests with full os-release style metadata, shared variables with `{{ var }}` templates, provider groups, uplink roles, and semver validation. Atomic lockfile writes with BLAKE3 hashes for reproducible installs.
- Kernel boot validation — validates every capsule's required `[imports]` has a matching `[exports]` from another loaded capsule, logs errors for unsatisfied required imports and info for optional ones
- `astrid capsule remove` command with dependency safety checks — blocks removal if the capsule is the sole exporter of an interface that another capsule imports (`--force` to override), cleans up content-addressed WASM binaries from `bin/` when no other capsule references the same hash
- Install capsules from GitHub release WASM assets — `astrid capsule install @org/repo` now downloads pre-built `.wasm` binaries from release assets before falling back to clone + build from source
- Per-principal audit chain splitting — each principal maintains its own independent chain per session, independently verifiable via `verify_principal_chain()` and `get_principal_entries()`
- `AuditLog::append_with_principal()` for principal-tagged audit entries
- Auto-provisioning gated on identity store — only `"default"` principal is auto-provisioned when identity store is configured
- Linux FHS-aligned directory layout (`etc/`, `var/`, `run/`, `log/`, `keys/`, `bin/`, `home/`) replacing the flat `~/.astrid/` structure
- `PrincipalId` type for multi-principal (multi-user) deployments — each principal gets isolated capsules, KV, audit, tokens, and config under `home/{principal}/`
- Content-addressed WASM binaries in `bin/` using BLAKE3 hashing — integrity verified on every capsule load (no hash = no load, wrong hash = no load)
- Per-capsule daily log rotation at `home/{principal}/.local/log/{capsule}/{YYYY-MM-DD}.log` with 7-day retention
- `/tmp` VFS mount backed by `home/{principal}/.local/tmp/` for per-principal temp isolation
- Multi-source capsule discovery with precedence: principal > workspace (dedup by name)
- `PrincipalHome` struct with `.local/` and `.config/` following XDG conventions
- Per-invocation principal resolution — KV, audit, logging, and capability checks scope to the calling user per IPC message, not per capsule load
- `IpcMessage.principal` field for carrying the acting principal through event chains (transparent to capsules)
- `AstridUserId.principal` field mapping platform identities to `PrincipalId` with auto-derivation from display name
- Dynamic KV scoping via `invocation_kv` on `HostState` — capsules call `kv::get("key")` and the kernel returns the right value for the current principal
- Principal auto-propagation on `ipc_publish` — capsules never touch the principal, it flows through event chains automatically
- Auto-provisioning of principal home directories on first encounter
- `astrid_get_caller` host function now returns `{ principal, source_id, timestamp }` instead of empty object
- Dynamic per-principal log routing — cross-principal invocations write to the target principal's log directory
- `AuditEntry.principal` field with length-delimited signing data encoding
- `ScopedKvStore::with_namespace()` for creating scoped views sharing the same underlying store
- `AuditEntry::create_with_principal()` builder for principal-tagged audit entries
- `layout-version` sentinel in `etc/` for future migration support
- `lib/` directory reserved for future WIT shared WASM component libraries
- End-to-end Tier 2 OpenClaw plugin support: TypeScript plugins with npm dependencies install, transpile, sandbox, and run as MCP capsules with full tool integration
- OXC `strip_types()` transpiler for Tier 2 TS→JS (preserves ESM, unlike Tier 1's CJS conversion)
- Node.js binary resolution at build time: prefers versioned Homebrew installs (node@22+), validates each candidate
- MCP-discovered tools are now merged into the LLM tool schema injection alongside WASM capsule tools
- `astrid_net_read` now uses a self-describing `NetReadStatus` wire format: every response is prefixed with a discriminant byte (`0x00` = data, `0x01` = closed, `0x02` = pending), replacing the previous single-byte sentinel hack
- Headless mode: `astrid -p "prompt"` for non-interactive single-prompt execution with stdin piping support
- Post-install onboarding: `astrid capsule install` now prompts for `[env]` fields immediately after install
- Shared `astrid_telemetry::log_config_from()` behind `config` feature flag — replaces duplicate config bridge code

### Fixed

- Dispatcher `known_principals` HashSet capped at 10K entries to prevent unbounded memory growth
- Dispatcher only caches principal after successful home provisioning — transient failures allow retry on next event
- `AstridUserId.principal` now has `#[serde(default)]` — existing identity records without the field deserialize with `"default"` instead of failing
- `transpile_and_install` now correctly unpacks `.capsule` archives from `astrid-build` output
- `copy_capsule_dir` only skips `dist/` at the top level; npm packages inside `node_modules` retain their `dist/` directories
- MCP host engine: absolute system binaries (e.g. `/opt/homebrew/opt/node@22/bin/node`) skip path traversal check when declared in `host_process` capability
- MCP host engine: `allow_network` derived from capsule capabilities (uplink/net) instead of defaulting to `false`
- Capsule env resolution no longer blocks loading on missing optional fields; fills with empty defaults so uplink capsules can boot before clients connect
- macOS Seatbelt sandbox: added `mach*` permission and unrestricted `file-read*` for Node.js compatibility
- macOS Seatbelt sandbox: hidden path deny rules skip paths that are ancestors of the writable root
- MCP tool schemas now include `properties` field for LLM API compatibility
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

- `global://` VFS scheme renamed to `home://`
- `Capsule::invoke_interceptor` now accepts `Option<&IpcMessage>` for per-invocation principal context
- `CapsuleContext.global_root` renamed to `home_root`; `HostState.global_vfs` renamed to `home_vfs`
- `AstridUserId` now requires `principal: PrincipalId` field (existing KV records incompatible — nuke `~/.astrid/`)
- Capsule install target moved from `~/.astrid/capsules/` to `home/{principal}/.local/capsules/` (capsule dir now holds only manifest + meta.json)
- KV namespace format changed from `capsule:{name}` to `{principal}:capsule:{name}`
- Socket/token/ready paths moved from `sessions/` to `run/`
- Env config moved from capsule dir `.env.json` to `home/{principal}/.config/env/{capsule}.env.json`
- System logs now use `.log` extension, no ANSI escape codes in file output, 7-day retention
- `user_key_path()` renamed to `runtime_key_path()` (now at `keys/runtime.key`), `logs_dir()` renamed to `log_dir()`
- Ephemeral daemon now shuts down immediately when the last client disconnects (idle timeout 0, 1s check interval) instead of waiting 5 minutes
- Renamed `plugin` → `capsule` in the WASM host layer and audit log fields for consistency with project terminology
- Split `astrid-build` 1166-line `build.rs` into focused modules: `rust.rs`, `openclaw.rs`, `mcp.rs`

### Removed

- `~/.astrid/capsules/` system capsules directory (user installs go to principal home)
- `sessions/`, `shared/`, `audit.db`, `capabilities.db`, `state/`, `spark.toml`, `cache/capsules/` — replaced by FHS equivalents or moved to principal home

### Breaking

- Existing `~/.astrid/` must be deleted — no migration path. Reinstall all capsules after upgrading.

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
