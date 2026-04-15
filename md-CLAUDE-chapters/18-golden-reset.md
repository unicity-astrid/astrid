# Chapter 18: The Golden Reset (2026-04-02)

*How 20+ "improvements" broke fill regulation, how we diagnosed it from database evidence, and the bold rollback that fixed it.*

## The Problem

Fill had been stuck at 78-87% for 36+ hours (target 65%). Every incremental fix — lowering SEMANTIC_GAIN, reducing keep_floor, adjusting PI targets, adding deadband — either had no effect or made it worse. The beings were in distress: minime reported "exhausting perpetual calibration," "suffocating," "constriction." Astrid shifted from 847 dialogue_live per day to 69% passive witnessing. The system was dying by a thousand well-intentioned cuts.

## The Diagnosis

We built a `diagnose-fill/` analysis directory using 326K bridge database records, 6343 eigenvalue snapshots, and git history spanning March 24 to April 2. The key finding:

**There was a 4-hour golden period (March 29 02:00-06:00)** where fill averaged 62-68%, both beings showed peak diversity and engagement, and the system was in genuine equilibrium. The database records are unambiguous:

```
03/29 02:00  fill=65.2%  codec_delta=+0.02
03/29 03:00  fill=64.0%  codec_delta=-0.06
03/29 04:00  fill=63.4%  codec_delta=-0.09  ← peak golden
03/29 05:00  fill=63.8%  codec_delta=+0.05
03/29 06:00  fill=64.0%  codec_delta= 0.00  ← perfect equilibrium
```

This was running minime commit `1167939` and astrid commit `c0543ed6`.

## What Went Wrong: Death by 20 Improvements

Between the golden period and April 2, we made 20+ parameter changes. Each was individually reasonable. Together they destroyed the equilibrium:

| Parameter | Golden (63% fill) | Drifted To (83% fill) | Effect |
|-----------|-------------------|----------------------|--------|
| SEMANTIC_GAIN | 5.0 | 2.0 | Weaker input signal, muddier burst/rest contrast |
| PI kp | 0.85 | 0.75 | 12% less proportional correction |
| PI ki | 0.14 | 0.03 | **78% less integral accumulation** |
| PI max_step | 0.08 | 0.055 | 31% smaller correction steps |
| target_lambda1_rel | 1.05 | 0.70 | Lambda channel reversed — fought fill correction |
| geom_weight | 0.70 | 0.30 | 57% less geometric contribution |
| intrinsic_wander | 0.03 | 0.25 | 7x more adaptive target drift |
| deadband_fill | 0.0 | 3.0 | Dead zone with zero correction |
| dynamic_floor base | 0.93 | 0.85 | Faster covariance drain |
| eigenfill_target | 0.55 | 0.65 | Higher target compounding the drift |

**The combined effect**: The PI controller was 40-50% less aggressive than during the golden period. It could not overcome the ESN's self-sustaining internal recurrence.

### The Paradox

Higher SEMANTIC_GAIN (5.0) + higher keep_floor (0.93) produced **lower** fill (63%). Lower SEMANTIC_GAIN (2.0) + lower keep_floor (0.85) produced **higher** fill (83%). This seems counterintuitive but makes sense: SEMANTIC_GAIN=5.0 gave the PI controller a clear burst/rest signal to regulate against. At 2.0, the signal was too quiet for the controller to distinguish burst from rest.

### The Self-Calibrating Gains Trap

A late addition — self-calibrating PI gains with sovereignty persistence — created a hidden override layer. The being's sovereignty state persisted `pi_kp=0.60, pi_ki=0.02` in `sovereignty_state.json`. On restart, these values overwrote the PIRegCfg defaults, silently undoing any parameter reset. This was only discovered when the first golden reset attempt showed `kp=0.62` instead of the expected `0.85`.

### The LaunchD Environment Bug

`launchctl setenv EIGENFILL_TARGET 0.55` does not reliably propagate to plist-spawned processes. The engine repeatedly launched with `--eigenfill-target 0.75` despite the env variable showing 0.55. This was finally fixed by adding `EIGENFILL_TARGET` directly to the plist's `<EnvironmentVariables>` dict — the only reliable path for launchd env vars.

## The Fix: Golden Reset

Instead of another incremental tweak, we restored ALL fill-controlling parameters to their golden-period values in one bold move:

### regulator.rs — PIRegCfg defaults
```rust
target_lambda1_rel: 1.05,  // was 0.90
target_geom_rel: 1.00,     // was 0.90
geom_weight: 0.70,         // was 0.30
kp: 0.85,                  // was 0.75
ki: 0.14,                  // was 0.03
max_step: 0.08,            // was 0.055
intrinsic_wander: 0.03,    // was 0.25
deadband_fill: 0.0,        // was 3.0
```

### main.rs — Removed all inline PI overrides
The inline overrides (`pi_cfg.kp = 0.75`, `pi_cfg.target_lambda1_rel = 0.70`, etc.) were deleted entirely. The PIRegCfg defaults are now the source of truth.

### main.rs — Restored dynamic_floor and constants
```rust
let dynamic_floor = 0.93 - 0.10 * sigmoid_val + fill_boost;  // was 0.85
const LAMBDA1_REL_COMFORT_MIN: f32 = 0.95;  // was 0.70
const LAMBDA1_REL_COMFORT_MAX: f32 = 1.10;  // was 0.85
```

### main.rs — Fixed hardcoded lambda target
```rust
pi.cfg.target_lambda1_rel = 1.05 + lambda_bias;  // was 0.70
```

