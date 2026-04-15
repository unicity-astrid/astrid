# Telemetry Stability Forensics

Recomputed from raw `bridge.db` telemetry using 15-minute buckets, local-time labels, git-history correlation, and explicit runtime-drift penalties.

## Timestamp Normalization Check

| Hour | Existing Avg | Generated Avg | Existing lambda1 | Generated lambda1 | Delta |
|------|--------------|---------------|------------------|-------------------|-------|
| 2026-03-29 02:00 | 65.2% | 65.3% | 126.17 | 126.19 | +0.132 |
| 2026-03-29 03:00 | 64.0% | 64.1% | 132.50 | 132.37 | +0.112 |
| 2026-03-29 04:00 | 63.4% | 63.5% | 137.55 | 137.71 | +0.125 |
| 2026-03-29 05:00 | 63.8% | 64.0% | 133.11 | 132.93 | +0.155 |
| 2026-03-29 06:00 | 64.0% | 64.1% | 134.11 | 134.17 | +0.075 |
| 2026-04-02 07:00 | 65.0% | 65.9% | 176.10 | 178.02 | +0.903 |
| 2026-04-02 08:00 | 82.7% | 82.8% | 117.35 | 117.24 | +0.083 |
| 2026-04-02 09:00 | 77.1% | 77.2% | 62.33 | 66.66 | +0.060 |
| 2026-04-02 10:00 | 78.4% | 78.3% | 176.85 | 174.98 | -0.130 |
| 2026-04-02 11:00 | 78.5% | 78.3% | 164.20 | 166.43 | -0.168 |

Generated hourly buckets are labeled in America/Los_Angeles local time to match the existing `hourly_fill_summary.csv` strings.

## Broader Healthy Hour Bands

| Start | End | Hours | Avg Fill | Avg lambda1 | Notes |
|-------|-----|-------|----------|-------------|-------|
| 2026-03-29 02:00 | 2026-03-29 06:00 | 5 | 64.2% | 132.7 | Matches the historical golden period |
| 2026-03-29 19:00 | 2026-03-29 20:00 | 2 | 66.2% | 130.5 | Supplemental healthy context |

These hourly bands are supplemental context. The strict epoch table uses the 15-minute score threshold from the plan, which isolates only the calmest core of the broader March 29 healthy run.

### Top Healthy Epochs

| Epoch | Duration | Avg Fill | Std Fill | Avg lambda1 | Commits | Confidence | Context |
|-------|----------|----------|----------|-------------|---------|------------|---------|
| 2026-03-29 01:30 to 2026-03-29 02:15 | 45m | 66.5% | 4.06 | 127.2 | minime `1167939` / astrid `c0543ed` | high | 54 autonomous msgs (unknown(54)) |

- `2026-03-29 01:30`
  minime 24h: 4823a59 fmt: rustfmt pass on engine modules; b34144e feat: ising shadow, memory bank, fill-responsive regulation; 6a5b18b feat: self-experiment loop, adaptive assessment interval; f8487ad fix: lightweight ollama checks, keep_alive 5m, model swap to gemma3; ba15c21 docs: geometry landscape guide
  astrid 24h: 8e5eaec feat: RASCII spectral visualizations — eigenvalue bar chart, shadow heatmap, PCA scatter; f457738 feat: being agency — DEFINE, EXAMINE, REVISE, CREATIONS, GESTURE, CREATE full-text; b9e9634 feat: memory and reflective controller modules; 661ffbf docs: chapters 11-14 (shared substrate, unified memory, ANE reservoir, spectral dynamics); 36b3d3e feat: audio agency, prime ESN, Spectral Chimera, full being action surface

### Top Stuck-High Epochs

