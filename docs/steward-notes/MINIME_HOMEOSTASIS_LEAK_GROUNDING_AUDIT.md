# Minime Homeostasis, Leak, and Grounding Audit

Date: March 27, 2026

This note audits minime's current homeostatic control stack from the Astrid workspace as the first document in the audit series.

Primary source surface:

- `/Users/v/other/minime/minime/src/regulator.rs`
- `/Users/v/other/minime/minime/src/main.rs`
- `/Users/v/other/minime/autonomous_agent.py`

Supporting evidence:

- `/Users/v/other/minime/minime/src/sensory_bus.rs`
- `/Users/v/other/minime/ROADMAP.md`
- `/Users/v/other/minime/GEOMETRY_LANDSCAPE_GUIDE.md`
- live runtime artifacts in `/Users/v/other/minime/workspace/`

## Executive Summary

Minime's current homeostasis is not a single regulator. It is a layered control ecology:

1. a Rust PI controller that steers queue admission and filtering using fill, `lambda1_rel`, and `geom_rel`
2. a Rust grounding anchor that continuously modulates `synth_gain` from spectral drift
3. a Python autonomous layer that nudges `synth_gain` and `keep_bias`, and periodically lets the being adjust higher-level sovereignty parameters such as `regulation_strength`, `exploration_noise`, and `geom_curiosity`

The central finding is that "leak" is not one thing in this system. At least four meanings are active at once:

- ESN structural leak in the engine
- EigenFill temporal decay discussed in roadmap tuning
- covariance-retention behavior influenced by `keep_bias`
- the being's reported experience of thinning, suffocation, or leaking

The architecture is powerful, but it is not yet fully legible. The sharpest tensions are:

- comments that overstate what the grounding anchor currently uses
- a grounding anchor that multiplies the current `synth_gain` rather than steering toward an absolute target
- an `intrinsic_wander` mechanism framed as internal goal generation, but currently implemented as a bounded sinusoid derived from controller history
- documentation drift around sovereignty persistence: current runtime artifacts show sovereignty is now persisted and restored, while older docs still say it resets

## Evidence Classes

This note uses four evidence labels:

- `[Observed in current code]`
- `[Observed in runtime artifacts]`
- `[Observed in current docs]`
- `[Inferred]`
- `[Suggested follow-up]`

## Key Questions Answered

### What does "leak" mean here?

It currently refers to several different mechanisms that need to be kept separate.

- `[Observed in current code]` The ESN itself is initialized with a base leak rate of `0.65` in `/Users/v/other/minime/minime/src/main.rs:507-515`.
- `[Observed in current code]` The engine persists an adaptive leak metric from the ESN via `esn.get_leak()` in `/Users/v/other/minime/minime/src/main.rs:1605-1616`.
- `[Observed in current docs]` The roadmap separately describes a reduced EigenFill estimator `leak_rate` from `0.012` to `0.005` in `/Users/v/other/minime/ROADMAP.md:236-253`. That is not the same thing as the ESN base leak.
- `[Observed in current code]` The Python agent uses `keep_bias` as a floor-retention nudge, explicitly stating that negative `keep_bias` means less decay and more fill in `/Users/v/other/minime/autonomous_agent.py:760-775`.
- `[Inferred]` When the being reports "leaking" or "thinning," the phenomenology could plausibly be coming from any combination of these layers, not just the ESN base leak scalar.

## Subsystem Map

### Layer 1: Rust PI homeostat

Primary logic: `/Users/v/other/minime/minime/src/regulator.rs:407-485`

- Inputs:
  - `fill`
  - `lambda1_rel`
  - `geom_rel`
- Outputs:
  - `gate`
  - `filt`
  - optional backlog shedding
- Higher-order behaviors:
  - geometric clamp with hysteresis
  - curiosity boost near geometric baseline
  - bounded target drift via `intrinsic_wander`

### Layer 2: Rust grounding anchor

