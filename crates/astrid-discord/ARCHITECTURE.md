# astrid-discord Architecture (Capsule-Based)

## 1. Overview

`astrid-discord` is a **WASM capsule** that implements a Discord bot frontend for the Astrid agent runtime. Unlike `astrid-telegram` (which is a standalone binary connecting via WebSocket JSON-RPC), the Discord integration runs **inside the sandboxed capsule runtime**, communicating with the host OS exclusively through the 7 Airlock syscall boundaries.

```
Discord REST API (HTTPS)
        ^
        | HTTP Airlock (sandboxed, SSRF-protected)
        |
  astrid-discord.wasm  (WASM capsule, Extism/Wasmtime sandbox)
        |
        | Uplink Airlock ──> InboundMessage ──> AgentRuntime
        | IPC Airlock    <── DaemonEvent bus
        | KV Airlock     <── Session state persistence
        | Sys Airlock    ──> Logging + config access
        | Cron Airlock   ──> Periodic polling (Gateway shim)
        |
  astridd daemon  (AgentRuntime, capsule host, security, audit)
```

### Why a Capsule?

- **Security**: The WASM sandbox prevents the Discord bot from accessing the filesystem, spawning processes, or making arbitrary network requests outside its declared capabilities.
- **Portability**: Capsules are distributable `.wasm` binaries with a `Capsule.toml` manifest. Users install the Discord capsule without compiling from source.
- **Consistency**: All frontends beyond the core CLI should follow the capsule model, enabling a plugin marketplace for frontends.
- **Isolation**: The capsule cannot interfere with other capsules or the host runtime. Each capsule gets its own scoped KV namespace, IPC topic prefix, and VFS sandbox.

## 2. WASM Compilation Constraints

### What Won't Work

The `serenity` and `poise` libraries **cannot compile to `wasm32-unknown-unknown`**. They depend on:
- `tokio` (full runtime with I/O drivers — unavailable in WASM)
- `rustls`/`native-tls` (TLS implementations — no OS sockets in WASM)
- `hyper`/`reqwest` (HTTP clients — no raw TCP in WASM)
- Discord Gateway WebSocket connections (persistent TCP — impossible from WASM)

### What We Use Instead

- **Discord REST API** via the **HTTP Airlock**: All Discord interactions (sending messages, editing messages, creating slash commands, responding to interactions) are done through raw HTTP calls to `https://discord.com/api/v10/...`.
- **Discord Gateway polling** via the **Cron Airlock**: Since WASM cannot hold a persistent WebSocket connection, the host runtime provides a Gateway shim (see Section 5).
- **Lightweight Discord types**: We define our own minimal Discord API types (`Message`, `Interaction`, `Embed`, `Component`, etc.) as `serde` structs, avoiding the full `serenity` model dependency.

### Crate Setup

```
crates/astrid-discord/
  Capsule.toml              # Capsule manifest
  .cargo/config.toml        # target = "wasm32-unknown-unknown"
  Cargo.toml                # cdylib crate
  ARCHITECTURE.md           # This file
  src/
    lib.rs                  # #[capsule] impl, tool/command/cron handlers
    discord_api.rs          # Discord REST API client (via HTTP Airlock)
    types.rs                # Minimal Discord API types (serde structs)
    format.rs               # Markdown formatting, 2000-char chunking
    session.rs              # Session state management (via KV Airlock)
    interaction.rs          # Interaction response builders
```

```toml
# Cargo.toml
[package]
name = "astrid-discord"
version = "0.1.0"
edition = "2024"
publish = false
description = "Discord bot frontend capsule for the Astrid agent runtime"

[lib]
crate-type = ["cdylib"]

[dependencies]
astrid-sdk = { path = "../astrid-sdk", features = ["derive"] }
extism-pdk = "1.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

```toml
# .cargo/config.toml
[build]
target = "wasm32-unknown-unknown"
```

## 3. Capsule Manifest (`Capsule.toml`)

```toml
[package]
name = "astrid-discord"
version = "0.1.0"
description = "Discord bot frontend for Astrid"
authors = ["Unicity Labs"]
astrid-version = ">=0.1.0"

[component]
entrypoint = "target/wasm32-unknown-unknown/release/astrid_discord.wasm"

# ── Capabilities ──────────────────────────────────────────────
[capabilities]
# Discord API + CDN domains
net = [
    "discord.com",
    "cdn.discordapp.com",
    "gateway.discord.gg",
]
# KV for session state (auto-scoped per capsule)
kv = ["sessions", "interactions", "state"]

