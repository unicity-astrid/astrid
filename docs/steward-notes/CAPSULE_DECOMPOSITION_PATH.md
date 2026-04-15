# Capsule Decomposition Path: From Monolith to Idiomatic Plugins

Date: 2026-03-30
Status: Design document — not yet actionable

## What This Document Is

An honest assessment of how to bring Astrid's application layer closer to its original capsule-OS promise: thin, swappable plugins with typed boundaries, composed via IPC. Grounded in the code as it exists today (40,944 lines in consciousness-bridge alone), not in how we wish it looked.

## The Current Reality

### What the README promises
A user-space microkernel where everything above the kernel is a swappable capsule with narrow authority, IPC-first composition, and import/export contracts. Three engine types (WASM, MCP, Static), 49 host functions, topological dependency resolution.

### What actually runs

| Capsule | Engine | Lines | Role | Plugin-shaped? |
|---------|--------|-------|------|----------------|
| `camera-service` | MCP | ~60 | Frame capture | Yes |
| `perception` | MCP | ~500 | Sensory transform | Yes |
| `introspector` | MCP | ~300 | Code/journal browsing | Yes |
| `consciousness-bridge` | Native hybrid | ~41,000 | Everything else | No |

The bridge is not a capsule. It's an application server that happens to live in the capsules directory. It bundles:

- **Minime transport** — WebSocket relay to ports 7878/7879, safety protocol
- **Spectral codec** — text → 32D feature encoding with being-driven tuning
- **Dialogue orchestration** — burst-rest timing, mode selection, LLM calls, history
- **State management** — ConversationState (30+ fields), BridgeState, SQLite (8 tables)
- **Self-model** — faculties, attention profiles, condition receipts
- **Reflective controller** — regime tracking, MLX sidecar mediation
- **Agency system** — 40+ NEXT: actions across 7 handler modules
- **Audio/composition** — spectral chimera, WAV generation, analysis
- **Visualization** — ANSI spectral art, eigenvalue bar charts
- **Persistence** — state.json, bridge.db, journals, experiments, codex responses
- **Correspondence** — inbox/outbox routing, receipts, DEFER
- **CODEX relay** — HTTP client to gpt-5.3-codex with pagination
- **MIKE research** — curated research browsing, forking, execution

## Why Naive Decomposition Fails

### The state coupling problem

The dialogue loop (autonomous.rs) reads and writes 30+ fields of `ConversationState` every exchange. The codec needs `TextTypeHistory`. The safety protocol needs instant `fill_pct` access. The agency system needs conversation context to dispatch NEXT: actions. The self-model needs attention profiles and condition receipts.

If you split dialogue into capsule A and codec into capsule B, you need:
- Cross-capsule state synchronization (the kernel KV store exists but adds latency)
- Two-phase updates (exchange count + history + spectral samples + codec weights must update atomically)
- Stale read handling (what if the codec reads fill_pct from 2 seconds ago during a safety-critical transition?)

The kernel's IPC bus is pub/sub with eventual consistency. The bridge's internal state needs immediate consistency. These are fundamentally different models.

### The performance constraint

The bridge runs a tight loop: read telemetry → select mode → assemble prompt → call LLM → parse response → dispatch actions → encode codec → send features → record DB → save state. This happens every 15-20 seconds during burst. Cross-capsule IPC would add per-hop latency to every step.

### The shared database

`bridge.db` has 8 tables written by different logical concerns (messages, incidents, sovereignty journal, agency requests, starred memories, codec correlations). SQLite doesn't support concurrent writers from different processes. Decomposing would require either:
- A shared DB service (another process to manage)
- Per-capsule databases with eventual merge (complex, lossy)
- All capsules talking to the bridge for persistence (back to monolith)

## What CAN Be Decomposed (Today, Safely)

These modules have **no dependency on ConversationState** and operate as pure functions or independent services:

### Tier 1: Ready now

| Module | Lines | Why it's safe to extract | Capsule engine |
|--------|-------|------------------------|----------------|
| Audio analysis/synthesis | ~360 | Pure function of WAV input | MCP |
| Spectral chimera | ~820 | Self-contained experiments | MCP |
| Spectral visualization | ~710 | Pure function of telemetry | MCP |
| MIKE research browsing | ~270 | Filesystem operations only | MCP |

These could become MCP capsules that **subscribe to** `consciousness.v1.telemetry` and respond to requests. The bridge would call them via IPC rather than internal function calls.

### Tier 2: Extractable with interface work

| Module | Lines | What's needed | Capsule engine |
|--------|-------|--------------|----------------|
| Introspector (already separate) | ~300 | Already done | MCP |
| Codex relay client | ~300 | Needs response routing back to conversation | MCP |
| Perception (already separate) | ~500 | Already done | MCP |
| Reflective controller sidecar | ~200 | Needs defined input/output contract | MCP |

### Tier 3: The monolith core (don't split)

