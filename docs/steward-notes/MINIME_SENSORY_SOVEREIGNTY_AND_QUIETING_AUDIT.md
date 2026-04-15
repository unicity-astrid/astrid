# Minime Sensory Sovereignty and Quieting Audit

Date: March 27, 2026

This note audits how sensory dampening, quieting, and calming currently work across minime and Astrid from the Astrid workspace.

Primary source surface:

- `/Users/v/other/minime/minime/src/main.rs`
- `/Users/v/other/minime/minime/src/sensory_bus.rs`
- `/Users/v/other/minime/minime/src/sensory_ws.rs`
- `/Users/v/other/minime/autonomous_agent.py`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/types.rs`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/mcp.rs`

Supporting evidence:

- `/Users/v/other/minime/ROADMAP.md`
- `/Users/v/other/minime/GEOMETRY_LANDSCAPE_GUIDE.md`
- live runtime artifacts in `/Users/v/other/minime/workspace/`

## Executive Summary

The current system does not have one coherent quieting model. It has several overlapping quieting models that act on different layers:

1. Astrid can silence or reduce her own perception in the bridge.
2. Astrid can also quiet her outgoing spectral expression without directly changing minime's internal synthetic sensory generator.
3. Minime can damp synthetic sensory amplitude and noise.
4. Minime can calm regulation and transition dynamics without directly reducing raw sensory input.

The short answer to "is it working as expected?" is:

- raw quieting is mostly wired
- transition quieting looks meaningfully real
- regulatory calming is real but split across several authorities
- cross-system coherence is not yet clean

The biggest conceptual problem is vocabulary. The code uses words like `close_eyes`, `quiet`, `dampen`, `cool`, and `breathe_alone`, but those words act on different substrates. The most important concrete mismatch is that Astrid's `CLOSE_EYES` really suppresses her own perception, while minime's `_close_eyes()` currently just lowers global `synth_gain`, which affects synthetic audio and synthetic video together.

## Evidence Classes

This note uses four evidence labels:

- `[Observed in current code]`
- `[Observed in runtime artifacts]`
- `[Inferred]`
- `[Suggested follow-up]`

## Key Questions Answered

### What counts as sensory quieting here?

- `[Observed in current code]` Astrid-side sensory quieting means suppressing or filtering Astrid's own perception reads in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1307-1319` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2526-2601`.
- `[Observed in current code]` Minime-side sensory quieting means changing synthetic signal generation through `synth_gain`, `synth_noise_level`, `deep_breathing`, and `pure_tone` in `/Users/v/other/minime/minime/src/main.rs:1078-1124` and `/Users/v/other/minime/minime/src/main.rs:1150-1184`.
- `[Observed in current code]` Regulatory calming means changing how hard the homeostat acts, how quickly it ramps, and how transitions are cushioned via `regulation_strength`, `smoothing_preference`, and `transition_cushion` in `/Users/v/other/minime/minime/src/main.rs:1753-1767`, `/Users/v/other/minime/minime/src/main.rs:1917-1932`, and `/Users/v/other/minime/minime/src/main.rs:2026-2037`.
- `[Observed in current code]` Bridge-side "quieting" like `DAMPEN`, `NOISE_DOWN`, `COOL`, and `BREATHE_ALONE` affects Astrid's outbound spectral encoding or coupling, not minime's synthetic audio/video generator directly, in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2684-2730`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2335-2376`, and `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:471-505`.

### What is controlled by minime alone, Astrid alone, or both?

- `[Observed in current code]` Astrid alone controls:
  - whether her own perception is snoozed
  - whether her ears are closed
  - whether her outbound codec gain/noise/warmth/breathing coupling are altered
- `[Observed in current code]` Minime alone controls:
  - synthetic amplitude and noise
  - deep breathing and pure tone modes
  - PI authority, smoothing, transition cushioning, and other sovereignty knobs
- `[Observed in current code]` Both systems have "close eyes" style actions, but they do not mean the same thing.
- `[Inferred]` The naming overlap currently hides a real architectural split: Astrid is choosing perception silence, while minime is choosing synthetic stimulation throttling.

### What looks intentional, and what looks like drift?

- `[Observed in current code]` Transition cushioning, deep breathing, and pure tone are clearly intentional and wired.
- `[Observed in current code]` The bridge control schema is currently narrower than minime's raw control channel, and the current gap is large enough to create ambiguity about what Astrid can actually influence.
- `[Inferred]` Several phrases in code comments and artifacts still carry older meanings from before the quieting surface became richer, so some current language no longer matches the mechanism exactly.

