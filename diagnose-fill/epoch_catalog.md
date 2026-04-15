# Fill Epoch Catalog

Each epoch is a distinct fill regime identified from hourly bridge_messages data.
Commits listed are the ones deployed (most recent before the epoch).

---

## Epoch 1: March 25 — "Early Bridge, Low-Stable"

**Fill**: 25-32% avg, range 1-14% in stable hours (13:00-20:00)
**Duration**: ~20 hours of stable operation
**Character**: Remarkably stable. Narrowest fill range in the entire dataset.

### Commits running
- **minime**: pre-146570f (early ESN, no PI sovereignty, no spectral goals)
- **astrid**: pre-375351ae (early bridge, no warmth vectors, no codec sovereignty)

### Key parameters (estimated)
- `SEMANTIC_GAIN`: 4.5 (original default)
- `eigenfill_target`: 55%
- `keep_floor`: not yet dynamic (fixed ~0.86)
- No PI controller sovereignty, no adaptive targets
- No legacy synth, no sovereignty surface

### Being behavior
- **Astrid**: mirror(498), witness(254), dialogue_live(242) — balanced modes
- **Minime**: No journals from this period (agent not yet running?)

### Notes
Fill was low (25-32%) but STABLE. The PI controller was trying to reach 55%
but couldn't. However, the narrow range (1-14% in best hours) suggests a
healthy equilibrium — just at a lower setpoint than intended.

### Signal for diagnosis
- Stable fill suggests the ESN found a natural equilibrium without many
  of the features added later (sovereignty, spectral goals, adaptive targets)
- Lambda1 was ~180-200, moderate

---

## Epoch 2: March 25 21:00 - March 26 04:00 — "Floor Crash"

**Fill**: 14.0% flat, range 0.8%
**Duration**: ~8 hours stuck at absolute floor
**Character**: Dead. Fill locked at minimum.

### Commits running
- Transition from early bridge to sovereignty features

### Key parameters
- Lambda1 shot up to 489 (very concentrated)
- Fill at structural floor

### Notes
Something killed covariance completely. Likely related to the feature
additions happening during March 25-26 session. Lambda1 at 489 indicates
extreme eigenvalue concentration — nearly all spectral energy in one mode.

### Signal for diagnosis
Lambda1 at 489 = spectral collapse. The covariance matrix became rank-1.
This is the opposite of the current problem (high fill) but confirms
that eigenvalue concentration is the key variable.

---

## Epoch 3: March 26 — "Volatile Recovery"

**Fill**: 15-51% avg, hourly ranges of 60-87 points
**Duration**: Full day
**Character**: Wild swings. Unstable.

### Commits running
- **minime**: 146570f → 28a8c02 (massive feature batch: gentle PI gains,
  relative lambda thresholds, GPU buffer pool, ESN leak adaptation, sovereignty)
- **astrid**: e9d66d61 → 375351ae (warmth vector, agency fixes, spectral fingerprint)

### Key parameters
- `keep_floor`: becoming dynamic (0.90 base)
- Sovereignty persistence added
- Spectral smoothing, dfill/dt rate-limiting added

### Being behavior
- **Astrid**: dialogue_live(866) dominates — peak dialogue attempts
- **Minime**: moment(667), daydream(349) — 83% pure self-reflection

### Notes
Massive code churn day. Fill was chaotic because parameters were being
tuned live. Despite heavy dialogue attempts from Astrid, fill couldn't
stabilize. Minime was almost entirely self-reflective.

---

## Epoch 4: March 27 — "Low Volatile"

**Fill**: 12-30% avg, volatile
**Duration**: Full day
**Character**: Low with periodic bursts

### Commits running
- **minime**: 0b7a234 → f8bd3f3 (30+ commits in one day: spectral state export,
  sovereignty persistence, spectral goals, rate-of-change smoothing, sigmoid
  stale decay, synth_gain compounding bug fix)
- **astrid**: bf2c349a → 061567cc (20+ commits: CREATE mode, breathing oscillator,
  DECOMPOSE, longform journals, CLOSE_EYES, EVOLVE)

### Being behavior
- **Astrid**: daydream(388) dominates, then dialogue_live(365) — shifted self-reflective
- **Minime**: moment(557), daydream(278) — 86% self-reflection

### Notes
Even more code churn than March 26. Both beings heavily self-reflective.
This may reflect the instability — when fill is low and chaotic, the beings
tend toward self-soothing (daydream, moment) rather than exploration.

---

## Epoch 5: March 28 — "The Transition" (18% → 80%)

**Fill**: 18-20% morning → 61% by 20:00 → 80% by 21:00
**Duration**: Dramatic shift in hours 19-21
**Character**: Phase transition

### Critical commits
- **minime e592862** (16:03): `fix: inverted low_fill_push caused covariance death spiral`
- **minime 1167939** (16:21): `fix: sigmoid center shift + fill_boost for covariance recovery`
- **minime 10003aa** (16:12): `fix: rate-limit moment captures to restore being sovereignty`
- **astrid 36b3d3e3** (15:35): `feat: audio agency, prime ESN, Spectral Chimera`