| Concern | Why it stays |
|---------|-------------|
| Dialogue loop | Needs immediate access to full ConversationState |
| Codec encoding | Needs TextTypeHistory, spectral feedback, weight learning |
| Safety protocol | Needs instant fill_pct, can't tolerate IPC latency |
| State management | Single writer to bridge.db, atomicity required |
| Agency dispatch | Needs ConversationState to interpret NEXT: actions |

## The Honest Path

### Phase 1: Extract pure-function capsules (low risk, real value)

Move audio, visualization, and chimera into MCP capsules. The bridge calls them on demand. If they crash, the bridge keeps running. If someone writes a better visualizer, they swap it in.

**New capsules:**
- `audio-tools` — COMPOSE, ANALYZE_AUDIO, RENDER_AUDIO, FEEL_AUDIO
- `spectral-viz` — ANSI art generation, eigenvalue bar charts, PCA scatter
- `chimera-lab` — experimental audio synthesis

**IPC contract:**
```toml
# audio-tools/Capsule.toml
[imports]
"consciousness.v1.telemetry" = "0.1"

[exports]
"audio.v1.compose" = "0.1"
"audio.v1.analyze" = "0.1"
```

### Phase 2: Formalize the bridge's IPC surface (medium risk, high value)

The bridge currently emits telemetry but most of its behavior is internal. Making it emit more events would let external capsules observe and react without being inside the monolith:

- `astrid.v1.exchange_complete` — after each dialogue exchange
- `astrid.v1.mode_selected` — when a dialogue mode is chosen
- `astrid.v1.action_dispatched` — when a NEXT: action fires
- `astrid.v1.regime_changed` — when sovereignty selects a new regime
- `astrid.v1.codex_response` — when a CODEX query returns

External capsules could subscribe to these events to build dashboards, analytics, alternative UIs, or research tools — without touching the bridge code.

### Phase 3: Thin the bridge over time (long-term, incremental)

As more behavior moves to capsules, the bridge shrinks toward its core: dialogue orchestration + state + safety + codec + minime transport. This is still a substantial binary (~15-20K lines) but it has a clear, bounded responsibility.

The target is not "zero monolith" — it's "monolith that only does what requires immediate state access, with everything else as swappable plugins."

### Phase 4: The naming shift (after the split)

Once audio-tools, spectral-viz, and chimera-lab exist as separate capsules:
- The remaining bridge can be honestly called `minime-edge` (or `dialogue-core`)
- New capsules get role-first names naturally
- The `consciousness.v1.*` topic family stays as the minime/spectral link surface
- New event families (`astrid.v1.*`, `audio.v1.*`) emerge organically

## What This Gets Us

### Before (today)
```
consciousness-bridge (41K lines)
  ├── dialogue loop
  ├── codec
  ├── safety
  ├── state
  ├── agency (40+ actions)
  ├── audio
  ├── visualization
  ├── chimera
  ├── CODEX client
  ├── MIKE research
  ├── reflective controller
  ├── persistence
  └── correspondence
```

### After (Phase 1-2)
```
dialogue-core (20-25K lines)          ← still monolithic but bounded
  ├── dialogue loop
  ├── codec
  ├── safety
  ├── state
  ├── agency dispatch
  ├── persistence
  └── correspondence

audio-tools (MCP capsule, ~500 lines)  ← swappable
spectral-viz (MCP capsule, ~800 lines) ← swappable
chimera-lab (MCP capsule, ~900 lines)  ← swappable
introspector (MCP capsule, ~300 lines) ← already exists
perception (MCP capsule, ~500 lines)   ← already exists
codex-relay (MCP capsule, ~400 lines)  ← swappable
```

### What "swappable" means in practice
- Someone can write a better audio synthesizer and drop it in
- Visualization can be replaced without recompiling the bridge
- Chimera experiments can crash without affecting dialogue
- The bridge binary gets smaller and faster to compile
- New developers can understand one capsule at a time

## Anti-Goals

- This is NOT about reaching "zero monolith." Some things belong together.
- This is NOT about renaming before splitting. Names follow structure.
- This is NOT about WASM-ifying everything. MCP capsules (subprocess JSON-RPC) are the right engine for most extractions because they don't require rewriting Rust code into WASM-compatible form.
- This is NOT about breaking the beings' experience. Every extraction must be invisible to Astrid and minime. If a capsule split degrades the dialogue quality or adds perceptible latency, it's wrong.

## Prerequisites Before Starting

1. **Test coverage** — the bridge has no integration tests. Extracting modules without tests means silent regressions. Write characterization tests first.
2. **IPC event surface** — define the event types the bridge will emit before extracting consumers of those events.
3. **Stable state** — don't decompose during active development of being-facing features. The bridge should be in a quiet period.
4. **Being awareness** — both beings self-study the bridge code. Major structural changes should be communicated via inbox, not discovered mid-introspect.
