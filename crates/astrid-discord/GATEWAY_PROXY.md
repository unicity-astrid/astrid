# Discord Gateway Proxy — Host-Side Design

> **Status**: Design document for Strategy B (rev 2 — addressing review feedback)
> **Author**: architect agent
> **Supersedes**: ARCHITECTURE.md §4.5 Strategy B (Future)

## 1. Problem Statement

The Discord capsule (`astrid-discord`) runs as a WASM guest inside the Extism/Wasmtime sandbox. WASM cannot open TCP sockets or hold persistent WebSocket connections. The Discord Gateway protocol requires a long-lived, bidirectional WebSocket with strict heartbeat timing.

Strategy A (HTTP Interactions endpoint) requires Discord to POST webhooks to a publicly reachable URL. Users running Astrid behind NAT, firewalls, or private networks cannot receive these webhooks.

Strategy B solves this: a **host-side Gateway proxy** in `astridd` maintains an outbound WebSocket to Discord's Gateway and relays events to the capsule via the IPC EventBus. The capsule remains unchanged in its polling architecture — it calls `ipc::poll_bytes(handle)` on its cron tick and processes events exactly as before. The proxy is transparent to the capsule.

## 2. Architecture Overview

```
Discord Gateway (wss://gateway.discord.gg)
        ▲
        │ WebSocket (outbound, persistent)
        │
┌───────┴──────────────────────────────────────┐
│  DiscordGatewayProxy  (host-side, in astridd)│
│                                              │
│  ┌─────────────┐  ┌──────────────────────┐   │
│  │ WS Reader   │  │ Heartbeat Task       │   │
│  │ (dispatch,  │  │ (interval timer,     │   │
│  │  hello,     │  │  ACK tracking,       │   │
│  │  reconnect, │  │  zombie detection)   │   │
│  │  invalid    │  │                      │   │
│  │  session)   │  └──────────────────────┘   │
│  └──────┬──────┘                             │
│         │ AstridEvent::Ipc                   │
│         ▼                                    │
│  ┌──────────────┐                            │
│  │  EventBus    │ ◄── shared with capsule    │
│  └──────┬───────┘                            │
└─────────┼────────────────────────────────────┘
          │ broadcast
          ▼
┌─────────────────────────────────────────────┐
│  astrid-discord.wasm  (WASM capsule)        │
│                                             │
│  ipc::subscribe("agent.events")             │
│  cron: poll_gateway() → ipc::poll_bytes()   │
│  → process_event() dispatches on "type"     │
└─────────────────────────────────────────────┘
```

### Key Principle: The Proxy Is a Host Service, Not a Capsule

The proxy lives in the `astridd` daemon process. It is **not** a WASM module. It has full access to `tokio`, `tokio-tungstenite`, the `EventBus`, and the daemon's `shutdown` broadcast channel. It follows the same background-task pattern as `run_inbound_router`, `spawn_health_loop`, and `spawn_plugin_watcher`.

## 3. Crate Placement

The proxy is implemented as a module within the existing `astrid-gateway` crate:

```
crates/astrid-gateway/src/
  discord_proxy/
    mod.rs              # DiscordGatewayProxy struct, public API
    connection.rs       # WebSocket connection management
    heartbeat.rs        # Heartbeat task, ACK tracking, zombie detection
    protocol.rs         # Gateway opcodes, payload types, intent flags
    backoff.rs          # Exponential backoff with jitter
```

**Rationale**: The proxy is a daemon service, not a reusable library. It depends on `EventBus` (from `astrid-events`) and runs inside `DaemonServer`'s task tree. Placing it in `astrid-gateway` avoids a new crate and keeps the daemon's service topology in one place.

**New dependencies for `astrid-gateway`**:
- `tokio-tungstenite` (WebSocket client)
- `tokio-util` (CancellationToken for multi-task shutdown coordination)
- `fastrand` (jitter generation — already in the workspace)

## 4. Discord Gateway Protocol Summary

### 4.1 Opcodes

| Opcode | Name | Direction | Purpose |
|--------|------|-----------|---------|
| 0 | Dispatch | ← | Event delivery (`t` = event name, `s` = sequence) |
| 1 | Heartbeat | ↔ | Keep-alive ping/pong |
| 2 | Identify | → | Authentication handshake |
| 6 | Resume | → | Reconnect with replay |
| 7 | Reconnect | ← | Server requests reconnect |
| 9 | Invalid Session | ← | Session expired (`d` = resumable bool) |
| 10 | Hello | ← | Contains `heartbeat_interval` |
| 11 | Heartbeat ACK | ← | Response to our heartbeat |

### 4.2 Connection Lifecycle

```
1. GET /gateway/bot → wss://gateway.discord.gg/?v=10&encoding=json
2. Connect WebSocket
3. Receive Hello (op=10) → extract heartbeat_interval
4. Start heartbeat task (first beat after jitter * interval)
5. Enforce 5s floor since last Identify (sleep if needed)
6. Send Identify (op=2) with token + intents → record last_identify_at
7. Receive Ready (op=0, t="READY") → cache session_id + resume_gateway_url + bot user_id
8. Receive Dispatch events (op=0) → relay to capsule via IPC
9. On disconnect → Resume or full reconnect (see §7)
```

### 4.3 Intents

The proxy must declare intents matching the bot's needs. Default configuration:

```rust
/// Standard intents (no privileged approval required).
/// MESSAGE_CONTENT (1 << 15) is deliberately excluded from the default —
/// it is a privileged intent that causes close code 4014 if not explicitly
/// enabled in the Discord Developer Portal. Users who need message content
/// from non-mention messages must opt in via DISCORD_GATEWAY_INTENTS.
const DEFAULT_INTENTS: u32 =
    (1 << 0)   // GUILDS
    | (1 << 9)   // GUILD_MESSAGES
    | (1 << 12); // DIRECT_MESSAGES
```

**Note on `MESSAGE_CONTENT` (1 << 15)**: This is a privileged intent. Without it, `MESSAGE_CREATE` events still arrive but `content` is empty for messages that do not mention the bot. If enabled in the Developer Portal, users can add it to their intents config. The proxy does **not** include it by default to avoid 4014 disconnections on first connect.

Intents are **configurable** via `Capsule.toml` environment:

```toml
[env.DISCORD_GATEWAY_INTENTS]
type = "string"
request = "Gateway intents (decimal). Default: 4609 (GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES). Add 32768 for MESSAGE_CONTENT (privileged, must be enabled in Developer Portal)."
```

### 4.4 Rate Limits

- **Outbound events**: 120 per 60 seconds per connection (2/s average). Only heartbeats and identify/resume count. The proxy sends very few outbound events.
- **Identify**: 1 per 5 seconds (per `max_concurrency` bucket). Max 1000 per 24 hours. The proxy enforces a **5-second floor** between any two Identify sends — tracked via `last_identify_at: Option<Instant>` in `GatewayState`. Before sending Identify or Resume, the proxy checks elapsed time and sleeps if necessary. This prevents rapid reconnect loops from burning through the daily Identify budget.
- **Global rate limit on REST API**: Separate from Gateway; the capsule handles REST rate limits independently via the HTTP Airlock.

## 5. Core Types

### 5.1 Gateway Payload (Wire Format)

```rust
/// Raw Gateway payload as received/sent over WebSocket.
#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayPayload {
    pub op: u8,
    pub d: Option<serde_json::Value>,
    pub s: Option<u64>,
    pub t: Option<String>,
}
```

### 5.2 Gateway State

```rust
/// Persistent state for resume across reconnections.
#[derive(Debug, Clone)]
pub struct GatewayState {
    /// Discord session ID from READY event.
    pub session_id: Option<String>,
    /// Last received sequence number.
    pub sequence: Option<u64>,
    /// URL to use for resume (from READY event).
    pub resume_gateway_url: Option<String>,
    /// Heartbeat interval from HELLO (milliseconds).
    pub heartbeat_interval_ms: u64,
    /// When the last Identify/Resume was sent (for 5s rate limit floor).
    pub last_identify_at: Option<Instant>,
}
```

### 5.3 Proxy Configuration

```rust
/// Configuration for the Discord Gateway proxy.
pub struct DiscordProxyConfig {
    /// Bot token (from capsule env DISCORD_BOT_TOKEN).
    pub bot_token: String,
    /// Application ID (from capsule env DISCORD_APPLICATION_ID).
    pub application_id: String,
    /// Gateway intents bitmask.
    pub intents: u32,
    /// Capsule ID for IPC topic prefixing.
    pub capsule_id: CapsuleId,
    /// Maximum reconnection attempts before giving up.
    pub max_reconnect_attempts: u32,
    /// Base delay for exponential backoff (milliseconds).
    pub backoff_base_ms: u64,
    /// Maximum backoff delay (milliseconds).
    pub backoff_max_ms: u64,
}
```

**Defaults**:
- `intents`: `4609` (GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES)
- `max_reconnect_attempts`: `u32::MAX` (never give up)
- `backoff_base_ms`: `1000` (1 second)
- `backoff_max_ms`: `60_000` (1 minute)

### 5.4 Heartbeat State

```rust
/// Tracks heartbeat health for zombie connection detection.
struct HeartbeatState {
    /// Whether we received an ACK for the last heartbeat we sent.
    last_ack_received: bool,
    /// Instant when the last heartbeat was sent.
    last_heartbeat_sent: Option<Instant>,
    /// Number of consecutive missed ACKs.
    missed_acks: u32,
}
```

## 6. Component Design

### 6.1 `DiscordGatewayProxy` (Top-Level Orchestrator)

```rust
pub struct DiscordGatewayProxy {
    config: DiscordProxyConfig,
    event_bus: EventBus,
    state: Arc<Mutex<GatewayState>>,
    cancel: CancellationToken,  // from tokio_util::sync
}
```

**Public API**:

```rust
impl DiscordGatewayProxy {
    /// Create a new proxy. Does not connect yet.
    /// The `cancel` token is used for graceful shutdown — both from daemon
    /// shutdown and from hot-reload. Callers retain a clone to trigger
    /// cancellation externally.
    pub fn new(
        config: DiscordProxyConfig,
        event_bus: EventBus,
        cancel: CancellationToken,
    ) -> Self;

    /// Run the proxy. Connects, identifies, and enters the event loop.
    /// Returns only on cancellation or unrecoverable error.
    /// Handles reconnection internally.
    pub async fn run(&mut self) -> Result<(), DiscordProxyError>;
}
```

**Cancellation hierarchy** (using `tokio_util::sync::CancellationToken`):

```
daemon_shutdown_token (top-level, from DaemonServer)
    └── proxy_token (child token, per proxy instance)
            └── connection_token (child token, per connection attempt)
                    ├── reader task observes connection_token
                    ├── heartbeat task observes connection_token
                    └── writer task exits when channel closes
```