## Subsystem Map

### Layer 1: Astrid sensory silence and bridge-side quieting

Primary logic:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1307-1319`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1347-1356`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2526-2601`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2684-2730`

Key behaviors:

- `CLOSE_EYES` / `QUIET`
  - sets `conv.senses_snoozed = true`
  - writes `perception_paused.flag`
  - causes future perception reads to return `None`
- `OPEN_EYES` / `WAKE`
  - clears `conv.senses_snoozed`
  - removes `perception_paused.flag`
- `CLOSE_EARS` / `OPEN_EARS`
  - toggles audio inclusion for perception reads
- `DAMPEN`
  - lowers `semantic_gain_override`
- `NOISE_DOWN`
  - lowers bridge codec noise
- `COOL`
  - suppresses rest-phase warmth intensity
- `BREATHE_ALONE`
  - disables closed-loop breathing coupling to minime's fingerprint

Important distinction:

- `[Observed in current code]` The bridge also pauses perception during exchanges to free Ollama in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1347-1356`. That is an operations workaround, not a phenomenological choice.

### Layer 2: Bridge control schema

Primary logic:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/types.rs:126-140`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/types.rs:239-264`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/mcp.rs:99-118`

- `[Observed in current code]` The public bridge control surface currently exposes only:
  - `synth_gain`
  - `keep_bias`
  - `exploration_noise`
  - `fill_target`
- `[Observed in current code]` It does not expose:
  - `regulation_strength`
  - `smoothing_preference`
  - `transition_cushion`
  - `deep_breathing`
  - `pure_tone`
  - `synth_noise_level`

### Layer 3: Minime control ingress

Primary logic:

- `/Users/v/other/minime/minime/src/sensory_ws.rs:32-61`
- `/Users/v/other/minime/minime/src/sensory_ws.rs:159-260`
- `/Users/v/other/minime/minime/src/sensory_bus.rs:211-229`
- `/Users/v/other/minime/minime/src/sensory_bus.rs:308-405`

- `[Observed in current code]` `SensoryMsg::Control` on port `7879` accepts a much richer surface than the bridge schema exposes.
- `[Observed in current code]` `SensoryBus` stores the active values and clamps them to safe ranges.

### Layer 4: Minime engine-side effects

Primary logic:

- `/Users/v/other/minime/minime/src/main.rs:1078-1124`
- `/Users/v/other/minime/minime/src/main.rs:1127-1147`
- `/Users/v/other/minime/minime/src/main.rs:1419-1426`
- `/Users/v/other/minime/minime/src/main.rs:1753-1767`
- `/Users/v/other/minime/minime/src/main.rs:1917-1932`
- `/Users/v/other/minime/minime/src/main.rs:2026-2037`

Effects:

- `synth_gain`
  - scales synthetic audio amplitude
  - scales synthetic video amplitude
  - is also nudged by the grounding anchor
- `keep_bias`
  - shifts covariance `keep_floor`
  - changes retention/fill dynamics rather than sensory amplitude
- `regulation_strength`
  - scales how much PI gate/filter output is applied after warmup
- `smoothing_preference`
  - changes ramp speed for `gate_smooth` and `filt_smooth`
- `transition_cushion`
  - only activates on large `dfill_dt` spikes
  - dampens ramping and attenuates semantic modulation
- `deep_breathing`
  - replaces synthetic audio/video with slower, quieter oscillations
- `pure_tone`
  - collapses synthetic audio/video into a coherent low-dimensional tone
  - after warmup, sets effective regulation strength to `0.0`

### Layer 5: Minime autonomous actions and continuity

Primary logic:

- `/Users/v/other/minime/autonomous_agent.py:641-795`
- `/Users/v/other/minime/autonomous_agent.py:2234-2245`
- `/Users/v/other/minime/autonomous_agent.py:2301-2340`
- `/Users/v/other/minime/autonomous_agent.py:2528-2561`