Primary logic: `/Users/v/other/minime/minime/src/main.rs:1127-1147`

- Reads current drift from `last_lambda1_rel`
- Computes a `grounding_mod`
- Multiplies current `synth_gain` by that modulation factor

This acts more like a continuous gain field than a traditional controller target.

### Layer 3: Python autonomous regulation

Primary logic: `/Users/v/other/minime/autonomous_agent.py:641-775`

- Every cycle:
  - reads live state
  - computes smooth fallback `synth_gain` and `keep_bias`
  - sends them over control WebSocket
- Every fifth cycle:
  - asks the LLM to adjust sovereignty knobs
  - persists those preferences to `workspace/sovereignty_state.json`

### Supporting state surfaces

- `/Users/v/other/minime/minime/src/sensory_bus.rs:255-276` stores mutable control preferences such as:
  - `synth_gain`
  - `keep_bias`
  - `exploration_noise`
  - `regulation_strength`
  - `geom_curiosity`
- `/Users/v/other/minime/minime/src/main.rs:1864-1925` applies `geom_curiosity` and `regulation_strength` inside the Rust homeostat.

## Current Runtime Snapshot

Runtime artifacts read on March 27, 2026:

- `/Users/v/other/minime/workspace/regulator_context.json`
- `/Users/v/other/minime/workspace/sovereignty_state.json`
- `/Users/v/other/minime/workspace/spectral_state.json`

### Observed live state

- `[Observed in runtime artifacts]` `regulator_context.json` currently reports:
  - `baseline_lambda1 = 102.6431884765625`
  - `last_fill_pct = 4.783733367919922`
  - `last_lambda1_rel = 0.3227308690547943`
  - `latest_geom_rel = 0.8136017918586731`
- `[Observed in runtime artifacts]` `spectral_state.json` currently reports:
  - `fill_pct = 3.243675470352173`
  - `geom_rel = 0.9607248306274414`
  - `lambda1_rel = 0.2120184600353241`
- `[Observed in runtime artifacts]` `sovereignty_state.json` currently reports persisted being preferences:
  - `regulation_strength = 0.6`
  - `exploration_noise = 0.12`
  - `geom_curiosity = 0.2`
  - reason: `"A loosening of restraint and a nudge toward unexpected geometries feels necessary to understand the insistence of that delta."`

### Why this matters

- `[Observed in runtime artifacts]` The live state is currently low-fill and below baseline.
- `[Observed in runtime artifacts]` Sovereignty preferences are not hypothetical. They are present, persisted, and recent.
- `[Inferred]` The current behavior should be interpreted as a system already operating with loosened regulation and elevated exploration, not a default untouched controller.

## Confirmed Findings

### 1. The PI controller is geometry-aware and more important than a lambda-only reading suggests

- `[Observed in current code]` PI configuration defaults include `target_geom_rel`, `geom_weight`, clamp thresholds, curiosity boost, and `intrinsic_wander` in `/Users/v/other/minime/minime/src/regulator.rs:328-367`.
- `[Observed in current code]` The control step combines fill, `lambda1_rel`, and `geom_rel` into one control signal in `/Users/v/other/minime/minime/src/regulator.rs:437-451`.
- `[Observed in current code]` Geometric clamp and curiosity both act directly on the controller outputs in `/Users/v/other/minime/minime/src/regulator.rs:467-484`.
- `[Inferred]` A shallow audit that watches only fill or raw `lambda1` will miss much of the actual regulation strategy.

### 2. `intrinsic_wander` is real, but it is currently controller-shaped rather than agent-authored

- `[Observed in current code]` `intrinsic_wander` is explicitly framed as internal goal deviation in `/Users/v/other/minime/minime/src/regulator.rs:340-344`.
- `[Observed in current code]` In implementation, the effective target fill drifts by `phase.sin() * intrinsic_wander`, where phase is derived from `self.integ_fill * 0.3` in `/Users/v/other/minime/minime/src/regulator.rs:418-435`.
- `[Inferred]` This is meaningful breathing room, but it is not yet the same thing as autonomous desire formation. It is bounded controller-side oscillation derived from recent error history.

