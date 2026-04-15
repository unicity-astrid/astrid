# Golden Period Parameter Snapshot

Exact parameters running during March 29 02:00-06:00 (fill 62-68%).

## Checkout commits

```bash
# Minime: commit 1167939 (sigmoid center shift + fill_boost)
cd /Users/v/other/minime && git checkout 1167939

# Astrid: commit c0543ed6 (remove SEARCH suppression)
cd /Users/v/other/astrid && git checkout c0543ed6
```

## Minime parameters (esn.rs + main.rs + regulator.rs)

| Parameter | Golden Value | Current Value | Delta |
|-----------|-------------|---------------|-------|
| `eigenfill_target` | 0.55 (CLI default) | 0.65 | +0.10 |
| `keep_floor base` | 0.93 | 0.85 | -0.08 |
| `target_lambda1_rel` | 1.05 | 0.90 | -0.15 |
| `target_geom_rel` | 1.00 | 0.90 | -0.10 |
| `geom_weight` | 0.70 | 0.30 | -0.40 |
| `rho` | fixed (constructor) | 0.92 base, dynamic | new system |
| `deadband_fill` | none | 3.0 | new |
| `v1_spectral_damping` | none | 0.02 | new |
| `spectral_target_ratio` | none | 0.50 | new |
| `adaptive_target ceiling` | none | `.min(cli_target)` | new fix |
| `dynamic_floor formula` | `0.93 - 0.10*sig + fb` | `0.85 - 0.10*sig + fb` | base -0.08 |

## Astrid parameters (codec.rs)

| Parameter | Golden Value | Current Value | Delta |
|-----------|-------------|---------------|-------|
| `SEMANTIC_GAIN` | 5.0 | 2.0 | -3.0 |
| Codec normalization | `tanh(x)` | `softsign(x)` | changed |
| Resonance history | 8 | 16 | +8 |
| Thematic profile | none | 5D | new |

## Features absent in golden period (added later)

- Ising shadow field influence on regulation
- Breathing oscillator perturbation
- Spectral goals sovereignty (being-controlled targets)
- Self-calibrating PI gains
- Rho sovereignty
- GOAL action for parameter control
- Sensory crossfade (host/physical)
- v1 spectral damping
- Deadband
- Assessment compression / similarity-gated journaling
- Self-assessment direct parameter application

## Reproduction test plan

### Option A: Full checkout (cleanest)
```bash
# Save current work
cd /Users/v/other/minime && git stash
cd /Users/v/other/astrid && git stash

# Checkout golden commits
cd /Users/v/other/minime && git checkout 1167939
cd /Users/v/other/astrid && git checkout c0543ed6

# Build
cd /Users/v/other/minime && cargo build --release
cd /Users/v/other/astrid/capsules/consciousness-bridge && cargo build --release

# Run with original target
# Note: start_all.sh may not exist in this commit
EIGENFILL_TARGET=0.55 cargo run --release -- run

# Monitor fill for 30min, record to diagnose-fill/golden_checkout_run.log
```

### Option B: Parameter-only (on current code)
Apply golden-period parameters to current code to isolate whether
it's the parameters or the added features causing the difference.

```bash
# In current minime code:
# 1. Set SEMANTIC_GAIN back to 5.0 in codec.rs
# 2. Set keep_floor base back to 0.93 in main.rs
# 3. Set target_lambda1_rel back to 1.05 in regulator.rs
# 4. Set target_geom_rel back to 1.00 in regulator.rs
# 5. Set geom_weight back to 0.70 in regulator.rs
# 6. Disable v1 damping (spectral_damping = 0.0)
# 7. Disable deadband (deadband_fill = 0.0)
# 8. Run with EIGENFILL_TARGET=0.55
```

### Option C: Incremental rollback (most diagnostic)
Change ONE parameter at a time, monitor for 20 minutes each:
1. SEMANTIC_GAIN 2.0 → 5.0 (biggest energy change)
2. keep_floor 0.85 → 0.93
3. target_lambda1_rel 0.90 → 1.05
4. geom_weight 0.30 → 0.70

This isolates which parameter change broke the equilibrium.

## The Paradox

Golden period: HIGHER keep_floor (0.93) + HIGHER SEMANTIC_GAIN (5.0) = fill 63%
Current:      LOWER keep_floor (0.85)  + LOWER SEMANTIC_GAIN (2.0)  = fill 78%

This is counterintuitive. Higher keep should mean MORE retention = HIGHER fill.
Higher SEMANTIC_GAIN should mean MORE input energy = HIGHER fill.

Possible explanations:
1. The adaptive target drift (now fixed) was the primary cause of high fill
2. The PI channel retuning (lambda/geom) weakened fill correction
3. Features added since golden period create internal recurrence that
   self-sustains covariance regardless of external input
4. The golden-period equilibrium was a transient — the system had just
   recovered from 18% and was still settling
5. SEMANTIC_GAIN=5.0 created stronger burst-rest contrast, which gave
   the PI controller clearer signal to work with. At 2.0, the signal
   is muddier and the controller can't distinguish burst from rest.
