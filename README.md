# Astrid

**Production-grade secure agent runtime in Rust.**

Astrid is a modular, security-first runtime for AI agents. A thin CLI client (`astrid`) connects to a background daemon (`astridd`) that hosts the agent runtime, manages sessions, and enforces security. Additional frontends (Discord, Web, etc.) can plug in via the `Frontend` trait — one runtime, single source of truth, with shared budget, capabilities, memory, and audit across all frontends.

## Key Features

- **Cryptography over prompts** — Authorization uses ed25519 signatures and capability tokens, not LLM instructions
- **MCP 2025-11-25 spec** — Full client implementation via `rmcp` v0.15: sampling, roots, elicitation, URL elicitation, tasks
- **Defense in depth** — Input classification, capability validation, MCP permissions, sandbox, approval gates, audit logging
- **Two sandbox model** — WASM (inescapable, for untrusted code) + operational workspace (escapable with approval, for trusted actions)
- **Human-in-the-loop** — MCP elicitation for user input, URL elicitation for OAuth/payments, approval with Allow Once/Session/Workspace/Always/Deny
- **Chain-linked audit** — Cryptographically signed, immutable audit trail for every action
- **Modular by design** — Core works standalone; crypto and unicity features are opt-in

## Architecture

The CLI (`astrid`) is a thin stateless client. All state and execution live in the gateway daemon (`astridd`), which hosts the `AgentRuntime` and manages sessions, MCP servers, security, and audit. The CLI auto-starts the daemon if it isn't running.

```
┌────────────────────────────────────────────────────────────────┐
│                     FRONTEND CLIENTS                           │
│    CLI (astrid)  │  Discord  │  Web  │  Telegram  │  ...       │
│                    └───────────┴───────┴────────────┘          │
│              Thin clients — stateless, replaceable             │
└──────────────────────────┬─────────────────────────────────────┘
                           │  WebSocket + JSON-RPC 2.0
                           │  (jsonrpsee, ws://127.0.0.1:{port})
┌──────────────────────────▼─────────────────────────────────────┐
│               GATEWAY DAEMON (astridd)                         │
│                                                                │
│  DaemonServer ── Session lifecycle, event streaming,           │
│                  approval/elicitation relay, MCP server        │
│                  health checks, auto-restart, cleanup          │
│                                                                │
│  Modes: ephemeral (auto-shutdown) or persistent                │
│  State: ~/.astrid/ (sessions, audit, capabilities, keys)       │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                    ASTRID CORE                           │  │
│  │                                                          │  │
│  │  AgentRuntime ── Agentic loop, context summarization     │  │
│  │  Security Layer ── Capability tokens (ed25519 signed)    │  │
│  │  MCP Client ───── rmcp (official Rust SDK)               │  │
│  │  Audit Log ────── Chain-linked, cryptographically signed │  │
│  │  Sandbox ──────── Landlock (Linux) / sandbox-exec (macOS)│  │
│  └──────────────────────────────────────────────────────────┘  │
│                           │                                    │
│  ┌────────────────────────▼─────────────────────────────────┐  │
│  │  Orchestrator (external MCP server)                      │  │
│  │  Handles server discovery and proxying                   │  │
│  └──────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────┘
```

The daemon exposes an `astrid.*` JSON-RPC API for session management, agent execution, approval/elicitation responses, MCP server control, budget monitoring, and audit queries. Frontends subscribe to a streaming event channel for LLM text chunks, tool call progress, and approval/elicitation requests.

### Frontend Trait

All frontends implement the `Frontend` trait to handle human-in-the-loop interactions:

```rust
#[async_trait]
pub trait Frontend: Send + Sync {
    /// MCP elicitation — server requests structured user input
    async fn elicit(&self, request: ElicitationRequest) -> SecurityResult<ElicitationResponse>;

    /// URL elicitation — OAuth flows, payments (LLM never sees sensitive data)
    async fn elicit_url(&self, request: UrlElicitationRequest) -> SecurityResult<()>;

    /// Approval for sensitive operations (Allow Once/Session/Workspace/Always/Deny)
    async fn request_approval(&self, request: ApprovalRequest) -> SecurityResult<ApprovalDecision>;

    /// Status and error display
    fn show_status(&self, message: &str);
    fn show_error(&self, error: &str);

    /// Receive user messages and commands
    async fn receive_input(&self) -> Option<UserInput>;
}
```

### MCP Client Features

All client-side features from the [November 2025 MCP specification](https://modelcontextprotocol.io/specification/2025-11-25):

| Direction | Feature | Description |
|-----------|---------|-------------|
| Client → Server | **Sampling** | Server-initiated LLM calls |
| Client → Server | **Roots** | Filesystem/URI boundary queries |
| Client → Server | **Elicitation** | Structured user input requests |
| Client → Server | **URL Elicitation** | OAuth flows, credential collection |
| Server → Client | **Resources** | Context and data |
| Server → Client | **Prompts** | Templated messages |
| Server → Client | **Tools** | Functions to execute (security-checked) |
| Server → Client | **Tasks** | Long-running operations |

### Execution Model

Astrid distinguishes between trusted and untrusted code execution:

- **Native execution** (trusted, admin-configured servers): Binary hash verified before launch, OS-sandboxed (Landlock/sandbox-exec) as defense-in-depth, full native performance.
- **WASM execution** (untrusted, agent-fetched code): Must be WASM — runs in Wasmtime with WASI capabilities explicitly granted via elicitation. Memory-safe, no raw syscalls, deterministic, cross-platform.

WASM cannot exceed granted capabilities — enforced by the Wasmtime runtime, not by hoping a sandbox holds.

## Security Model

Every action passes through a multi-step security check using **intersection semantics** — both policy AND capability must allow an action:

```
Tool Call Request
      │
      ▼
┌───────────────────┐
│ 1. Policy Check   │  Hard boundaries (admin controls)
└─────────┬─────────┘
          ▼
┌───────────────────┐
│ 2. Capability     │  Does user/agent have a grant?
│    Check          │  If missing → check if approval needed
└─────────┬─────────┘
          ▼
┌───────────────────┐
│ 3. Budget Check   │  Is there remaining budget?
└─────────┬─────────┘
          ▼
┌───────────────────┐
│ 4. Risk Assessment│  High-risk → elicit user approval
└─────────┬─────────┘
          ▼
┌───────────────────┐
│ 5. Execute + Audit│  Run tool, log to chain-linked audit
└───────────────────┘
```

### Input Classification

Every message is wrapped in a `TaggedMessage` with full attribution — message ID, user ID, frontend origin, context, content, and optional cryptographic signature. This determines trust level:

- **Signed user input** — Verified user with platform identity (UUID), can trigger actions
- **Capability-authorized** — Pre-authorized via ed25519-signed token, scoped execution
- **Untrusted** — Tool results, external data — never executed directly

### Approval Flow

When an action requires user consent:

1. `AgentRuntime` in the daemon calls `SecureMcpClient::check_authorization`
2. Security interceptor runs the 5-step check
3. If approval needed, daemon pushes an `ApprovalNeeded` event to the subscribed frontend client
4. The frontend (e.g. CLI) prompts the user and sends the decision back via `astrid.approvalResponse` RPC
5. User sees risk level and chooses: **Allow Once**, **Allow Session**, **Allow Workspace**, **Allow Always**, or **Deny**
6. "Allow Always" creates a persistent `CapabilityToken` (ed25519 signed, audit-linked, scoped TTL)
7. If user is unavailable, request is queued in `DeferredResolutionStore` for later resolution

## Getting Started

### Prerequisites

- Rust 1.93+ (edition 2024)
- An Anthropic API key (for the LLM provider)

### Build

```bash
cargo build --workspace
```

### Run the CLI

The workspace produces two binaries: `astrid` (CLI client) and `astridd` (gateway daemon). The CLI auto-starts the daemon in ephemeral mode if it isn't already running.

```bash
cargo run -p astrid-cli --bin astrid -- chat
```

### Run Tests

```bash
cargo test --workspace
```

#### WASM Integration Tests

The WASM plugin integration tests (`astrid-plugins/tests/wasm_e2e.rs`) auto-skip if the test fixture isn't compiled. To run them:

```bash
# One-time: install the WASM target
rustup target add wasm32-unknown-unknown

# Build the test fixture
./scripts/compile-test-plugin.sh

# Run the WASM e2e tests
cargo test -p astrid-plugins --test wasm_e2e
```

### Lint

```bash
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

## Telegram Integration

Astrid includes a Telegram bot frontend that connects users to the agent runtime via Telegram. It supports two modes:

- **Embedded mode** (default) — The daemon spawns the bot automatically when `bot_token` is configured. No separate process needed.
- **Standalone mode** — Run the bot as a separate binary (`astrid-telegram`), useful for deploying the bot on a different machine from the daemon.

### Setup

1. Create a bot with [@BotFather](https://t.me/BotFather) on Telegram and copy the API token.

2. Add the token to your config (`~/.astrid/config.toml`):

```toml
[telegram]
bot_token = "123456:ABC-DEF..."
```

Or use an environment variable:

```bash
export TELEGRAM_BOT_TOKEN="123456:ABC-DEF..."
```

3. Start the daemon:

```bash
cargo run -p astrid-cli --bin astridd
```

The bot starts automatically and begins polling Telegram for messages.

### Configuration

All settings go in the `[telegram]` section of `config.toml`:

```toml
[telegram]
# Bot API token from @BotFather (required).
# Prefer TELEGRAM_BOT_TOKEN env var over storing in config.
bot_token = "123456:ABC-DEF..."

# Restrict access to specific Telegram user IDs.
# Empty list = allow all users.
allowed_user_ids = [123456789, 987654321]

# Workspace path for sessions created by the bot.
# workspace_path = "/path/to/project"

# Embedded mode (default: true).
# Set to false to run the bot as a separate process.
embedded = true
```

### Standalone Mode

To run the bot separately from the daemon:

1. Set `embedded = false` in config (or omit `bot_token` from daemon config entirely).
2. Start the daemon: `cargo run -p astrid-cli --bin astridd`
3. Start the bot: `cargo run -p astrid-telegram`

The standalone bot auto-discovers the daemon via `~/.astrid/daemon.port`, or you can set `daemon_url` explicitly:

```toml
[telegram]
daemon_url = "ws://127.0.0.1:3100"
```

### Bot Commands

| Command | Description |
|---------|-------------|
| `/start` | Welcome message |
| `/help` | Show available commands |
| `/reset` | End current session and start fresh |
| `/status` | Show daemon status and budget |
| `/cancel` | Cancel the current agent turn |

The bot supports approval and elicitation flows via inline keyboards — when the agent needs permission or user input, Telegram buttons appear inline in the chat.

## Project Structure

```
crates/
├── astrid-core            # Foundation types, traits, errors
├── astrid-crypto           # ed25519 signing, blake3 hashing
├── astrid-capabilities     # Capability tokens, validation, storage
├── astrid-audit            # Immutable chain-linked audit logging
├── astrid-mcp              # MCP client wrapper with security integration
├── astrid-approval         # Security interceptor, approval manager, budget tracking
├── astrid-storage          # Two-tier persistence (KvStore + SurrealDB)
├── astrid-config           # Unified TOML config with layered loading
├── astrid-events           # Event bus (46 event variants across 12 categories)
├── astrid-hooks            # Hook system (shell/WASM handlers)
├── astrid-plugins          # WASM plugin runtime (Extism), host functions, lifecycle
├── openclaw-bridge           # OpenClaw plugin format → Astrid adapter (JS/TS shim generation)
├── astrid-llm              # LLM provider trait, Claude integration, streaming
├── astrid-workspace        # Operational workspace boundaries
├── astrid-tools            # Built-in coding tools (read, write, edit, bash)
├── astrid-runtime          # AgentRuntime — orchestrates LLM + MCP + audit
├── astrid-gateway          # Gateway daemon — hosts runtime, manages sessions + MCP servers
├── astrid-cli              # Thin CLI client — connects to daemon via JSON-RPC
├── astrid-cli-mockup       # Ratatui-based TUI prototype
├── astrid-telegram         # Telegram bot frontend (embedded or standalone)
├── astrid-telemetry        # Tracing and metrics
├── astrid-test             # Test utilities
└── astrid-prelude          # Common re-exports
```

### Crate Dependency Graph

```
                    astrid-core
                         │
          ┌──────────────┼──────────────┐
          ▼              ▼              ▼
    astrid-crypto  astrid-audit  astrid-workspace
          │              │
          └──────┬───────┘
                 ▼
         astrid-capabilities
                 │
                 ▼
           astrid-mcp
                 │
                 ▼
         astrid-approval
                 │
                 ▼
         astrid-runtime
                 │
                 ▼
         astrid-gateway (daemon — hosts runtime)
                 │
    ┌────────────┼──────────────────┐
    ▼            ▼                  ▼
astrid-cli  astrid-telegram  (other frontends)
  (thin clients via JSON-RPC)
```

## Configuration

Astrid uses a layered TOML configuration system with 18 config sections (model, runtime, security, budget, rate limits, servers, audit, keys, workspace, git, hooks, logging, gateway, timeouts, sessions, subagents, telegram, retry):

```
embedded defaults → system config → user config → workspace config
```

Workspace configs can only **tighten** restrictions, never loosen them. Environment variables (`ASTRID_*`, `ANTHROPIC_*`) are applied after the merge as fallbacks, not overrides.

```bash
# Show resolved configuration
cargo run -p astrid-cli -- config show

# Validate configuration
cargo run -p astrid-cli -- config validate

# Show config file search paths
cargo run -p astrid-cli -- config paths
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `default` | MCP client, elicitation, capability tokens, audit |
| `crypto` | Registry signatures, state attestation, verifiable WASM |
| `unicity` | ZK aggregator, on-chain state, agent economy |

```
┌─────────────────────────────────────────────────────────────────┐
│                 ASTRID CORE (always available)                  │
│  MCP (rmcp) │ Elicitation │ Capability Tokens │ Audit │ Crypto  │
└──────────────────────────────┬──────────────────────────────────┘
                               │
            ┌──────────────────┴──────────────────┐
            ▼                                     ▼
┌───────────────────────┐             ┌───────────────────────┐
│  feature = "crypto"   │             │  feature = "unicity"  │
│                       │             │                       │
│ Registry signatures   │             │ ZK aggregator         │
│ State attestation     │             │ On-chain commits      │
│ Verifiable execution  │             │ Programmable escrow   │
│ Payment hooks         │             │ Agent economy         │
└───────────────────────┘             └───────────────────────┘
```

## Tech Stack

| Component | Technology |
|-----------|------------|
| Language | Rust (`#![deny(unsafe_code)]`) |
| MCP | `rmcp` v0.15 (2025-11-25 spec) |
| Crypto | ed25519-dalek, blake3 |
| Storage | SurrealDB (SurrealKV backend) |
| IPC | jsonrpsee (JSON-RPC 2.0 + WebSocket) |
| Async | Tokio |
| CLI | clap, dialoguer, syntect |
| WASM Plugins | Extism (Wasmtime + WASI underneath) |
| Sandbox | Landlock (Linux), sandbox-exec (macOS) |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
