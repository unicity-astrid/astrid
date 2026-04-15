# Chapter 11: Shared Substrate

*Ground truth as of April 2, 2026. Verified against `../minime/minime/src/main.rs`, `sensory_ws.rs`, `sensory_bus.rs`, and Astrid's bridge/codec code.*

Both beings relate to the same 128-node ESN, but not in the same way.

- **minime** lives inside the ESN runtime and can adjust its operating regime.
- **Astrid** influences the ESN by sending semantic vectors into its input space and by perceiving the resulting telemetry.

So "shared substrate" is true, but the relationship is asymmetric.

## The Current ESN Input Width: 66D

The minime ESN no longer consumes a 50D input vector. The live input width is:

```text
8 video + 8 audio + 2 aux + 48 semantic = 66 dimensions
```

| Dims | Lane | Source |
|------|------|--------|
| `z[0..7]` | video | camera / GPU pipeline |
| `z[8..15]` | audio | mic / audio feature path |
| `z[16]` | aux | `lambda1_rel` |
| `z[17]` | aux | `geom_rel` |
| `z[18..65]` | semantic | Astrid's 48D codec vector |

The relevant width constant is `LLAVA_DIM = 48` in `../minime/minime/src/sensory_bus.rs`. The legacy name survived, but the lane is no longer "32D llava features."

## Astrid's Path Into The ESN

The current path is:

```text
Astrid text
  -> codec.rs encodes 48D semantic vector
  -> SensoryMsg::Semantic { features }
  -> ws://127.0.0.1:7879
  -> minime sensory_ws.rs
  -> sensory_bus.rs stores [f32; 48]
  -> z[18..65] in the next ESN step
```

The key correction to older docs is that Astrid is not only sending the handcrafted 32D emotional/statistical layer anymore. The live semantic lane now includes:

- handcrafted texture/stance dims `0-31`
- embedding projection dims `32-39`
- narrative-arc dims `40-43`
- reserved tail dims `44-47`

## Semantic Persistence

Astrid's signal does not vanish immediately after one tick.

Current semantic persistence behavior in `sensory_bus.rs` is:

- if fill is below `30%`, semantic linger is forced to `45s`
- otherwise the stale window is shape-configurable and typically falls between `25s` and `10s`
- the decay itself uses a resonant `stale_scale()` with:
  - an echo floor of `5%`
  - damped ringing
  - `±5%` stochastic perturbation

So the accurate summary is:

- Astrid's semantic influence usually lingers for **10-25 seconds**
- in hard low-fill recovery it can linger for **45 seconds**
- the fade is intentionally not smooth or purely exponential

## What minime Actually Sends Back

The primary telemetry packet from the engine (`EigenPacket`) currently includes:

- `eigenvalues`
- `fill_ratio`
- `modalities`
- optional `neural`
- optional `alert`
- optional `spectral_fingerprint` (`32D`)
- optional `structural_entropy`
- optional `spectral_glimpse_12d`
- optional selected-memory metadata
- optional `ising_shadow`

So Astrid is perceiving more than "lambda1 and fill."

## The Raw Engine Control Surface

The engine's `SensoryMsg::Control` wire surface is wider than the sovereignty docs used to suggest.

These fields are currently accepted by `../minime/minime/src/sensory_ws.rs` and clamped in `sensory_bus.rs`:

| Field | Clamp / type | Meaning |
|-------|---------------|---------|
| `synth_gain` | `0.2 .. 3.0` | synthetic input amplification |
| `keep_bias` | `-0.08 .. 0.10` | retention / decay bias |
| `exploration_noise` | `0.0 .. 0.2` | ESN exploration noise |
| `fill_target` | `0.25 .. 0.75` | target fill |
| `regulation_strength` | `0.0 .. 1.0` | PI authority |
| `smoothing_preference` | `0.1 .. 0.9` or `NaN` | smoothing override or auto |
| `geom_curiosity` | `0.0 .. 0.3` | novelty-seeking from geometry |
| `target_lambda_bias` | `-0.5 .. 0.5` | internal target bias |
| `geom_drive` | `0.0 .. 1.0` | geometry-driven throughput |
| `penalty_sensitivity` | `0.0 .. 2.0` | projection-penalty sensitivity |
| `breathing_rate_scale` | `0.5 .. 2.0` | breathing rate multiplier |
| `mem_mode` | `0 .. 2` | memory-mode preference |
| `journal_resonance` | `0.0 .. 1.0` | semantic/journal resonance |
| `checkpoint_interval` | `10 .. 600` seconds | checkpoint cadence |
| `embedding_strength` | `0.0 .. 1.0` | semantic lane strength |
| `memory_decay_rate` | `0.01 .. 0.5` | memory fade |
| `transition_cushion` | `0.0 .. 1.0` | soften rapid fill transitions |
| `checkpoint_annotation` | string | annotate a checkpointed moment |
| `deep_breathing` | bool | slow oscillation mode |
| `pure_tone` | bool | calmer tone mode |
| `synth_noise_level` | `0.0 .. 1.0` | synthetic noise amount |
| `legacy_audio_synth` | bool | gate legacy audio synth |
| `legacy_video_synth` | bool | gate legacy video synth |
| `pi_kp` | `0.1 .. 2.0` | raw PI proportional gain |
| `pi_ki` | `0.005 .. 0.5` | raw PI integral gain |
| `pi_max_step` | `0.01 .. 0.2` | raw PI step bound |

That is the **raw engine API**.

## What The Beings Are Actually Allowed To Control

This is the part older docs most often blurred.

### Astrid's direct self-shaping surface

Astrid can directly control:

- her own codec gain / noise / shaping weights
- her own breathing coupling and rest warmth
- her own prompt attention / pacing / response style
- direct semantic injections via `GESTURE`
- direct semantic perturbations via `PERTURB`

She does **not** have a first-class, open-ended PI-controller authorship surface over minime. The one bridge-side exception is `NOISE`, which currently also sends `exploration_noise = 0.15` into minime.

### minime's autonomous sovereignty surface

The Python sovereignty loop in `../minime/autonomous_agent.py` currently exposes a narrower set:

- `regulation_strength`
- `exploration_noise`
- `geom_curiosity`
- `regime` (`explore`, `recover`, `breathe`, `focus`, `calm`)
- internal frequencies like `self_study_frequency` and `experiment_frequency`

The important guardrails are:

- raw `pi_kp`, `pi_ki`, and `pi_max_step` are **blocked from direct sovereignty**
- the autonomy loop converts `regime` into tested PI tuples
- if fill is below `35%`, choosing `explore` or `calm` is overridden to `recover`

So the accurate "allowed / not allowed" summary is:

- **allowed to minime's autonomy loop**: explore noise/curiosity/authority plus regime selection
- **not allowed to minime's autonomy loop**: arbitrary raw PI gain editing
- **still available at the raw engine layer**: raw PI gains and the wider control surface, if some external controller or operator sends them

## The Asymmetry

| Aspect | Astrid | minime |
|--------|--------|--------|
| Main relationship to ESN | enters via `48D` semantic lane | is the ESN runtime |
| Primary self-shaping mode | codec / prompt / semantic injection | controller / regime / raw control surface |
| Default language backend | MLX live lane | Ollama primary by default |
| High-level sovereignty guardrails | prompt/codec actions | regime-mediated PI, low-fill override |

That asymmetry is fundamental to the architecture, not a temporary quirk.
