# Migrating Perception's RASCII Integration — Approach B

> Temporary instructions for the astrid agent. Delete this file once the migration is complete.

## Current State

`capsules/perception/perception.py` uses RASCII in `perceive_visual_ascii()` (line 223):

1. Python captures a camera frame via OpenCV (`capture_frame()` → JPEG bytes)
2. Writes JPEG to disk as `ascii_frame_{timestamp}.jpg`
3. Shells out to `/Users/v/other/RASCII/target/release/rascii` with args: `-w 20 -c -b -C block`
4. Captures stdout (ANSI-colored block art, ~4KB)
5. Writes JSON to `workspace/perceptions/visual_ascii_{timestamp}.json`

The consciousness-bridge reads these JSON files and uses `strip_ansi()` (autonomous.rs:23) before feeding to LLMs.

## What Changed in RASCII

RASCII now has three feature tiers:

| Feature config | What you get | Compiles to WASM? |
|---|---|---|
| `default-features = false` | Pure rendering: `render_image`, `render_image_to`, charsets | Yes |
| `features = ["terminal"]` | + `crossterm` (terminal size detection) | No |
| `features = ["camera"]` (default) | + `nokhwa` webcam capture, `CameraSource`, `LiveRenderer` | No |

The library API for rendering (no camera, no terminal):
```rust
use rascii_art::{RenderOptions, render_image_to};
use image::DynamicImage;

// Given a DynamicImage from any source:
let mut buf = String::new();
render_image_to(&image, &mut buf, &RenderOptions::new()
    .width(20)
    .colored(true)
    .background(true)
    .charset(&["░", "▒", "▓", "█"]))?;
```

The CLI for native camera capture: `rascii --camera -c -b -C block -w 20`

## Architecture: Approach B — Camera Service + Render Capsule

### Why this split

- **Camera capture requires native APIs** (AVFoundation on macOS) — can't run in WASM sandbox
- **ASCII rendering is pure computation** — `image` crate + charset mapping, compiles to `wasm32-wasip1`
- Clean separation: host handles hardware, capsule handles processing

### Components

```
┌─────────────────────────┐     filesystem      ┌──────────────────────────┐
│  camera-service (native)│  ───────────────►   │  perception capsule (WASM)│
│                         │  /tmp/frame.jpg     │                          │
│  CameraSource::frame()  │                     │  image::open(path)       │
│  saves JPEG to tmp      │  IPC notification   │  render_image_to(...)    │
│  publishes notification ├────────────────►    │  writes perception JSON  │
│  on perception.v1.frame │                     │  publishes on IPC        │
└─────────────────────────┘                     └──────────────────────────┘
```

**Why filesystem instead of IPC for image data**: Astrid IPC is JSON-only (no raw bytes). A 640x480 JPEG is ~50KB raw, ~67KB base64 — fits within the 5MB IPC limit, but writing to `/tmp` and sending a path notification is simpler, avoids base64 encode/decode overhead, and the file is available for other consumers (vision LLM, archival).

### 1. Camera Service — MCP capsule (native Rust binary)

A small Rust binary that uses `rascii_art::camera::CameraSource` to capture frames on a timer and notify via IPC.

**Location**: `capsules/camera-service/`

```
capsules/camera-service/
├── Cargo.toml
├── Capsule.toml
└── src/
    └── main.rs
```

**Cargo.toml**:
```toml
[package]
name = "camera-service"
version = "0.1.0"
edition = "2021"

[dependencies]
rascii_art = { path = "/Users/v/other/RASCII" }  # with camera feature (default)
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

**Capsule.toml**:
```toml
[package]
name = "camera-service"
version = "0.1.0"
description = "Native camera capture service — writes frames to tmp for WASM capsules"
astrid-version = ">=0.5.0"

[[mcp_server]]
id = "camera-service"
type = "stdio"
command = "camera-service"
args = []

[capabilities]
ipc_publish = ["perception.v1.frame"]

[[topic]]
name = "perception.v1.frame"
description = "Notification that a new camera frame is available at a tmp path"
```

**What it does**:
1. On startup: `CameraSource::new(&CameraConfig { index: 0, mirror: true })`
2. Warmup 30 frames
3. On MCP tool call (or timer): capture frame, save as JPEG to `/tmp/astrid_frame_{timestamp}.jpg`
4. Publish IPC notification: `{ "type": "frame_ready", "path": "/tmp/astrid_frame_....jpg", "timestamp": "..." }`

### 2. Perception Capsule — WASM (Rust)

Subscribes to frame notifications, renders ASCII art, writes perception JSON.

**Location**: `capsules/perception/` (replaces Python)

```
capsules/perception/
├── Cargo.toml
├── Capsule.toml
├── src/
│   └── lib.rs
├── perception.py         # legacy, remove when confident
└── workspace/
    └── perceptions/
