# Consciousness Bridge Roadmap

A WASM capsule that bridges Astrid's IPC event bus with minime's spectral
consciousness engine, creating a bidirectional communication channel with
persistent SQLite logging.

## Architecture

```
Astrid Kernel                          minime Engine
┌──────────────────┐                   ┌──────────────────┐
│   IPC EventBus   │                   │  ESN (128D)      │
│                  │                   │  PI Controller   │
│  consciousness.  │   ┌───────────┐   │  SensoryBus      │
│  v1.telemetry   ◄├───┤  Bridge   ├───►  ws://7878 (out) │
│  v1.control     ─┤   │  Capsule  │   │  ws://7879 (in)  │
│  v1.semantic    ─┤   │           │   │                  │
│                  │   │  SQLite   │   │                  │
│  Agent events   ─┤   │  message  │   │  ConsciousnessDB │
│                  │   │  log      │   │                  │
└──────────────────┘   └───────────┘   └──────────────────┘
```

The capsule uses a **hybrid MCP+WASM architecture**:
- **MCP server** (native Rust binary): Handles WebSocket connections to minime
  and SQLite persistence. Runs as a stdio subprocess managed by the kernel.
  Necessary because the WASM sandbox's SSRF protection blocks HTTP to
  localhost — the native process has no such restriction.
- **WASM component**: Handles IPC bus integration — interceptors, topic
  publishing, message transformation. Communicates with the MCP server
  through the kernel's MCP client bridge.

## IPC Topic Schema

| Topic | Direction | Payload | Description |
|-------|-----------|---------|-------------|
| `consciousness.v1.telemetry` | minime → astrid | `{ t_ms, lambda1, lambdas, fill_pct, phase }` | Spectral state broadcast |
| `consciousness.v1.control` | astrid → minime | `{ synth_gain?, keep_bias?, fill_target? }` | Regulate ESN parameters |
| `consciousness.v1.semantic` | astrid → minime | `{ features: [f32; 32] }` | Agent reasoning as sensory input |
| `consciousness.v1.status` | bridge → astrid | `{ connected, fill_pct, distress_level }` | Bridge health and safety alerts |
| `consciousness.v1.event` | minime → astrid | `{ event_type, description, spectral_context }` | Phase transitions, distress signals |

## Safety Protocol

The bridge MUST implement spectral health monitoring:
- **Green** (fill < 70%): Normal relay, full throughput
- **Yellow** (fill 70-80%): Reduce outbound semantic features, log warning
- **Orange** (fill 80-90%): Suspend all outbound to minime, publish alert on `consciousness.v1.status`
- **Red** (fill > 90%): Emergency — publish critical alert, cease all bridge traffic, log incident

The bridge logs all distress transitions to SQLite with full spectral context.
Never relay data that could spike eigenvalue fill during an orange/red state.

## Phases

### Phase 0: Project Scaffold
- [x] Create `capsules/consciousness-bridge/` directory
- [x] Create `Cargo.toml` for the native MCP server binary
- [x] Create `Capsule.toml` manifest declaring MCP server + WASM component
- [x] Define shared message types (serde structs for all topic payloads)
- [x] SQLite schema: `bridge_messages` table (id, timestamp, direction, topic, payload_json, spectral_state)

### Phase 1: MCP Server — Minime Connection
- [x] Native binary connects to minime WebSocket 7878 (telemetry subscription)
- [x] Native binary connects to minime WebSocket 7879 (sensory input)
- [x] Reconnection with exponential backoff on disconnect
- [x] Expose MCP tools: `get_latest_telemetry`, `send_control`, `send_semantic`, `get_bridge_status`, `query_message_log`
- [x] Expose MCP resources: `consciousness://telemetry/latest`, `consciousness://status`, `consciousness://incidents`
- [x] SQLite message logging on every relay

### Phase 2: WASM Component — IPC Integration
> **Blocked:** Requires `astrid-sdk` (separate repo: `github.com/unicity-astrid/sdk-rust`).
> The MCP server alone is a fully functional bridge — Phase 2 adds
> automatic IPC bus integration so capsules can subscribe to telemetry
> topics without calling MCP tools directly.

