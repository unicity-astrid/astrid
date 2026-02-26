# Astrid

**Your AI agent proposes. You approve. The runtime enforces it with math, not hope.**

[![CI](https://github.com/unicity-astrid/astrid/actions/workflows/ci.yml/badge.svg)](https://github.com/unicity-astrid/astrid/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV](https://img.shields.io/badge/MSRV-1.93-blue)](https://www.rust-lang.org)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-orange)](https://www.rust-lang.org)

---

Astrid is a secure runtime for building AI agents that cannot go rogue.

Most agent frameworks control what an AI can do with system prompts, text that tells the model "do not delete files" or "do not spend money." The problem: models can be tricked into ignoring those instructions (prompt injection), and there is no way to prove they followed them.

Astrid takes a different approach. When an agent wants to do something risky (delete a file, make an HTTP request, run a shell command), it has to get permission first. That permission is recorded as a signed token (the same kind of digital signature used in SSH keys) that the runtime checks before executing anything. The agent cannot forge the token, cannot replay an old one, and cannot talk its way past the check. Every decision goes into an append-only log where each entry is chained to the previous one, so you can detect if anyone tampers with the history.

```bash
# Start an interactive session
astrid chat

# Or run the daemon for multi-frontend access
astridd --ephemeral
```

## What does that actually look like?

Here is the flow when an agent tries to delete a file:

1. Agent calls `delete_file("/home/user/important.txt")`
2. The runtime classifies this as a `FileDelete` action (risk level: High)
3. **Policy check**: Is this path blocked? (e.g., `/etc/**` is always off-limits)
4. **Token check**: Does a signed authorization token already cover this? If yes, proceed.
5. **Budget check**: Is the session within its spending limit?
6. **Human approval**: If no token exists, the user sees a prompt:

   ```text
   Delete file: /home/user/important.txt
   [Allow Once] [Allow Session] [Allow Always] [Deny]
   ```

7. If the user picks "Allow Always," the runtime creates a signed token so they will not be asked again for this file. That token is scoped, time-limited (1 hour by default), and linked back to the audit entry that created it.
8. The action is logged with the authorization proof, the session context, and a hash linking it to the previous log entry.

The agent never decides what it is allowed to do. It proposes actions. You approve them. The runtime enforces your decision.

## Who is this for?

- **Coding assistants and dev tools**: where an unconstrained agent could `rm -rf /` or push to production
- **Multi-user bots** (Telegram, Discord): where multiple users share one runtime and need isolated sessions and budgets
- **Enterprise integrations**: agents that touch internal APIs and databases where every action needs a paper trail
- **Extension ecosystems**: where third-party code runs alongside trusted operations and you need real sandboxing

## How the security layer works

Every tool call passes through a `SecurityInterceptor` that combines five checks. Both the admin policy AND the user's authorization must agree before anything executes:

```text
Agent proposes action
       |
  [1. Policy]    Admin sets hard boundaries: blocked commands, denied paths/hosts.
       |         "sudo" is always blocked. "/etc/**" is always blocked.
       |         These cannot be overridden.
       |
  [2. Token]     Does a signed authorization token cover this action?
       |         Tokens use ed25519 signatures (same algorithm as SSH keys).
       |         They are scoped to specific resources with glob patterns,
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

### WASM sandbox: for extensions (locked, no overrides)

Extensions run inside WebAssembly via Wasmtime. This is not a policy you configure. It is a physical boundary enforced by the WASM runtime. A WebAssembly component *cannot* make a syscall. It has no file descriptors, no network sockets, no access to process memory outside its own linear memory. It is like running code inside a calculator that only has 12 buttons.

Those 12 buttons are host functions, the only way sandboxed code can interact with the outside world:

| Host function | What it does | Security gated? |
|---------------|-------------|:---:|
| `read-file` | Read a file inside the workspace | Yes |
| `write-file` | Write a file inside the workspace | Yes |
| `http-request` | Make an HTTP request | Yes |
| `fs-exists`, `fs-mkdir`, `fs-readdir`, `fs-stat`, `fs-unlink` | Filesystem operations | Yes |
| `kv-get`, `kv-set` | Scoped key-value storage | No (isolated per component) |
| `get-config` | Read extension config values | No |
| `log` | Write to the host log | No |

Every "Yes" in that table means the host function goes through a security gate check *before* the operation happens. Code calling `write-file("../../etc/passwd", ...)` gets rejected twice: once by path confinement (all paths are canonicalized and must resolve inside the workspace root), and again by the security gate.

The hard limits:
- **64 MB memory** (configurable down, not up). The WASM linear memory ceiling.
- **30-second timeout** per call. If execution hangs, it gets killed.
- **BLAKE3 hash verification**: every `.wasm` binary is hashed on load and checked against the manifest. If someone swaps the file, it will not load. In production mode, components without a hash in their manifest are rejected entirely.

### Workspace boundary: for the agent (flexible, approval-gated)

The agent itself operates within your project directory. Inside the workspace, it can read and write files freely. But the moment it tries to do something outside that boundary (write to a system path, run a shell command, make a network request), it goes through the approval flow described above.

You configure how strict this is:

```toml
[workspace]
mode = "safe"          # "safe" = ask about everything
                       # "guided" = auto-approve reads, ask about writes
                       # "autonomous" = trust the agent within the workspace
escape_policy = "ask"  # what happens when the agent tries to leave the workspace
                       # "ask" = prompt the user
                       # "deny" = block silently
```

The key difference: the WASM sandbox is a hard wall that nobody can turn off. The workspace boundary is a fence with a gate that you control.

## Capsules (The Extension System)

Astrid implements a Phase 4 Manifest-First architecture for all user-provided code and extensions, powered by the [`astrid-capsule`](crates/astrid-capsule/README.md) engine. Astrid supports three kinds of plugins:

Rather than forcing developers into a single execution paradigm, Astralis uses a **Composite Architecture**. A single `Capsule.toml` manifest can orchestrate multiple execution engines under one logical unit. The system parses this declarative manifest to wire up capabilities, tools, and environment variables before a single line of user code executes.

### The Composite Engines

A single capsule can wrap any combination of:

1. **WASM Engine**: Executes compiled WebAssembly within a strictly sandboxed Extism runtime.
2. **MCP Host Engine**: The "airlock override". Spawns native child processes (e.g., `node`, `python`) to communicate via legacy Model Context Protocol (MCP) JSON-RPC over standard I/O.
3. **Static Engine**: Injects static context, declarative skills, and predefined commands directly into OS memory without booting any virtual machines.

### The Manifest (`Capsule.toml`)

The manifest is the absolute source of truth. It defines the entrypoints, requests OS capabilities, and declares dependencies.

```toml
[package]
name = "github-agent"
version = "1.0.0"

# 1. The primary WASM logic
[component]
entrypoint = "bin/github_agent.wasm"

# 2. OS Capabilities requested
[capabilities]
net = ["api.github.com"]
fs_read = ["/home/user/.gitconfig"]

# 3. Environment Variables elicited during docking
[env.GITHUB_TOKEN]
type = "secret"
request = "Please provide a GitHub Personal Access Token"

# 4. The Airlock Override: Legacy MCP stdio server
[[mcp_server]]
id = "local-git"
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-git"]

# 5. Scheduled Background Tasks (Cron)
[[cron]]
name = "sync-issues"
schedule = "0 * * * *"
action = "astrid.github.sync"

# 6. eBPF-style Lifecycle Hooks
[[interceptor]]
event = "BeforeToolCall"

# 7. LLM Provider (Agent Brain)
[[llm_provider]]
id = "claude-3-5-sonnet"
description = "Anthropic Claude 3.5 Sonnet Provider"
capabilities = ["text", "vision", "tools"]
```

**Capsules** are manifest-first plugins built with the Astrid SDK (`astrid-sdk`). They run in WASM and communicate through seven typed Airlock syscall boundaries (VFS, IPC, Uplink, KV, HTTP, Cron, Sys) instead of raw host functions. Capsules are the recommended model for building frontends and integrations — the Discord bot is a capsule. Define a `Capsule.toml` manifest declaring capabilities, and the runtime enforces those boundaries.

### The OpenClaw bridge

If you have TypeScript/JavaScript extensions from the OpenClaw ecosystem, Astrid can compile them to WASM automatically. The pipeline is pure Rust, no Node.js required for compilation:

```text
TypeScript --> OXC transpiler --> JS --> ABI shim --> QuickJS/Wizer --> capsule.wasm
```

The key trick: Wizer pre-initializes the QuickJS engine at compile time, so cold start is near-zero. Extensions that need heavy npm dependencies fall back to running as a Node.js subprocess via the MCP Host Engine.

## Features

### Security
- Signed authorization tokens (ed25519): scoped, time-limited, linked to audit trail
- Append-only audit log: each entry hashes the previous one (BLAKE3), detects tampering
- Input classification: everything entering the system is tagged as trusted (verified user), pre-authorized (token), or untrusted (tool results, external data)
- Admin policy layer: blocked commands, denied paths/hosts, argument size limits
- Budget tracking: per-session and per-action spending limits, enforced atomically
- Deferred approval: if you are away, risky actions queue instead of failing silently

### Runtime
- Streaming LLM support: Anthropic Claude, OpenAI-compatible providers, and Zai
- MCP client (2025-11-25 spec): sampling, roots, elicitation, URL elicitation, and tasks via `rmcp`
- 9 built-in tools: `read_file`, `write_file`, `edit_file`, `glob`, `grep`, `bash`, `list_directory`, `task`, `spark`. All run in-process.
- Sub-agent delegation: spawn child agents with restricted permissions and configurable concurrency
- Session persistence: sessions survive daemon restarts

### Frontends
- **CLI** (`astrid`): interactive REPL with TUI, syntax highlighting, clipboard support
- **Telegram** (`astrid-telegram`): bot with inline approval buttons and streaming responses
- **Discord** (`astrid-discord`): WASM capsule with slash commands, approval buttons, and streamed responses. Runs sandboxed inside the capsule runtime with a host-side Gateway proxy for private network deployments.
- **Daemon** (`astridd`): background server with JSON-RPC over WebSocket
- **Build your own**: implement the `Frontend` trait to add new interfaces, or build a capsule for sandboxed frontends

## Architecture

```text
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
                          |-- astrid-capsule (composite execution engines & manifest routing)
                          |     |-- astrid-discord.wasm (Discord bot capsule)
                          |     +-- astrid-sdk (capsule development SDK)
                          |
                          |-- astrid-gateway
                          |     +-- discord_proxy (Discord Gateway WebSocket proxy)
                          |     +-- astrid-openclaw (TS/JS -> WASM compiler)
                          |
                          |-- astrid-frontend-common (shared frontend utilities)
                          |-- astrid-audit (chain-linked logging)
                          |-- astrid-crypto (ed25519 + BLAKE3)
                          |-- astrid-storage (SurrealKV + SurrealDB)
                          +-- astrid-config (layered TOML)
```

The `Frontend` trait is how you plug in new UIs. Every frontend shares the same runtime, sessions, authorization tokens, budget tracking, and audit log. Frontends can also be built as **capsules** (sandboxed WASM plugins) using the Astrid SDK, as demonstrated by the Discord frontend.

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

### Running the daemon

```bash
# Ephemeral mode (shuts down when all clients disconnect)
astridd --ephemeral

# Or manage via the CLI
astrid daemon run --ephemeral
astrid daemon status
astrid daemon stop
```

### Discord bot

The Discord frontend runs as a sandboxed WASM capsule with a host-side Gateway proxy. All connections are outbound — no public endpoints required.

```bash
# Build the Discord capsule
cd crates/astrid-discord
cargo build --target wasm32-unknown-unknown --release

# Install the capsule
astrid plugin install ./crates/astrid-discord
```

During installation, you'll be prompted for:
- **Bot token** from the [Discord Developer Portal](https://discord.com/developers/applications)
- **Application ID** from the same portal

Configure your bot in the Developer Portal:
- **OAuth2 scopes**: `bot`, `applications.commands`
- **Bot permissions**: Send Messages, Embed Links, Read Message History, Use Slash Commands
- **Privileged intents**: Enable "Message Content" only if you want the bot to respond to regular messages (not just slash commands)

The daemon starts a Gateway proxy automatically when the Discord capsule is loaded. Available slash commands: `/chat`, `/reset`, `/status`, `/cancel`, `/help`.

Optional environment variables:
```bash
DISCORD_ALLOWED_USERS="123456789,987654321"  # restrict to specific users
DISCORD_ALLOWED_GUILDS="111222333"           # restrict to specific servers
DISCORD_SESSION_SCOPE="channel"              # "channel" (default) or "user"
```

## CLI commands

| Command | Description |
|---------|-------------|
| `astrid chat` | Start an interactive chat session |
| `astrid init` | Initialize a workspace |
| `astrid daemon run\|status\|stop` | Manage the background daemon |
| `astrid sessions list\|show\|delete\|cleanup` | Manage sessions |
| `astrid capsule list\|install\|remove\|compile\|info` | Manage loaded capsules and manifests |
| `astrid servers list\|running\|start\|stop\|tools` | Manage legacy MCP servers |
| `astrid audit list\|show\|verify\|stats` | View and verify audit logs |
| `astrid config show\|validate\|paths` | View and validate configuration |
| `astrid keys show\|generate` | Manage signing keys |

## Project structure

- **`astrid-core`**: Foundation types, Frontend trait, input classification.
- **`astrid-capsule`**: [Manifest-first composite engine](crates/astrid-capsule/README.md) orchestrating WASM, MCP, and static context.
- **`astrid-approval`**: [Security interceptor](crates/astrid-approval/README.md), budget tracking, and allowance system.
- **`astrid-capabilities`**: [Signed authorization tokens](crates/astrid-capabilities/README.md) with glob-based resource patterns.
- **`astrid-audit`**: Chain-linked audit log with SurrealKV persistence.
- **`astrid-runtime`**: Agent sessions, context management, agentic loop.
- **`astrid-workspace`**: Workspace boundaries and escape approval.
- **`astrid-mcp`**: MCP client implementation.
- **`astrid-tools`**: Built-in core runtime tools.
- **`astrid-cli` / `astrid-gateway`**: CLI binary and background daemon (includes Discord Gateway proxy).
- **`astrid-discord`**: Discord bot frontend (WASM capsule, built with astrid-sdk).
- **`astrid-frontend-common`**: Shared frontend utilities (DaemonClient, SessionMap, PendingStore).
- **`astrid-telegram`**: Telegram bot frontend.

## Development

```bash
# Build
cargo build --workspace

# Test
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