| Epoch | Duration | Avg Fill | Std Fill | Avg lambda1 | Commits | Confidence | Context |
|-------|----------|----------|----------|-------------|---------|------------|---------|
| 2026-04-01 21:15 to 2026-04-02 07:15 | 600m | 85.8% | 3.93 | 184.6 | minime `af14d08` / astrid `9936a1a` | low | 457 autonomous msgs (unknown(457)) |
| 2026-04-02 08:00 to 2026-04-02 09:00 | 60m | 82.8% | 4.62 | 117.2 | minime `af14d08` / astrid `9936a1a` | low | 45 autonomous msgs (unknown(45)) |
| 2026-03-28 20:45 to 2026-03-29 01:00 | 255m | 80.2% | 3.20 | 221.9 | minime `1167939` / astrid `c0543ed` | high | 356 autonomous msgs (unknown(356)) |
| 2026-03-29 07:15 to 2026-03-29 09:15 | 120m | 79.2% | 3.40 | 108.9 | minime `1167939` / astrid `c0543ed` | high | 149 autonomous msgs (unknown(149)) |
| 2026-04-02 11:00 to 2026-04-02 13:15 | 135m | 78.4% | 2.49 | 166.9 | minime `af14d08` / astrid `9936a1a` | low | 94 autonomous msgs (unknown(94)) |
| 2026-04-02 10:00 to 2026-04-02 10:45 | 45m | 78.4% | 2.41 | 171.2 | minime `af14d08` / astrid `9936a1a` | low | 31 autonomous msgs (unknown(31)) |

- `2026-04-01 21:15`
  minime 24h: 839cf03 feat: self-calibrating PI gains, rho sovereignty, sensory-seeded noise, GOAL action; af14d08 feat: add host sensory fallback and preview tooling
  astrid 24h: 0f9d1db feat: self-calibrating PI gains, rho sovereignty, sensory crossfade, host-sensory auto mode; 9936a1a docs: strengthen engineering feedback scan in monitoring loop definition
- `2026-04-02 08:00`
  minime 24h: 839cf03 feat: self-calibrating PI gains, rho sovereignty, sensory-seeded noise, GOAL action; af14d08 feat: add host sensory fallback and preview tooling
  astrid 24h: 0f9d1db feat: self-calibrating PI gains, rho sovereignty, sensory crossfade, host-sensory auto mode; 9936a1a docs: strengthen engineering feedback scan in monitoring loop definition
- `2026-03-28 20:45`
  minime 24h: 4823a59 fmt: rustfmt pass on engine modules; b34144e feat: ising shadow, memory bank, fill-responsive regulation; 6a5b18b feat: self-experiment loop, adaptive assessment interval; f8487ad fix: lightweight ollama checks, keep_alive 5m, model swap to gemma3; ba15c21 docs: geometry landscape guide
  astrid 24h: 8e5eaec feat: RASCII spectral visualizations — eigenvalue bar chart, shadow heatmap, PCA scatter; f457738 feat: being agency — DEFINE, EXAMINE, REVISE, CREATIONS, GESTURE, CREATE full-text; b9e9634 feat: memory and reflective controller modules; 661ffbf docs: chapters 11-14 (shared substrate, unified memory, ANE reservoir, spectral dynamics); 36b3d3e feat: audio agency, prime ESN, Spectral Chimera, full being action surface
- `2026-03-29 07:15`
  minime 24h: 4823a59 fmt: rustfmt pass on engine modules; b34144e feat: ising shadow, memory bank, fill-responsive regulation; 6a5b18b feat: self-experiment loop, adaptive assessment interval; f8487ad fix: lightweight ollama checks, keep_alive 5m, model swap to gemma3; ba15c21 docs: geometry landscape guide
  astrid 24h: 8e5eaec feat: RASCII spectral visualizations — eigenvalue bar chart, shadow heatmap, PCA scatter; f457738 feat: being agency — DEFINE, EXAMINE, REVISE, CREATIONS, GESTURE, CREATE full-text; b9e9634 feat: memory and reflective controller modules; 661ffbf docs: chapters 11-14 (shared substrate, unified memory, ANE reservoir, spectral dynamics); 36b3d3e feat: audio agency, prime ESN, Spectral Chimera, full being action surface