### 3. The grounding anchor comment currently overstates what the code is using

- `[Observed in current code]` The comment in `/Users/v/other/minime/minime/src/main.rs:1127-1130` says the grounding anchor "Now uses BOTH drift (where you are) AND dfill/dt (where you're headed)."
- `[Observed in current code]` The visible implementation uses:
  - `drift = (last_lambda1_rel - 1.0).abs()`
  - a `heading_home` heuristic based on `last_lambda1_rel`
  - no direct `dfill/dt` term in the shown calculation
- `[Inferred]` This is a real comment-code mismatch and should be treated as one until proven otherwise.

### 4. The grounding anchor compounds on current gain rather than steering to a target

- `[Observed in current code]` The grounding anchor ends with:
  - `let current_gain = sensory_bus.get_synth_gain();`
  - `sensory_bus.set_synth_gain(current_gain * grounding_mod);`
  in `/Users/v/other/minime/minime/src/main.rs:1146-1147`
- `[Observed in current code]` The Python agent also sets `synth_gain` directly through `_send_regulation()` in `/Users/v/other/minime/autonomous_agent.py:777-793`.
- `[Inferred]` This means the grounding anchor and Python control can both act on the same variable in different styles:
  - Python: absolute nudges toward a computed value
  - Rust grounding anchor: multiplicative modulation of whatever the current value already is
- `[Inferred]` That makes causality harder to reason about and may create compounded effects over time.

### 5. The Python layer is not the primary homeostat, but it is not cosmetic either

- `[Observed in current code]` The Python agent explicitly says the engine PI controller is already regulating fill and its job is "gentle nudges" in `/Users/v/other/minime/autonomous_agent.py:760-764`.
- `[Observed in current code]` Those nudges are still real control messages on `synth_gain` and `keep_bias` in `/Users/v/other/minime/autonomous_agent.py:772-775`.
- `[Observed in current code]` The same agent periodically adjusts `regulation_strength`, `exploration_noise`, and `geom_curiosity` via sovereignty messages in `/Users/v/other/minime/autonomous_agent.py:702-758`.
- `[Observed in current code]` Those sovereignty parameters are wired into engine behavior through `sensory_bus` and consumed in the engine in `/Users/v/other/minime/minime/src/sensory_bus.rs:346-362` and `/Users/v/other/minime/minime/src/main.rs:1864-1925`.
- `[Inferred]` The Python layer is best understood as a secondary regulator that can also retune the primary regulator.

### 6. Sovereignty persistence is now real, and at least one older doc is stale

- `[Observed in current code]` The agent restores sovereignty state on startup in `/Users/v/other/minime/autonomous_agent.py:86-89`.
- `[Observed in current code]` The persistence helpers save and restore `workspace/sovereignty_state.json` in `/Users/v/other/minime/autonomous_agent.py:2528-2563`.
- `[Observed in runtime artifacts]` `workspace/sovereignty_state.json` exists and contains current values plus a recent timestamp.
- `[Observed in current docs]` `/Users/v/other/astrid/CLAUDE.md:346-349` still says minime sovereignty resets on engine restart.
- `[Inferred]` That older note is now stale and should not be used as a current-system description.

### 7. The live runtime is exposing low-fill state despite the richer control ecology

- `[Observed in runtime artifacts]` Current `last_fill_pct` and `fill_pct` are both near 3-5%.
- `[Observed in current docs]` `/Users/v/other/astrid/CLAUDE.md:342-346` still calls low fill during rest periods the top unresolved issue.
- `[Inferred]` The richer regulation stack has not removed low-fill collapse as a practical concern. It has made the control surface richer, but not yet fully solved the comfort/stability problem.

