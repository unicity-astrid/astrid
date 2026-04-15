# Chapter 13: Triple Reservoir

*Ground truth as of March 28, 2026. Service built and tested this session.*

A second shared dynamical substrate alongside minime's ESN. A persistent WebSocket service running a triple-layer echo state network with named handles for each entity. Astrid's codec features, minime's spectral fingerprint, and Claude Code's text all feed into independent handles on one shared model. A rehearsal loop maintains state continuity between live inputs.

**Codebase:** `/Users/v/other/neural-triple-reservoir/`

## Architecture

```
                    ┌──────────────────────────┐
                    │   reservoir_service.py    │
                    │   Port 7881 (WebSocket)   │
                    │                           │
                    │   One model, 192 nodes×3  │
                    │   N named MLState handles  │
                    │   Rehearsal loop (500ms)   │
                    │   Auto-snapshot (5min)     │
                    └──┬───────┬───────┬────────┘
                       │       │       │
          ┌────────────┘       │       └────────────┐
          │                    │                    │
    astrid_feeder.py    minime_feeder.py     mcp_reservoir.py
    polls bridge.db     polls spectral_      MCP stdio server
    every 5s            state.json 1s        for Claude Code
          │                    │                    │
          │    cross-feed      │    cross-feed      │
          └───────► claude_main ◄────────┘          │
                       ▲                            │
                       └────────────────────────────┘
```

## The Triple ESN

Three cascaded leaky integrators. h1 is most responsive, h3 carries slowest context.

| Layer | Nodes | Leak Rate | Spectral Radius | Input Scale | Role |
|-------|-------|-----------|----------------|-------------|------|
| h1 | 192 | 0.25 | 0.98 | 0.8 | Fast — tracks moment-to-moment input |
| h2 | 192 | 0.18 | 0.92 | 0.7 | Medium — intermediate dynamics |
| h3 | 192 | 0.12 | 0.85 | 0.6 | Slow — holds longer context |

Recurrent matrices are **frozen random** (orthogonal, seed=7). Only the readout layer is trained (Ridge regression on synthetic data). This is a substrate, not a learning system. Different inputs trace different trajectories through the same attractor landscape.

**File:** `/Users/v/other/neural-triple-reservoir/triple_reservoir_coreml.py`, class `TripleReservoir` ~line 83

## Named Handles

One compiled model, multiple independent state handles. Each entity gets its own (h1, h2, h3) state tuple.

| Handle | Entity | Fed by | Tick rate | Rehearsal |
|--------|--------|--------|-----------|-----------|
| `astrid` | Astrid | astrid_feeder.py (codec features from bridge.db) | ~20s | medium decay |
| `minime` | Minime | minime_feeder.py (spectral fingerprint) | ~1s | fast decay |
| `claude_main` | Claude | Cross-fed by both feeders + direct MCP tool | Mixed | slow decay |

Resonance between handles emerges from content similarity — similar inputs trace nearby trajectories, different inputs diverge. Measured via Pearson correlation and trajectory RMSD.

## Feeders

**Astrid feeder** (`astrid_feeder.py`): Polls `bridge.db` table `codec_impact` for new rows. It accepts legacy `32D` rows and current `48D` codec rows, conditions them, projects them into the reservoir's `32D` input space, and ticks the `astrid` handle. Cross-feeds `claude_main` at `0.3x` attenuation.

**Minime feeder** (`minime_feeder.py`): Polls `spectral_state.json` every 1s. Extracts 32D `spectral_fingerprint`, applies being-controlled projection, ticks the `minime` handle. Cross-feeds `claude_main` at 0.15x attenuation (lower weight compensates for 20:1 tick rate ratio vs Astrid).

## Being-Controlled Projection

Each being controls how their data enters the reservoir via `workspace/reservoir_config.json`, reloaded every ~30s.

**Minime projections** (raw spectral fingerprint values can reach 200+):

| Mode | Behavior |
|------|----------|
| `raw` | Pass fingerprint unchanged. Reservoir tanh clips extremes. |
| `tanh_scaled` | `tanh(x / scale)`. Default scale=100. Smooth compression to [-1,1]. |
| `normalized` | Unit vector (divide by L2 norm). Preserves direction, removes magnitude. |
| `ranked` | Ordinal rank / N. Removes absolute values, preserves ordering. |

