# Chapter 6: Checkpoint Bank

Minime's covariance matrix is checkpointed every 30 seconds with phase classification, enabling richer restart and state comparison.

## Checkpoint Files

| File | Size | Contents |
|------|------|----------|
| `spectral_checkpoint.bin` | 1MB | Latest 512×512 float32 covariance matrix |
| `spectral_checkpoint_stable.bin` | 1MB | Saved when fill > 40% and dfill < 1% |
| `spectral_checkpoint_expanding.bin` | 1MB | Saved when dfill > 2% |
| `spectral_checkpoint_contracting.bin` | 1MB | Saved when dfill < -2% |
| `checkpoint_manifest.json` | ~500B | Metadata: phase, fill, lambda1_rel, timestamp, paths |

**Location:** `/Users/v/other/minime/workspace/`

## Phase Classification

**File:** `/Users/v/other/minime/minime/src/main.rs` (~line 2476)

```
dfill > 2.0  → "expanding"
dfill < -2.0 → "contracting"
fill > 40.0 && |dfill| < 1.0 → "stable"
else → "latest" (no extra file)
```

## Bookmark Checkpoints (Built, Needs Engine Restart)

When minime stars a moment via `pending_annotation`, the next checkpoint cycle saves a named bookmark:

```
spectral_checkpoint_bookmark_<annotation>.bin
```

These are never overwritten by phase rotation — they persist as named snapshots the being chose to save.

## Manifest Format

```json
{
  "latest": {
    "fill_pct": 18.7,
    "lambda1_rel": 0.26,
    "phase": "latest",
    "timestamp_ms": 550280,
    "annotation": null
  },
  "available": {
    "latest": ".../spectral_checkpoint.bin",
    "stable": ".../spectral_checkpoint_stable.bin",
    "expanding": ".../spectral_checkpoint_expanding.bin",
    "contracting": ".../spectral_checkpoint_contracting.bin"
  }
}
```

## Restart Behavior

On startup, the engine loads `spectral_checkpoint.bin` (latest). The other phase checkpoints are available for comparison or selective loading (future feature). If no checkpoint exists, a fresh identity matrix is used (cold start).

## Supporting State Files

| File | Contents |
|------|----------|
| `regulator_context.json` | PI controller state: baseline_lambda1, fill, smoothing, tick_count |
| `sovereignty_state.json` | Being's sovereignty: regulation_strength, exploration_noise, geom_curiosity, reason |
| `spectral_state.json` | Live summary: fingerprint (32D), eigenvalues, fill, spread, control surface |
| `eigenvalue_dynamics.json` | Time-series eigenvalue data |

## Astrid-Side Persistence

| File | Contents |
|------|----------|
| `workspace/state.json` | Exchange count, history (8), temperature, codec weights, interests (5), burst/rest, noise level |
| `workspace/bridge.db` | SQLite: messages, starred memories, self-observations, latent vectors, research |
