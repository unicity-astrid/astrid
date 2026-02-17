# Astrid

**Your AI agent proposes. You approve. The runtime enforces it with math, not hope.**

![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)
![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue)
![MSRV](https://img.shields.io/badge/MSRV-1.93-blue)
![Rust](https://img.shields.io/badge/Rust-2024_edition-orange)

---

Astrid is a secure runtime for building AI agents that can't go rogue.

Most agent frameworks control what an AI can do with system prompts, text that tells the model "don't delete files" or "don't spend money." The problem: models can be tricked into ignoring those instructions (prompt injection), and there's no way to prove they followed them.

Astrid takes a different approach. When an agent wants to do something risky (delete a file, make an HTTP request, run a shell command), it has to get permission first. That permission is recorded as a signed token (the same kind of digital signature used in SSH keys) that the runtime checks before executing anything. The agent can't forge the token, can't replay an old one, and can't talk its way past the check. Every decision goes into an append-only log where each entry is chained to the previous one, so you can detect if anyone tampers with the history.

```bash
# Start an interactive session
astrid chat

# Or run the daemon for multi-frontend access
astridd --ephemeral
```

## What does that actually look like?

Here's the flow when an agent tries to delete a file:

1. Agent calls `delete_file("/home/user/important.txt")`
2. The runtime classifies this as a `FileDelete` action (risk level: High)
3. **Policy check**: Is this path blocked? (e.g., `/etc/**` is always off-limits)
4. **Token check**: Does a signed authorization token already cover this? If yes, proceed.
5. **Budget check**: Is the session within its spending limit?
6. **Human approval**: If no token exists, the user sees a prompt:

   ```
   Delete file: /home/user/important.txt
   [Allow Once] [Allow Session] [Allow Always] [Deny]
   ```

7. If the user picks "Allow Always," the runtime creates a signed token so they won't be asked again for this file. That token is scoped, time-limited (1 hour by default), and linked back to the audit entry that created it.
8. The action is logged with the authorization proof, the session context, and a hash linking it to the previous log entry.

The agent never decides what it's allowed to do. It proposes actions. You approve them. The runtime enforces your decision.

## Who is this for?

- **Coding assistants and dev tools**: where an unconstrained agent could `rm -rf /` or push to production
- **Multi-user bots** (Telegram, Discord): where multiple users share one runtime and need isolated sessions and budgets
- **Enterprise integrations**: agents that touch internal APIs and databases where every action needs a paper trail
- **Plugin ecosystems**: where third-party code runs alongside trusted operations and you need real sandboxing

## How the security layer works

Every tool call passes through a `SecurityInterceptor` that combines five checks. Both the admin policy AND the user's authorization must agree before anything executes:

```
Agent proposes action
       |
  [1. Policy]    Admin sets hard boundaries: blocked commands, denied paths/hosts.
       |         "sudo" is always blocked. "/etc/**" is always blocked.
       |         These can't be overridden.
       |
  [2. Token]     Does a signed authorization token cover this action?
       |         Tokens use ed25519 signatures (same algorithm as SSH keys).
       |         They're scoped to specific resources with glob patterns,
       |         have expiration times, and link back to the audit entry
       |         that created them.
       |
  [3. Budget]    Is the session within its spending limit?
       |         Per-action and per-session limits are enforced atomically.
       |
  [4. Approval]  If no token exists, ask the human.
       |           Options: Allow Once, Allow Session, Allow Always, Deny.
       |           "Allow Always" creates a signed token for future use.
       |           If the human is unavailable, the action is queued (not silently skipped).
       |
  [5. Audit]     Log the decision (allowed or denied) with the authorization proof.
                 Each entry contains the hash of the previous entry,
                 so tampering with history is detectable.
```

This is real code. Look at `crates/astrid-approval/src/interceptor.rs`. The `SecurityInterceptor::intercept()` method implements this exact flow. The tests in that file cover policy blocks, budget enforcement, token-based authorization, and the "Allow Always" flow that creates new tokens.

## Two sandboxes

Untrusted code and trusted agent actions are fundamentally different threats, so they get different sandboxes.

### WASM sandbox: for plugins (locked, no overrides)

Plugins run inside WebAssembly via Wasmtime. This isn't a policy you configure. It's a physical boundary enforced by the WASM runtime. A plugin *cannot* make a syscall. It has no file descriptors, no network sockets, no access to process memory outside its own linear memory. It's like running code inside a calculator that only has 12 buttons.

Those 12 buttons are host functions, the only way a plugin can interact with the outside world:

| Host function | What it does | Security gated? |
|---------------|-------------|:---:|
| `read-file` | Read a file inside the workspace | Yes |
| `write-file` | Write a file inside the workspace | Yes |
| `http-request` | Make an HTTP request | Yes |
| `fs-exists`, `fs-mkdir`, `fs-readdir`, `fs-stat`, `fs-unlink` | Filesystem operations | Yes |
| `kv-get`, `kv-set` | Plugin-scoped key-value storage | No (isolated per plugin) |
| `get-config` | Read plugin config values | No |
| `log` | Write to the host log | No |

Every "Yes" in that table means the host function goes through a `PluginSecurityGate` check *before* the operation happens. A plugin calling `write-file("../../etc/passwd", ...)` gets rejected twice: once by path confinement (all paths are canonicalized and must resolve inside the workspace root), and again by the security gate.

The hard limits:
- **64 MB memory** (configurable down, not up). The WASM linear memory ceiling.
- **30-second timeout** per call. If a plugin hangs, it gets killed.
- **BLAKE3 hash verification**: every `.wasm` binary is hashed on load and checked against the manifest. If someone swaps the file, it won't load. In production mode, plugins without a hash in their manifest are rejected entirely.

If you genuinely need to remove the guardrails (you're running trusted first-party plugins in a controlled environment, or you just like living dangerously), a `--yolo` flag is planned. It'll do exactly what it sounds like. You've been warned.

### Workspace boundary: for the agent (flexible, approval-gated)

The agent itself (not plugins) operates within your project directory. Inside the workspace, it can read and write files freely. But the moment it tries to do something outside that boundary (write to a system path, run a shell command, make a network request), it goes through the approval flow described above.

You configure how strict this is:

```toml
[workspace]
mode = "safe"          # "safe" = ask about everything
                       # "guided" = auto-approve reads, ask about writes
                       # "autonomous" = trust the agent within the workspace
escape_policy = "ask"  # what happens when the agent tries to leave the workspace
                       # "ask" = prompt the user
                       # "deny" = block silently
                       # "allow" = let it through (you probably don't want this)
```

The key difference: the WASM sandbox is a hard wall that nobody can turn off. The workspace boundary is a fence with a gate that you control.

## Plugin system

Astrid supports two kinds of plugins:

**WASM plugins** run in the sandbox described above. You write them in Rust (or any language that compiles to WASM), and they communicate through the `astrid:plugin@0.1.0` WIT interface. Three exports: `describe-tools` (tell the LLM what you can do), `execute-tool` (handle a tool call), `run-hook` (respond to lifecycle events). Each plugin gets its own isolated key-value store.

**MCP plugins** are external processes that speak the Model Context Protocol. Any MCP server can be a plugin. These get OS-level sandboxing (Landlock on Linux) and go through the same security interceptor.

### The OpenClaw bridge

If you have TypeScript/JavaScript plugins from the [OpenClaw](https://docs.openclaw.ai) ecosystem, Astrid can compile them to WASM automatically. The pipeline is pure Rust, no Node.js or npm required for compilation:

```
Plugin.ts --> OXC transpiler --> JS --> ABI shim --> QuickJS/Wizer --> plugin.wasm
```

The key trick: Wizer pre-initializes the QuickJS engine at compile time, so cold start is near-zero. Plugins that need npm dependencies fall back to running as a Node.js subprocess behind an MCP bridge instead.

```bash
# Compile an OpenClaw plugin to a sandboxed WASM binary
astrid plugin compile ./my-openclaw-plugin
```

## Features

### Security
- Signed authorization tokens (ed25519): scoped, time-limited, linked to audit trail
- Append-only audit log: each entry hashes the previous one (BLAKE3), detects tampering
- Input classification: everything entering the system is tagged as trusted (verified user), pre-authorized (token), or untrusted (tool results, external data)
- Admin policy layer: blocked commands, denied paths/hosts, argument size limits
- Budget tracking: per-session and per-action spending limits, enforced atomically
- Deferred approval: if you're away, risky actions queue instead of failing silently

### Runtime
- Streaming LLM support: Anthropic Claude, OpenAI-compatible providers (LM Studio, vLLM), and Zai
- MCP client (2025-11-25 spec): sampling, roots, elicitation, URL elicitation, and tasks via `rmcp` v0.15
- 8 built-in tools: `read_file`, `write_file`, `edit_file`, `glob`, `grep`, `bash`, `list_directory`, `task`. All run in-process.
- Sub-agent delegation: spawn child agents with restricted permissions and configurable concurrency
- Session persistence: sessions survive daemon restarts

### Frontends
- **CLI** (`astrid`): interactive REPL with TUI, syntax highlighting, clipboard support
- **Telegram** (`astrid-telegram`): bot with inline approval buttons and streaming responses
- **Daemon** (`astridd`): background server with JSON-RPC over WebSocket
- **Build your own**: implement the `Frontend` trait to add new interfaces

## Architecture

```
astrid (CLI) ----+
                 +--> astridd (daemon)
astrid-telegram--+        |
                          |-- astrid-runtime (orchestration)
                          |     |-- astrid-llm (provider abstraction)
                          |     |-- astrid-mcp (MCP client + server lifecycle)
                          |     |-- astrid-tools (built-in tools)
                          |     |-- astrid-approval (security interceptor)
                          |     |-- astrid-capabilities (signed tokens)
                          |     +-- astrid-workspace (boundaries)
                          |
                          |-- astrid-plugins (WASM + MCP plugins)
                          |     +-- openclaw-bridge (TS/JS -> WASM compiler)
                          |
                          |-- astrid-audit (chain-linked logging)
                          |-- astrid-crypto (ed25519 + BLAKE3)
                          |-- astrid-storage (SurrealKV + SurrealDB)
                          +-- astrid-config (layered TOML)
```

The `Frontend` trait is how you plug in new UIs. Every frontend (CLI, Telegram, or whatever you build) shares the same runtime, sessions, authorization tokens, budget tracking, and audit log.

## Quick start

### Prerequisites

- Rust 1.93+ (edition 2024)
- An Anthropic API key (or any OpenAI-compatible provider)

### Install from source

```bash
git clone https://github.com/unicity-astrid/astrid.git
cd astrid
cargo build --release
```

This produces two binaries:
- `target/release/astrid`: the CLI client
- `target/release/astridd`: the daemon server

### First run

```bash
# Initialize a workspace (creates .astrid/ directory)
astrid init

# Start chatting (creates ~/.astrid/config.toml on first run)
astrid chat
```

On first run, Astrid prompts for your API key and writes a config template to `~/.astrid/config.toml`.

### Configuration

```toml
# ~/.astrid/config.toml

[model]
provider = "claude"
model = "claude-sonnet-4-20250514"
# api_key = ""  # or set ANTHROPIC_API_KEY env var
max_tokens = 4096
temperature = 0.7

[budget]
session_max_usd = 5.0
per_action_max_usd = 0.50

[security.policy]
require_approval_for_delete = true
require_approval_for_network = true

[workspace]
mode = "safe"          # "safe", "guided", or "autonomous"
escape_policy = "ask"  # "ask", "deny", or "allow"
```

Config is layered: defaults < system (`/etc/astrid/config.toml`) < user (`~/.astrid/config.toml`) < workspace (`.astrid/config.toml`) < environment variables. Workspace configs can only tighten security, never loosen it.

### MCP servers

```toml
[servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@anthropics/mcp-server-filesystem", "/tmp"]
auto_start = true
```

### Running the daemon

```bash
# Ephemeral mode (shuts down when all clients disconnect)
astridd --ephemeral

# Or manage via the CLI
astrid daemon run --ephemeral
astrid daemon status
astrid daemon stop
```

## CLI commands

| Command | Description |
|---------|-------------|
| `astrid chat` | Start an interactive chat session |
| `astrid init` | Initialize a workspace |
| `astrid daemon run\|status\|stop` | Manage the background daemon |
| `astrid sessions list\|show\|delete\|cleanup` | Manage sessions |
| `astrid servers list\|running\|start\|stop\|tools` | Manage MCP servers |
| `astrid audit list\|show\|verify\|stats` | View and verify audit logs |
| `astrid config show\|validate\|paths` | View and validate configuration |
| `astrid keys show\|generate` | Manage signing keys |
| `astrid hooks list\|enable\|disable\|info\|stats\|test\|profiles` | Manage hooks |
| `astrid plugin list\|install\|remove\|compile\|info` | Manage plugins |
| `astrid doctor` | Run system health checks |

## Implementing a frontend

All frontends implement the `Frontend` trait from `astrid-core`:

```rust
use astrid_core::frontend::{Frontend, FrontendContext};
use async_trait::async_trait;

struct MyFrontend;

#[async_trait]
impl Frontend for MyFrontend {
    fn get_context(&self) -> FrontendContext { /* ... */ }

    // MCP elicitation: server asking the user for input
    async fn elicit(
        &self, request: ElicitationRequest,
    ) -> SecurityResult<ElicitationResponse> { /* ... */ }

    // URL-based flows (OAuth, payments). The LLM never sees sensitive data.
    async fn elicit_url(
        &self, request: UrlElicitationRequest,
    ) -> SecurityResult<UrlElicitationResponse> { /* ... */ }

    // Human-in-the-loop approval for risky actions
    async fn request_approval(
        &self, request: ApprovalRequest,
    ) -> SecurityResult<ApprovalDecision> { /* ... */ }

    fn show_status(&self, message: &str) { /* ... */ }
    fn show_error(&self, error: &str) { /* ... */ }

    async fn receive_input(&self) -> Option<UserInput> { /* ... */ }

    // ... identity resolution, verification, and cross-frontend linking
}
```

The trait handles: user prompts (text, secret, select, confirm), URL flows where the LLM shouldn't see sensitive data, tiered approval (once / session / workspace / always / deny), tool lifecycle events, and cross-frontend identity.

## Writing plugins

### WASM plugin

Define a `plugin.toml` manifest:

```toml
id = "my-plugin"
name = "My Plugin"
version = "0.1.0"

[entry_point]
type = "wasm"
path = "plugin.wasm"
```

Your plugin implements three WASM exports:

- `describe-tools`: returns tool definitions for the LLM
- `execute-tool`: handles tool invocations
- `run-hook`: responds to lifecycle events

Host functions available inside the sandbox: `log`, `http-request`, `read-file`, `write-file`, `kv-get`, `kv-set`, `get-config`, `fs-exists`, `fs-mkdir`, `fs-readdir`, `fs-stat`, `fs-unlink`. All go through security checks.

### MCP plugin

```toml
id = "my-mcp-plugin"
name = "My MCP Plugin"
version = "0.1.0"

[entry_point]
type = "mcp"
command = "node"
args = ["./server.js"]
```

MCP plugins run as child processes over stdio. The runtime manages their lifecycle, applies OS-level sandboxing, and routes tool calls through the same security interceptor.

## Project structure

```
crates/
  astrid-core/          Foundation types: Frontend trait, identity, input classification, errors
  astrid-crypto/        Ed25519 key pairs, BLAKE3 hashing, signature verification
  astrid-capabilities/  Signed authorization tokens with glob-based resource patterns
  astrid-approval/      Security interceptor, budget tracking, allowance system, deferred resolution
  astrid-audit/         Chain-linked audit log with SurrealKV persistence
  astrid-mcp/           MCP client, server lifecycle, binary verification
  astrid-llm/           LLM providers (Claude, OpenAI-compat, Zai)
  astrid-runtime/       Agent sessions, context management, agentic loop, sub-agents
  astrid-tools/         Built-in tools: read, write, edit, glob, grep, bash, list, task
  astrid-workspace/     Workspace boundaries and escape approval
  astrid-plugins/       Plugin trait, WASM loader, MCP plugins, npm registry, lockfile
  astrid-config/        Layered TOML configuration with validation
  astrid-gateway/       Daemon: JSON-RPC, health checks, agent management
  astrid-events/        Async event bus with broadcast subscribers
  astrid-hooks/         User-defined hooks: command, HTTP, WASM, agent handlers
  astrid-storage/       SurrealKV (raw KV) + SurrealDB (query engine)
  astrid-telemetry/     Logging with multiple formats and per-crate directives
  astrid-cli/           CLI binary (astrid) and daemon binary (astridd)
  astrid-telegram/      Telegram bot frontend
  openclaw-bridge/      TypeScript/JavaScript to WASM compilation pipeline
wit/
  astrid-plugin.wit     WIT interface for the WASM plugin ABI
packages/
  openclaw-mcp-bridge/  MCP bridge for Tier 2 plugins (TypeScript)
```

## Development

```bash
# Build
cargo build --workspace

# Test (runs on Ubuntu and macOS in CI)
cargo test --workspace -- --quiet

# Format check
cargo fmt --all -- --check

# Clippy (pedantic, no unsafe, no integer overflow)
cargo clippy --workspace --all-features -- -D warnings
```

All crates enforce `#![deny(unsafe_code)]`. There is no unsafe Rust anywhere in the codebase. Clippy runs at pedantic level, and integer arithmetic overflow is a compile error.

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2026 Joshua J. Bouw and Unicity Labs.
