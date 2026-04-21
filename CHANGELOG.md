# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Changelog tracking starts with 0.2.0. Prior versions were not tracked.

## [Unreleased]

### Breaking

- **WASM engine migrated from Extism to wasmtime Component Model.** The kernel now loads Component Model binaries via `Component::from_binary`, not Extism modules. Existing capsules compiled with `extism-pdk` will not load — they must be rebuilt with the migrated SDK targeting `wasm32-wasip2`. This is a coordinated multi-repo migration (SDK + 16 capsule repos). (#632)
- **WIT host function signatures retyped.** All 49 functions now use proper typed params/returns (`result<T, string>`, WIT records, `u64` handles) instead of `string`-based JSON blobs. The `HostResult` 0x00/0x01 prefix encoding is removed — errors are returned via WIT `result` types. (#632)
- **Guest export `astrid-hook-trigger` signature changed.** Was `func(input: list<u8>) -> list<u8>`. Now `func(action: string, payload: list<u8>) -> capsule-result`. The action name and payload are separate typed parameters; the return is the typed `capsule-result` record. (#632)
- **`capsule_abi` module removed from `astrid-core`.** Types (`CapsuleAbiContext`, `CapsuleAbiResult`, `LogLevel`, etc.) are replaced by `wasmtime::component::bindgen!` generated types. (#632)
- **Approval API simplified.** `risk-level` removed from `approval-request` WIT record. `decision` removed from `approval-response`. Capsules declare action + resource, get back approved/denied. Risk classification was speculative complexity — the kernel manages allowance-based approval without risk levels. (#641)

### Fixed

- **bwrap mount ordering hides capsule directory on Linux.** Hidden `--tmpfs` overlays (e.g. `~/.astrid`) were applied before writable `--bind` mounts, erasing capsule directories inside the hidden path. Reordered so bind-mounts come after tmpfs and punch through. Mirrors the ancestor check already present in the macOS Seatbelt path (PR #534). (#648)
- **bwrap silently fails on Ubuntu 24.04+ (AppArmor).** `kernel.apparmor_restrict_unprivileged_userns=1` blocks user namespaces required by bwrap. Added a cached probe at startup that detects this and falls back to unsandboxed execution with a clear warning and distro-specific install instructions. (#648)
- **`capsule remove` no longer deletes env config by default.** User configuration (API keys, secrets) in `env.json` is preserved across uninstall/reinstall cycles. Use `--purge` to explicitly delete saved configuration. (#647)
- **`astrid-build` targets `wasm32-wasip2`** for Component Model capsules. Was still targeting `wasm32-wasip1`, producing plain WASM modules. (#649)
- **`astrid-build` bundles SDK shared WIT** (`astrid-contracts.wit`) into capsule archives as a WIT dep, so `wit_type` references in `Capsule.toml` resolve at install time without manual WIT duplication. (#649)
- **JSON Schema field names converted to snake_case** in `wit_schema` to match `serde(rename_all = "snake_case")` wire convention. (#649)
- **`reqwest::blocking` inside `#[tokio::main]` panics on first run.** All HTTP call sites in `self_update.rs`, `init.rs`, and `capsule/install.rs` used `reqwest::blocking::Client`, which creates an internal tokio runtime that panics on drop inside the outer async context. Converted to async `reqwest`. Only manifested on fresh installs (no cache/lock files to short-circuit). (#645)
- **`INTERNAL_SUBSCRIBER_COUNT` debug_assert race.** `EventDispatcher` subscribed to the event bus inside `tokio::spawn(dispatcher.run())`, so the assert could fire before the spawned task started. Moved subscription into `EventDispatcher::new()`. (#645)

### Added

- **Per-principal `PrincipalProfile` + `profile.toml`.** New `astrid_core::profile` module with the per-principal policy struct (enablement, groups, auth methods, network egress, process spawn, resource quotas) plus loader and atomic writer at `~/.astrid/home/{principal}/.config/profile.toml`. Missing file falls back to `Default`; malformed TOML, unknown fields, failed validation, or a future `profile_version` are hard errors. Save is atomic on Unix (temp write at `0o600`, then `rename`). Validation fires on both load and save. Pure data plumbing — Layer 3 enforcement in `invoke_interceptor`, hot-reload, management IPC, and CLI are separate follow-ups. (#663)
- **Content-addressed WIT store** at `~/.astrid/wit/{blake3}.wit`. Install-time WIT files (including `deps/`) are recursively hashed, deduped, and stored with atomic writes. Per-capsule `wit/` is removed after addressing; `meta.json.wit_files` is the authoritative manifest. Append-only by design for replay preservation. (#649)
- **`astrid wit gc`** — admin-only mark-sweep GC for the WIT content store. Dry-run by default, `--force` to delete. Scans all principal homes + workspace. (#649)
- **Per-invocation `home://` and `/tmp/` VFS scoping.** A shared capsule serving multiple agents now resolves `home://` and `/tmp/` against the *invoking* agent's home directory (`~/.astrid/home/{principal}/`) instead of the capsule owner's. The security gate accepts a `principal_home` parameter and `WasmEngine::invoke_interceptor` builds a per-principal VFS bundle when the invocation principal differs from the capsule's. Unregistered principals (no home directory on disk) receive a clean denial — the kernel does not auto-create principal homes. Single-tenant installs (all traffic under `default`) see no behavior change. Precursor to multi-tenancy (#653). (#549)
- **Per-invocation `SecretStore` and capsule log re-scoping.** `has_secret` now reads secrets from the *invoking* agent's KV namespace (and OS keychain scope), and `astrid_log` writes to the invoking agent's `~/.astrid/home/{principal}/.local/log/{capsule}/{date}.log`. `HostState` gains `invocation_secret_store` / `invocation_capsule_log` fields plus `effective_secret_store()` / `effective_capsule_log()` accessors; `WasmEngine::invoke_interceptor` installs per-principal resources alongside the existing invocation VFS bundle and clears them on exit. Unregistered principals receive `None` — no attacker home is auto-created. Finishes #653 Layer 1's side-channel isolation started in #659. (#661)

### Removed

- **Raw `.wasm` release asset install paths.** Capsule distribution is now `.capsule` archive or clone+build. Raw WASM assets can't carry WIT dependencies. (#649)
- **`install_standard_wit()` from init.** Fetched stale per-interface WIT from upstream repo; shared contracts are now bundled by `astrid-build` into each capsule archive. (#649)

### Added (prior)

- **WIT-driven IPC topic schemas.** Capsules declare `wit_type = "record-name"` on `[[topic]]` entries in `Capsule.toml`. At install time, `wit-parser` reads the record from the capsule's `wit/` directory, extracts field names, types, and `///` doc comments into JSON Schema, and bakes it into `meta.json`. At runtime, `WasmEngine::load()` populates the `SchemaCatalog` from baked schemas. The LLM sees typed field descriptions without capsule authors writing JSON Schema by hand. (#643)
- `astrid-build::wit_schema` module — converts WIT records to JSON Schema. Handles primitives, `option<T>`, `list<T>`, tuple, enum, flags, variant, result, nested records, and type aliases. (#643)
- `wit_type: Option<String>` field on `TopicDef` in `Capsule.toml` — references a WIT record by kebab-case name. (#643)
- Schema catalog (`SchemaCatalog`) for A2UI Track 2 — maps IPC topics to schema definitions. Populated at capsule load time from baked `meta.json` schemas. (#632, #643)
- Epoch-based WASM timeout with `EpochTickerGuard` RAII type — replaces Extism wall-clock timeout. 5-minute deadline for interceptors, u64::MAX for daemons/run-loops, 10-minute safety net for lifecycle hooks. (#632)
- 64MB per-capsule WASM memory limit via `StoreLimitsBuilder` (matches old Extism setting). Global budget for multi-tenant hosting is a follow-up (#639). (#632)
- New WIT record types: `spawn-request`, `interceptor-handle`, `net-read-status` (variant), `capability-check-request/response`, `identity-*-request`, `elicit-request`. (#632)

### Removed

- `extism` dependency — replaced by direct `wasmtime` 43 + `wasmtime-wasi` 43. (#632)
- `capsule_abi.rs` (252 lines) — hand-written WIT type mirrors. (#632)
- `host/shim.rs` (430 lines) — Extism dispatch shim, `WasmHostFunction` enum, `register_host_functions()`, manual memory helpers. (#632)
- `RiskLevel` enum and all references — removed from WIT, IPC payloads, approval engine, audit entries, CLI renderers, policy engine, and test fixtures. Approval prompts now render with a single style. The allowance store handles "don't ask again" patterns without risk classification. (#641)

## [0.5.1] - 2026-03-25

### Added

- `cargo install astrid` now also installs `astrid-build` (capsule compiler) alongside `astrid` and `astrid-daemon`. Previously required a separate `cargo install astrid-build`.

### Fixed

- `astrid capsule install` no longer blocks when a new capsule exports an interface already exported by an installed capsule. Multiple providers (e.g. two LLM providers) can now coexist — prints an informational note instead of prompting for replacement.

## [0.5.0] - 2026-03-24

### Changed

- `workspace://` VFS scheme renamed to `cwd://` — the scheme maps to the daemon's CWD at boot; the old name implied a structured project workspace concept that was never implemented.

- **Tools are now a pure IPC convention.** Removed kernel-side tool dispatch (`WasmCapsuleTool`, `CapsuleTool` trait, `inject_tool_schemas`, `CapsuleToolContext`), `ToolDef` and `[[tool]]` from manifest, `inject_tool_schemas` from `astrid-build`. The kernel no longer parses or manages tool schemas. Tool capsules use IPC interceptors on `tool.v1.execute.<name>` and `tool.v1.request.describe`. The router capsule handles discovery and dispatch.
- **LLM providers are now a pure IPC convention.** Removed `LlmProviderDef` and `[[llm_provider]]` from manifest, `LlmProviderInfo` and `llm_providers` from `CapsuleMetadataEntry`. The kernel no longer parses or manages provider metadata. LLM capsules self-describe via `llm.v1.request.describe` interceptors; the registry capsule discovers them via `hooks::trigger`.
- **Removed dead cron host functions.** `astrid_cron_schedule` and `astrid_cron_cancel` were never implemented (stubs only). `CronDef` and `[[cron]]` removed from manifest. WIT spec updated: 49 host functions across 10 domain interfaces.
- Append-only artifact store — `bin/` and `wit/` are never deleted on capsule remove. Content-addressed artifacts are the audit trail; deleting them breaks provability. Future `astrid gc` for explicit cleanup.
- Replace `[dependencies]` provides/requires string arrays with `[imports]`/`[exports]` namespaced TOML tables — semver version requirements on imports (`^1.0`), exact versions on exports (`1.0.0`), optional imports, namespace/interface name validation
- **WIT spec:** Rewrite `wit/astrid-capsule.wit` to document all 51 host ABI functions (was 7). Split monolithic `host` interface into 11 domain-specific interfaces (fs, ipc, uplink, kv, net, http, sys, cron, process, elicit, approval, identity). Updated guest exports to reflect actual entry points (`astrid_hook_trigger`, `astrid_tool_call`, `run`, `astrid_install`, `astrid_upgrade`). Bumped package version to `0.2.0`.

### Added

- `cargo install astrid` installs both `astrid` (CLI) and `astrid-daemon` binaries from a single crate. The CLI crate now includes the daemon as a second `[[bin]]` entry point.
- `astrid self-update` command — checks GitHub releases for newer versions, downloads platform-specific binary to `~/.astrid/bin/`, no sudo required. Startup update banner (cached 24h) notifies on interactive commands.
- `astrid init` PATH setup — detects shell (zsh/bash/fish), offers to append `~/.astrid/bin` to the appropriate RC file
- Standard WIT interface installation during `astrid init` — fetches 9 WIT files (llm, session, spark, context, prompt, tool, hook, registry, types) from the canonical WIT repo and installs to `~/.astrid/home/{principal}/wit/` for capsule and LLM access via `home://wit/`
- Short-circuit interceptor chain — interceptors return `Continue`, `Final`, or `Deny` to control the middleware chain. A guard at priority 10 can veto an event before the core handler at priority 100 ever sees it. Wire format: discriminant byte (0x00/0x01/0x02) + payload, backward compatible with existing capsules.
- Export conflict detection on `capsule install` — detects when a new capsule exports interfaces already provided by an installed capsule, prompts user to replace. Nix-aligned approach: conflicts derived from exports data, no name-based `supersedes` field needed.
- Interceptor priority — `priority` field on `[[interceptor]]` in Capsule.toml (lower fires first, default 100). Enables layered interception (e.g. input guard before react loop).
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
- `--snapshot-tui` mode — renders the full TUI to stdout as ANSI-colored text frames using ratatui's `TestBackend`. Each significant event (ready, input, tool call, approval, response) produces a frame dump. Configurable with `--tui-width` and `--tui-height`. Enables automated smoke testing without an interactive terminal.

### Fixed

- `cwd://` VFS scheme was handled in the security gate (capability checks) but not in the runtime path resolver — capsules using `cwd://` paths at runtime received a security denial because the path resolved to `<cwd>/cwd:/path` instead of `<cwd>/path`
- `sandbox-exec` (Seatbelt) crashes with SIGABRT on macOS 15+ (Darwin >= 24) — skip sandboxing on affected versions
- Headless approval response published to wrong IPC topic (`astrid.v1.approval.response` instead of `astrid.v1.approval.response.{request_id}`) and used wrong decision string (`allow` instead of `approve`)
- `[[component]].capabilities` (fs_read, fs_write, host_process) not merged into root capabilities — security gate couldn't see them
- Lifecycle hooks (`on_install`) couldn't access `home://` VFS — added `home_root` to `LifecycleConfig`
- `astrid init` standard WIT files were installed to `~/.astrid/wit/astrid/` (root-level, no VFS scheme). Capsules access the VFS via `home://` which maps to `~/.astrid/home/{principal}/` — the files were unreachable. Now installed to `~/.astrid/home/{principal}/wit/`, accessible as `home://wit/` (fixes #598)
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
- `astrid_read_file` host function trapped (WASM abort) on recoverable errors (file-not-found, permission denied) — now returns status-prefix wire format (`0x00`+content / `0x01`+error), paired with SDK-side decoding. Eliminates crashes in memory, agents, identity, and fs capsules when reading optional files.

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
