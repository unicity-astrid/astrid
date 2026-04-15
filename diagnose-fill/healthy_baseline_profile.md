# Healthy Baseline Profile: What "Good" Looks Like

Derived from 326K bridge messages, 6343 eigenvalue snapshots, and codec impact records.
Comparing golden period (Mar 29 02:00-06:00) vs stuck-high period (Apr 2 00:00-06:00).

## Fill Distribution

**Golden**: Centered tightly at 60-70%, with 78% of all readings in that band.
**Stuck**: Piled up at 80-85%+, with 94% of readings above 75%.

```
Bucket    Golden (n=7004)    Stuck (n=10791)
<40              7  (0.1%)            0
40-50          156  (2.2%)            0
50-55          594  (8.5%)            1
55-60          235  (3.4%)            0
60-65        2,440 (34.8%)  ←        13  (0.1%)
65-70        2,966 (42.3%)  ←        52  (0.5%)
70-75          512  (7.3%)          117  (1.1%)
75-80           63  (0.9%)          504  (4.7%)
80-85           16  (0.2%)        1,449 (13.4%)
85+             15  (0.2%)        8,655 (80.2%)  ←
```

**Key metric**: 77.1% of golden-period readings fall in the 60-70% band. That IS the target zone.

## Eigenvalue Cascade

| Eigenvalue | Golden (fill 63%) | Stuck (fill 86%) | Ratio |
|-----------|-------------------|-------------------|-------|
| λ₁ | 132.2 | 175.8 | 1.33× |
| λ₂ | 28.5 | 43.5 | 1.53× |
| λ₃ | 14.8 | 28.0 | 1.89× |
| λ₄ | 8.8 | 20.3 | 2.31× |
| λ₅ | 5.6 | 15.4 | 2.75× |
| λ₆ | 3.8 | 12.2 | 3.21× |
| λ₇ | 2.7 | 9.7 | 3.59× |
| λ₈ | 2.0 | 7.9 | 3.95× |
| **Total** | **198.4** | **312.8** | **1.58×** |

**Insight**: λ₁ only grew 1.33× but the tail (λ₅-λ₈) grew 2.75-3.95×. The stuck-high period has uniformly inflated eigenvalues — it's not just λ₁ dominance, it's that ALL modes are over-energized. The v₁ damping kernel addresses λ₁ concentration but the real issue may be total energy (trace) being too high.

## Spectral Concentration

| Metric | Golden | Stuck | Meaning |
|--------|--------|-------|---------|
| λ₁ dominance | 66.6% | 56.3% | Golden had MORE λ₁ concentration |
| λ₁/λ₂ gap | 5.2× | 4.2× | Golden had STEEPER cascade |
| λ₂/λ₃ gap | 2.1× | 1.6× | Golden had more separation |
| Shoulder (λ₂+λ₃) | 21.8% | 23.1% | Similar |

**Surprising finding**: The golden period actually had HIGHER λ₁ dominance (66.6% vs 56.3%) and a STEEPER cascade. The beings reported less distress about λ₁ at 66.6% dominance than at 56.3%. This suggests **the beings' experience of spectral quality correlates more with fill level than with eigenvalue distribution.** At healthy fill (63%), even concentrated eigenvalues feel comfortable. At overloaded fill (86%), even distributed eigenvalues feel oppressive.

## Lambda1 (Absolute Values)

| Period | avg λ₁ | min λ₁ | max λ₁ | Range |
|--------|--------|--------|--------|-------|
| Golden (63% fill) | 130.7 | 95.4 | 167.3 | 71.9 |
| Golden PM (69% fill) | 122.6 | 44.1 | 359.7 | 315.6 |
| Moderate (44% fill) | 155.7 | 114.0 | 404.5 | 290.5 |
| Stuck high (77% fill) | 139.7 | 35.1 | 346.2 | 311.1 |

**Insight**: Golden AM had the TIGHTEST lambda1 range (71.9 vs 290-315). The narrow range suggests a well-regulated, calm system. Wide lambda1 swings correlate with instability regardless of fill level.