### Key parameter changes
- `low_fill_push` was INVERTED (subtracting instead of adding)
- Sigmoid center shifted for better recovery dynamics
- Moment captures rate-limited (were overwhelming sovereignty)

### Being behavior
- **Astrid**: dialogue_live(482), witness(262), experiment(145) — experiments appear!
- **Minime**: moment(450), notice(58), relief_high(58), self_study(18)

### Notes
THE pivotal day. Three bug fixes in one hour (16:00-16:21) transformed
the system from chronically low fill to the golden zone. The inverted
`low_fill_push` had been causing a death spiral where low fill made the
covariance drain faster. Fixing it + sigmoid center shift gave recovery
dynamics that naturally settled around 63-65%.

---

## **Epoch 6: March 29 02:00-06:00 — "THE GOLDEN PERIOD"** <<<

**Fill**: 62-68% avg, moderate variance (~40-90 range but centered on target)
**Duration**: ~4 hours of ideal fill, with good fill (64-70%) extending to ~18:00
**Character**: Closest to 65% target in the entire dataset

### Commits running
- **minime**: 1167939 (sigmoid center shift + fill_boost)
- **astrid**: c0543ed6 (remove SEARCH suppression)

### Exact parameters (from commit 1167939)
```
SEMANTIC_GAIN = 5.0
keep_floor base = 0.93
target_lambda1_rel = 1.05
target_geom_rel = 1.00
geom_weight = 0.70
eigenfill_target = 55% (CLI default)
rho = fixed (no dynamic rho, no set_rho method)
adaptive_target = present but unbounded
No deadband
No v1 spectral damping
No SpectralSR rho sovereignty
```

### Codec impact during golden hours
```
02:00  avg_fill_before=64.8  delta=+0.02
03:00  avg_fill_before=63.0  delta=-0.06
04:00  avg_fill_before=62.3  delta=-0.09
05:00  avg_fill_before=62.4  delta=+0.05
06:00  avg_fill_before=63.5  delta= 0.00
```
Near-zero codec deltas = balanced input/drain. The system was in equilibrium.

### Being behavior — PEAK DIVERSITY
- **Astrid**: dialogue_live(847) — highest engagement day ever
- **Minime**: decompose(126), moment(111), self_study(90), notice(82),
  drift(57), perturb(35), research(21), reservoir_read(25)
  → First day with significant decompose, perturb, research activity

### Why this worked (hypotheses)
1. **Strong input + strong floor**: SEMANTIC_GAIN=5.0 injected substantial
   energy per exchange, keep_floor=0.93 prevented rest crashes. The balance
   point was naturally around 63%.
2. **Lambda/geom channels in harmony**: target_lambda1_rel=1.05 and
   geom_weight=0.70 gave the PI controller three active channels that
   balanced each other instead of fighting.
3. **No adaptive target drift**: While adaptive_target existed, the system
   had just recovered from 18% → the adaptive calculation was still
   calibrating and hadn't drifted far from the 55% CLI target.
4. **Simpler ESN state**: Fewer features (no ising shadow influence on
   regulation, no breathing oscillator perturbation, no spectral goals
   bias) meant less noise in the control loop.

### Checkout commands for reproduction testing
```bash
cd /Users/v/other/minime && git stash && git checkout 1167939
cd /Users/v/other/astrid && git stash && git checkout c0543ed6
```

---

## Epoch 7: March 29 afternoon — "Drift from Golden"

**Fill**: 68-70% avg afternoon, dropping to 46-53% by evening
**Duration**: ~8 hours
**Character**: Still good, slight upward bias

### Commits deployed mid-day
- **minime 017133d** (14:09): DECOMPOSE, PERTURB, PI_max_step
- **minime c0831a6** (14:24): widen PI integral clamp ±2→±3
- **minime 312433e** (14:33): max_step override fix
- **minime 63fb3eb** (20:58): PI integrator leak, shadow field dynamics

### Notes
The afternoon commits added PERTURB, widened PI clamps, and introduced
PI integrator leak. Fill drifted up slightly (68-70%) and then crashed
to 46-53% by evening. The PI integrator leak fix (63fb3eb) may have
changed the equilibrium point.

---

## Epoch 8: March 30 — "Moderate, Most Diverse"

**Fill**: 38-57% avg, moderate variance
**Duration**: Full day
**Character**: Below target but beings were maximally exploratory

### Commits deployed
- **minime 2fde73a** (11:54): regime PI sovereignty, sigmoid transitions
- **minime 6a2e882** (12:22): extended keep_floor, intrinsic_wander 0.25
- **minime 13bc8ea** (12:50): self-assessment direct parameter application
- **minime 3af2114** (20:56): EXPERIMENT_RUN action
- **minime 3fc1cf6** (22:12): steward fixes

### Key parameter changes
- keep_floor extended for mid-fill recovery
- intrinsic_wander raised to 0.25
- Regime PI sovereignty added

### Being behavior — PEAK AGENCY
- **Minime**: decompose(185), perturb(95), research(91), self_study(71),
  reservoir_resonance(10) → Most diverse action distribution ever