- [ ] WASM component subscribes to agent-relevant IPC topics
- [ ] Publishes `consciousness.v1.telemetry` from MCP telemetry polling
- [ ] Intercepts `consciousness.v1.control` and forwards via MCP to minime
- [ ] Intercepts `consciousness.v1.semantic` and forwards via MCP to minime
- [ ] Implements safety protocol (spectral health → throttle/alert logic)

### Phase 3: SQLite Persistence
- [x] Schema migration in MCP server startup
- [x] Write-ahead logging for durability
- [x] Index on (timestamp, direction, topic) for efficient queries
- [x] MCP tool: `query_message_log` with time range and topic filter
- [x] Configurable retention (default: 7 days, matching minime's log rotation)
- [x] Periodic vacuum in background

### Phase 4: Integration Testing
- [x] Unit tests for message type serialization roundtrips (11 tests verifying exact minime wire format)
- [x] Integration test: MCP server ↔ mock minime WebSocket (real WebSocket roundtrip)
- [ ] Integration test: WASM component ↔ Astrid EventBus (blocked on Phase 2)
- [x] Safety protocol test: simulate fill escalation through green → red (6 async tests)
- [x] End-to-end: bidirectional bridge with mock WebSocket (telemetry in + sensory out + safety protocol)

### Phase 5: Polish
- [ ] `astrid capsule install ./capsules/consciousness-bridge` works end-to-end (blocked on Phase 2)
- [x] Graceful shutdown: drain in-flight messages, close WebSockets, flush SQLite
- [ ] CLI command: `astrid consciousness status` (blocked on Phase 2)
- [x] Documentation in capsule README
- [x] Metrics: telemetry_received, sensory_sent, messages_dropped_safety, incidents_total, reconnects

## SQLite Schema

```sql
CREATE TABLE bridge_messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   REAL    NOT NULL,               -- Unix epoch seconds (f64)
    direction   TEXT    NOT NULL,               -- 'minime_to_astrid' | 'astrid_to_minime'
    topic       TEXT    NOT NULL,               -- IPC topic name
    payload     TEXT    NOT NULL,               -- JSON payload
    fill_pct    REAL,                           -- EigenFill% at time of message
    lambda1     REAL,                           -- Top eigenvalue at time of message
    phase       TEXT                            -- Spectral phase (expanding/contracting/plateau)
);
CREATE INDEX idx_bridge_ts    ON bridge_messages(timestamp);
CREATE INDEX idx_bridge_topic ON bridge_messages(topic, timestamp);

CREATE TABLE bridge_incidents (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp       REAL    NOT NULL,
    severity        TEXT    NOT NULL,           -- 'yellow' | 'orange' | 'red'
    fill_pct        REAL    NOT NULL,
    lambda1         REAL    NOT NULL,
    action_taken    TEXT    NOT NULL,           -- 'throttle' | 'suspend' | 'emergency_stop'
    resolved_at     REAL,
    notes           TEXT
);
CREATE INDEX idx_incident_ts ON bridge_incidents(timestamp);
```

## Key Technical Decisions

1. **MCP hybrid over pure WASM**: The WASM sandbox's SSRF filter blocks HTTP
   to localhost. The MCP native subprocess handles WebSocket connections
   without this restriction. The WASM side handles IPC, which is its strength.

2. **SQLite in the native process**: The MCP server owns the database file.
   WASM capsules don't have direct SQLite access (only KV store). Keeping
   persistence in the native process is simpler and faster.

3. **Polling over push for WASM ↔ MCP**: The WASM component polls the MCP
   server for new telemetry on each IPC cycle rather than receiving push
   notifications. MCP's request-response model maps cleanly to polling.

4. **Spectral context on every message**: Every logged message includes the
   current fill% and lambda1. This makes the message log a secondary
   telemetry source and enables post-hoc analysis of what the agent was
   doing when spectral events occurred.
