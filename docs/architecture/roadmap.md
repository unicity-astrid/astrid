# Astrid OS Transition: Implementation Roadmap

This document breaks down the transition from the current "Fat Core" to the WASM Microkernel architecture into actionable, sequential phases. This serves as the master checklist for tracking progress.

---

## Phase 1: The IPC Message Bus (Foundation)
Before plugins can replace native frontends, they need a way to communicate asynchronously.
- [ ] **Define the IPC Schema (`astrid-events`):** Standardize the event payload schemas (e.g., `UserInput`, `AgentResponse`, `ApprovalRequired`) using a format like Protobuf, MessagePack, or JSON.
- [ ] **Expand the Message Broker (`astrid-events`):** Enhance the existing `EventBus` to handle topic-based routing and serialization/deserialization across the WASM boundary.
- [ ] **WASM IPC Host Functions (`astrid-plugins`):** 
  - Expose `astrid_publish(topic, payload)`.
  - Expose `astrid_subscribe(topic)` (yielding a stream or polling mechanism for the WASM guest).

## Phase 2: Virtual File System (VFS) & Capabilities
Abstract away the physical filesystem to support the Overlayfs and secure sandbox model.
- [ ] **Create `astrid-vfs` Crate:** A new crate dedicated entirely to virtualizing the filesystem, path resolution, and mounts, keeping `astrid-plugins` focused only on WASM execution.
- [ ] **Capability-Based Handles (`astrid-capabilities`):** Transition from string-based path approvals to granting cryptographic `DirHandle` tokens to plugins at spawn time.
- [ ] **VFS Trait (`astrid-vfs`):** Create the core VFS trait (`open`, `read`, `write`, `close`).
- [ ] **Host FS Driver (`astrid-vfs`):** Implement the VFS trait backed by `std::fs` (with strict workspace boundary enforcement).
- [ ] **WASM VFS Host Functions (`astrid-plugins`):** Map the WASM `astrid_vfs_*` host functions to the new `astrid-vfs` crate, dropping the old string-based `host/fs.rs` implementation.

## Phase 3: The Overlay & Storage Layer
Protect the host and provide state isolation.
- [ ] **Overlay VFS Driver (`astrid-vfs`):** Implement the Copy-on-Write overlay. Reads fall through to `LowerDir`, writes go to `UpperDir`.
- [ ] **Key-Value Host Functions (`astrid-plugins`):** Expose `astrid_kv_get` and `astrid_kv_set` host functions, wired directly into the native SurrealDB instance.

## Phase 4: Sandboxing the Telegram Frontend
Migrate the first native frontend into a WASM plugin as a proof-of-concept.
- [ ] **Create `astrid-sdk` Crate:** A new `wasm32-wasip1` compatible crate that provides safe, idiomatic Rust wrappers around the raw `astrid_*` host functions (e.g., `astrid::publish()`, `astrid::vfs::read()`). This is the SDK all WASM plugins (including first-party ones) will use.
- [ ] **WASM HTTP Host Functions:** Ensure `astrid_http_request` supports long-polling or the necessary primitives for the Telegram API.
- [ ] **The Telegram WASM Plugin:** Rewrite the Telegram bot to use `astrid-sdk`. It uses the HTTP host functions to talk to Telegram, and the IPC host functions to emit `UserInput` events.
- [ ] **Daemon Decoupling:** Remove the native `astrid-telegram` crate from the `astridd` daemon's dependency tree. The daemon simply loads `telegram.wasm` at startup.

## Phase 5: Sandboxing the CLI Frontend
Migrate the primary terminal interface into a WASM plugin.
- [ ] **WASI Terminal Support:** Ensure the WASM runtime (Extism/Wasmtime) is configured to pass `stdin`, `stdout`, and `stderr` through to the `astrid-cli.wasm` plugin.
- [ ] **State Decoupling:** Refactor the CLI codebase to remove direct dependencies on `astrid-storage`, `astrid-audit`, and `astrid-config`. It must query the daemon via the IPC bus for all state.
- [ ] **The CLI WASM Plugin:** Compile the refactored CLI to `wasm32-wasip1`.
- [ ] **The Native Shim:** Reduce the main `astrid` binary to a tiny shim that boots the WASM engine, loads `astrid-cli.wasm`, and connects the TTY.

## Phase 6: Component Model Adoption (Future)
- [ ] **WIT Interface Definitions:** Define the core Host Functions as strict WIT interfaces.
- [ ] **Dynamic Linking:** Migrate the plugin loader to support `wasm-tools compose` workflows, allowing plugins to depend on standalone WASM components rather than statically linking everything.
