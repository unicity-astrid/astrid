# Astrid

**An operating system for AI agents.**

[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)
[![Rust 2024](https://img.shields.io/badge/Rust-2024_edition-orange)](https://www.rust-lang.org)

---

Astrid is a user-space microkernel that treats AI agents the way Linux treats processes. It has a kernel with a boot sequence, a virtual filesystem with copy-on-write overlay, ed25519 capability tokens, an IPC event bus, WASM process isolation, and a cryptographic audit trail where each entry hashes the previous.

The kernel is fixed. Everything else is a swappable **capsule**: providers, orchestrators, tools, frontends, interceptors. You do not fork Astrid to customize it. You compose capsules into a configuration that fits your use case. Same core OS, different capsule sets, infinite configurations.

Currently v0.3.0. Runs in user space. The only frontend today is the built-in CLI (`astrid chat`). The architecture is designed for unikernel deployment.

## Why capsules matter

Most agent frameworks bake their assumptions into the code. The LLM provider is a library import. The orchestration loop is a hardcoded function. The tool set is a static list. Changing any of these means forking the framework, understanding its internals, and maintaining a divergent copy.

Astrid inverts this. The kernel provides sandboxing, IPC, a filesystem, capability tokens, budget enforcement, and audit. Everything above that boundary is a capsule: an isolated WASM process described by a `Capsule.toml` manifest. Capsules declare what they provide (`tool:search_issues`, `llm:claude-sonnet`, `uplink:telegram`) and what they require. The kernel resolves dependencies via topological sort and boots them in order.

This is not a plugin system bolted onto an application. It is the application's architecture.

### What this makes possible

**Run completely offline.** Swap the Anthropic provider capsule for one that talks to Ollama or vLLM. Everything else works identically. The orchestrator does not know or care which model backend is running. Compliance teams approve the provider capsule in isolation, not the whole codebase. Most frameworks hardcode their LLM provider. Astrid makes it a config change.

**Build novel agent architectures.** Write a custom orchestrator capsule: a debate system with three specialist personas, a Monte Carlo tree search planner, a chain-of-verification loop. Plug it in and the rest of the OS works unchanged. Agent architecture is the frontier of AI research. Astrid gives researchers a production runtime where sandboxing, budget enforcement, and audit are already solved, so they can focus on the intelligence layer.

**Cut LLM costs with transparent caching.** Install a caching capsule as middleware between orchestrator and provider. Seen this prompt before? Return the cached response with no API call. Neither orchestrator nor provider needs modification. Composable middleware that drops in as a capsule.

**Autonomous agents that work overnight.** Replace the default orchestrator with an autonomous worker capsule. It generates code, runs tests, reads errors, self-corrects, and loops until green. Same tools, same providers, same audit trail, different decision-making loop. The difference between a chatbot and an autonomous agent is the orchestration logic, and that logic is a swappable capsule.

**Mix and match providers.** Run multiple provider capsules simultaneously: Anthropic, OpenAI, a local model. A routing capsule examines each request and picks the best provider by complexity, cost, or latency. Every provider speaks the same event schema. Routing is just another capsule.

**Ship custom distros.** Same core OS, different capsule sets per customer. Enterprise A gets a strict approval-gated orchestrator. Startup B gets an autonomous worker with a local model. Package as a `config.toml` plus capsules. One codebase, infinite configurations. Security patches ship to everyone simultaneously.

**Self-modifying agents.** An agent decides its memory system is inadequate and installs a better memory capsule. It determines a task requires a more capable model and upgrades its own provider capsule. It restructures its orchestration mid-session. Every component above the kernel is swappable at the discretion of a human or the agent itself. The agent is not trapped inside a fixed architecture. It evolves its own capabilities at runtime, within the boundaries the kernel enforces. The approval gate means a human can require sign-off on self-modification, or the agent can be granted a capability token that lets it reconfigure autonomously. Same security model either way.

> These scenarios are architecturally possible today. The kernel, IPC bus, capsule manifest system, and dependency resolver all exist and are tested. The capsule types for LLM providers, orchestrators, tools, interceptors, uplinks, and cron jobs are all defined in the manifest schema. What varies is how many capsules have been built on top of this foundation so far.

## The agent has agency. The human has authority.

Traditional computing puts the human at the center. The human operates the computer, drives the tools, makes every decision. AI agents invert this relationship. The agent operates, reasons, and acts. The human supervises, approves, and steers. This is not a minor workflow change. It is a fundamental shift in how humans and computers relate to each other.

Most agent frameworks ignore this shift. They bolt an LLM onto a traditional application and hope for the best. Astrid is built for the inverted model from the ground up. The kernel enforces boundaries so the agent can act freely within them. The approval gates keep humans in the loop at the moments that matter, not at every step. The audit trail provides cryptographic accountability for every action the agent takes. The capability system lets trust expand gradually as the agent proves reliable.

The operating system sits between the agent and the world, the same way an OS sits between a process and hardware. The agent has agency. The human has authority. The kernel mediates.

How much authority you keep is up to you. `mode = "safe"` asks before every action outside the workspace. `mode = "guided"` auto-allows reads, asks for writes. `mode = "autonomous"` takes the guardrails off. And for daring Astrinauts: `mode = "yolo"`.

## The security model

Every sensitive action passes through five layers before it executes:

```text
Agent proposes action
       |
  [1. Policy]    Hard blocks. "sudo" is always denied. Path traversal is always denied.
       |         Admin-controlled deny lists, allowed paths, denied hosts.
       |         Cannot be overridden by tokens or approvals.
       |
  [2. Token]     Does a valid ed25519 capability token cover this action?
       |         Scoped to resource patterns via globset matching.
       |         Time-bounded. Linked to the audit entry that created it.
       |
  [3. Budget]    Is the session within its spending limit?
       |         Per-action and per-session limits, enforced atomically.
       |         Dual-budget: session budget AND workspace budget must both allow.
       |         Reservation-based: cost is held during approval, refunded on denial.
       |
  [4. Approval]  No token? Ask the human.
       |         Allow Once / Allow Session / Allow Workspace / Allow Always / Deny.
       |         "Allow Always" mints a signed capability token for next time.
       |         "Allow Session" creates a scoped allowance that auto-matches future calls.
       |         Human unavailable? The action queues, not silently skips.
       |
  [5. Audit]     Every decision - allowed, denied, deferred - is logged.
                 Each entry is signed by the runtime's ed25519 key.
                 Each entry contains the content hash of the previous.
                 Tamper with the history and the chain breaks.
```

This is real code. [`SecurityInterceptor`](crates/astrid-approval/src/interceptor/mod.rs) implements this exact flow. The tests cover policy blocks, budget exhaustion, budget reservation refund on denial, budget refund on async cancellation, capability token authorization, the "Allow Session" allowance minting path, and the "Allow Always" token minting path.

## Two sandboxes

**WASM sandbox.** Capsules run in WebAssembly via Extism/Wasmtime. No syscalls, no file descriptors, no host memory access. Every external resource (filesystem, network, IPC, KV storage) is gated behind a capability-checked host function. The [syscall table](crates/astrid-sys/src/lib.rs) defines 48 host functions across filesystem, IPC, storage, network, identity, lifecycle, process management, approval, scheduling, hooks, and clock subsystems. Hard limits: 64 MB memory ceiling (1024 WASM pages), 10-second wall-clock timeout for tool capsules, BLAKE3 hash verification on capsule source trees.

**VFS overlay.** The agent operates against a copy-on-write filesystem. The workspace is the read-only lower layer. Writes go into an ephemeral upper layer backed by a temp directory. Session ends: commit the diff to the workspace, or drop the temp directory to discard. Path traversal (`../../etc/passwd`) is rejected at the VFS layer before reaching the host filesystem. File handles use capability-based `DirHandle`/`FileHandle` types. You cannot construct a path to a directory you have not been granted.

## The host ABI

The WASM-to-host boundary is a flat syscall table. WASM guests cannot import arbitrary host functions. The [complete ABI](crates/astrid-sys/src/lib.rs) covers:

| Subsystem | Syscalls |
|---|---|
| **Filesystem** | `astrid_fs_exists`, `astrid_read_file`, `astrid_write_file`, `astrid_fs_mkdir`, `astrid_fs_readdir`, `astrid_fs_stat`, `astrid_fs_unlink` |
| **IPC** | `astrid_ipc_publish`, `astrid_ipc_subscribe`, `astrid_ipc_recv` (blocking), `astrid_ipc_poll` (non-blocking), `astrid_ipc_unsubscribe` |
| **Uplinks** | `astrid_uplink_register`, `astrid_uplink_send` |
| **Storage** | `astrid_kv_get`, `astrid_kv_set`, `astrid_kv_delete`, `astrid_kv_list_keys`, `astrid_kv_clear_prefix` |
| **Network** | `astrid_http_request`, `astrid_net_bind_unix`, `astrid_net_accept`, `astrid_net_poll_accept`, `astrid_net_read`, `astrid_net_write`, `astrid_net_close_stream` |
| **Identity** | `astrid_identity_resolve`, `astrid_identity_link`, `astrid_identity_unlink`, `astrid_identity_create_user`, `astrid_identity_list_links` |
| **Lifecycle** | `astrid_elicit` (user input during install), `astrid_has_secret`, `astrid_signal_ready`, `astrid_get_caller`, `astrid_get_config` |
| **Process** | `astrid_spawn_host`, `astrid_spawn_background_host`, `astrid_read_process_logs_host`, `astrid_kill_process_host` |
| **Approval** | `astrid_request_approval` (blocks guest until human responds or timeout) |
| **Security** | `astrid_check_capsule_capability` |
| **Scheduling** | `astrid_cron_schedule`, `astrid_cron_cancel` |
| **Hooks** | `astrid_trigger_hook`, `astrid_get_interceptor_handles` |
| **Clock** | `astrid_clock_ms` |
| **Logging** | `astrid_log` |

Every parameter crosses the boundary as raw `Vec<u8>`. No string encoding validation at the ABI layer. The [SDK](crates/astrid-sdk/) adds typed ergonomics on top, mirroring `std` module layout: `fs`, `net`, `process`, `env`, `time`, `log`, plus Astrid-specific modules: `ipc`, `kv`, `http`, `hooks`, `cron`, `uplink`, `identity`, `approval`, `runtime`.

## Capsules

Capsules are processes in the OS model: isolated execution units described by a `Capsule.toml` manifest. A capsule can combine multiple engines:

- **WASM** - compiled sandbox, full host ABI access via syscalls
- **MCP** - native subprocess proxied via JSON-RPC (wraps `rmcp`, MCP 2025-11-25 spec)
- **Static** - declarative context injection (files, prompts)

A manifest can declare tools, commands, skills, interceptors, cron jobs, IPC topics, LLM providers, uplinks, and MCP servers. Dependencies between capsules are resolved via topological sort at boot. Capsules declare what they `provide` and `require` using typed capability prefixes (`tool:`, `llm:`, `topic:`, `uplink:`), and the kernel ensures all requirements are satisfied before starting a capsule.

```rust
use astrid_sdk::prelude::*;

#[derive(Default)]
pub struct MyTools;

#[capsule]
impl MyTools {
    #[astrid::tool]
    fn search_issues(&self, args: SearchArgs) -> Result<SearchResult, SysError> {
        let token = env::var("GITHUB_TOKEN")?;
        let resp = http::get(&format!(
            "https://api.github.com/search/issues?q={}", args.query
        ))?;
        // ...
    }
}
```

The `#[capsule]` proc macro generates all WASM ABI boilerplate: `extern "C"` exports, JSON serialization across the boundary, tool schema generation, and dispatch routing for tools, commands, hooks, cron handlers, install, and upgrade entry points. Capsule authors depend on `astrid-sdk` and `serde`.

TypeScript and JavaScript plugins from the OpenClaw ecosystem compile to WASM via an all-Rust pipeline (OXC transpiler, QuickJS/Wizer, export stitcher). No Node.js required.

## Quick start

**Prerequisites:** Rust 1.94+, an Anthropic API key (or any OpenAI-compatible endpoint).

```bash
git clone https://github.com/unicity-astrid/astrid.git
cd astrid
cargo build --release

# Initialize a workspace
./target/release/astrid init

# Start a session (daemon boots automatically)
ANTHROPIC_API_KEY=sk-... ./target/release/astrid chat

# Or start a persistent daemon for multi-frontend use
./target/release/astrid start
./target/release/astrid status
./target/release/astrid stop
```

Three binaries work together: `astrid` (CLI frontend), `astrid-daemon` (kernel process), and `astrid-build` (capsule compiler). When you run `astrid chat`, the CLI spawns the daemon as a background process, connects over a Unix domain socket, and renders streaming events. The daemon manages VFS, capsules, IPC, audit, and security. The CLI manages input and display.

A starter distro with pre-built capsules for a complete coding agent experience is coming soon.

## Architecture

Astrid follows a strict kernel/user-space divide. The kernel (native Rust daemon) owns all privileged resources. Capsules (WASM guests) have zero ambient authority and must request everything through the host ABI.

### Kernel crates

| Crate | Role |
|---|---|
| `astrid-kernel` | Boots the runtime. Owns VFS, IPC bus, capsule registry, MCP client, audit log, KV store. Listens on Unix socket for CLI connections. |
| `astrid-approval` | `SecurityInterceptor`: the five-layer gate. Policy engine, budget tracker, allowance store, approval manager. |
| `astrid-capabilities` | Ed25519-signed capability tokens with glob resource patterns and time bounds. |
| `astrid-audit` | Chain-linked cryptographic audit log. Each entry is signed and hashes the previous. SurrealKV-backed with chain verification. |
| `astrid-vfs` | Copy-on-write overlay filesystem. `Vfs` trait with `HostVfs` and `OverlayVfs` implementations. Capability-based `DirHandle`/`FileHandle`. |
| `astrid-events` | IPC event bus. Broadcast-based with async receivers and synchronous subscriber callbacks. Types re-exported from `astrid-types`. |
| `astrid-types` | Shared data types: IPC payloads, LLM schemas, kernel API. Minimal deps, WASM-compatible. Used by both kernel and SDK. |
| `astrid-capsule` | Capsule runtime: manifest parsing, WASM/MCP/static engines, dependency resolution via toposort, capsule registry, hot-reload watcher. |
| `astrid-mcp` | MCP client/server lifecycle. Wraps `rmcp` with binary hash verification, capability gating, and elicitation support. |
| `astrid-crypto` | Ed25519 key pairs (via `ed25519-dalek`), BLAKE3 content hashing, signatures. Keys are zeroized on drop. |
| `astrid-storage` | Two-tier persistence. Tier 1: raw KV via embedded SurrealKV. Tier 2: full SurrealDB query engine with SurrealQL. |
| `astrid-config` | Layered TOML configuration: workspace > user > system > env > defaults. Workspace configs can only tighten security, never loosen it. |
| `astrid-workspace` | Workspace boundary detection and process sandbox configuration. |
| `astrid-hooks` | Hook system for session lifecycle, tool calls, and approval flows. Handlers: command, HTTP, WASM. |
| `astrid-core` | Foundation types: `SessionId`, `Permission`, identity primitives, elicitation types, session tokens. |

### User-space crates

| Crate | Role |
|---|---|
| [`astrid-sdk`](https://github.com/unicity-astrid/sdk-rust) | Safe Rust SDK for capsule authors. Mirrors `std` layout. Includes `astrid-sys` (syscall table) and `astrid-sdk-macros` (`#[capsule]` proc macro). Standalone repo. |
| `astrid-openclaw` | TypeScript-to-WASM compiler for OpenClaw plugin compatibility. All-Rust pipeline: OXC + QuickJS/Wizer. |

### Binaries

| Binary | Crate | Role |
|---|---|---|
| `astrid` | `astrid-cli` | Terminal frontend. Connects to daemon over Unix socket. TUI rendering, capsule management, `start`/`status`/`stop` lifecycle commands. |
| `astrid-daemon` | `astrid-daemon` | Background kernel process. Boots the kernel, loads capsules, serves IPC requests. Spawned by CLI or started directly. |
| `astrid-build` | `astrid-build` | Capsule compiler and packager. Handles Rust, OpenClaw (JS/TS), and legacy MCP projects. |

### Infrastructure crates

| Crate | Role |
|---|---|
| `astrid-telemetry` | Structured logging with `tracing`. JSON and human-readable outputs. |
| `astrid-prelude` | Common re-exports for internal crates. |

## Storage

Two tiers, one API surface:

| Deployment | KV backend | DB backend |
|---|---|---|
| Dev / single-agent | SurrealKV (embedded) | SurrealDB (embedded, SurrealKV) |
| Production / multi-node | SurrealKV (embedded) | SurrealDB (over TiKV, Raft) |

Capsule KV stores are namespace-scoped per capsule. The kernel, audit log, capability store, and identity system use the DB tier. Scaling from embedded to distributed is a config change.

## Current state

**v0.3.0.** The core runtime works end-to-end:

- Kernel boots, discovers and loads capsules, manages VFS overlay, listens on Unix socket
- SecurityInterceptor with all five layers, tested with policy blocks, budget exhaustion, token auth, session/workspace allowances, and the "Allow Always" token minting path
- WASM sandbox with 48 host functions, 64 MB memory ceiling, 10-second tool timeout
- Chain-linked audit log with ed25519 signatures and chain integrity verification
- MCP client (2025-11-25 spec) via `rmcp` with capability gating and binary hash verification
- IPC event bus with broadcast subscribers and capability-scoped publish/subscribe ACLs
- Capsule dependency resolution via topological sort with typed capability matching
- Capsule manifest supporting tools, commands, skills, interceptors, cron jobs, IPC topics, LLM providers, uplinks, and MCP servers
- OpenClaw TypeScript-to-WASM compiler (OXC + QuickJS/Wizer, Tier 1 plugins)
- CLI with TUI, streaming responses, session persistence, capsule management
- Layered configuration with workspace-level security tightening

**Not yet done:** Multi-node SurrealDB (TiKV/Raft). WASM Component Model migration (Extism to WIT bindings). Additional frontends beyond CLI. Capsule registry for distribution.

## Development

```bash
cargo build --workspace
cargo test --workspace -- --quiet
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --all -- --check
```

All crates enforce `#![deny(unsafe_code)]` except `astrid-sys` and `astrid-sdk` (WASM FFI requires it). Clippy runs at pedantic level. Integer arithmetic overflow is a lint error (`clippy::arithmetic_side_effects = "deny"`).

## Contributing

Contributions are welcome. Astrid uses a tiered contributor system to protect security-critical code while keeping the door open for new contributors. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full process, including issue-first workflow, tier descriptions, and security-critical crate restrictions.

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2025-2026 Joshua J. Bouw and Unicity Labs.