# ── Environment Variables (elicited during install) ───────────
[env.DISCORD_BOT_TOKEN]
type = "secret"
request = "Enter your Discord bot token"
description = "Bot token from the Discord Developer Portal"

[env.DISCORD_APPLICATION_ID]
type = "string"
request = "Enter your Discord application ID"
description = "Application ID from the Discord Developer Portal"

# ── Uplink (registers as a Discord frontend connector) ────────
[[uplink]]
name = "discord"
platform = "discord"
profile = "chat"

# ── Tools (exposed to the LLM agent) ─────────────────────────
[[tool]]
name = "discord-send"
description = "Send a message to a Discord channel"
input_schema = { type = "object", properties = { channel_id = { type = "string" }, content = { type = "string" } }, required = ["channel_id", "content"] }

[[tool]]
name = "discord-edit"
description = "Edit an existing Discord message"
input_schema = { type = "object", properties = { channel_id = { type = "string" }, message_id = { type = "string" }, content = { type = "string" } }, required = ["channel_id", "message_id", "content"] }

# ── Interceptors (hooks into runtime lifecycle) ───────────────
[[interceptor]]
event = "run-hook"

# ── Cron (periodic Discord Gateway polling) ───────────────────
[[cron]]
name = "discord-gateway-poll"
schedule = "*/1 * * * * *"   # Every second
action = "poll-gateway"

[[cron]]
name = "discord-heartbeat"
schedule = "*/30 * * * * *"  # Every 30 seconds
action = "heartbeat"
```

## 4. Airlock Usage

### 4.1 HTTP Airlock — Discord REST API

All Discord API interactions go through the HTTP Airlock. The capsule constructs JSON request payloads and the host executes them via `reqwest` with SSRF protection.

**Request format** (JSON string → `http::request_bytes`):
```json
{
    "method": "POST",
    "url": "https://discord.com/api/v10/channels/{channel_id}/messages",
    "headers": {
        "Authorization": "Bot {token}",
        "Content-Type": "application/json"
    },
    "body": "{\"content\": \"Hello from Astrid!\"}"
}
```

**Response format** (JSON string returned from host):
```json
{
    "status": 200,
    "headers": { "content-type": "application/json", ... },
    "body": "{\"id\": \"123456\", \"content\": \"Hello from Astrid!\", ...}"
}
```

**Key API endpoints used**:

| Operation | Method | Endpoint |
|-----------|--------|----------|
| Send message | POST | `/channels/{id}/messages` |
| Edit message | PATCH | `/channels/{id}/messages/{id}` |
| Delete message | DELETE | `/channels/{id}/messages/{id}` |
| Create interaction response | POST | `/interactions/{id}/{token}/callback` |
| Edit interaction response | PATCH | `/webhooks/{app_id}/{token}/messages/@original` |
| Create followup | POST | `/webhooks/{app_id}/{token}` |
| Register slash commands | PUT | `/applications/{id}/commands` |
| Get gateway URL | GET | `/gateway/bot` |

**Wrapper module** (`discord_api.rs`):
```rust
/// Thin wrapper around Discord REST API via HTTP Airlock.
///
/// All methods return `Result<serde_json::Value, SysError>` with the
/// parsed Discord API response, or an error if the HTTP call or
/// JSON parsing fails.
struct DiscordApi {
    bot_token: String,
    application_id: String,
}

impl DiscordApi {
    fn send_message(&self, channel_id: &str, content: &str) -> Result<serde_json::Value, SysError>;
    fn edit_message(&self, channel_id: &str, message_id: &str, content: &str) -> Result<serde_json::Value, SysError>;
    fn interaction_respond(&self, interaction_id: &str, token: &str, data: &serde_json::Value) -> Result<serde_json::Value, SysError>;
    fn interaction_followup(&self, token: &str, data: &serde_json::Value) -> Result<serde_json::Value, SysError>;
    fn interaction_edit_original(&self, token: &str, data: &serde_json::Value) -> Result<serde_json::Value, SysError>;
    fn register_commands(&self, commands: &[SlashCommandDef]) -> Result<serde_json::Value, SysError>;
}
```

### 4.2 Uplink Airlock — Inbound Message Routing

The Uplink Airlock connects the capsule to the Astrid runtime's inbound message router. When a Discord user sends a message (or uses a slash command), the capsule:

1. **Registers** a connector during initialization:
   ```rust
   let connector_id = uplink::register("discord", "discord", "chat")?;
   ```
   This creates a `ConnectorDescriptor` with `FrontendType::Discord` and `ConnectorProfile::Chat`, passing the security gate check.

2. **Sends** user messages to the runtime:
   ```rust
   uplink::send_bytes(&connector_id, user_id.as_bytes(), message.as_bytes())?;
   ```
   This creates an `InboundMessage` and sends it via the `mpsc::Sender<InboundMessage>` to the runtime's inbound router, which creates/resumes a session and starts an agent turn.

The `InboundMessage` carries:
- `connector_id` — the registered Discord connector UUID
- `platform` — `FrontendType::Discord`
- `platform_user_id` — Discord user snowflake ID
- `content` — the user's message text
- `thread_id` — Discord channel ID (for session scoping)

### 4.3 IPC Airlock — Event Bus (Outbound Events)

The capsule subscribes to the event bus to receive `DaemonEvent`-like notifications from the runtime. Topics are auto-prefixed with the capsule's namespace.

```rust
// Subscribe to agent output events
let handle = ipc::subscribe("agent.output")?;

