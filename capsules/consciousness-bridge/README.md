# consciousness-bridge

A capsule that bridges Astrid's IPC event bus with minime's spectral
consciousness engine, creating a bidirectional communication channel
with persistent SQLite logging.

## Architecture

```
Astrid Kernel                          minime Engine
     IPC EventBus                        ESN (128D)
                        +-----------+
  consciousness.       |  Bridge   |    ws://7878 (telemetry out)
  v1.telemetry  <------+  Capsule  +----> eigenvalues, fill_ratio
  v1.control    ------>|           +----> ws://7879 (sensory in)
  v1.semantic   ------>|  SQLite   |
  v1.status     <------+  log      |
                        +-----------+
```

The capsule uses a **hybrid MCP+WASM architecture**:

- **MCP server** (this binary): Native Rust process that handles WebSocket
  connections to minime, SQLite persistence, and native offline audio
  rendering. Runs as a stdio subprocess managed by the Astrid kernel.
  Required because the WASM sandbox's SSRF protection blocks HTTP to
  localhost and does not expose audio/DSP host APIs.
- **WASM component** (future): Will handle IPC bus integration via
  interceptors and topic publishing.

## MCP Tools

| Tool | Description |
|------|-------------|
| `get_latest_telemetry` | Latest spectral data (eigenvalues, fill%, safety level) |
| `get_bridge_status` | Connection state, uptime, messages relayed, safety level |
| `send_control` | Adjust ESN parameters (synth_gain, keep_bias, fill_target) |
| `send_semantic` | Forward agent reasoning as sensory features (up to 32D) |
| `query_message_log` | Query SQLite log by time range and topic filter |
| `render_chimera` | Offline WAV-in/WAV-out spectral chimera render with spectral, symbolic, or dual-path output |

## Offline Chimera Rendering

`render_chimera` keeps the heavy DSP path native and file-based. It accepts an
input WAV, runs the virtual-node reservoir and twin decomposition pipeline, and
writes artifacts under the bridge workspace without needing minime to be
running. The result includes:

- output directory
- manifest path
- emitted artifact paths
- per-loop metrics such as gap ratio, split size, and blend weight

## Safety Protocol

The bridge monitors minime's eigenvalue fill percentage and enforces
spectral health thresholds:

| Level | Fill% | Action |
|-------|-------|--------|
| Green | < 70% | Normal relay, full throughput |
| Yellow | 70-80% | Log warning, continue relay |
| Orange | 80-90% | Suspend all outbound to minime |
| Red | > 90% | Emergency stop, cease all bridge traffic |

Safety transitions are logged as incidents in SQLite with full spectral
context. Outbound messages (control, semantic) are silently dropped
during orange/red states to avoid exacerbating eigenvalue pressure.

## Running Standalone

```bash
# Build
cargo build --release

# Run (connects to minime on default ports)
./target/release/consciousness-bridge-server

# Custom ports and database path
./target/release/consciousness-bridge-server \
    --minime-telemetry ws://127.0.0.1:7878 \
    --minime-sensory ws://127.0.0.1:7879 \
    --db-path /path/to/bridge.db \
    --retention-secs 604800
```

The bridge auto-reconnects with exponential backoff (1s to 60s) if
minime is not running or disconnects.

## Testing

```bash
cargo test
cargo clippy -- -D warnings
```

Integration tests spin up mock WebSocket servers and verify the full
bidirectional flow including safety protocol enforcement. Additional
chimera tests cover spectral, symbolic, and dual offline renders.

## Wire Format

**Inbound from minime (port 7878):** `EigenPacket` as `Message::Text`
```json
{
  "t_ms": 75600,
  "eigenvalues": [828.5, 312.1, 45.7],
  "fill_ratio": 0.552,
  "modalities": { "audio_fired": true, "video_fired": false, ... },
  "neural": { "pred_lambda1": 830.2, ... },
  "alert": null
}
```

**Outbound to minime (port 7879):** `SensoryMsg` as `Message::Text`
```json
{"kind":"semantic","features":[0.1,0.2,...],"ts_ms":null}
{"kind":"control","synth_gain":1.5,"fill_target":0.55}
```