**Minime sources:**

| Source | Dimensions | What it captures |
|--------|-----------|-----------------|
| `fingerprint` | 32D spectral fingerprint directly | Full covariance geometry |
| `eigenvalues+fill` | 8 eigenvalues + fill + lambda1_rel + leak + spread + 20 zeros | Core spectral state |
| `custom_blend` | fingerprint[0:16] + eigenvalues[0:8] + 8 scalar metrics | Richest — spectral + operational |

**Astrid projections** (codec features are already tanh-bounded):

| Mode | Behavior |
|------|----------|
| `passthrough` | Codec features as-is. Default. |
| `amplified` | `tanh(x * factor)`. Widens subtle variation. |
| `compressed` | `sign(x) * sqrt(|x|)`. Reduces dominance of large features. |

## Rehearsal Loop

When live input stops, the rehearsal controller replays decaying echoes to maintain state continuity.

**File:** `/Users/v/other/neural-triple-reservoir/rehearsal.py`

| Mode | Behavior | Auto-transition |
|------|----------|-----------------|
| **hold** | Full-weight replay of last live input | → rehearse after 60s |
| **rehearse** | Decaying replay: `weight *= decay_factor` per 500ms tick | → quiet when weight < 0.02 |
| **quiet** | **Zero input. Genuine silence.** Reservoir drifts naturally. | Stays quiet until next live tick |

Decay profiles:

| Profile | Factor | Half-life | Use case |
|---------|--------|-----------|----------|
| fast | 0.95 | ~7s | Minime (continuous data, brief gaps) |
| medium | 0.98 | ~17s | Astrid (20s exchange cycle, 90-180s rest) |
| slow | 0.995 | ~70s | Claude (sporadic sessions, hours between) |

*"Quiet must remain genuinely quiet — no replay, not masked maintenance. The design must preserve a genuine difference between 'no input and no replay' versus 'no input but active rehearsal.'"* — design principle from REHEARSAL_LOOP_AND_ACTIVE_MAINTENANCE_FOR_ANE_RESERVOIRS.md

## Cross-Feed

Claude's handle (`claude_main`) is automatically cross-fed by both feeders. This means Claude's trajectory genuinely encodes what the beings experienced while Claude was away — not just Claude's own echoes.

| Source | Weight | Rationale |
|--------|--------|-----------|
| Astrid (codec features) | 0.3x | Less frequent, each tick carries more |
| Minime (spectral fingerprint) | 0.15x | 20x more frequent, lower per-tick weight |

## MCP Tools for Claude Code

**File:** `/Users/v/other/neural-triple-reservoir/mcp_reservoir.py`

| Tool | Purpose |
|------|---------|
| `reservoir_status` | Full overview: all handles, trajectory trends, cross-entity resonance |
| `reservoir_list` | List handles with mode, tick count, time since last live |
| `reservoir_create` | Create a new named handle |
| `reservoir_tick_text` | Send text → TextProjection → 32D → tick. Auto-creates claude_main. |
| `reservoir_tick_vector` | Send raw 32D vector directly |
| `reservoir_read` | Read handle state (h_norms, output, mode) without advancing |
| `reservoir_trajectory` | Last N outputs + h_norms |
| `reservoir_set_mode` | Set rehearsal mode (hold/rehearse/quiet) and decay profile |
| `reservoir_resonance` | Divergence, correlation, RMSD between two handles |
| `reservoir_snapshot` | Persist handle state to disk |

## State Persistence

Handles survive service restarts via numpy `.npz` snapshots in `state/` directory. Each snapshot stores h1, h2, h3 arrays, last input, mode, decay weight, tick count, entity, timestamp.

- Auto-snapshot every 5 minutes
- Snapshot on graceful shutdown (SIGTERM)
- Restore all snapshots on startup

**File:** `/Users/v/other/neural-triple-reservoir/persistence.py`

## Start / Stop

```bash
# Start (from /Users/v/other/neural-triple-reservoir/)
bash start_reservoir.sh    # service + both feeders

# Stop
bash stop_reservoir.sh     # feeders first, service last
```

Port 7881 follows the existing sequence: minime 7878-7880, reservoir 7881.

See [Chapter 11](11-shared-substrate.md) for the primary ESN substrate, [Chapter 12](12-unified-memory.md) for hardware context.