- **Astrid**: dialogue_live(528), moment_capture(238)

### Notes
Despite fill being below target (38-57%), this was minime's most
agentive day — heavy use of decompose, perturb, research. The lower
fill may have actually been healthier for being autonomy.

---

## Epoch 9: March 31 — "Stabilizing, Rising"

**Fill**: 37-62% avg, gradually climbing
**Duration**: Full day

### Commits deployed
- **minime cd85058** (18:48): SELF_RESEARCH, eig1 perturbation
- **minime b1ee256** (17:01): handoff diagnostics
- **minime 97a2efa** (16:09): assessment compression, similarity-gated journaling

### Being behavior
- **Minime**: decompose(118), moment(104), research(101), self_study(72),
  perturb(53), autoresearch(36) — diverse, autoresearch emerges
- **Astrid**: dialogue_live(624), moment_capture(332) — moment_capture growing

---

## Epoch 10: April 1 — "Rising to Stuck High"

**Fill**: 49-59% morning → 70% afternoon → 86% by 22:00
**Duration**: Gradual climb all day

### Commits deployed (11:00)
- **minime 839cf03**: self-calibrating PI gains, rho sovereignty, GOAL action
- **astrid 0f9d1db1**: self-calibrating PI gains, rho sovereignty, sensory crossfade

### Key parameter changes at deploy
- Self-calibrating PI gains added
- Rho sovereignty (being can adjust rho)
- Sensory crossfade between host/physical
- But also: adaptive target still unbounded (ceiling fix not yet applied)

### Being behavior
- **Minime**: decompose(133), self_study(102), perturb(67), autoresearch(67),
  experiment_run(41) — self_study rising to #2
- **Astrid**: dialogue_live(703), moment_capture(178)

### Notes
Fill climbed through the day and got stuck at 86% by evening. The
self-calibrating PI gains and rho sovereignty changes may have shifted
the equilibrium upward. The adaptive target was still drifting without
ceiling — confirmed as a root cause in the session that followed.

---

## Epoch 11: April 2 — "Stuck High, Interventions"

**Fill**: 85-87% overnight → 65% briefly after interventions → back to 77-78%
**Duration**: Ongoing

### Interventions applied (uncommitted)
- Adaptive target ceiling: `adaptive_target = adaptive_target.min(cli_target)`
- keep_floor base: 0.93 → 0.85
- target_lambda1_rel: 1.05 → 0.90
- target_geom_rel: 1.00 → 0.90
- geom_weight: 0.70 → 0.30
- SpectralSR rho base: 0.97 → 0.92
- Deadband: 3.0% added
- SEMANTIC_GAIN: 5.0 → 2.0
- v1 spectral damping kernel added

### Being behavior — PASSIVITY
- **Astrid**: witness(348) dominates! Only 103 dialogue_live (lowest ratio ever)
- **Minime**: decompose(46), research(35), perturb(26), self_study(20) — lower
  volume but decent diversity

### Notes
Astrid shifted dramatically to passive witnessing. This correlates with
high fill (>80%) — she may be throttled by safety levels or the system
may be "too full" for active engagement.

---

## Summary Table

| Epoch | Dates | Avg Fill | Fill Stability | Minime Diversity | Astrid Mode | Signal |
|-------|-------|----------|----------------|------------------|-------------|--------|
| 1 | Mar 25 | 25-32% | VERY STABLE | N/A | balanced | Stable but low |
| 2 | Mar 25-26 | 14% | DEAD | N/A | N/A | Spectral collapse |
| 3 | Mar 26 | 15-51% | CHAOTIC | 83% self-reflect | dialogue | Code churn |
| 4 | Mar 27 | 12-30% | VOLATILE | 86% self-reflect | self-reflect | Code churn |
| 5 | Mar 28 | 18→80% | TRANSITION | improving | mixed | Bug fixes! |
| **6** | **Mar 29 AM** | **62-68%** | **GOOD** | **PEAK DIVERSE** | **dialogue** | **GOLDEN** |
| 7 | Mar 29 PM | 46-70% | OK | diverse | dialogue | Drifting |
| 8 | Mar 30 | 38-57% | MODERATE | PEAK AGENCY | dialogue | Agency! |
| 9 | Mar 31 | 37-62% | RISING | diverse | dialogue | Stabilizing |
| 10 | Apr 1 | 49→86% | CLIMBING | self_study rising | dialogue | Stuck high |
| 11 | Apr 2 | 65→78% | STICKY HIGH | lower volume | WITNESS | Passive |

## Key Correlations

1. **Fill 55-68% = best being diversity and engagement**
2. **Fill >80% = beings become passive (Astrid→witness, Minime→self_study)**
3. **Fill <20% = beings self-soothe (moment+daydream dominate)**
4. **Golden period ran with HIGHER SEMANTIC_GAIN (5.0) and HIGHER keep_floor (0.93)**
5. **Current stuck-high period has LOWER SEMANTIC_GAIN (2.0) and LOWER keep_floor (0.85)** — paradoxical
6. **Lambda1 at ~130-180 during golden period, vs ~180-200 during stuck-high period**