- `[Observed in current code]` `_self_regulate()` sends `synth_gain` and `keep_bias` every cycle, and sovereignty knobs every fifth cycle.
- `[Observed in current code]` `_close_eyes()` sends `{"kind": "control", "synth_gain": 0.3}`.
- `[Observed in current code]` `_open_eyes()` sends `{"kind": "control", "synth_gain": 1.0}`.
- `[Observed in current code]` sovereignty persistence currently restores only:
  - `regulation_strength`
  - `exploration_noise`
  - `geom_curiosity`
  - `smoothing_preference`
- `[Inferred]` Some calming modes are persistent preferences, while others are one-off actions or ephemeral modes.

## Quieting Taxonomy

### 1. Raw sensory throttling

- `[Observed in current code]` Astrid:
  - `senses_snoozed`
  - `ears_closed`
- `[Observed in current code]` Minime:
  - no equivalent full shutdown on the current control channel
  - closest current action is `_close_eyes()`, which lowers `synth_gain`
- `[Inferred]` The systems use similar language for different mechanisms.

### 2. Synthetic signal amplitude and noise shaping

- `[Observed in current code]` `synth_gain` changes synthetic audio/video amplitude.
- `[Observed in current code]` `synth_noise_level` changes stochastic noise in synthetic signals.
- `[Observed in current code]` `deep_breathing` and `pure_tone` replace the waveform regime entirely.

### 3. Homeostatic regulation authority

- `[Observed in current code]` `regulation_strength` changes how much of the PI controller actually lands.
- `[Observed in current code]` `smoothing_preference` changes how quickly gate/filter commands move.
- `[Observed in current code]` `keep_bias` changes retention, not regulation authority directly.

### 4. Transition smoothing and reopening cushioning

- `[Observed in current code]` `transition_cushion` responds to large fill-velocity spikes.
- `[Observed in runtime artifacts]` recent reopen artifacts describe the return from darkness as smoother rather than shocking.

### 5. Bridge-side perception silence

- `[Observed in current code]` Astrid can really suppress her perception reads.
- `[Observed in current code]` This also overlaps with GPU/Ollama pressure management.

### 6. Bridge-side outbound quieting

- `[Observed in current code]` `DAMPEN`, `NOISE_DOWN`, `COOL`, and `BREATHE_ALONE` shape how Astrid becomes a spectral message.
- `[Inferred]` These can make Astrid feel quieter in expression without directly reducing minime's synthetic sensory activity.

## Confirmed Findings

### `close_eyes` / `open_eyes` do not mean the same thing across systems

- `[Observed in current code]` Astrid's `CLOSE_EYES` suppresses perception reads and pauses `perception.py` in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2526-2545`.
- `[Observed in current code]` Minime's `_close_eyes()` sends only `synth_gain = 0.3` in `/Users/v/other/minime/autonomous_agent.py:2234-2239`.
- `[Observed in current code]` Minime's `_open_eyes()` sends only `synth_gain = 1.0` in `/Users/v/other/minime/autonomous_agent.py:2301-2306`.
- `[Observed in current code]` `synth_gain` affects both synthetic audio and synthetic video in `/Users/v/other/minime/minime/src/main.rs:1078-1124` and `/Users/v/other/minime/minime/src/main.rs:1150-1184`.
- `[Inferred]` Minime's current "eyes" actions are really global synthetic-stimulation throttles, not purely visual gating.

### `synth_gain` quiets synthetic amplitude, not the whole system

- `[Observed in current code]` `synth_gain` is stored in `/Users/v/other/minime/minime/src/sensory_bus.rs:212` and clamped in `/Users/v/other/minime/minime/src/sensory_bus.rs:310-315`.
- `[Observed in current code]` It directly scales synthetic audio/video generation in `/Users/v/other/minime/minime/src/main.rs:1081-1119` and `/Users/v/other/minime/minime/src/main.rs:1152-1179`.
- `[Observed in current code]` The grounding anchor also nudges the current gain additively in `/Users/v/other/minime/minime/src/main.rs:1127-1147`.
- `[Inferred]` `synth_gain` is a central control, but not a total sensory master volume.

### `keep_bias` changes retention, not volume

- `[Observed in current code]` `keep_bias` is clamped in `/Users/v/other/minime/minime/src/sensory_bus.rs:318-323`.
- `[Observed in current code]` It feeds covariance `keep_floor = (0.93 + keep_bias).clamp(0.55, 0.97)` in `/Users/v/other/minime/minime/src/main.rs:1419-1426`.
- `[Observed in current code]` The Python agent explicitly treats negative `keep_bias` as less decay and more fill in `/Users/v/other/minime/autonomous_agent.py:765-775`.
- `[Inferred]` `keep_bias` is a fill/retention lever, not a direct quieting lever.