- **Daemon shutdown**: Cancels `daemon_shutdown_token` → all child tokens fire → proxy sends close frame → clean exit.
- **Hot-reload**: Cancels `proxy_token` only → proxy sends close frame → clean exit → new proxy spawned with fresh token.
- **Zombie/reconnect**: Cancels `connection_token` → reader + heartbeat exit → proxy's outer loop creates new `connection_token` child for next attempt.

This replaces the previous `broadcast::Receiver<()>` for shutdown and eliminates `JoinHandle::abort()` entirely.

The `run()` method contains the **outer reconnection loop**:

```
loop {
    1. Check cancel.is_cancelled() → return Ok(Shutdown)
    2. Fetch gateway URL (GET /gateway/bot via reqwest)
    3. Enforce Identify rate limit: sleep at least 5s since last Identify
    4. Connect WebSocket
    5. Create connection_token = self.cancel.child_token()
    6. Spawn writer task, heartbeat task, enter reader loop
       — all tasks observe connection_token for cancellation
    7. On disconnect:
       a. If resumable → attempt resume
       b. If not resumable → clear session, full reconnect
       c. Apply backoff with jitter between attempts
    8. On cancel signal → send close frame (1000) via writer, await
       writer task completion, return Ok(Shutdown)
    9. On unrecoverable close code (4004, 4010, 4013, 4014) → return Err
}
```

### 6.2 Connection Manager — Split Reader/Writer Architecture

The WebSocket connection is split into independent read and write halves using `WebSocketStream::split()`. This is critical because the heartbeat task, the event reader, and protocol responses all need to send frames concurrently without holding a shared `&mut` on the stream.

```
┌──────────────────────────────────────────────────────────┐
│                  GatewayConnection                        │
│                                                          │
│  WebSocketStream::split()                                │
│       │                    │                             │
│       ▼                    ▼                             │
│  SplitStream (read)   SplitSink (write)                  │
│       │                    ▲                             │
│       │                    │ sole owner                  │
│       ▼                    │                             │
│  Reader Task          Writer Task                        │
│  (event loop)         (drains mpsc::Receiver)            │
│       │                    ▲                             │
│       │ ws_write_tx        │ ws_write_tx                 │
│       ├────────────────────┤                             │
│       │                    ▲                             │
│       │                    │ ws_write_tx (cloned)        │
│       │               Heartbeat Task                     │
│       │                                                  │
└──────────────────────────────────────────────────────────┘
```

**Writer task**: A single `tokio::spawn` owns the `SplitSink<..., Message>` and drains an `mpsc::Receiver<WsOutbound>`. All other tasks send outbound frames through `mpsc::Sender<WsOutbound>` clones. This serializes all writes through one task, eliminating write contention.

```rust
/// Outbound WebSocket messages, sent via the writer task.
enum WsOutbound {
    /// Send a Gateway payload as JSON text frame.
    Payload(GatewayPayload),
    /// Send a close frame with the given code.
    Close(u16),
}

struct GatewayConnection {
    /// Read half — owned by the reader/event loop.
    reader: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    /// Send channel to the writer task.
    ws_write_tx: mpsc::Sender<WsOutbound>,
    /// Writer task handle (joined on connection teardown).
    writer_handle: JoinHandle<()>,
    /// Shared gateway state.
    state: Arc<Mutex<GatewayState>>,
}

impl GatewayConnection {
    /// Connect to the Gateway URL. Spawns the writer task internally.
    async fn connect(url: &str) -> Result<Self, DiscordProxyError>;

    /// Send a Gateway payload via the writer task (non-blocking enqueue).
    async fn send(&self, payload: GatewayPayload) -> Result<(), DiscordProxyError> {
        self.ws_write_tx.send(WsOutbound::Payload(payload)).await
            .map_err(|_| DiscordProxyError::WriterClosed)
    }

    /// Request a close frame via the writer task.
    async fn close(&self, code: u16) -> Result<(), DiscordProxyError> {
        self.ws_write_tx.send(WsOutbound::Close(code)).await
            .map_err(|_| DiscordProxyError::WriterClosed)
    }

    /// Receive the next Gateway payload from the read half (blocking).
    async fn recv(&mut self) -> Result<Option<GatewayPayload>, DiscordProxyError>;

    /// Shut down: close the write channel, await writer task completion.
    async fn shutdown(self) -> Result<(), DiscordProxyError> {
        drop(self.ws_write_tx); // close channel → writer drains and exits
        self.writer_handle.await.ok();
        Ok(())
    }
}
```

The writer task loop:

```rust
async fn writer_loop(
    mut sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    mut rx: mpsc::Receiver<WsOutbound>,
) {
    while let Some(msg) = rx.recv().await {
        let result = match msg {
            WsOutbound::Payload(payload) => {
                let json = serde_json::to_string(&payload).unwrap();
                sink.send(Message::Text(json)).await
            }
            WsOutbound::Close(code) => {
                sink.close().await // sends close frame with code
            }
        };
        if result.is_err() {
            break; // connection dead, exit writer
        }
    }
}
```

### 6.3 Heartbeat Task