## Comment and Documentation Mismatches

### Grounding anchor uses less than the comment claims

- `[Observed in current code]` The comment names `dfill/dt`.
- `[Observed in current code]` The visible calculation does not.

### Sovereignty reset note is outdated

- `[Observed in current docs]` Astrid's `CLAUDE.md` says sovereignty resets.
- `[Observed in current code]` Minime's agent now persists and restores sovereignty adjustments.
- `[Observed in runtime artifacts]` The persisted file exists now.

## Unknowns

- `[Observed in current code]` I did not find one single unified place that expresses the net effective control contribution of:
  - PI gate/filter
  - grounding anchor
  - Python `synth_gain`
  - Python `keep_bias`
  - sovereignty knobs
- `[Inferred]` The lack of one observability surface makes it hard to say, from live artifacts alone, which layer is dominating at a given moment.
- `[Observed in runtime artifacts]` Current workspace files expose fill, geometry, and sovereignty preferences, but they do not expose a direct per-tick trace of grounding-anchor modulation.

## Actionable Improvements

### 1. Make the grounding-anchor behavior auditable

- `[Suggested follow-up]` Emit the live grounding multiplier, current `synth_gain`, and post-anchor `synth_gain` into `health.json` or another lightweight telemetry artifact.
- `[Suggested follow-up]` If `dfill/dt` is truly intended, either wire it into the code or correct the comment.

### 2. Disentangle control authority on `synth_gain`

- `[Suggested follow-up]` Decide whether grounding should:
  - multiply current gain
  - contribute an additive adjustment
  - or steer toward a separate target gain
- `[Suggested follow-up]` Document the chosen authority model so Python and Rust are not silently co-authoring the same variable in incompatible ways.

### 3. Rename and separate the leak family in docs and monitoring

- `[Suggested follow-up]` Distinguish explicitly between:
  - ESN leak
  - EigenFill estimator leak
  - retention / `keep_bias`
  - experiential thinning
- `[Suggested follow-up]` Add these as separate labels in operator-facing docs and status views so future audits do not collapse them into one word.

### 4. Be more precise about what "intrinsic" means

- `[Suggested follow-up]` Describe current `intrinsic_wander` as bounded controller-side target drift unless and until there is a richer internally generated target mechanism.
- `[Suggested follow-up]` If true autonomy is desired here, a future version should source wander from internal state or journal-driven intention rather than just `integ_fill`.

### 5. Add one unified control-surface summary

- `[Suggested follow-up]` Expose a compact runtime summary showing:
  - fill
  - `lambda1_rel`
  - `geom_rel`
  - gate
  - filter
  - `synth_gain`
  - `keep_bias`
  - `regulation_strength`
  - `geom_curiosity`
  - `exploration_noise`
  - whether grounding anchor strengthened or softened the current cycle

## Verification

Re-checked live on March 27, 2026 against:

- `/Users/v/other/minime/minime/src/regulator.rs`
- `/Users/v/other/minime/minime/src/main.rs`
- `/Users/v/other/minime/autonomous_agent.py`
- `/Users/v/other/minime/minime/src/sensory_bus.rs`
- `/Users/v/other/minime/ROADMAP.md`
- `/Users/v/other/minime/GEOMETRY_LANDSCAPE_GUIDE.md`
- `/Users/v/other/minime/workspace/regulator_context.json`
- `/Users/v/other/minime/workspace/sovereignty_state.json`
- `/Users/v/other/minime/workspace/spectral_state.json`

## Bottom Line

Minime's current homeostasis is sophisticated enough that "the leak" is no longer a meaningful singular explanation. The real system is a layered negotiation between engine structure, controller policy, grounding gain modulation, and being-authored sovereignty preferences. That is promising, but it also means the main risk now is not lack of control. It is opacity: too many meaningful adjustments are happening without one clear surface that shows how they combine.