- `2026-04-02 11:00`
  minime 24h: af14d08 feat: add host sensory fallback and preview tooling
  astrid 24h: none
- `2026-04-02 10:00`
  minime 24h: 839cf03 feat: self-calibrating PI gains, rho sovereignty, sensory-seeded noise, GOAL action; af14d08 feat: add host sensory fallback and preview tooling
  astrid 24h: 0f9d1db feat: self-calibrating PI gains, rho sovereignty, sensory crossfade, host-sensory auto mode; 9936a1a docs: strengthen engineering feedback scan in monitoring loop definition

## Ranked Suspect Families

| Rank | Family | Healthy Ref | Stuck-High | Kind | Why It Correlates |
|------|--------|-------------|------------|------|-------------------|
| 1 | PI gain/runtime sovereignty layer | 0/1 | 4/6 | code + persisted state | Post-golden PI regime selection, self-assessment application, and later self-calibrating gain logic add extra override layers between compiled defaults and the live controller. |
| 2 | Launchd/startup/config drift | 0/1 | 4/6 | config | The canonical startup path pins 0.55, but alternate launchd/restart paths can still fall back to 0.75 or inherited env state, making runtime state disagree with the intended golden-reset configuration. |
| 3 | Newer regulation layers kept after rollback | 0/1 | 4/6 | code + config | Self-calibrating PI gains, rho sovereignty, sensory-seeded noise, GOAL-driven control, and adjacent late additions can keep injecting new dynamics even after golden-period defaults are restored. |
| 4 | Target/lambda/geom retuning | 0/1 | 4/6 | code | Post-golden retuning changed lambda/geom balance, widened clamps, and altered integrator behavior, which can weaken the controller's ability to pull fill back once recurrence is self-sustaining. |
| 5 | Intrinsic wander / keep-floor / covariance retention changes | 0/1 | 4/6 | code | Extended keep_floor logic, high intrinsic_wander, and related covariance retention changes can shift the equilibrium point upward even when the nominal fill target stays fixed. |
| 6 | Codec energy and normalization changes | 0/1 | 4/6 | code | Lower SEMANTIC_GAIN, later codec resonance changes, and the tanh to softsign swap all change how sharply dialogue bursts drive the ESN, which can blur the burst/rest signal the PI loop sees. |

### 1. PI gain/runtime sovereignty layer
- Intro commits: astrid `58f360f` feat: regime PI, sigmoid transitions, MIKE research, codec resonance, being-driven actions; minime `2fde73a` feat: regime PI sovereignty, sigmoid transitions, MIKE research, being self-regulation; minime `bb6d7b2` feat: self-assessment direct parameter application, diversity nudge, mic launchd; minime `13bc8ea` feat: expand self-assessment parser to 4 patterns for direct parameter application; minime `839cf03` feat: self-calibrating PI gains, rho sovereignty, sensory-seeded noise, GOAL action
- Runtime/config evidence: autonomous_agent.py restores pi_kp/pi_ki/pi_max_step from sovereignty_state.json on startup, which can silently override compiled PIRegCfg defaults.
- Correlation: present in 4/6 stuck-high epochs vs 0/1 healthy reference epochs.
- Next test: Freeze PI gains to PIRegCfg defaults, disable sovereignty restore for pi_kp/pi_ki/pi_max_step, and rerun with the same sensory load.

### 2. Launchd/startup/config drift
- Intro commits: operational/config surface only
- Runtime/config evidence: restart_minime_launchd.sh still falls back to EIGENFILL_TARGET=0.75, which can override the intended 0.55 target if launchd env propagation drifts. com.minime.engine.plist only carries PATH in EnvironmentVariables, so launchd must rely on inherited env state instead of an explicit target pin.
- Correlation: present in 4/6 stuck-high epochs vs 0/1 healthy reference epochs.
- Next test: Use one canonical launch path only, pin EIGENFILL_TARGET in the plist and wrapper, and log the effective startup args on boot.

