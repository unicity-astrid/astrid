# User-Space Capsules & Airlock Management

This document details the plan for this phase of the Astrid OS transition. We are moving away from 
monolithic application architecture and fully embracing a **User-Space Microkernel** model.

Drawing inspiration from bare-metal OS design, the WebAssembly Component Model, and space logistics, this architecture aims to provide "infinite extensibility" while maintaining the strict sandbox, zero-ambient-authority security of the Astrid Microkernel.

---

## 1. The "Manifest-First" Paradigm

A **Capsule** in Astrid is *not* strictly defined as a WebAssembly binary. A Capsule is defined entirely by its **`Capsule.toml` manifest**.

This "Manifest-First" approach guarantees a **Zero-Friction** developer experience. It allows Astrid to seamlessly absorb the existing ecosystem of MCP servers, OpenClaw plugins, and CLI extensions (like Gemini extensions) without forcing developers to rewrite them in Rust or compile them to WASM.

### The Four Types of Capsules

#### A. The "No-Code" Static Capsule
If a user just wants to add context files and some declarative CLI slash-commands, they do not write any code. They just provide a `Capsule.toml` and their markdown files.
*How it runs:* The Astrid Kernel simply reads the TOML and registers the context and commands in its internal memory. No WASM VM or host process is booted.

#### B. The Legacy Host MCP Capsule (The "Airlock Override")
To support existing, standard MCP servers (written in Node.js, Python, or Go), a Capsule can declare a `host_process` capability.
*How it runs:* During `capsule install`, the user gets a clear warning that this capsule runs un-sandboxed code on their host machine. If approved, the Astrid Kernel spawns the process natively (e.g., `npx`) and seamlessly routes its `stdio` through the IPC Message Bus.

#### C. The AstridClaw Compiled Capsule (OpenClaw Bridge)
Astrid possesses a standalone developer tool and translation engine called **AstridClaw** (`astrid-openclaw`). **Crucially, AstridClaw is not part of the OS Kernel.** It is a toolchain feature integrated into the Package Manager (CLI).
*How it works:* When a developer wants to use an OpenClaw JS/TS plugin, AstridClaw compiles the JavaScript into a pure WebAssembly binary (using engines like `wizer` and `oxc`).
*Syscall Translation:* During compilation, AstridClaw intercepts OpenClaw-specific host function calls and translates (thunks) them into standard Astrid System API calls (`astrid::sys`). This ensures the resulting WASM binary runs natively on Astrid, keeping the core Kernel completely ignorant of OpenClaw's legacy ABI.
*The Result:* The output is a standard, Pure WASM Capsule (Type D) that can be installed perfectly into the OS sandbox.

#### D. The "Pure" WASM Capsule
For high-security agent logic, native background services, or untrusted third-party tools, developers use the `astrid-sys` Rust SDK to compile true WebAssembly components natively. These run with absolute zero-ambient-authority inside the Astrid sandbox.

---

## 2. The `Capsule.toml` Specification & Persistent State

The `Capsule.toml` manifest is the sole source of truth for the OS. It declares what the capsule provides, what capabilities it requires to function, and the configuration state that must **survive OS restarts**.

Below is a comprehensive example of how the manifest tracks persistent details and features.

```toml
[package]
name = "github-agent-tools"
version = "1.0.0"
description = "Provides GitHub MCP tools and CLI commands."
authors = ["Astrid Core Team"]
repository = "https://github.com/astralis-os/github-agent-tools"
homepage = "https://astralis.ai"
documentation = "https://docs.astralis.ai/github-agent-tools"
license = "MIT OR Apache-2.0"
readme = "README.md"
keywords = ["github", "mcp", "tools"]
categories = ["development-tools"]
astrid-version = ">=0.1.0"
publish = true
exclude = ["tests/", "scripts/"]

[package.metadata.custom_tool]
my_custom_key = "value"

[dependencies]
# Capsules can declare dependencies on other capsules for dynamic linking or IPC routing
git-core-tools = "1.2.0"
astrid-openclaw = ">=0.5.0"

[component]
# If it's a WASM or OpenClaw capsule, specify the entry point. Omit for static or legacy host capsules.
entrypoint = "github_tools.wasm"

[capabilities]
# Strictly defined capabilities the OS must prompt for and grant on installation.
net = ["api.github.com"]
kv = ["read", "write"]
fs_read = ["workspace://", "host://~/.ssh/"] # VFS boundary requests
# host_process = ["npx"] # Used ONLY for Legacy Host MCP overrides

[env]
# Environment variables the OS explicitly elicits from the user when docking (`capsule install`).
# You should NEVER ship hardcoded API keys in a Capsule.toml. Instead, declare them here,
# and the OS will securely prompt the user for them using the `request` string.
# These values are permanently tracked by the Astrid Kernel, survive system restarts, 
# and are securely injected into the capsule's environment upon boot.
github_token = { type = "secret", request = "Please enter your GitHub API token" }
default_repo = { type = "string", default = "astralis/astrid", request = "What is the default repository to target?" }

# ==========================================
# High-Level OS Integrations
# ==========================================

[[context_file]]
name = "github-rules"
file = "context.md"

[[command]]
name = "/gh"
description = "Interact with GitHub from the Astrid CLI"
# For static commands, you can link to a declarative file. For WASM capsules, 
# the OS routes this command as an IPC event to the running process.
file = "commands/gh.toml" 

[[mcp_server]]
id = "github-mcp"
description = "GitHub API tools for LLMs"
# type = "wasm-ipc" | "stdio" (defaults to wasm-ipc if [component] is provided)
# If type="stdio", requires `command` and `args` to be specified here, and `host_process` in capabilities.

[[skill]]
name = "github-reviewer"
description = "Expert context for reviewing GitHub PRs"
file = "SKILL.md" 

[[uplink]]
name = "telegram"
platform = "telegram"
profile = "human"

[[llm_provider]]
id = "claude-3-5-sonnet"
description = "Anthropic's flagship model"
capabilities = ["text", "vision", "tools"]

[[interceptor]]
# The OS builds a routing table from this to synchronously pass events through the WASM sandbox
event = "BeforeToolCall"

[[cron]]
# The OS scheduler will trigger the "generate_daily_summary" action every day at midnight
name = "daily_summary"
schedule = "0 0 * * *"
action = "generate_daily_summary"
```