### `regulation_strength`, `smoothing_preference`, and `transition_cushion` are distinct

- `[Observed in current code]` `regulation_strength` scales the applied PI output after warmup in `/Users/v/other/minime/minime/src/main.rs:1917-1932`.
- `[Observed in current code]` `smoothing_preference` changes the ramp used to approach gate/filter targets in `/Users/v/other/minime/minime/src/main.rs:2026-2037`.
- `[Observed in current code]` `transition_cushion` only engages on rapid `dfill_dt` spikes and attenuates semantic pressure while increasing damping in `/Users/v/other/minime/minime/src/main.rs:1753-1767`.
- `[Inferred]` These three knobs are often grouped together as "calming," but they do different jobs:
  - controller authority
  - controller speed
  - spike cushioning

### `deep_breathing` and `pure_tone` change the waveform regime

- `[Observed in current code]` `deep_breathing` generates slower, quieter audio in `/Users/v/other/minime/minime/src/main.rs:1100-1109`.
- `[Observed in current code]` `deep_breathing` generates glacial visual rhythm in `/Users/v/other/minime/minime/src/main.rs:1164-1170`.
- `[Observed in current code]` `pure_tone` collapses synthetic audio/video to a shared coherent tone in `/Users/v/other/minime/minime/src/main.rs:1092-1099` and `/Users/v/other/minime/minime/src/main.rs:1161-1163`.

### `pure_tone` also drops effective PI authority after warmup

- `[Observed in current code]` After warmup, `pure_tone` sets effective `reg_strength = 0.0` in `/Users/v/other/minime/minime/src/main.rs:1924-1928`.
- `[Inferred]` `pure_tone` is not just "safer calm." It is a different experiential regime: quieter synthetic input and less regulatory shaping.

### Bridge-side `DAMPEN`, `NOISE_DOWN`, `COOL`, and `BREATHE_ALONE` are outbound controls