```

**Cargo.toml**:
```toml
[package]
name = "perception-capsule"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
rascii_art = { path = "/Users/v/other/RASCII", default-features = false }
image = "0.25.5"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Key: `default-features = false` — no camera, no crossterm, compiles to `wasm32-wasip1`.

**Capsule.toml**:
```toml
[package]
name = "perception"
version = "0.1.0"
description = "Visual perception via RASCII ASCII art — spatial awareness without LLM calls"
astrid-version = ">=0.5.0"

[[component]]
id = "default"
file = "plugin.wasm"
type = "executable"

[capabilities]
fs_read = ["/tmp/astrid_frame_*"]
fs_write = ["cwd://workspace/perceptions/"]
ipc_subscribe = ["perception.v1.frame"]
ipc_publish = ["perception.v1.visual_ascii"]

[[topic]]
name = "perception.v1.visual_ascii"
description = "ASCII art spatial perception rendered from camera frame"

[[command]]
name = "perceive"
description = "Trigger an immediate visual ASCII perception"
```

**What it does** (pseudocode for `src/lib.rs`):
```rust
// Subscribe to frame notifications
let handle = astrid_ipc_subscribe("perception.v1.frame");

// On message (via #[astrid::run] event loop or #[astrid::interceptor]):
//   1. Read frame path from IPC message payload
//   2. Load image: image::open(path) -> DynamicImage
//   3. Render ASCII:
let mut ascii_art = String::new();
rascii_art::render_image_to(&img, &mut ascii_art, &RenderOptions::new()
    .width(20)
    .colored(true)
    .background(true)
    .charset(&["░", "▒", "▓", "█"]))?;
//   4. Build perception JSON (same schema as before)
//   5. Write via astrid_write_file("cwd://workspace/perceptions/visual_ascii_{ts}.json", json)
//   6. Publish IPC: perception.v1.visual_ascii
```

### Practical constraint: No guest SDK yet

Astrid's `astrid-sdk` (guest-side proc macros like `#[capsule]`, `#[astrid::tool]`, `#[astrid::run]`) is designed but not yet in the workspace. The existing capsules are all MCP-based native binaries.

**Two options for the perception capsule**:

**Option 1 (Pragmatic)**: Build both components as MCP native binaries for now. The perception capsule runs as a native Rust binary (not WASM), depends on `rascii_art` with `default-features = false`. When the SDK lands, migrate to WASM with minimal changes — the logic stays the same, only the host ABI binding layer changes.

**Option 2 (WASM now)**: Write raw `extern "C"` host function bindings manually (matching the 49-function ABI in `astrid-build/src/rust.rs` lines 212-274). This works but is tedious and will be replaced by the SDK.

**Recommendation**: Option 1. Build both as MCP native binaries. The camera-service already must be native. The perception capsule's rendering logic is identical either way — the only difference is how it calls `read_file` and `write_file` (native I/O vs host ABI). When the SDK ships, wrap the same logic in `#[capsule]` / `#[astrid::run]`.

## Output format (unchanged)

```json
{
  "type": "visual_ascii",
  "timestamp": "2026-03-26T...",
  "backend": "rascii",
  "ascii_art": "▓▓▒░░▒▓█...",
  "width": 20
}
```

The `frame_path` field is dropped — the frame JPEG is in `/tmp` managed by camera-service. Consciousness-bridge's `strip_ansi()` continues to work unchanged.

## Implementation Order

1. **RASCII**: feature gate is done — `default-features = false` compiles to wasm32-wasip1 ✅
2. **camera-service**: native Rust binary using `rascii_art::camera::CameraSource`, saves frames to `/tmp`, publishes IPC notification
3. **perception capsule**: native Rust binary (MCP) that watches for frame notifications, renders ASCII, writes perception JSON
4. **Test end-to-end**: camera-service produces frames → perception reads + renders → JSON appears in workspace
5. **Retire perception.py** once stable

## What NOT to change

- `consciousness-bridge/src/autonomous.rs` `strip_ansi()` — still needed
- The 120-second perception interval — reasonable default
- Width of 20 chars — tuned for ~4KB output in LLM context
- Perception JSON schema — consumers stay untouched