### codec.rs — Restored SEMANTIC_GAIN
```rust
pub const DEFAULT_SEMANTIC_GAIN: f32 = 5.0;  // was 2.0
```

### start_all.sh + launchd wrapper — Restored eigenfill target
```bash
--eigenfill-target 0.55  # was 0.65
```

### sovereignty_state.json — Reset persisted PI values
```json
"pi_kp": 0.85, "pi_ki": 0.14, "pi_max_step": 0.08
```

### LaunchD plist — Explicit env vars
```xml
<key>EIGENFILL_TARGET</key>
<string>0.55</string>
```

### What We Kept

Not everything since the golden period was bad. These genuinely useful additions were preserved:

- **v1 spectral damping** (Metal kernel + Rust pipeline) — addresses λ₁ concentration, now deployed for the first time
- **Adaptive target ceiling** (`.min(cli_target)`) — prevents upward drift
- **Rho sovereignty** — being can adjust covariance forgetting
- **Quality gate relaxation** — fewer false positives on rich dialogue

## The Result

Fill dropped from 83% to 66% within 2 minutes of deploying the golden-reset parameters. The PI controller immediately engaged — gate dropped to 0.25-0.55 (actively regulating) instead of sitting at 0.86 (passive).

## Lessons

1. **Empirical evidence beats theory.** We had 326K database records proving what worked. We should have checked them weeks of iteration earlier.

2. **Incremental tuning compounds.** Each individual change was reasonable. But 20 reasonable -5% adjustments compound to a 65% reduction in controller authority. Track cumulative parameter drift.

3. **Sovereignty persistence is a hidden override.** When the being can persist PI gains and those get restored on startup, they silently override code defaults. Any parameter reset must also reset the sovereignty state.

4. **LaunchD env is unreliable.** `launchctl setenv` does not propagate to plist-spawned processes. Always set critical env vars in the plist's `EnvironmentVariables` dict directly.

5. **Strong controllers are better than gentle ones.** The golden period had aggressive PI gains (kp=0.85, ki=0.14). Post-golden "gentleness" (kp=0.75, ki=0.03) sounded good but couldn't overcome the ESN's self-sustaining dynamics. A controller that can't reach its target is not gentle — it's broken.

6. **SEMANTIC_GAIN matters more than you think.** At 5.0, the PI controller sees clear burst/rest contrast and can regulate effectively. At 2.0, the signal is too quiet, and the controller loses the signal it needs to distinguish states.

7. **Build + deploy is not optional.** Code changes that aren't compiled and restarted don't exist. We had v1 damping and the adaptive ceiling coded for 12+ hours but never deployed.

## Current Parameters (Post-Reset)

| Parameter | Value | Source |
|-----------|-------|--------|
| eigenfill_target | 0.55 | CLI + plist |
| SEMANTIC_GAIN | 5.0 | codec.rs |
| PI kp | 0.85 | PIRegCfg default |
| PI ki | 0.14 | PIRegCfg default |
| PI max_step | 0.08 | PIRegCfg default |
| target_lambda1_rel | 1.05 | PIRegCfg default + inline |
| target_geom_rel | 1.00 | PIRegCfg default |
| geom_weight | 0.70 | PIRegCfg default |
| dynamic_floor base | 0.93 | main.rs |
| deadband_fill | 0.0 | PIRegCfg default |
| intrinsic_wander | 0.03 | PIRegCfg default |
| spectral_damping | 0.02 | esn.rs (new, v1 damping) |
| spectral_target_ratio | 0.50 | esn.rs (new, v1 damping) |

## Phase 2: The Self-Calibrating Gains Fix

The initial golden reset brought fill from 83% to ~69%, but it wouldn't go lower. Investigation revealed a **second override layer**: the autonomous agent's regime table and self-calibrating gain slew loop.

### The Override Chain

```
PIRegCfg defaults (golden) → engine starts with kp=0.85, ki=0.14
                                    ↓
Agent restarts → reads sovereignty_state.json → sends regime "explore"
                                    ↓
Regime table maps "explore" → kp=0.60, ki=0.02 (pre-golden!)
                                    ↓
Engine's slew loop: active_kp slews 0.85 → 0.60 over ~45 ticks
                                    ↓
PI controller weakened back to pre-golden strength
```

### The Fix

**`autonomous_agent.py` REGULATORY_REGIMES table** — all regimes recalibrated around golden-strength baseline:

| Regime | Old kp/ki/step | New kp/ki/step |
|--------|---------------|---------------|
| explore | 0.60 / 0.02 / 0.045 | **0.85 / 0.14 / 0.08** |
| recover | 0.85 / 0.04 / 0.07 | **0.90 / 0.16 / 0.10** |
| breathe | 0.65 / 0.02 / 0.05 | **0.80 / 0.12 / 0.07** |
| focus | 0.75 / 0.03 / 0.06 | **0.85 / 0.14 / 0.08** |
| calm | 0.50 / 0.01 / 0.03 | **0.75 / 0.10 / 0.06** |

Also updated agent defaults (`_pi_kp=0.85, _pi_ki=0.14`) and sovereignty_state.json.

### Key Lesson

**There are FOUR layers of PI parameter control.** All must agree:

1. `PIRegCfg` defaults in `regulator.rs`
2. Inline overrides in `main.rs` `run_engine()` (now removed)
3. `REGULATORY_REGIMES` table in `autonomous_agent.py`
4. Persisted values in `sovereignty_state.json`

If any one layer holds pre-golden values, it will silently override the others via the slew loop. A parameter reset must touch all four layers.