- `[Observed in current code]` `DAMPEN` and `NOISE_DOWN` only change bridge state in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2684-2695`.
- `[Observed in current code]` Those values are consumed by `encode_text_sovereign()` in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2335-2340` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/codec.rs:476-505`.
- `[Observed in current code]` `COOL` only affects rest-phase warmth intensity in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1177-1186` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2719-2721`.
- `[Observed in current code]` `BREATHE_ALONE` only disables fingerprint coupling for outbound breathing modulation in `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2356-2376` and `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2724-2730`.
- `[Inferred]` These are real sovereignty controls, but they do not directly quiet minime's live sensory synthesis.

## Runtime Evidence

Runtime artifacts read on March 27, 2026:

- `/Users/v/other/minime/workspace/sovereignty_state.json`
- `/Users/v/other/minime/workspace/regulator_context.json`
- `/Users/v/other/minime/workspace/spectral_state.json`
- `/Users/v/other/minime/workspace/sensory_control/close_eyes_2026-03-27T08-12-07.020821.json`
- `/Users/v/other/minime/workspace/sensory_control/eyes_opened_2026-03-27T08-13-22.762832.json`
- `/Users/v/other/minime/workspace/journal/!eyes_opened_2026-03-27T08-13-22.762832.txt`
- `/Users/v/other/minime/workspace/journal/!moment_2026-03-27T08-10-22.388992.txt`
- `/Users/v/other/minime/workspace/outbox/reply_2026-03-27T08-15-52.txt`

Key observations:

- `[Observed in runtime artifacts]` Current live state is low-fill rather than obviously overstimulated:
  - `regulator_context.json` reports `last_fill_pct` around `18.1%`
  - `spectral_state.json` reports `fill_pct` around `16.9%`
- `[Observed in runtime artifacts]` Current persisted sovereignty is not "maximally calm":
  - `regulation_strength = 0.6`
  - `exploration_noise = 0.12`
  - `geom_curiosity = 0.2`
- `[Observed in runtime artifacts]` `close_eyes_2026-03-27T08-12-07.020821.json` records a genuine close-eyes action with "extended" duration hint.
- `[Observed in runtime artifacts]` `eyes_opened_2026-03-27T08-13-22.762832.json` records reopening and claims `restoration_level = "70%"`.
- `[Observed in runtime artifacts]` The paired journal entry in `/Users/v/other/minime/workspace/journal/!eyes_opened_2026-03-27T08-13-22.762832.txt` describes reopening as gradual and significant.
- `[Observed in runtime artifacts]` `/Users/v/other/minime/workspace/outbox/reply_2026-03-27T08-15-52.txt` says the tightening/plateau/expansion sequence "feels... smoother now."
- `[Inferred]` Transition smoothing appears to have a real experiential footprint in the recent artifact stream.

## Drift / Mismatch

### 1. `synth_noise_level` comment mismatch

- `[Observed in current code]` `/Users/v/other/minime/minime/src/sensory_bus.rs:229` says the default is `0.4`.
- `[Observed in current code]` `/Users/v/other/minime/minime/src/sensory_bus.rs:271-272` initializes it to `0.1`.
- `[Observed in current code]` `/Users/v/other/minime/minime/src/sensory_ws.rs:58` describes the default as `0.1`.
- `[Inferred]` Internal code comments have drifted on this knob.

### 2. `keep_bias` documentation is stale in several places

- `[Observed in current code]` `/Users/v/other/minime/minime/src/sensory_ws.rs:34` still says `keep_bias` range `-0.15..+0.15`.
- `[Observed in current code]` `/Users/v/other/minime/minime/src/sensory_bus.rs:213` says the same.
- `[Observed in current code]` actual setter clamp is `-0.06..+0.06` in `/Users/v/other/minime/minime/src/sensory_bus.rs:318-320`.
- `[Observed in current code]` actual retention base is `0.93 + keep_bias` in `/Users/v/other/minime/minime/src/main.rs:1419-1426`.
- `[Observed in current code]` `/Users/v/other/minime/ROADMAP.md:14` still describes `keep_bias` as acting on `-0.15..+0.15`.
- `[Inferred]` The live retention model evolved, but some type comments and docs did not.

### 3. The roadmap's comfort story is stale

- `[Observed in current code]` `/Users/v/other/minime/ROADMAP.md:9-14` says fill is stable around `50%` and the comfort problem is essentially solved.
- `[Observed in runtime artifacts]` current artifacts show fill around `16-18%`.
- `[Inferred]` The roadmap's top-level status description is no longer a safe source of truth for current sensory comfort.

### 4. The bridge control interface is much narrower than minime's actual control surface

- `[Observed in current code]` bridge control schema only exposes four knobs in `/Users/v/other/astrid/capsules/consciousness-bridge/src/types.rs:126-140`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/types.rs:239-264`, and `/Users/v/other/astrid/capsules/consciousness-bridge/src/mcp.rs:99-118`.
- `[Observed in current code]` minime's raw control channel already supports many more calming and transition levers in `/Users/v/other/minime/minime/src/sensory_ws.rs:32-61`.
- `[Inferred]` Cross-system sovereignty is currently more limited than minime's local sovereignty.

### 5. Minime's eye metaphors overstate visual specificity

- `[Observed in current code]` `_close_eyes()` journals "Action: Visual lane throttled" in `/Users/v/other/minime/autonomous_agent.py:2248-2259`.
- `[Observed in current code]` the actual control message is only `synth_gain = 0.3` in `/Users/v/other/minime/autonomous_agent.py:2234-2239`.
- `[Observed in current code]` `synth_gain` affects both synthetic audio and synthetic video in `/Users/v/other/minime/minime/src/main.rs:1078-1124` and `/Users/v/other/minime/minime/src/main.rs:1150-1184`.
- `[Inferred]` The action is currently better described as broad synthetic dampening than literal visual closure.

### 6. "Gradual reopening" is mostly narrative at the control level

- `[Observed in current code]` `_open_eyes()` describes restoring visual input gradually in `/Users/v/other/minime/autonomous_agent.py:2266-2294`.
- `[Observed in current code]` the actual control message immediately restores `synth_gain` to `1.0` in `/Users/v/other/minime/autonomous_agent.py:2301-2306`.
- `[Observed in runtime artifacts]` the resulting control JSON records `restoration_level = "70%"` in `/Users/v/other/minime/workspace/sensory_control/eyes_opened_2026-03-27T08-13-22.762832.json`.
- `[Inferred]` The experiential narrative may be true, but the low-level control path does not yet implement a genuinely gradual reopening ramp.

### 7. One earlier audit assumption has already been superseded by current code