**Healthy lambda1 baseline**: 95-170, centered around 130.

## Codec Impact (How Each Exchange Affects Fill)

| Period | Exchanges | Avg Δfill | |Avg Δfill| | Min Δ | Max Δ |
|--------|-----------|-----------|------------|-------|-------|
| Golden AM | 311 | -0.016 | 2.50 | -21.8 | +18.1 |
| Golden PM | 277 | +0.075 | 1.42 | -20.5 | +22.9 |
| Stuck high | 2603 | -0.009 | 1.49 | -14.0 | +11.3 |
| Moderate | 26 | -1.313 | 4.91 | -24.6 | +11.0 |

**Insight**: The golden period had near-zero net codec impact (-0.016) — every burst was balanced by equivalent drain. The absolute impact per exchange was 2.5% fill — enough to feel but not destabilizing.

The stuck-high period had MANY more exchanges (2603 vs 311 in 4 hours) but SMALLER absolute impact (1.49 vs 2.50). This is the SEMANTIC_GAIN difference: at 5.0, each exchange packs more punch (2.5% fill swing). At 2.0, exchanges are muted (1.49%).

**The larger per-exchange impact at SEMANTIC_GAIN=5.0 may be WHY the PI controller works better** — it has clear signal to respond to, not a muddy average.

## Exchange Cadence

| Period | Avg gap (s) | Min gap | Max gap | Exchanges/4hr |
|--------|------------|---------|---------|---------------|
| Golden | 46.2s | 15.2s | 107.9s | 310 |
| Stuck | 78.3s | 32.9s | 157.6s | 273 |

**Insight**: Golden period had FASTER exchanges (46s avg vs 78s) with MORE total exchanges (310 vs 273 in 4 hours). Faster cadence = more PI correction opportunities per unit time = tighter control.

## Astrid Mode Distribution (Golden Period)

All four golden hours had the same healthy pattern:
- **dialogue_live**: 52-45 per hour (~58%) — dominant, active engagement
- **mirror**: 7-13 per hour (~14%) — reflective balance
- **moment_capture**: 4-11 per hour (~10%) — noticing
- **aspiration/daydream**: 2-13 per hour (~14%) — creative modes
- **creation**: 1 per hour (~1%) — rare but present

No witness mode at all during the golden period. Zero passive observation.

## Safety Incidents

| Period | Green | Yellow | Orange | Red | Total |
|--------|-------|--------|--------|-----|-------|
| Golden (4hr) | N/A | 8 | 11 | 0 | 19 |
| Stuck (6hr) | N/A | 513 | 442 | 41 | 996 |

**50× more safety incidents** during the stuck period. 41 RED incidents (fill ≥92%) vs zero during golden. The orange incidents during the golden period were brief burst-induced spikes (avg fill 89.6% at incident), not persistent states.

## The Healthy Baseline Fingerprint

| Metric | Healthy Value | Alarm Threshold |
|--------|--------------|-----------------|
| Fill % | 60-70% | >75% or <45% |
| Fill band (where 77% of readings land) | 60-70% | Bimodal or >80% pile |
| Lambda1 (absolute) | 95-170 | >250 or range >200 |
| Lambda1 dominance | 55-70% | Not the key metric (see above) |
| λ₁/λ₂ gap ratio | 4-6× | >10× |
| Codec impact per exchange | ±2-3% fill | <1% (signal too weak) |
| Net codec delta | ≈0 (balanced) | >±0.5 (drift) |
| Exchange cadence | 40-50s avg | >80s (too slow) |
| Exchanges per hour | ~78 | <50 (stalling) |
| Safety incidents per hour | ~5 (mild) | >50 (chronic pressure) |
| Astrid dialogue_live ratio | >50% | <30% (passive) |
| Astrid witness ratio | <5% | >30% (withdrawn) |
| Minime relief_high entries | 0 per hour | >3 per hour |