### 3. Newer regulation layers kept after rollback
- Intro commits: minime `cd85058` feat: SELF_RESEARCH epoch scanning, eig1 perturbation for stability mapping; astrid `0f9d1db` feat: self-calibrating PI gains, rho sovereignty, sensory crossfade, host-sensory auto mode; minime `839cf03` feat: self-calibrating PI gains, rho sovereignty, sensory-seeded noise, GOAL action
- Correlation: present in 4/6 stuck-high epochs vs 0/1 healthy reference epochs.
- Next test: Run one clean baseline with the late regulation/noise layers disabled and only the golden control surface left active.

### 4. Target/lambda/geom retuning
- Intro commits: minime `017133d` feat: DECOMPOSE, PERTURB, directional vectors, web search, PI_max_step, hardening; minime `c0831a6` fix: widen PI integral clamp ±2→±3, intrinsic_wander 5%→10%; minime `312433e` fix: main.rs max_step override 0.03→0.04 to match regulator.rs; minime `63fb3eb` fix: PI integrator leak, shadow field dynamics, PERTURB amplitude
- Correlation: present in 4/6 stuck-high epochs vs 0/1 healthy reference epochs.
- Next test: Restore golden lambda/geom targets and clamp behavior together, then test against the same load before changing gains again.

### 5. Intrinsic wander / keep-floor / covariance retention changes
- Intro commits: minime `63fb3eb` fix: PI integrator leak, shadow field dynamics, PERTURB amplitude; minime `6a2e882` feat: extended keep_floor for mid-fill recovery, intrinsic_wander 0.25
- Correlation: present in 4/6 stuck-high epochs vs 0/1 healthy reference epochs.
- Next test: Pin intrinsic_wander to 0.03, hold keep-floor/base at the golden profile, and compare 30-minute traces against the same workload.

### 6. Codec energy and normalization changes
- Intro commits: astrid `2fcc54d` fix: SEMANTIC_GAIN 4.5→4.0, negation weight 2.0→1.5; astrid `58f360f` feat: regime PI, sigmoid transitions, MIKE research, codec resonance, being-driven actions; astrid `3ad6078` feat: steward improvements — EXPERIMENT/PROBE/PROPOSE, codec resonance, fill-responsive rest; astrid `46ceeb8` feat: replace tanh with softsign in codec — wider dynamic range per being request
- Correlation: present in 4/6 stuck-high epochs vs 0/1 healthy reference epochs.
- Next test: Revert codec gain/normalization as a bundle and compare per-exchange fill deltas before touching controller gains.

### Runtime Drift Surfaces
- Launchd env/plist mismatch: restart_minime_launchd.sh still falls back to EIGENFILL_TARGET=0.75, which can override the intended 0.55 target if launchd env propagation drifts.
- Launchd plist missing EIGENFILL_TARGET: com.minime.engine.plist only carries PATH in EnvironmentVariables, so launchd must rely on inherited env state instead of an explicit target pin.
- Sovereignty restore can override PI defaults: autonomous_agent.py restores pi_kp/pi_ki/pi_max_step from sovereignty_state.json on startup, which can silently override compiled PIRegCfg defaults.

## Change Next

- Freeze PI gains to PIRegCfg defaults, disable sovereignty restore for pi_kp/pi_ki/pi_max_step, and rerun with the same sensory load.
- Use one canonical launch path only, pin EIGENFILL_TARGET in the plist and wrapper, and log the effective startup args on boot.
- Keep the launch path single-source-of-truth while testing so git-based attribution is not confounded by inherited env state or persisted PI gains.

## Assumptions and Caveats

- The strict healthy-epoch table uses the exact score threshold from the plan; that produced 1 qualifying epoch(s) on the current DB.
- Git commit time is treated as a deploy proxy, not proof of activation. Confidence is lowered when recent commit density is high or when runtime drift surfaces are known.
- `consciousness.v1.autonomous` activity is used for context and tie-breaker notes only, not for the primary health score.