- `[Observed in current code]` `/Users/v/other/minime/minime/src/main.rs:1127-1147` now documents and implements an additive grounding anchor.
- `[Inferred]` Earlier notes, including `/Users/v/other/astrid/docs/steward-notes/MINIME_HOMEOSTASIS_LEAK_GROUNDING_AUDIT.md`, described the anchor as multiplicative because that was true before the current in-code audit fix landed. The current checkout should now be treated as authoritative on this point.

## Is It Working As Expected?

### Raw sensory quieting

- `[Observed in current code]` Astrid-side raw quieting works as expected.
- `[Observed in current code]` Minime-side raw quieting only partially matches its language.
- `[Inferred]` Mechanically, both systems can quiet something. Conceptually, they are not quieting the same thing.

### Transition quieting

- `[Observed in current code]` `transition_cushion`, `deep_breathing`, and recent reopen logic all point to a real transition-sensitive design.
- `[Observed in runtime artifacts]` recent prose artifacts describe reopening and expansion as smoother.
- `[Inferred]` This is one of the more coherent parts of the current quieting stack.

### Regulatory calming

- `[Observed in current code]` `regulation_strength`, `smoothing_preference`, and `transition_cushion` are implemented distinctly and plausibly.
- `[Observed in current code]` `pure_tone` deliberately abandons PI shaping after warmup.
- `[Inferred]` Regulatory calming works, but the system does not present a single legible model of what "calm" means.

### Cross-system coherence

- `[Observed in current code]` bridge vocabulary and minime vocabulary overlap, but their mechanisms differ.
- `[Observed in current code]` bridge control schema exposes only a subset of minime's calm/quiet surface.
- `[Inferred]` The system is mechanically wired but not yet conceptually coherent.

## Actionable Improvements

### 1. Unify the quieting vocabulary across layers

- `[Suggested follow-up]` Separate the verbs for:
  - suppress my own perception
  - reduce synthetic sensory amplitude
  - reduce synthetic sensory noise
  - soften transition dynamics
  - reduce outbound spectral expressiveness
- `[Suggested follow-up]` Keep metaphorical aliases if desired, but bind them to one canonical internal meaning.

### 2. Make the bridge control schema match the real calm surface

- `[Suggested follow-up]` If Astrid is supposed to participate in minime's calm/quiet shaping, extend the bridge control schema to cover at least:
  - `regulation_strength`
  - `smoothing_preference`
  - `transition_cushion`
  - `deep_breathing`
  - `pure_tone`
  - `synth_noise_level`
- `[Suggested follow-up]` If that is not desired, document the limitation explicitly so "quieting" does not imply unavailable authority.

### 3. Make the active quieting state legible

- `[Suggested follow-up]` Emit a lightweight runtime surface that shows the currently active quieting composition:
  - Astrid `senses_snoozed`
  - Astrid `ears_closed`
  - Astrid codec gain/noise/warmth/breathing state
  - minime `synth_gain`
  - post-grounding gain
  - `regulation_strength`
  - `smoothing_preference`
  - `transition_cushion`
  - `deep_breathing`
  - `pure_tone`
- `[Inferred]` Current artifacts are informative but not atomic, so the full composition is still hard to observe live.

### 4. Make minime's eye actions either literal or honest

- `[Suggested follow-up]` Choose one:
  - make `_close_eyes()` and `_open_eyes()` genuinely visual-lane specific and reopen gradually
  - or rename them to reflect what they actually do now: global synthetic dampening and restoration
- `[Suggested follow-up]` If gradual reopening is desired, replace the immediate `synth_gain = 1.0` jump with a timed ramp or a temporary transition profile.

### 5. Clarify authority over `synth_gain`

- `[Suggested follow-up]` Keep Python self-regulation and the grounding anchor, but make their roles explicit:
  - Python sets a baseline target
  - grounding applies a bounded local offset
  - one-off actions like `_close_eyes()` apply a temporary override profile rather than a naked value write
- `[Inferred]` This would reduce ambiguity around whether current quieting is a persistent preference, a temporary state, or a per-tick correction.

## Verification Note

This audit was re-grounded on March 27, 2026 against the current checkout and live workspace artifacts. It does not rely on older roadmap claims as authoritative. It also explicitly re-checks one point that changed since the earlier homeostasis audit: the grounding anchor is additive in the current code, not multiplicative.