### State Persistence
*   **The KV Airlock:** Beyond the `[env]` block, if a capsule needs to store dynamic data (e.g., caching a user's recent PRs) that survives restarts, it uses the `astrid::sys::kv_set()` and `kv_get()` airlocks. 
*   **The OS Guarantee:** The Astrid Kernel manages this state centrally (via SurrealDB or a local KV store). Even if the capsule crashes or is updated to a new version, its key-value state remains perfectly intact on the host.

---

## 3. The `astrid-sys` System SDK (For Pure WASM Capsules)

Writing a Pure WASM Capsule should not require deep knowledge of WASI or `extern "C"` FFI. We will build a Rust SDK (`astrid-sys`) that uses macros to abstract the low-level System API (Syscalls).

### Macro-Driven System Calls
Developers use an `#[astrid::capsule]` macro to define their entry points. The macro automatically generates the required WASI exports and performs serialization to interface with the Astrid Kernel.

```rust
use astrid_sys::prelude::*;

#[astrid::capsule]
struct MyService;

#[astrid::capsule_impl]
impl MyService {
    // Automatically registered to the IPC Message Bus on process start
    #[astrid::subscribe("user_input.my_command")]
    pub fn handle_command(&self, ctx: &mut Context, input: UserInput) -> Result<(), SysError> {
        // Safe wrappers around Kernel Syscalls (The Airlocks)
        let config = astrid::sys::kv_get("my_setting")?;
        
        let file_content = astrid::sys::fs_read("workspace://README.md")?;
        
        astrid::sys::ipc_publish("agent.response", AgentResponse::Text("Hello from User-Space!".into()))?;
        Ok(())
    }
}
```

### The System API / Airlocks (`astrid::sys`)
Provides safe, idiomatic Rust wrappers over the Kernel Syscalls defined in Phases 1-3. These act as the secure "airlocks" through which a sealed Capsule interacts with the host environment.

---

## 4. Security, Provisioning, and "Zero-Friction" Installation

Astrid aims for a **"just enable it"** zero-friction user experience while preserving the zero-ambient-authority security model. 

1.  **Manifest Declaration:** The `Capsule.toml` declares required capabilities.
2.  **Zero-Friction Docking (`capsule install <path>`):** 
    *   If a Capsule only requires static context, standard IPC routing, or isolated WASM execution without sensitive host access, it installs **silently and instantly**. No permission prompts.
    *   **The Airlock Prompt:** If a Capsule requests an explicit sandbox escape (`host_process` or physical disk access outside the workspace), the OS halts the installation and presents a clear security warning.
    *   **AstridClaw Auto-Wrapping (JIT Compilation):** For ecosystem integration (e.g., `capsule install openclaw:some-plugin`), the CLI fetches the JS artifact, runs AstridClaw to compile it to WASM and translate the syscalls, auto-generates a synthesized `Capsule.toml`, and proceeds with silent installation.
3.  **Implicit Handles:** Once docked, the Kernel stores the cryptographic handles (like `DirHandle`) securely. The WASM code calls `astrid::sys::fs_read`, and the Kernel mathematically enforces the boundary.

---

## 5. Implementation Steps

To ensure this architecture is implemented systematically, this is broken down into the following 
trackable milestones:

- [x] **Step 4.1: `astrid-sys` Foundation (The Airlocks)** 
  Build the `astrid-sys` crate. Define the core System API wrappers (`ipc`, `vfs`, `kv`) and implement the `#[astrid::capsule]` macro to handle WASM FFI boilerplate and data serialization for Pure WASM capsules.
  
- [x] **Step 4.2: Capsule Manifest & Kernel Loader (Manifest-First)** 
  Define the `Capsule.toml` schema to support Static, Host (Legacy MCP), and Pure WASM modes. Update the `astridd` Kernel to parse these manifests, provision capability handles, collect and persist settings across restarts, and route logic without assuming every capsule is a WASM binary.

- [x] **Step 4.3: Zero-Friction Installation Pipeline (`astrid-cli`)** 
  Build the `capsule install` command logic. Implement silent approvals for safe WASM executions and static features, setting collection prompts, and enforce the "Airlock Prompt" for dangerous `host_process` capabilities.

- [x] **Step 4.4: AstridClaw (OpenClaw Compilation & Syscall Translation)** 
  Refine the `astrid-openclaw` into the standalone AstridClaw transpiler tool. Implement the **Syscall Translation Layer** to ensure OpenClaw host functions (e.g., JS `fs.readFile`) are mapped directly to `astrid::sys::fs_read` WASM imports, keeping the core Kernel pure.

- [x] **Step 4.5: AstridClaw CLI Integration (JIT Auto-Wrapping)** 
  Integrate AstridClaw directly into the CLI Package Manager. Implement registry resolution so `capsule install openclaw:name` automatically fetches the JS plugin, compiles it to WASM via AstridClaw, synthesizes a `Capsule.toml`, and docks it.

- [x] **Step 4.6: Legacy Host MCP Support (The Escape Hatch)** 
  Implement the `host_process` capability within the Kernel to securely spawn and manage native host commands (like `npx` or `python`) and pipe their `stdio` to the IPC Message Bus for legacy servers that cannot be compiled by AstridClaw.

- [x] **Step 4.7: IPC Routing for MCP and Commands** 
  Standardize the IPC event schemas within `astrid-events` for routing MCP JSON-RPC requests and CLI slash-commands seamlessly between the Shell, the LLM Orchestrator, and the loaded Capsules.

---

## 6. Phase 5/6: The "Decoupled Brain" & IPC Routing

The ultimate realization of the Microkernel OS metaphor is removing the hardcoded LLM execution loop from the core OS (`astrid-runtime`). In Phase 6, LLM Providers will transition into **User-Space Capsules** communicating over the IPC Message Bus.

### 6.1 The System Configuration (Routing Table)
Instead of hardcoding "Claude" or "OpenAI", the `astrid-config` system (`~/.astrid/config.toml`) will act as an IPC routing table. It maps specific agent "roles" to specific Capsule IDs. 
*(Note: It is assumed that all providers are capsules, so the `capsule:` prefix is omitted for zero-friction DX).*

```toml
# In ~/.astrid/config.toml
[agents]
# The OS routes standard LLM generation requests to the 'anthropic' capsule
primary = "anthropic/claude-3-5-sonnet"
# The OS tries fallback capsules in order if the primary fails
fallback = ["openai/gpt-4o", "local-lm/llama-3"]

# Users can define custom sub-agent roles dynamically.
# The OS will route tasks requiring these roles to the specified capsules.
[[agents.role]]
name = "code_reviewer"
primary = "openai/gpt-4o"
fallback = ["anthropic/claude-3-haiku"]

[[agents.role]]
name = "creative_writer"
primary = "anthropic/claude-3-opus"
```

### 6.2 Agent IPC Execution
1. The user issues a prompt in a frontend Uplink (e.g., Telegram).
2. The core OS (`astridd`) reads the `config.toml` routing table to find the primary agent.
3. The OS issues a Point-to-Point binary IPC message to that specific capsule (e.g. `sys.llm.request.anthropic`).
4. The `anthropic` Capsule processes the request within its secure WASM sandbox, makes the HTTP call using the `astrid::sys::http` Airlock, and streams the tokens back to the OS via the Event Bus.
5. The core OS routes the tokens to the original Uplink.

This "Decoupled Brain" architecture allows developers to build local models, enterprise firewalls, and custom swarms simply by distributing new Capsules, without ever modifying the core OS.

---

## 7. The End-State: Repository Decoupling

Following the true Microkernel philosophy, the core Astrid OS repository should **only** contain the Kernel (`astridd`) and the System SDK (`astrid-sys`). 

Toolchain compilers (AstridClaw) and User-Space Applications (the CLI and Telegram services) are fundamentally not part of the OS.

**The Pragmatic Strategy:**
During active development of the System API and IPC ABI, moving these components to separate repositories would cause massive friction and "dependency hell" across commits. 
Therefore, they will remain in the primary Cargo Workspace (`crates/astrid-cli`, `crates/astrid-openclaw`) for now, but **must be treated as external dependencies**.
1. `astrid-cli` must **never** import `astrid-core` directly. It must only communicate via `astrid-sys` (or IPC if not yet WASM).
2. `astrid-openclaw` (AstridClaw) must have **zero** dependencies on the Kernel.

**The Future (Ejection):**
Once the `astrid::sys` ABI stabilizes, these crates will be ejected from the monorepo into their own dedicated repositories (e.g., `astrid-toolchain-claw`, `astrid-app-cli`), fully realizing the modular, decoupled OS vision.