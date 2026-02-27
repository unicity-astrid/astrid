# Astrid OS Transition: Implementation Roadmap

This document breaks down the transition from the legacy "Fat Core" to the pure User-Space Microkernel architecture into actionable, sequential phases. This serves as the master checklist for tracking progress.

---

## Phase 1: The IPC Message Bus (Foundation)
Before capsules can replace native frontends, they need a way to communicate asynchronously.
- [x] **Define the IPC Schema (`astrid-events`):** Standardize the event payload schemas (e.g., `UserInput`, `AgentResponse`, `ApprovalRequired`).
- [x] **Expand the Message Broker (`astrid-events`):** Enhance the existing `EventBus` to handle topic-based routing and serialization/deserialization across the WASM boundary.
  - *Security Task:* Enforce rate limits and message quotas to prevent IPC Denial-of-Service.
- [x] **WASM IPC Host Functions (`astrid-capsule`):** 
  - Expose `astrid_ipc_publish(topic, payload)`.
  - Expose `astrid_ipc_subscribe(topic)` and `astrid_ipc_poll()`.

## Phase 2: Virtual File System (VFS) & Capabilities
Abstract away the physical filesystem to support strict sandboxing.
- [x] **Create `astrid-vfs` Crate:** A new crate dedicated entirely to virtualizing the filesystem, path resolution, and mounts.
- [x] **Capability-Based Handles (`astrid-capabilities`):** Introduce cryptographic `DirHandle` and `FileHandle` tokens. 
- [x] **VFS Trait (`astrid-vfs`):** Create the core VFS trait (`open`, `read`, `write`, `close`).
- [x] **WASM VFS Host Functions (`astrid-capsule`):** Refactor WASM host functions to route through the VFS airlocks.

## Phase 3: The Overlay & Storage Layer
Protect the host and provide state isolation.
- [x] **Overlay VFS Driver (`astrid-vfs`):** Implement the Copy-on-Write overlay. Reads fall through to `LowerDir`, writes go to `UpperDir`.
- [x] **Key-Value Host Functions (`astrid-capsule`):** Expose `astrid_kv_get` and `astrid_kv_set` host functions, wired directly into the native storage layer.

## Phase 4: The User-Space Microkernel (Manifest-First)
Pivot from monolithic execution to the Extism-based Capsule architecture.
- [x] **Create `astrid-sdk` Crate:** Build the `wasm32-wasip1` compatible crate that provides safe, idiomatic Rust wrappers (Airlocks) and the `#[astrid::capsule]` macro.
- [x] **The `Capsule.toml` Specification:** Define the strict schema for package metadata, tool schemas, and capabilities.
- [x] **Multi-Component WASI Engine:** Upgrade `astrid-capsule` to support an array of `[[component]]`s natively, establishing the foundation for WASI shared libraries and perfect tool isolation.
- [x] **First-Party Core Capsules:** Scaffold `astrid-capsule-shell` and `astrid-capsule-fs` as pure WASM payloads utilizing the new SDK.

## Phase 5: Autonomous Git Worktrees
Provide unblocked, autonomous agent execution while maintaining perfect safety for local state.
- [x] **Pure Git Worktree Isolation (`astrid-workspace`):** Isolate agents into ephemeral git worktrees rather than complex CoW routing.
- [x] **The `.astridignore` Indexer (`astrid-vfs`):** Implement the blazing-fast `IgnoreBoundary` to mathematically blind agents to hidden secrets.
- [x] **RAII Garbage Collection:** Implement `ActiveWorktree` to automatically commit WIP changes and natively wipe the 5-10GB worktree physical directories upon drop.
- [x] **Host-Level Sandboxing:** Implement dynamic `SandboxCommand` generation (`bwrap` on Linux, `sandbox-exec` on macOS) to forcefully jail legacy host processes escaping the WASM sandbox.

## Phase 6: Ecosystem Registry & Universal Migrator
Establish a frictionless App Store experience using decentralized infrastructure.
- [x] **The Universal Migrator (`astrid-cli`):** Build `astrid build` to parse `Cargo.toml`, `mcp.json`, and `gemini-extension.json` automatically.
- [x] **Extism Schema Extraction:** Boot Rust `.wasm` files inside the CLI Builder to automatically extract and inject `schemars` JSON into the synthesized `Capsule.toml`.
- [x] **Apple-Style Fat Binaries:** Enable native MCP servers to ship multi-architecture slices within a single `.capsule` archive, relying on `astridd` to dynamically resolve the host target triple.
- [x] **Decentralized GitHub Discovery:** Implement `@org/repo` namespace resolution to hit GitHub Release APIs or execute JIT source compilation if no binaries are found.

## Phase 7: The "Decoupled Brain" & Global Routing
Wire the newly built isolated components together to replace the legacy native agent logic.
- [ ] **Global Event Bus Re-Wire:** Connect the isolated `ExtismHost` event buses into the daemon's global `EventBus` so capsules can actually communicate with each other.
- [ ] **Dynamic Configuration (`astrid-config`):** Implement the IPC routing table (`config.toml`) that maps specific agent roles to specific Capsule IDs.
- [ ] **The LLM Orchestrator Capsule:** Eject the hardcoded `astrid-runtime` agent execution loop into a dedicated `astrid-core-orchestrator` WASM Capsule.
- [ ] **The Tool Router Capsule:** Build the internal middleware capsule that receives JSON tool calls from the Orchestrator, verifies capabilities, and routes the execution to the correct `astrid-capsule-fs` or `astrid-capsule-shell` instance.

## Phase 8: Frontend Ejection
Remove the native shell and Telegram integrations from the daemon.
- [ ] **WASI Terminal Uplink:** Eject `astrid-cli` chat logic into an `astrid-core-cli-uplink` WASM capsule communicating purely via `astrid_uplink_send`.
- [ ] **Telegram Uplink:** Eject `astrid-telegram` into an `astrid-core-telegram-uplink` WASM capsule using the HTTP airlocks for long-polling.
- [ ] **The Final Purge:** Delete all business logic from `astridd`. The final OS daemon is only responsible for the Event Bus, VFS, and booting `.capsule` files.