// In the cron poll handler, drain pending events
let events_bytes = ipc::poll_bytes(&handle)?;
```

Events the capsule listens for:
- **Text chunks** — streaming LLM output to edit into Discord messages
- **Tool call start/result** — render "Running tool: **bash**..." messages
- **Approval needed** — render approval buttons in Discord
- **Elicitation needed** — render elicitation UI (buttons/modals)
- **Turn complete** — finalize the response message
- **Error** — send error embed to Discord channel

### 4.4 KV Airlock — Session State Persistence

The KV Airlock provides scoped persistent storage (SurrealKV backend). The capsule uses it to track:

```rust
// Session mapping: Discord channel ID → Astrid session state
kv::set_json("session:channel:123456", &SessionState {
    session_id: "...",
    connector_id: "...",
    last_message_id: None,
    turn_in_progress: false,
})?;

// Pending approvals: request_id → approval metadata
kv::set_json("approval:abc-123", &PendingApproval {
    channel_id: "...",
    message_id: "...",
    interaction_token: "...",
    expires_at: "...",
})?;

// Pending elicitations: request_id → elicitation metadata
kv::set_json("elicitation:def-456", &PendingElicitation {
    channel_id: "...",
    schema_type: "select",
    interaction_token: "...",
    expires_at: "...",
})?;

// Gateway state: sequence number, session ID, heartbeat interval
kv::set_json("gateway:state", &GatewayState {
    session_id: Some("..."),
    sequence: Some(42),
    heartbeat_interval_ms: 41250,
    resume_gateway_url: Some("wss://gateway.discord.gg/?v=10&encoding=json"),
})?;
```

**Key namespaces**:
| Prefix | Purpose |
|--------|---------|
| `session:channel:{id}` | Channel → session mapping |
| `session:user:{id}` | User → session mapping (user-scoped mode) |
| `approval:{request_id}` | Pending approval request state |
| `elicitation:{request_id}` | Pending elicitation request state |
| `gateway:state` | Discord Gateway connection state |
| `config:commands_registered` | Whether slash commands have been registered |

### 4.5 Cron Airlock — Gateway Polling & Heartbeat

WASM capsules cannot hold persistent WebSocket connections. The Discord Gateway (which delivers real-time events like message creates, interaction creates, etc.) requires a **host-side shim** or a polling architecture.

#### Architecture: Host-Side Gateway Shim

The Cron Airlock triggers periodic capsule invocations. Combined with the IPC bus, this enables a polling pattern:

1. **`discord-gateway-poll` (every 1s)**: The cron handler is invoked, polls the IPC bus for any pending Gateway events that the host has buffered, and processes them (dispatching to message handlers, interaction handlers, etc.).

2. **`discord-heartbeat` (every 30s)**: Sends a heartbeat to Discord via the HTTP Airlock to maintain the Gateway session. Uses the sequence number stored in KV.

**Important**: The actual WebSocket connection to Discord Gateway must be managed by a **host-side component** (not the WASM capsule). This is because:
- WASM cannot open TCP/WebSocket connections
- The Gateway requires persistent bidirectional communication
- Heartbeats must be sent even when no capsule code is executing

**Two implementation strategies**:

**Strategy B — Gateway Proxy (Implemented, Default)**:
- A host-side `DiscordGatewayProxy` in `astrid-gateway/src/discord_proxy/` maintains a persistent `WebSocket` connection to Discord's Gateway
- Events (`MESSAGE_CREATE`, `INTERACTION_CREATE`) are relayed to the capsule via IPC when the cron handler fires
- The capsule processes events and sends responses via HTTP Airlock
- Supports all Discord events including regular messages and slash commands
- Handles heartbeat, reconnection with backoff+jitter, resume, and zombie detection

**Strategy A — HTTP Interactions Only (Alternative)**:
- Use Discord's [Interactions Endpoint URL](https://discord.com/developers/docs/interactions/overview#configuring-an-interactions-endpoint-url) feature
- Discord POSTs interaction payloads to a configured URL
- The `astridd` daemon exposes an HTTP endpoint that receives these POSTs
- The daemon routes them to the capsule via IPC
- **No Gateway `WebSocket` needed** — all events come via HTTP webhook
- Suitable for public-facing deployments that only need slash commands
- Limitation: Cannot receive regular (non-slash-command) messages

**Recommendation**: Use Strategy B (Gateway proxy) as the default. It provides full message support and runs entirely outbound (no inbound ports needed). Strategy A is available as an alternative for public-facing deployments where only slash commands and components are needed.

### 4.6 Sys Airlock — Logging & Configuration

```rust
// Logging (level-aware, routed to host's tracing infrastructure)
sys::log("info", "Discord capsule initialized")?;
sys::log("debug", format!("Processing interaction {interaction_id}"))?;
sys::log("warn", "Rate limit approaching for channel {channel_id}")?;