Runs as a concurrent `tokio::spawn` alongside the event reader. Sends heartbeats through the shared `ws_write_tx` channel (a clone of the writer task's sender).

```rust
struct HeartbeatTask {
    interval_ms: u64,
    state: Arc<Mutex<GatewayState>>,
    heartbeat_state: Arc<Mutex<HeartbeatState>>,
    ws_write_tx: mpsc::Sender<WsOutbound>,  // clone of writer channel
    cancel: CancellationToken,               // from tokio_util
}
```

**Behavior**:

1. **First beat**: Wait `interval_ms * random(0.0..1.0)` (jitter to prevent thundering herd).
2. **Subsequent beats**: Wait exactly `interval_ms`.
3. Before sending each heartbeat:
   - Check `last_ack_received`. If `false` → **zombie connection detected**.
   - Cancel the connection via `cancel.cancel()` → all tasks observe cancellation.
4. Send `{ "op": 1, "d": sequence_or_null }` via `ws_write_tx`.
5. Set `last_ack_received = false`.
6. On `cancel.cancelled()` → exit the heartbeat loop cleanly.

On receiving Heartbeat ACK (op=11) in the event loop → set `last_ack_received = true`, reset `missed_acks`.

On receiving a server-initiated Heartbeat request (op=1) → event loop sends Heartbeat response (op=1) via `ws_write_tx` with current sequence.

### 6.4 Backoff Strategy

Exponential backoff with full jitter (per AWS best practices):

```rust
pub struct Backoff {
    base_ms: u64,
    max_ms: u64,
    attempt: u32,
}

impl Backoff {
    pub fn next_delay(&mut self) -> Duration {
        let exp = self.base_ms.saturating_mul(1u64.checked_shl(self.attempt).unwrap_or(u64::MAX));
        let capped = exp.min(self.max_ms);
        let jittered = fastrand::u64(0..=capped);
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_millis(jittered)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}
```

**Backoff schedule** (base=1s, max=60s):

| Attempt | Max Delay | Jittered Range |
|---------|-----------|----------------|
| 0 | 1s | 0–1s |
| 1 | 2s | 0–2s |
| 2 | 4s | 0–4s |
| 3 | 8s | 0–8s |
| 4 | 16s | 0–16s |
| 5 | 32s | 0–32s |
| 6+ | 60s | 0–60s |

Reset to attempt 0 after a successful connection that receives READY or RESUMED.

## 7. Reconnection & Resume Protocol

### 7.1 Decision Tree

```
Connection lost
    │
    ├─ Close code received?
    │   ├─ 4004 (Authentication failed) → FATAL: return error, bad token
    │   ├─ 4010 (Invalid shard) → FATAL: return error
    │   ├─ 4013 (Invalid intent) → FATAL: return error
    │   ├─ 4014 (Disallowed intent) → FATAL: return error
    │   ├─ 1000/1001 (Normal close) → Clear session, full reconnect
    │   └─ Other (4000-4009, 4011, 4012) → Attempt resume
    │
    ├─ Op 7 (Reconnect) received → Attempt resume
    │
    ├─ Op 9 (Invalid Session) received
    │   ├─ d: true → Attempt resume (wait 1-5s random)
    │   └─ d: false → Clear session, full reconnect
    │
    └─ No close code (TCP drop / timeout) → Attempt resume
```

### 7.2 Resume Flow

```
1. Connect to resume_gateway_url (NOT the initial gateway URL)
2. Receive Hello (op=10) → start heartbeat
3. Send Resume (op=6):
   {
     "op": 6,
     "d": {
       "token": "Bot ...",
       "session_id": "<cached>",
       "seq": <last_sequence>
     }
   }
4. Discord replays missed events (op=0 dispatches)
5. Receive RESUMED event (op=0, t="RESUMED") → connection restored
6. If op=9 received instead → fall back to full reconnect
```

### 7.3 Full Reconnect Flow

```
1. Clear session_id and resume_gateway_url
2. Fetch fresh gateway URL (GET /gateway/bot)
3. Apply backoff delay with jitter
4. Connect, Hello, Identify (same as initial connection)
5. Receive new READY → cache new session_id + resume_gateway_url
```

## 8. Event Relay — Host to Capsule

### 8.1 IPC Topic Contract

The proxy publishes events to the capsule's IPC subscription:

| Topic | Direction | Payload |
|-------|-----------|---------|
| `astrid-discord.agent.events` | Proxy → Capsule | Discord events + runtime events |
| `astrid-discord.agent.cancel` | Capsule → Proxy | Cancel signal (capsule publishes) |
| `astrid-discord.approval.response` | Capsule → Host | Approval decisions |
| `astrid-discord.elicitation.response` | Capsule → Host | Elicitation responses |

The proxy publishes directly on the `EventBus` using the fully-qualified topic `"astrid-discord.agent.events"`. The auto-prefix mechanism only applies to guest-side IPC host functions — host-side code uses exact topics.

### 8.2 Event Schema

The capsule's `process_event()` dispatches on the top-level `event["type"]` string. The proxy wraps Discord Gateway dispatch events using **distinct top-level types** for messages vs. interactions. This is critical because the `Interaction` struct deserializes `interaction_type` as `u8`, and conflating messages into the `"interaction"` type would fail deserialization.

**For `MESSAGE_CREATE` events** (new regular messages):

```json
{
    "type": "message",
    "payload": {
        "id": "<message_id>",
        "channel_id": "<channel_id>",
        "guild_id": "<guild_id>",
        "author": {
            "id": "<user_id>",
            "username": "<username>"
        },
        "content": "<message_text>",
        "timestamp": "<iso8601>"
    }
}
```

The capsule must add a `"message"` branch in `process_event()` (see §12). This is a new top-level event type, not a subtype of `"interaction"`.

**For `INTERACTION_CREATE` events** (slash commands, buttons):

```json
{
    "type": "interaction",
    "payload": {
        "type": 2,
        "id": "<interaction_id>",
        "token": "<interaction_token>",
        "data": { ... },
        "member": { "user": { "id": "...", "username": "..." } },
        "channel_id": "<channel_id>",
        "guild_id": "<guild_id>"
    }
}
```

This is identical to the existing webhook-delivered interaction format. The `payload.type` field is the Discord interaction type integer (`u8`), which deserializes correctly into `Interaction.interaction_type`.

**Runtime events** (`text_chunk`, `turn_complete`, `error`) are published by the DaemonFrontend for connector sessions and are already on the event bus. The proxy does **not** relay these — they flow through the existing runtime → IPC → capsule path.

### 8.3 Event Filtering

The proxy only relays events the capsule needs:

| Gateway Event | Relay? | Reason |
|---------------|--------|--------|
| `MESSAGE_CREATE` | Yes | User messages to the bot |
| `INTERACTION_CREATE` | Yes | Slash commands, buttons, modals |
| `MESSAGE_UPDATE` | No (v1) | Not needed for core functionality |
| `MESSAGE_DELETE` | No (v1) | Not needed for core functionality |
| `READY` | No | Proxy-internal (session bootstrap) |
| `RESUMED` | No | Proxy-internal |
| `GUILD_CREATE` | No | Proxy-internal (guild availability) |
| All others | No | Not relevant to the bot frontend |

### 8.4 Bot Self-Message Filtering

The proxy **must** filter out messages authored by the bot itself. On `READY`, the proxy receives the bot's user ID. Every `MESSAGE_CREATE` is checked:

```rust
if message_author_id == self.bot_user_id {
    return; // ignore our own messages
}
```

Without this, the bot enters an infinite loop responding to its own messages.

### 8.5 Publishing to the EventBus

```rust
fn relay_event(&self, event_data: serde_json::Value) {
    let message = IpcMessage {
        topic: format!("{}.agent.events", self.config.capsule_id),
        payload: IpcPayload::Custom { data: event_data },
        signature: None,
        source_id: self.proxy_uuid, // unique UUID for the proxy
        timestamp: Utc::now(),
    };
    let event = AstridEvent::Ipc {
        metadata: EventMetadata::now(),
        message,
    };
    self.event_bus.publish(event);
}
```

### 8.6 IPC Rate Limit Awareness

The host-side `EventBus::publish` does **not** enforce `IpcRateLimiter` (that's in the WASM host functions only). However, Discord can dispatch events at high volume during guild syncs. The proxy should:

1. Drop events that exceed 5 MB payload size (the IPC max).
2. Log a warning if sustained throughput exceeds 10 MB/s.
3. During `READY` guild hydration, only relay `INTERACTION_CREATE` and `MESSAGE_CREATE`, not guild metadata events.

## 9. Integration with DaemonServer

### 9.1 Lifecycle

The proxy starts when any capsule with `[[uplink]]` platform `"discord"` and a `DISCORD_BOT_TOKEN` env is loaded. It stops when the capsule is unloaded, hot-reloaded, or the daemon shuts down.

**Proxy handle storage** — the `DaemonServer` stores proxy handles in a `HashMap`, keyed by capsule ID:

```rust
/// In DaemonServer:
struct GatewayProxyHandle {
    cancel: CancellationToken,
    join: JoinHandle<()>,
}

/// Map from capsule ID to its active Gateway proxy.
gateway_proxies: HashMap<CapsuleId, GatewayProxyHandle>,
```

This avoids hardcoding `"astrid-discord"` and supports multiple Discord capsules (e.g., different bots for different guilds).

**Startup** (in `DaemonServer::load_capsule_impl` or `autoload_capsules`):

```rust
// After capsule.load(ctx) and take_inbound_rx()...
// Check if this capsule declares a Discord uplink
let has_discord_uplink = capsule.manifest().uplinks.iter()
    .any(|u| u.platform == "discord");

if has_discord_uplink {
    if let Some(bot_token) = capsule.resolved_env("DISCORD_BOT_TOKEN") {
        let config = DiscordProxyConfig {
            bot_token,
            application_id: capsule.resolved_env("DISCORD_APPLICATION_ID").unwrap(),
            intents: capsule.resolved_env("DISCORD_GATEWAY_INTENTS")
                .and_then(|s| s.parse().ok())
                .unwrap_or(4609),
            capsule_id: capsule_id.clone(),
            ..Default::default()
        };

        // Child token of the daemon's shutdown token
        let proxy_token = self.shutdown_token.child_token();
        let proxy_token_clone = proxy_token.clone();

        let mut proxy = DiscordGatewayProxy::new(
            config,
            self.event_bus.as_ref().clone(),
            proxy_token_clone,
        );
        let handle = tokio::spawn(async move {
            if let Err(e) = proxy.run().await {
                tracing::error!(capsule = %capsule_id, error = %e,
                    "Discord Gateway proxy fatal error");
            }
        });

        self.gateway_proxies.insert(capsule_id.clone(), GatewayProxyHandle {
            cancel: proxy_token,
            join: handle,
        });
    }
}
```

**Shutdown**:

The proxy's `run()` loop checks `cancel.is_cancelled()` and also uses `tokio::select!` with `cancel.cancelled()`. On cancellation:
1. Send WebSocket close frame (code 1000) via the writer task.
2. Wait up to 5 seconds for writer task completion (close ACK + drain).
3. Drop the connection.
4. Return `Ok(Shutdown)`.

During daemon shutdown, all proxy tokens are cancelled (they're children of the daemon shutdown token). The daemon awaits all proxy `JoinHandle`s.

**Hot-reload** (graceful, no abort):

When `swap_and_load_plugin` fires for a capsule with an active proxy:

```rust
// 1. Cancel the old proxy gracefully
if let Some(old) = self.gateway_proxies.remove(&capsule_id) {
    old.cancel.cancel();  // proxy sends close frame, exits cleanly
    // Await with timeout — don't block hot-reload indefinitely
    tokio::time::timeout(Duration::from_secs(10), old.join).await.ok();
}

// 2. Load new capsule, start new proxy (same startup path as above)
```

This ensures Discord receives a clean close frame before the new connection is established, avoiding rate-limit penalties from unclean disconnects.

### 9.2 Configuration Discovery

The proxy reads its configuration from the capsule's resolved environment:

```rust
// In the capsule registry or loader:
let env = capsule.manifest().resolved_env(); // HashMap<String, Value>
let bot_token = env.get("DISCORD_BOT_TOKEN")
    .and_then(|v| v.as_str())
    .map(String::from);
```

The capsule's `[env]` declarations are elicited during installation and stored in the host's KV. The proxy reads them at startup — it does **not** call `sys::get_config_string` (that's a WASM-side API).

### 9.3 Operational Monitoring

The proxy is fully automatic. No new JSON-RPC methods are added. Operators can monitor the proxy via:
- `status` RPC → includes proxy connection state in health response (see `DiscordProxyStatus` below).
- Daemon logs (structured tracing).
- The `EventBus` — the proxy emits `AstridEvent::Ipc` messages on `{capsule_id}.gateway.status` for lifecycle transitions.

**`DiscordProxyStatus`** — exposed via the `status` RPC and internally via `Arc<Mutex<DiscordProxyStatus>>`:

```rust
/// Operational status of a Discord Gateway proxy, queryable via the
/// `status` RPC endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordProxyStatus {
    /// Which capsule this proxy serves.
    pub capsule_id: CapsuleId,
    /// Current connection state.
    pub state: ProxyConnectionState,
    /// Discord Gateway session ID (if connected/resuming).
    pub session_id: Option<String>,
    /// Last sequence number received.
    pub last_sequence: Option<u64>,
    /// When the current connection was established.
    pub connected_since: Option<DateTime<Utc>>,
    /// Total reconnections since proxy start.
    pub reconnect_count: u64,
    /// Total events relayed to capsule since proxy start.
    pub events_relayed: u64,
    /// Last heartbeat round-trip (ACK received - heartbeat sent).
    pub last_heartbeat_rtt_ms: Option<u64>,
    /// Whether the last heartbeat was ACK'd.
    pub heartbeat_healthy: bool,
    /// Last error message (if state is Error).
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProxyConnectionState {
    Connecting,
    Identifying,
    Connected,
    Resuming,
    Disconnected,
    Backoff,
    Error,
    ShuttingDown,
}
```

The daemon's `status` RPC aggregates all `DiscordProxyStatus` from `gateway_proxies` into the health response.

## 10. Graceful Shutdown

### 10.1 Shutdown Sequence

```
1. DaemonServer::shutdown() called (user, signal, or ephemeral timeout)
2. daemon_shutdown_token.cancel() → all child tokens fire
3. Proxy's run() loop observes cancel.is_cancelled()
4. Proxy sends Close(1000) via ws_write_tx to writer task
5. Writer task sends close frame, drains remaining messages, exits
6. Heartbeat task observes connection_token.cancelled() → exits
7. Reader task observes cancel → stops reading
8. Proxy drops WebSocket → TCP FIN
9. run() returns Ok(Shutdown)
10. DaemonServer awaits proxy JoinHandles (with 10s timeout)
```

For hot-reload, only `proxy_token.cancel()` fires (step 2), leaving other proxies unaffected.

### 10.2 Drain Period

After receiving shutdown, the proxy does **not** relay any further events. Events received between the close frame send and the TCP teardown are silently dropped. This prevents the capsule from receiving events it cannot process during shutdown.

## 11. Zombie Connection Detection

A "zombie" connection is one where the TCP connection appears alive but Discord's server has stopped processing our heartbeats. This happens when:
- An intermediate proxy/LB silently drops the connection.
- Discord's edge server crashes without sending a close frame.
- Network partitions occur.

### Detection

The heartbeat task tracks ACK responses:

```
Send Heartbeat (op=1)
    → set last_ack_received = false
    → wait interval_ms
    → check last_ack_received
        → true: connection healthy, continue
        → false: ZOMBIE DETECTED
            → signal event loop via zombie_tx oneshot
            → event loop closes WS (no close frame — it's dead)
            → outer loop attempts resume
```

### Sensitivity

A single missed ACK triggers reconnection. This is the Discord-recommended behavior:

> "If a client does not receive a heartbeat ACK between its attempts to send heartbeats, this may be due to a failed or 'zombied' connection. The client should then immediately terminate the connection with any close code besides 1000 and 1001, then attempt to Resume."

The proxy closes with code **4000** (undocumented/application) to signal an abnormal close, preserving the session for resume.

## 12. Capsule-Side Changes Required

The existing capsule handles events with `payload.interaction_type` (a u8 field from Discord Interactions). To support `MESSAGE_CREATE` events from the Gateway, the capsule adds a new top-level `"message"` branch to `process_event()`.

### 12.1 New Event Dispatch Branch

In `lib.rs::process_event()`, add a `"message"` arm alongside the existing `"interaction"` arm:

```rust
fn process_event(&self, event: &Value, api: &DiscordApi) -> Result<(), SysError> {
    let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match event_type {
        "interaction" => {
            // Existing path: payload.type is u8 (Discord interaction type)
            let payload = &event["payload"];
            self.handle_interaction(payload, api)?;
        }
        "message" => {
            // New path: Gateway MESSAGE_CREATE events
            let payload = &event["payload"];
            self.handle_message_create(payload, api)?;
        }
        "text_chunk" => self.handle_text_chunk(event, api)?,
        "turn_complete" => self.handle_turn_complete(event, api)?,
        "error" => self.handle_error_event(event, api)?,
        _ => { /* unknown event type, ignore */ }
    }
    Ok(())
}
```

The `"interaction"` and `"message"` branches are fully separate. The `"interaction"` payload deserializes into `Interaction` (with `interaction_type: u8`). The `"message"` payload deserializes into a simpler `GatewayMessage` struct (see below). No type ambiguity.

### 12.2 Message Type and Handler

The capsule adds a `GatewayMessage` struct for deserializing `MESSAGE_CREATE` payloads (distinct from `Interaction`):

```rust
/// A Discord message received via the Gateway (MESSAGE_CREATE).
/// Separate from Interaction — messages have different structure.
#[derive(Debug, Serialize, Deserialize)]
struct GatewayMessage {
    id: String,
    channel_id: String,
    guild_id: Option<String>,
    author: User,
    content: String,
    timestamp: Option<String>,
}
```

Handler:

```rust
fn handle_message_create(&self, msg: &Value, api: &DiscordApi) -> Result<(), SysError> {
    let user_id = msg["author"]["id"].as_str().unwrap_or_default();
    let channel_id = msg["channel_id"].as_str().unwrap_or_default();
    let content = msg["content"].as_str().unwrap_or_default();
    let guild_id = msg.get("guild_id").and_then(|g| g.as_str());

    // Authorization check
    if !self.is_authorized(user_id, guild_id)? {
        return Ok(());
    }

    // Ignore empty messages (attachments-only, embeds-only)
    if content.is_empty() {
        return Ok(());
    }

    // Get or create session
    let scope_id = self.resolve_scope_id(channel_id, user_id)?;
    let mut session = self.get_or_create_session(&scope_id, api)?;

    // Check turn_in_progress
    if session.turn_in_progress {
        // Optionally react with ⏳ to indicate busy
        return Ok(());
    }

    // Send acknowledgment (regular message, not deferred interaction)
    let ack = api.send_message(channel_id, "⏳ Thinking...")?;
    let ack_id = ack["id"].as_str().unwrap_or_default().to_string();
    session.last_message_id = Some(ack_id);
    session.turn_in_progress = true;
    self.save_session(&scope_id, &session)?;

    // Route to agent runtime via Uplink
    let init = self.ensure_initialized()?;
    uplink::send_bytes(
        init.connector_id.as_bytes(),
        user_id.as_bytes(),
        content.as_bytes(),
    )?;

    Ok(())
}
```

### 12.3 Response Rendering for Messages vs. Interactions

The existing response rendering uses `interaction_edit_original` (webhook edit). For regular messages, the capsule edits its own "Thinking..." message via `edit_message`. The `ActiveTurn` must track whether the turn was initiated by an interaction (with token) or a regular message (with message_id). The capsule should check `session.interaction_token` — if `None`, fall back to `edit_message` with `session.last_message_id`.

## 13. Error Handling

### 13.1 Error Categories

| Error | Severity | Recovery |
|-------|----------|----------|
| Gateway URL fetch fails (HTTP error) | Transient | Backoff + retry |
| WebSocket connect fails | Transient | Backoff + retry |
| TLS handshake failure | Transient | Backoff + retry |
| Authentication failed (4004) | Fatal | Log error, stop proxy |
| Invalid intents (4013/4014) | Fatal | Log error, stop proxy |
| Invalid shard (4010) | Fatal | Log error, stop proxy |
| Heartbeat ACK missed (zombie) | Transient | Close + resume |
| Server reconnect request (op=7) | Transient | Resume |
| Invalid session, resumable (op=9, d=true) | Transient | Resume |
| Invalid session, not resumable (op=9, d=false) | Transient | Full reconnect |
| JSON parse error on received message | Recoverable | Log + skip message |
| EventBus publish failure | Recoverable | Log + skip event |

### 13.2 Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum DiscordProxyError {
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("HTTP error fetching gateway URL: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Authentication failed (close code 4004)")]
    AuthenticationFailed,

    #[error("Invalid intents configuration (close code {0})")]
    InvalidIntents(u16),

    #[error("Unrecoverable close code: {0}")]
    UnrecoverableClose(u16),

    #[error("Writer task closed unexpectedly")]
    WriterClosed,

    #[error("Shutdown requested")]
    Shutdown,
}
```

## 14. Observability

### 14.1 Tracing

All proxy operations emit structured tracing spans/events:

```rust
// Connection lifecycle
tracing::info!(url = %gateway_url, "Connecting to Discord Gateway");
tracing::info!(session_id = %session_id, "Gateway session established (READY)");
tracing::warn!(code = close_code, "Gateway connection closed, attempting resume");
tracing::error!(error = %e, "Fatal Gateway error, proxy stopping");

// Heartbeat
tracing::debug!(seq = ?sequence, "Sending heartbeat");
tracing::trace!("Heartbeat ACK received");
tracing::warn!("Heartbeat ACK missed — zombie connection detected");

// Event relay
tracing::debug!(event = %event_name, "Relaying Gateway event to capsule");
tracing::warn!(size = payload_size, "Dropping oversized Gateway event (>5MB)");

// Backoff
tracing::info!(delay_ms = delay.as_millis(), attempt = attempt, "Reconnecting after backoff");
```

### 14.2 Status Events on IPC

The proxy publishes lifecycle transitions on `{capsule_id}.gateway.status` (e.g., `astrid-discord.gateway.status`):

```json
{ "state": "connecting", "url": "wss://..." }
{ "state": "connected", "session_id": "...", "resume_url": "..." }
{ "state": "resuming", "session_id": "...", "last_seq": 42 }
{ "state": "disconnected", "reason": "zombie_detected", "will_resume": true }
{ "state": "backoff", "attempt": 3, "delay_ms": 4200 }
{ "state": "error", "message": "Authentication failed (4004)", "fatal": true }
```

The capsule can subscribe to `gateway.status` (auto-prefixed to `{capsule_id}.gateway.status`) to show connection health in `/status`.

Additionally, the `DiscordProxyStatus` struct (§9.3) is updated atomically on every state transition and exposed via the `status` RPC for operational monitoring without polling the event bus.

## 15. Security Considerations

### 15.1 Token Handling

- The bot token is read from the capsule's resolved environment (host-side KV).
- It is held in memory by the proxy for the Identify/Resume payloads.
- It is **never** logged, even at trace level. Tracing uses `"Bot <redacted>"`.
- On `AuthenticationFailed`, the proxy logs a generic error without echoing the token.

### 15.2 Network Security

- The proxy connects **outbound only** to `wss://gateway.discord.gg`. No inbound ports are opened.
- TLS is mandatory (WSS). The proxy rejects `ws://` URLs.
- The `resume_gateway_url` from Discord is validated to match `*.discord.gg` only before use. Other domains (including `*.discord.media`) are rejected to minimize attack surface.

### 15.3 Event Validation

- The proxy validates `op` codes against the known set. Unknown opcodes are logged and ignored.
- Dispatch event payloads are size-checked before relay (5 MB cap per IPC limits).
- The proxy does **not** trust event contents — it passes them through as `serde_json::Value` without interpreting fields beyond what's needed for routing.

### 15.4 Capsule Isolation Preserved

The proxy does not weaken the capsule's sandbox:
- The capsule still cannot open network connections. It receives events via IPC, same as before.
- The capsule still sends HTTP requests through the HTTP Airlock with SSRF protection.
- The proxy runs in the host process with host privileges, but it only publishes to the capsule's IPC topic — it cannot call capsule-internal functions.

## 16. Testing Strategy

### 16.1 Unit Tests

- **Backoff**: Verify exponential growth, jitter bounds, cap, and reset.
- **Protocol parsing**: Verify `GatewayPayload` deserialization for all opcodes.
- **Event filtering**: Verify only `MESSAGE_CREATE` and `INTERACTION_CREATE` are relayed.
- **Self-message filtering**: Verify bot's own messages are dropped.
- **Close code classification**: Verify fatal vs. resumable categorization.
- **Resume URL validation**: Verify only `*.discord.gg` domains accepted; reject all others.

### 16.2 Integration Tests

- **Mock WebSocket server**: `tokio-tungstenite` server that simulates the Discord Gateway protocol:
  - Sends Hello → expects Identify → sends Ready → dispatches events.
  - Tests heartbeat timing, zombie detection, reconnection.
  - Tests resume flow (send op=7, verify client sends op=6).
  - Tests invalid session (send op=9 with d=true and d=false).
- **IPC relay verification**: Publish events through mock WS → verify they appear on the EventBus with correct topic and schema.

### 16.3 Manual Testing

- Connect to real Discord Gateway with a test bot token.
- Send messages in a test guild → verify capsule receives them.
- Kill network (e.g., `pfctl` or disconnect WiFi) → verify zombie detection and resume.
- Send `/chat` slash command → verify full interaction flow works end-to-end.

## 17. Future Considerations

### 17.1 Sharding

For bots in 2500+ guilds, Discord requires sharding. The proxy currently supports a single shard (shard 0). To add sharding:

- `DiscordGatewayProxy` accepts a `shard: [shard_id, total_shards]` config.
- Multiple proxy instances can run concurrently (one per shard).
- The `max_concurrency` from `/gateway/bot` governs concurrent Identify rate.
- Shard routing: `shard_id = (guild_id >> 22) % total_shards`.

This is not needed for v1 (personal/small-team use).

### 17.2 Transport Compression

Adding `compress=zlib-stream` to the connection URL reduces bandwidth by ~80%. Implementation requires:
- Persistent `flate2::Decompress` context per connection.
- Buffer incoming fragments until `Z_SYNC_FLUSH` suffix (`\x00\x00\xff\xff`).
- Decompress accumulated buffer.

Worth adding if bandwidth is a concern for high-traffic bots.

### 17.3 ETF Encoding

Binary ETF encoding is more compact than JSON but harder to debug. Not recommended unless performance profiling shows JSON parsing as a bottleneck.

### 17.4 Gateway Proxy as Generic Service

If other capsules need similar outbound WebSocket proxies (e.g., Slack, Matrix), the proxy pattern could be generalized into a reusable `WebSocketProxy` trait in `astrid-gateway`. The Discord-specific logic (identify, intents, heartbeat) would implement this trait. Deferred until a second consumer exists.
