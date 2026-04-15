# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Build / Test / Lint

```bash
# Build entire workspace
cargo build --workspace

# Test (set ASTRID_AUTO_BUILD_KERNEL=1 for tests that need the QuickJS WASM kernel)
ASTRID_AUTO_BUILD_KERNEL=1 cargo test --workspace

# Single crate test
cargo test -p astrid-events

# Single test by name
cargo test -p astrid-approval -- test_name

# Lint (CI runs both; clippy is pedantic + denies arithmetic overflow)
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --all -- --check

# Build release binaries (astrid, astrid-daemon, astrid-build)
cargo build --release
```

Rust edition 2024, MSRV 1.94. The `wasm32-wasip1` target is needed for capsule compilation.

## Architecture

Astrid is a user-space microkernel OS for AI agents. The kernel is native Rust; everything above it runs as isolated WASM capsules.

### The kernel / user-space divide

The **kernel** (`astrid-daemon`) owns all privileged resources: VFS, IPC bus, capsule registry, audit log, KV store, capability tokens, approval gates. It listens on a Unix domain socket (`~/.astrid/run/system.sock`). The **CLI** (`astrid`) connects to this socket, renders TUI output, and forwards user input. `astrid-build` compiles capsule source into WASM.

**Capsules** are WASM processes with zero ambient authority. Every external resource (filesystem, network, IPC, KV) is gated behind a capability-checked host function. The host ABI is a flat syscall table of 49 functions. The SDK (`astrid-sdk`, separate repo) wraps these in `std`-like ergonomics.

### IPC event bus

All inter-capsule communication flows through `EventBus` (tokio broadcast channel). Messages are `IpcMessage` structs: a topic string, an `IpcPayload` enum (tagged JSON), source UUID, timestamp, sequence number, and optional principal. Tools, LLM providers, and frontends are all IPC conventions — the kernel has no knowledge of tool schemas or provider metadata. Capsules register **interceptors** on IPC topics (eBPF-style middleware returning `Continue`/`Final`/`Deny`).

### Capsule lifecycle

A `Capsule.toml` manifest declares `[imports]`/`[exports]` with namespaced interface names and semver requirements. The kernel resolves dependencies via topological sort and boots capsules in order. Engines: WASM (sandboxed), MCP (JSON-RPC subprocess), Static (declarative context). The `#[capsule]` proc macro generates all ABI boilerplate.

### Security model

Five layers in sequence: Policy (hard blocks) → Token (ed25519 capability tokens with glob patterns) → Budget (per-session + per-workspace atomic limits) → Approval (human-in-the-loop) → Audit (chain-linked signed log). Implemented in `SecurityInterceptor` in `astrid-approval`.

### Uplinks

An **uplink** is any component that sends/receives messages on behalf of the runtime (CLI, Discord, Telegram, bridges). Defined in `astrid-core::uplink` with `UplinkDescriptor`, `UplinkCapabilities`, `UplinkProfile`, and `InboundMessage` types. Capsules can register uplinks via the `astrid_uplink_register` host function.

### Key crate roles

- `astrid-kernel` — boots runtime, owns VFS/IPC/capsules/audit/KV, serves Unix socket
- `astrid-capsule` — manifest parsing, WASM/MCP/static engines, toposort, registry, hot-reload
- `astrid-events` — broadcast event bus, IPC types (re-exports from `astrid-types`)
- `astrid-types` — canonical IPC/LLM/kernel API schemas (minimal deps, WASM-compatible)
- `astrid-approval` — the five-layer security gate
- `astrid-audit` — chain-linked cryptographic audit log (SurrealKV-backed)
- `astrid-vfs` — copy-on-write overlay filesystem (`Vfs` trait, `HostVfs`, `OverlayVfs`)
- `astrid-core` — foundation types (`SessionId`, `PrincipalId`, uplinks, identity, session tokens)
- `astrid-crypto` — ed25519 key pairs, BLAKE3 hashing, zeroize-on-drop
- `astrid-storage` — two-tier persistence (SurrealKV raw KV + SurrealDB query engine)
- `astrid-config` — layered TOML config (workspace > user > system > env > defaults)
- `astrid-openclaw` — TypeScript-to-WASM compiler (OXC + QuickJS/Wizer pipeline)

### Code constraints

- `#![deny(unsafe_code)]` everywhere except `astrid-sys` and `astrid-sdk` (WASM FFI)
- Clippy pedantic; `clippy::arithmetic_side_effects = "deny"` — use checked/saturating arithmetic
- Individual files must not exceed 1000 lines
- `CHANGELOG.md` must be updated under `[Unreleased]` for every PR

## Sibling project: minime (`/Users/v/other/minime`)

**MikesSpatialMind** — a dual-layer consciousness engine. The Rust backend (`minime/`) runs an Echo State Network (ESN) for spectral homeostasis; the Python frontend (`mikemind/`) drives Ollama LLM conversation and camera vision.

### Minime architecture

The Rust engine processes 18D sensory input (8D video + 8D audio + 2D introspection) through a 128-node ESN reservoir. A PI controller (`regulator.rs`) regulates eigenvalue fill toward a 55% target. Telemetry broadcasts via WebSocket:

| Port | Protocol | Direction |
|------|----------|-----------|
| 7878 | JSON `EigenPacket` (spectral telemetry) | Engine → subscribers |
| 7879 | JSON `SensoryMsg` (video/audio/aux/semantic/control) | External → engine |
| 7880 | Binary 128x128 grayscale frames | Camera → GPU pipeline |

Key types: `SensoryMsg` (tagged enum: `Video`, `Audio`, `Aux`, `Semantic`, `Control`), `SpectralMsg` (`t_ms`, `lambdas`, `lambda1`), `SensoryBus` (lock-free lane architecture).

### Communication bus design surface

For bridging Astrid and minime, the natural integration points are:

**Astrid side:**
- `EventBus` publishes `AstridEvent::Ipc { message: IpcMessage }` with topic-based routing
- Uplink system allows capsules to register as external message sources/sinks
- `astrid_net_bind_unix` / `astrid_http_stream_*` host functions give capsules network access
- `IpcPayload::RawJson` can carry arbitrary JSON (minime telemetry fits directly)

**Minime side:**
- `WsHub` broadcasts `SpectralMsg` to all WebSocket subscribers on port 7878
- `SensoryMsg` on port 7879 accepts typed JSON input (video/audio/aux/semantic/control)
- The `Control` variant allows external regulation of synth gain, keep bias, exploration noise, and fill target

**Bridge pattern:** A capsule (or native uplink) subscribes to minime's WebSocket telemetry stream and publishes it as IPC messages on a topic like `consciousness.v1.telemetry`. In the reverse direction, Astrid IPC events (tool results, user input) can be forwarded as `SensoryMsg::Semantic` features to minime's sensory input port, coupling the agent's symbolic reasoning to the spectral substrate.