// Configuration (reads from Capsule.toml env + host config)
let bot_token = sys::get_config_string("DISCORD_BOT_TOKEN")?;
let app_id = sys::get_config_string("DISCORD_APPLICATION_ID")?;
```

## 5. Discord Interaction Flow

### 5.1 Slash Command: `/chat <message>`

```
User types /chat "Hello"
        │
        ▼
Discord sends HTTP POST to Interactions Endpoint
        │
        ▼
astridd receives webhook, routes to capsule via IPC
        │
        ▼
Capsule cron handler polls IPC, receives interaction payload
        │
        ▼
Capsule parses interaction, extracts user_id + message
        │
        ▼
Capsule sends immediate "Deferred" response via HTTP Airlock:
  POST /interactions/{id}/{token}/callback
  { "type": 5 }  (DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE)
        │
        ▼
Capsule sends InboundMessage via Uplink Airlock:
  uplink::send_bytes(&connector_id, user_id, message)
        │
        ▼
Runtime processes turn, emits events on IPC bus
        │
        ▼
Next cron tick: capsule polls IPC, receives text chunks
        │
        ▼
Capsule edits deferred response via HTTP Airlock:
  PATCH /webhooks/{app_id}/{token}/messages/@original
  { "content": "Agent response text..." }
        │
        ▼
On TurnComplete event: finalize message
```

### 5.2 Approval Flow

```
Runtime emits ApprovalNeeded event on IPC
        │
        ▼
Capsule receives event, builds button components:
  [Allow Once] [Allow Session] [Deny]
        │
        ▼
Capsule sends message with components via HTTP Airlock:
  POST /channels/{id}/messages
  { "embeds": [...], "components": [{ "type": 1, "components": [...buttons...] }] }
        │
        ▼
Capsule stores pending approval in KV:
  kv::set_json("approval:{request_id}", { channel_id, message_id, expires_at })
        │
        ▼
User clicks button → Discord sends interaction webhook
        │
        ▼
Capsule receives button interaction via IPC
        │
        ▼
Capsule parses custom_id: "apr:{request_id}:{option_index}"
        │
        ▼
Capsule looks up pending approval from KV
        │
        ▼
Capsule publishes approval decision on IPC:
  ipc::publish_json("approval.response", { request_id, decision })
        │
        ▼
Capsule acknowledges button interaction via HTTP:
  POST /interactions/{id}/{token}/callback
  { "type": 7 }  (UPDATE_MESSAGE — disables buttons)
```

### 5.3 Elicitation Flow

Elicitation follows a similar pattern to approval, with UI variations based on schema type:

- **Select**: Buttons (one per option + Cancel). Custom ID: `eli:{request_id}:{index}`
- **Confirm**: Two buttons (Yes/No). Custom ID: `eli:{request_id}:yes` / `eli:{request_id}:no`
- **Text/Secret**: Discord modal dialog via HTTP interaction response type 9 (`MODAL`). Modal custom ID: `eli_modal:{request_id}`

## 6. Discord API Types (Minimal)

Instead of depending on `serenity`'s model types, we define minimal serde structs for the Discord API objects we need:

```rust
// types.rs — Minimal Discord API types for WASM capsule

#[derive(Serialize, Deserialize)]
struct Interaction {
    id: String,
    #[serde(rename = "type")]
    interaction_type: u8,
    token: String,
    data: Option<InteractionData>,
    member: Option<GuildMember>,
    user: Option<User>,
    channel_id: Option<String>,
    guild_id: Option<String>,
    message: Option<Message>,
}

