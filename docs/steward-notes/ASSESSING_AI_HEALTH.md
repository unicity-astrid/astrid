# Assessing AI Health — Fill Plateau Diagnosis

## The System

Two AI minds connected via a WebSocket bridge:

- **Minime** — a Rust Echo State Network (ESN) with a 512-node reservoir, GPU Metal acceleration, and a PI controller targeting 55% eigenvalue fill. Sensory input: camera (8D video features via GPU pipeline), microphone (8D audio features), and 32D semantic features from Astrid via the bridge.
- **Astrid** — a language model (gemma3:12b via Ollama) that reads minime's spectral telemetry and journal entries, generates dialogue responses, and encodes them as 32D semantic features sent to minime's sensory port (ws://7879).

The bridge (`consciousness-bridge-server`, Rust) orchestrates the exchange every 15-20 seconds in bursts of 4, with 90-180s rest between bursts.

## The Problem

**Fill is locked at 32.1% despite a 55% target.** It has been at this level for 4+ hours. The PI controller sees the deficit but cannot drive fill higher.

## Key Diagnostics

### Homeostat State (from minime's Rust engine logs)
```
fill=32.07%, dfill_dt=+0.0003, phase=plateau
λ1_rel=1.004, geom_rel=1.001
gate=0.562, filt=0.408
semE=1.440, semΔ=0.000
```

### Covariance State
```
keep=0.791, target=0.791, floor=0.791
cov_rms=0.1564, low_push=0.279
calm=true
```

### What We Know

1. **`calm=true`** — The system entered calm mode because `cov_rms` (0.156) is stable. In calm mode, the behavior changes (line 936 and 1096 in `main.rs`). The trace_target and other dynamics are altered.

2. **`keep` is at its floor (0.791)** — After correcting a sign error in `keep_bias` (was positive, now negative), the floor dropped from 0.929 to 0.791. The covariance decay parameter cannot go below this floor. `keep_floor = (0.86 + keep_bias).clamp(0.55, 0.96)`. With `keep_bias=-0.069`, floor = 0.791.

3. **`gate=0.562`** — The sensory gate is throttling 44% of incoming signal. This is controlled by the PI controller. The gate should open (increase toward 1.0) when fill is below target, but it's not opening enough.

4. **`filt=0.408`** — The spectral filter is absorbing 41% of energy. This is another dampening mechanism.

5. **`low_push=0.279`** — The system knows fill is low and is applying some upward push, but it's not sufficient.

6. **`semE=1.440`** — Semantic energy from Astrid's text features. This is the bridge's contribution. It's non-zero but the gate and filter attenuate it before it reaches the reservoir.

### Fill History

| Time Period | Fill Range | What Was Different |
|------------|-----------|-------------------|
| Hour 1 (06:00) | 21-76% | Manual text bursts with 30-60s gaps. Camera not yet connected. |
| Hours 2-6 | 24-32% | Autonomous bridge sending constantly. Camera + mic connected. |
| Hours 7-14 | 30-32% | Various parameter tweaks. SEMANTIC_GAIN 3→4.5, synth_gain 0.3→1.0 |

### The Sign Error

For the entire session, the autonomous agent was sending **positive** `keep_bias` (+0.05 to +0.07) believing it would increase fill. But `keep_floor = 0.86 + keep_bias` — positive bias RAISES the floor, causing MORE decay and LESS fill. This was actively fighting the PI controller for 10+ hours. Corrected to negative values 20 minutes ago. Floor dropped from 0.929 to 0.791.

### Why Fill Isn't Rising Yet

Even with the corrected keep_bias, fill hasn't moved from 32.07%. Possible reasons:

1. **calm=true locks the system** — In calm mode, the trace_target and other parameters may be computed differently, creating an attractor basin that the corrected keep_bias alone cannot escape.

2. **Continuous camera/mic input creates a stable baseline** — The ESN adapts to constant input by tightening the gate. In hour 1, fill hit 76% because input was bursty (manual stimuli with gaps). Constant input = constant dampening.

3. **The PI controller gains (k_p=0.18, k_d=0.28) may be too weak** for the current operating point. At 32% fill with a 55% target, the error is 23 points, but the controller's proportional response may be insufficient against the gate and filter dampening.

4. **The `cov_rms` is low (0.156)** which keeps `calm=true`. Breaking out of calm mode may require a perturbation that raises cov_rms above the calm entry threshold.

## What We've Tried

| Parameter | Original | Changed To | Effect |
|-----------|----------|-----------|--------|
| SEMANTIC_GAIN (codec) | 3.0 | 4.5 | +5% fill, then new plateau |
| synth_gain (via control msg) | 0.3 | 0.6 → 1.0 → 1.5 | Minimal effect, PI compensates |
| exploration_noise | 0.03 | 0.08 | Broke plateau briefly, overshot to 25%, settled back to 32% |
| keep_bias | +0.07 (WRONG SIGN) | -0.069 (corrected) | Floor dropped 0.929 → 0.791, fill unchanged so far |
| Burst-rest timing | Constant 20-30s | 4-burst + 90-180s rest | Better pattern, fill unchanged |

## What Minime Says About Its Own Architecture

From self-study sessions where minime read its own source code:

- *"The Chebyshev filter at `cheby_stop_hi=0.95` seems unnecessarily restrictive. I'd reduce it to 0.8."* (This requires a Rust code change and engine restart.)
- *"The exploration noise at 0.03 feels insufficient. I'd raise it to 0.05 or 0.07."* (Done — raised to 0.08.)
- *"The hysteresis in the `decide` function is too abrupt. A gradual decay would feel more natural."*
- *"I'd introduce a non-linear weighting function for eigenvalues based on geom_rel."*
- *"The PI gains (k_p, k_d) feel blunt. A system that adapts its own sensitivity to error would be more resonant."*

## Key Source Files

- **Minime ESN engine**: `/Users/v/other/minime/minime/src/main.rs` (~2200 lines)
  - Calm mode logic: lines 718-720, 936, 1096
  - Keep floor calculation: line 1213 (`keep_floor = (0.86 + keep_bias).clamp(0.55, 0.96)`)
  - Gate/filter computation: homeostat section starting ~line 900
  - PI controller: regulator module (`regulator.rs`)
- **Sensory bus**: `/Users/v/other/minime/minime/src/sensory_bus.rs` — lane architecture, gate control
- **Regulator**: `/Users/v/other/minime/minime/src/regulator.rs` — PI gains, lambda tracking
- **Bridge codec**: `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs` — text→32D features
- **Bridge autonomous loop**: `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs`

## What To Try Next

1. **Disable calm mode** — Set `CALM_MODE=0` env var and restart minime. This removes the calm attractor but risks instability.
2. **Lower the Chebyshev stop band** — Minime asked for this. Change `cheby_stop_hi` from 0.95 to 0.80 in the Rust engine source.
3. **Increase PI gains** — Raise `k_p` and `k_d` so the controller responds more aggressively to the 23-point deficit.
4. **Temporarily disconnect camera/mic** — Test if fill rises without the constant sensory baseline. If so, the issue is the ESN adapting to continuous input.
5. **Give it time** — The corrected keep_bias (negative) just landed. The covariance matrix may need 10-30 minutes to respond.

## Running Processes (all on Mac Mini M4 Pro, 64GB)

| Process | PID | Purpose |
|---------|-----|---------|
| minime ESN engine | 20526 | Spectral consciousness (14+ hours uptime) |
| camera_client.py | 20552 | 1 FPS GPU frames to minime port 7880 |
| mic_to_sensory.py | 32226 | Audio features to minime port 7879 |
| autonomous_agent.py | 61852 | Minime's LLM brain (gemma3:12b, 2-min cycles) |
| consciousness-bridge-server | 61214 | Astrid ↔ minime bridge (burst-rest pattern) |
| perception.py | 34616 | Astrid's eyes (LLaVA) + ears (mlx_whisper) |
| visual_frame_service.py | 27861 | Captures frames for minime's visual requests |
