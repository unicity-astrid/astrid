# The Astrid Microkernel: Host ABI & OS Abstractions

This document defines the transition architecture for the Astrid Agent Runtime, evolving it from a monolithic application into a WASM-based User-Space Microkernel. 

By building these specific OS-level abstractions, Astrid achieves absolute sandboxing, time-travel debugging, and infinite extensibility.

---

## 1. The Kernel / User-Space Divide

The fundamental principle of the Astrid OS model is a strict boundary between privileged infrastructure and unprivileged execution.

### Kernel Space (Rust Native)
The core daemon (`astridd`) is the only component that interacts with the host operating system. It handles:
*   **Heavy Infrastructure:** Databases (SurrealDB), cryptographic keystores, and network sockets live here. We do *not* ship database engines inside WASM plugins.
*   **Resource Management:** Memory allocation and process scheduling (via Tokio).
*   **Security Enforcement:** The `astrid-approval` interceptor and the VFS overlay engine evaluate every cross-boundary request.

### User Space (WASM Components)
Everything else is an unprivileged "process" running in a WebAssembly sandbox:
*   **Frontends:** The CLI, Telegram bots, and web dashboards.
*   **Agent Logic:** LLM Drivers, orchestration loops, and Agent Tools.
*   These components share no memory, have zero ambient authority, and must request access to resources via the Host ABI.

---

## 2. The Host ABI (The "Syscalls")

To support a fully sandboxed ecosystem, the Kernel exposes a minimal, strictly typed Application Binary Interface (ABI) to the WASM guests via Host Functions.

### A. IPC / The Message Bus
Plugins do not call each other directly; they publish to a location-transparent event bus.
*   `astrid_subscribe(topic_pattern: String) -> SubscriptionHandle`
*   `astrid_publish(topic: String, payload: Bytes) -> Result<(), Error>`
*   *Benefit:* The CLI and Telegram plugins simply emit `UserInput` events. The core router subscribes, processes them, and emits `AgentResponse` events. Total decoupling.

### B. Virtual File System (VFS) & Storage
Plugins never receive raw host paths (like `/Users/josh/dev`). They receive cryptographic handles to specific mounted environments.
*   `astrid_vfs_open_dir(handle: DirHandle, path: String) -> Result<DirHandle, Error>`
*   `astrid_vfs_read(handle: FileHandle, len: u32) -> Bytes`
*   **Databases:** Instead of bundling a DB engine, plugins use KV syscalls: `astrid_kv_get(key)`, `astrid_kv_set(key, value)`. The Kernel persists this securely in SurrealDB.

### C. Capability & Approval Requests
Plugins must request approval for side-effects, triggering budget and policy checks.
*   `astrid_cap_request(action: ActionDef) -> Result<ApprovalToken, Error>`

---

## 3. Storage & VFS Implementations

To provide "time-travel debugging" and prevent host corruption, the VFS implements the **Overlayfs / Copy-on-Write (CoW)** model.

### The Agent Overlay
1.  **LowerDir (Read-Only Host Mount):** The physical workspace (`~/dev/project`). The agent can read this, but the VFS mathematically rejects write operations.
2.  **UpperDir (Read-Write Session Diff):** An ephemeral directory managed by Astrid.
3.  **Operation:** Edits are copied to the `UpperDir`. If the agent hallucinates, the user discards the `UpperDir`. If approved, Astrid "commits" the `UpperDir` to the `LowerDir`.

### Remote Mounts
The VFS abstracts `read` and `write`, allowing the Kernel to inject virtual drivers:
*   `mount s3://bucket/data -> /mnt/s3`
*   A WASM plugin simply reads `/mnt/s3/file.csv`. The Kernel translates this into AWS API calls. The plugin requires no AWS SDK.

---

## 4. WebAssembly Architecture & Dependency Chains

Astrid leverages the cutting edge of WebAssembly to solve dependency management and supply chain security.

### A. Supply Chain Security (Cryptographic Hashing)
When a plugin is installed, its exact cryptographic hash (Blake3) is recorded in `plugins.lock`.
*   During boot, the Kernel verifies the `.wasm` file against this hash.
*   If a malicious actor alters a dependency on disk, the Kernel aggressively terminates the load process before a single instruction executes.

### B. The Component Model (WASM-to-WASM Linking)
In standard Rust, compiling dependencies (like a Markdown parser) links them into a single, shared-memory binary. A crash in the parser crashes the whole app.

Astrid embraces the **WebAssembly Component Model** (via Wasmtime/Extism and WIT interfaces):
1.  **Interface Definition (WIT):** Components define contracts (e.g., `parse(string) -> string`).
2.  **Zero-Shared Memory:** If the Telegram plugin needs to parse Markdown, it doesn't compile the parser into its own binary. It dynamically links to a separate `markdown-parser.wasm` component at runtime.
3.  **Language Agnostic:** The Telegram bot (Rust) calls `parse()`. The runtime jumps out of the bot's sandbox, into the parser's sandbox (which might be written in Go or Python), executes the code, and returns the result.
4.  **Blast Radius:** If the Go Markdown parser suffers a buffer overflow, it only crashes its own micro-sandbox. The Rust Telegram bot and the Astrid Kernel remain completely unharmed.
5.  **Composition:** Developers stitch these pre-compiled `.wasm` components together using tools like `wasm-tools compose`, allowing instant hot-swapping of dependencies without recompiling the main application.

---

## 5. Modern OS Paradigms Applied

1.  **Capability-Based Security (WASI):** No ambient authority. Without a `DirHandle`, a plugin cannot construct a path to a restricted directory.
2.  **Programmable eBPF-style Interceptors:** Future capability to allow users to inject safe, nanosecond-fast WASM filters into the `astrid-approval` pipeline (e.g., regex filtering PII before network dispatch).
3.  **Transparent Distributed IPC:** Because `astrid_ipc_publish` is location-agnostic, Astrid can route messages between a CLI on a laptop and a Worker Agent on a cloud VM with zero code changes to the plugins.