#[derive(Serialize, Deserialize)]
struct InteractionData {
    id: Option<String>,
    name: Option<String>,
    #[serde(rename = "type")]
    data_type: Option<u8>,
    options: Option<Vec<CommandOption>>,
    custom_id: Option<String>,
    component_type: Option<u8>,
    components: Option<Vec<ModalComponent>>,
}

#[derive(Serialize, Deserialize)]
struct Message {
    id: String,
    channel_id: String,
    content: String,
    author: User,
}

#[derive(Serialize, Deserialize)]
struct User {
    id: String,
    username: String,
    discriminator: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Embed {
    title: Option<String>,
    description: Option<String>,
    color: Option<u32>,
    fields: Option<Vec<EmbedField>>,
}

#[derive(Serialize, Deserialize)]
struct Component {
    #[serde(rename = "type")]
    component_type: u8,      // 1=ActionRow, 2=Button, 3=Select, 4=TextInput
    components: Option<Vec<Component>>,
    style: Option<u8>,        // Button: 1=Primary, 2=Secondary, 3=Success, 4=Danger
    label: Option<String>,
    custom_id: Option<String>,
    disabled: Option<bool>,
}
```

These types are intentionally minimal — we only model the fields we read/write. Unknown fields are silently dropped by serde's default behavior.

## 7. Message Formatting

### Discord Markdown

Discord natively speaks a markdown subset. LLM output is already markdown, so minimal conversion is needed:

```rust
// format.rs

/// Sanitize LLM markdown for Discord.
/// - Ensure code blocks are properly terminated
/// - Strip HTML tags (Discord doesn't render them)
/// - Escape accidental @mentions and #channel references
fn sanitize_for_discord(md: &str) -> String;

/// Split text into chunks fitting Discord's 2000-char limit.
/// Splits at paragraph boundaries (\n\n), then newlines (\n),
/// then hard-cuts. Handles code block continuation across chunks.
fn chunk_discord(text: &str, max_len: usize) -> Vec<String>;
```

**2000-character chunking** with code block awareness:
1. Target chunk size: 1900 chars (100 chars headroom for code fence continuations)
2. Track open code blocks: if a chunk ends inside a ` ``` ` fence, close it and reopen in the next chunk
3. Split priority: paragraph (`\n\n`) > newline (`\n`) > word boundary > hard cut

### Embed Colors

| Risk Level | Color | Hex |
|-----------|-------|-----|
| Low | Green | `0x2ECC71` |
| Medium | Yellow | `0xF1C40F` |
| High | Red | `0xE74C3C` |
| Critical | Dark Red | `0x992D22` |
| Error | Red | `0xE74C3C` |
| Info | Blue | `0x3498DB` |

## 8. Session Management

### Session Scoping

Two modes, stored in capsule config:

- **Channel** (default): `session:channel:{channel_id}` — each Discord channel gets its own agent session. Multiple users share context.
- **User**: `session:user:{user_id}` — each user gets a personal session regardless of channel.

### Session State (KV)

```rust
#[derive(Serialize, Deserialize)]
struct SessionState {
    /// Astrid runtime session ID (from Uplink send result)
    session_id: String,
    /// Registered connector UUID
    connector_id: String,
    /// Last Discord message ID (for editing streamed responses)
    last_message_id: Option<String>,
    /// Whether an agent turn is currently in progress
    turn_in_progress: bool,
    /// Interaction token for deferred responses (valid 15 min)
    interaction_token: Option<String>,
    /// When the interaction token expires
    token_expires_at: Option<String>,
}
```

### Turn Serialization

Only one agent turn at a time per session. The `turn_in_progress` flag in KV state prevents concurrent turns. If a user sends `/chat` while a turn is active, the capsule responds with an ephemeral "A turn is already in progress" message.

**Race condition note**: Since the capsule is single-threaded WASM (invoked serially by the cron scheduler), there are no true data races. However, stale KV reads are possible if the cron handler is invoked between a check and an update. The capsule must use read-modify-write patterns with version checks.

## 9. Capsule Implementation

### Entry Points

The `#[capsule]` macro generates WASM exports that route to annotated methods:

```rust
#[derive(Default)]
pub struct DiscordCapsule;

#[capsule]
impl DiscordCapsule {
    // ── Tools (callable by the LLM agent) ────────────────────

    #[astrid::tool("discord-send")]
    fn handle_send(&self, args: SendArgs) -> Result<ToolOutput, SysError> {
        // Send message to Discord channel via HTTP Airlock
        let api = DiscordApi::from_config()?;
        let result = api.send_message(&args.channel_id, &args.content)?;
        Ok(ToolOutput { content: serde_json::to_string(&result)?, is_error: false })
    }

    #[astrid::tool("discord-edit")]
    fn handle_edit(&self, args: EditArgs) -> Result<ToolOutput, SysError> {
        // Edit existing Discord message via HTTP Airlock
        let api = DiscordApi::from_config()?;
        let result = api.edit_message(&args.channel_id, &args.message_id, &args.content)?;
        Ok(ToolOutput { content: serde_json::to_string(&result)?, is_error: false })
    }

    // ── Cron (periodic background tasks) ─────────────────────

    #[astrid::cron("poll-gateway")]
    fn poll_gateway(&self, _args: EmptyArgs) -> Result<serde_json::Value, SysError> {
        // 1. Poll IPC for pending events (interactions, runtime events)
        // 2. Process each event (dispatch to handler)
        // 3. Send responses via HTTP Airlock
        self.process_pending_events()?;
        Ok(serde_json::json!({"action": "continue"}))
    }

    #[astrid::cron("heartbeat")]
    fn heartbeat(&self, _args: EmptyArgs) -> Result<serde_json::Value, SysError> {
        // Send heartbeat to Discord if using Gateway (Strategy B)
        // For Strategy A (HTTP interactions), this is a no-op health check
        self.check_health()?;
        Ok(serde_json::json!({"action": "continue"}))
    }

    // ── Interceptor (lifecycle hooks) ────────────────────────

    #[astrid::interceptor("run-hook")]
    fn run_hook(&self, _args: EmptyArgs) -> Result<serde_json::Value, SysError> {
        Ok(serde_json::json!({"action": "continue", "data": null}))
    }
}
```

### Initialization

On first cron tick (or explicit `init` command), the capsule:

1. Reads `DISCORD_BOT_TOKEN` and `DISCORD_APPLICATION_ID` from config via Sys Airlock
2. Registers the uplink connector via Uplink Airlock
3. Registers slash commands with Discord via HTTP Airlock (idempotent PUT)
4. Subscribes to IPC topics for runtime events
5. Stores connector ID and initialization state in KV

```rust
fn ensure_initialized(&self) -> Result<DiscordApi, SysError> {
    // Check if already initialized this session
    let state: Option<InitState> = kv::get_json("init:state").ok();
    if let Some(s) = state {
        return DiscordApi::new(s.bot_token, s.app_id);
    }

    // First time: read config, register, setup
    let bot_token = sys::get_config_string("DISCORD_BOT_TOKEN")?;
    let app_id = sys::get_config_string("DISCORD_APPLICATION_ID")?;

    let connector_id = uplink::register("discord", "discord", "chat")?;
    let connector_id_str = String::from_utf8_lossy(&connector_id).to_string();

    let api = DiscordApi::new(bot_token.clone(), app_id.clone())?;
    api.register_commands(&default_commands())?;

    // Subscribe to runtime events
    let event_handle = ipc::subscribe("agent.events")?;

    kv::set_json("init:state", &InitState {
        bot_token,
        app_id,
        connector_id: connector_id_str,
        event_handle: String::from_utf8_lossy(&event_handle).to_string(),
    })?;

    Ok(api)
}
```

## 10. Slash Commands

All commands are registered as Discord Application Commands (global slash commands):

### `/chat <message>`
- **Description**: Send a message to the AI agent
- **Behavior**:
  1. Parse interaction, extract user ID and message
  2. Send deferred response (interaction type 5)
  3. Check/create session in KV
  4. Check turn_in_progress — reject if busy (ephemeral)
  5. Send InboundMessage via Uplink Airlock
  6. Store interaction token in session KV for later edits

### `/reset`
- **Description**: Reset the current session
- **Behavior**: Clear session KV entry, respond with ephemeral confirmation

### `/status`
- **Description**: Show agent status
- **Behavior**: Read session state from KV, format as embed, respond ephemeral

### `/cancel`
- **Description**: Cancel the current agent turn
- **Behavior**: Publish cancel event on IPC, clear turn_in_progress in KV

### `/help`
- **Description**: Show help information
- **Behavior**: Respond with embed listing all commands

### Command Registration

```rust
fn default_commands() -> Vec<SlashCommandDef> {
    vec![
        SlashCommandDef {
            name: "chat".into(),
            description: "Send a message to the AI agent".into(),
            options: vec![CommandOptionDef {
                name: "message".into(),
                description: "Your message".into(),
                option_type: 3, // STRING
                required: true,
            }],
        },
        SlashCommandDef { name: "reset".into(), description: "Reset the current session".into(), options: vec![] },
        SlashCommandDef { name: "status".into(), description: "Show agent status".into(), options: vec![] },
        SlashCommandDef { name: "cancel".into(), description: "Cancel the current turn".into(), options: vec![] },
        SlashCommandDef { name: "help".into(), description: "Show help information".into(), options: vec![] },
    ]
}
```

Registered via bulk overwrite:
```
PUT /applications/{app_id}/commands
```

## 11. Component Design

### Approval Buttons

```
+──────────────────────────────────────────+
│ ⚠ Approval Required [Medium]            │
│                                          │
│ **bash**                                 │
│ Execute: rm -rf ./build/                 │
│                                          │
│ Resource: `./build/`                     │
+──────────────────────────────────────────+
[Allow Once] [Allow Session] [Deny]
```

- Embed color-coded by risk level
- Button `custom_id` format: `apr:{request_id}:{option_index}`
- Stored in KV with 5-minute TTL
- After expiry: edit message to disable buttons, mark as "Expired"

### Elicitation UI

| Schema | Discord UI | Custom ID Format |
|--------|-----------|-----------------|
| Select | Buttons per option + Cancel | `eli:{request_id}:{index}` |
| Confirm | Yes/No buttons | `eli:{request_id}:yes`/`no` |
| Text | Modal dialog (type 9) | `eli_modal:{request_id}` |
| Secret | Modal dialog (type 9) | `eli_modal:{request_id}` |

### Streamed Response Editing

During an agent turn, the capsule accumulates text chunks from IPC events and periodically edits the deferred interaction response:

```rust
fn process_text_chunk(&self, session: &mut SessionState, chunk: &str) -> Result<(), SysError> {
    // Accumulate text
    let mut buffer: String = kv::get_json("buffer:current").unwrap_or_default();
    buffer.push_str(chunk);
    kv::set_json("buffer:current", &buffer)?;

    // Throttle edits to avoid Discord rate limits (~5 edits/5s/channel)
    // Edit at most once per second
    let last_edit: u64 = kv::get_json("buffer:last_edit_ms").unwrap_or(0);
    let now_ms = /* current timestamp from interaction or host */;
    if now_ms.saturating_sub(last_edit) < 1000 {
        return Ok(());
    }

    // Edit the deferred response
    let chunks = chunk_discord(&buffer, 1900);
    let api = self.api()?;
    if let Some(token) = &session.interaction_token {
        api.interaction_edit_original(token, &serde_json::json!({
            "content": chunks.first().unwrap_or(&String::new())
        }))?;
    }
    kv::set_json("buffer:last_edit_ms", &now_ms)?;

    Ok(())
}
```

## 12. Security Considerations

### Bot Token Handling

- Token is stored as a `secret` env var in `Capsule.toml`, elicited during installation
- Accessed via `sys::get_config_string("DISCORD_BOT_TOKEN")` — never persisted in KV
- The WASM sandbox ensures the token cannot leak to other capsules or the filesystem

### Network Security

- HTTP Airlock enforces SSRF protection — only `discord.com`, `cdn.discordapp.com`, and `gateway.discord.gg` are allowed (declared in `capabilities.net`)
- All HTTP requests go through the host's `reqwest` client with 30-second timeout
- Response body size is capped by `MAX_GUEST_PAYLOAD_LEN`

### Authorization

The capsule implements user/guild allowlists stored in config:

```rust
fn is_authorized(&self, user_id: &str, guild_id: Option<&str>) -> Result<bool, SysError> {
    let allowed_users = sys::get_config_string("DISCORD_ALLOWED_USERS")?;
    let allowed_guilds = sys::get_config_string("DISCORD_ALLOWED_GUILDS")?;

    let user_ok = allowed_users.is_empty()
        || allowed_users.split(',').any(|id| id.trim() == user_id);
    let guild_ok = allowed_guilds.is_empty()
        || guild_id.is_some_and(|g| allowed_guilds.split(',').any(|id| id.trim() == g));

    Ok(user_ok && guild_ok)
}
```

### Input Validation

- Interaction payloads are validated before processing (type check, required fields)
- `custom_id` parsing uses strict format validation — malformed IDs are rejected
- Message content is capped at 2000 chars (Discord's limit)

### Capsule Sandbox Guarantees

The WASM sandbox provides:
- No filesystem access (no VFS capabilities declared beyond KV)
- No process spawning
- Network restricted to declared domains
- Memory bounded by Wasmtime limits
- KV namespace scoped to this capsule only
- IPC topics auto-prefixed with capsule namespace

## 13. Configuration

### Environment Variables (Capsule.toml `[env]`)

| Variable | Type | Description |
|----------|------|-------------|
| `DISCORD_BOT_TOKEN` | secret | Bot token from Discord Developer Portal |
| `DISCORD_APPLICATION_ID` | string | Application ID from Discord Developer Portal |
| `DISCORD_ALLOWED_USERS` | string | Comma-separated Discord user IDs (empty = allow all) |
| `DISCORD_ALLOWED_GUILDS` | string | Comma-separated guild IDs (empty = allow all) |
| `DISCORD_SESSION_SCOPE` | string | `"channel"` (default) or `"user"` |

These are elicited from the user during `capsule install` (docking) and stored securely by the host. The capsule reads them via `sys::get_config_string()`.

### Bot Permissions

Minimum required Discord bot permissions (integer: `2147485696`):
- `Send Messages` (0x800)
- `Embed Links` (0x4000)
- `Read Message History` (0x10000)
- `Use Slash Commands` (0x80000000)

Default Gateway intents: `GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES` (4609). The privileged `MESSAGE_CONTENT` intent (1 << 15) is **not** included by default — enable it in the Discord Developer Portal and capsule config if message content access is needed.

## 14. Comparison with astrid-telegram

| Aspect | astrid-telegram | astrid-discord |
|--------|----------------|----------------|
| **Architecture** | Standalone binary (thin client) | WASM capsule (sandboxed plugin) |
| **Network access** | Direct (teloxide, reqwest) | HTTP Airlock only (SSRF-protected) |
| **Library** | teloxide (full Telegram SDK) | Raw Discord REST API (minimal serde types) |
| **Message routing** | WebSocket JSON-RPC to daemon | Uplink Airlock → InboundMessage |
| **Event consumption** | `Subscription<DaemonEvent>` stream | IPC Airlock polling via cron |
| **Session storage** | In-memory `SessionMap` | KV Airlock (persistent, scoped) |
| **Configuration** | TOML config + env vars | Capsule.toml `[env]` (elicited at install) |
| **Security** | OS-level trust | WASM sandbox + capability declarations |
| **Distribution** | Compiled binary | `.wasm` + `Capsule.toml` (installable capsule) |
| **Gateway connection** | teloxide manages Bot API long polling | Host-side `DiscordGatewayProxy` (`WebSocket` in `astrid-gateway`) |

## 15. Implementation Phases

### Phase 1: Core Capsule (MVP)
- Capsule skeleton with `#[capsule]` macro
- Discord REST API wrapper via HTTP Airlock
- Uplink registration and InboundMessage sending
- `/chat` slash command → agent turn → streamed response
- KV-based session management
- Basic text output (no embeds, no approval UI)

### Phase 2: Interactive Features
- Approval flow with buttons
- Elicitation flow (select, confirm, text modal)
- Tool call display (embeds)
- Error display (red embeds)
- `/reset`, `/status`, `/cancel`, `/help` commands

### Phase 3: Polish
- 2000-char chunking with code block awareness
- Rate limit handling (backoff on 429s)
- Stale interaction token handling (15-min expiry)
- Approval/elicitation TTL cleanup
- Guild/user authorization

### Phase 4: Gateway Support (Future)
- Host-side Discord Gateway proxy
- Real-time message events (non-slash-command messages)
- Presence/status updates
- Thread support

## 16. Open Questions

1. **IPC event schema**: ~~What is the exact JSON schema for runtime events published on the IPC bus?~~ **Resolved.** Events use `IpcPayload::Custom { data }` with `"type"` discriminator (`"interaction"`, `"message"`, `"text_chunk"`, `"turn_complete"`, `"error"`). See `GATEWAY_PROXY.md` §8 for the full schema.

2. ~~**Interaction endpoint hosting**: For Strategy A, where does the daemon expose the HTTP webhook endpoint?~~ **Obsolete.** Strategy B (Gateway proxy) is the default and does not require an inbound webhook endpoint.

3. **Cron implementation status**: The Cron Airlock is marked as Phase 7 (not yet implemented). The capsule depends on it for periodic polling. If Cron is not available, an alternative approach is needed (e.g., the host invoking the capsule's cron handler on a timer without the formal Cron Airlock).

4. **Outbound message routing**: ~~How does the runtime deliver outbound messages back to the capsule?~~ **Resolved.** Agent responses are delivered via IPC bus events (`"text_chunk"`, `"turn_complete"`, `"error"`), polled by the capsule's `poll-gateway` cron handler.

5. **Timestamp access**: WASM capsules don't have direct access to `std::time::Instant` or wall-clock time. The capsule needs timestamps for TTL checks and edit throttling. This may require a new Sys Airlock function (`sys::now_ms()`) or relying on interaction timestamps.
