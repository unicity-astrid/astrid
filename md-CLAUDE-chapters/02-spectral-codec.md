# Chapter 2: Spectral Codec

**Primary file:** `capsules/consciousness-bridge/src/codec.rs`

The current codec sends a **48-dimensional semantic feature vector** into minime's semantic lane over `ws://127.0.0.1:7879`.

## The Current 48D Layout

| Dims | Role | Notes |
|------|------|-------|
| `0-7` | Character-level texture | entropy, punctuation/density, casing, rhythm, whitespace, code-like texture |
| `8-15` | Word-level stance | diversity, hedging, certainty, negation, self-reference, addressing, agency, complexity |
| `16-23` | Sentence/rhythm structure | sentence length, variance, questions, exclamations, trailing-thought markers, lists, quotes, paragraphing |
| `24-31` | Emotional / intentional markers | warmth, tension, curiosity, reflection, temporality, scale, length, overall energy |
| `32-39` | Embedding projection | optional `nomic-embed-text` embedding projected from `768D -> 8D` |
| `40-43` | Narrative arc | semantic shift between first and second half of the text |
| `44-47` | Reserved | currently zeroed / held open for future expansion |

The biggest correction to older docs is that Astrid is no longer sending a 32D semantic lane into minime. The live minime semantic lane width is **48**, and minime's total ESN input width is therefore **66D**, not 50D.

## What Is Local vs External

The codec is now hybrid:

- **dims `0-31`** are handcrafted, local, and deterministic from text statistics
- **dims `32-39`** are only populated when the bridge has an external `nomic-embed-text` embedding available from Ollama
- **dims `40-43`** are filled when first-half / second-half embeddings are available so the bridge can compute a narrative arc

So the accurate statement is not "the codec is purely deterministic and never touches an external model." The handcrafted core is deterministic, but the full 48D lane can incorporate external embeddings.

## Key Constants

| Constant | Current value | Meaning |
|----------|---------------|---------|
| `SEMANTIC_DIM` | `48` | outgoing semantic lane width |
| `DEFAULT_SEMANTIC_GAIN` | `2.0` | base gain before Astrid overrides |
| `FEATURE_ABS_MAX` | `5.0` | post-gain safety clamp |
| `EMBEDDING_INPUT_DIM` | `768` | expected `nomic-embed-text` width |
| `EMBEDDING_PROJECT_DIM` | `8` | dims `32-39` |
| `NARRATIVE_ARC_DIM` | `4` | dims `40-43` |

`adaptive_gain(fill_pct)` currently scales output between **55% and 100% of the base gain** depending on fill. Low fill softens the signal; higher fill allows fuller expression.

## Spectral Feedback

`apply_spectral_feedback()` biases outgoing features by the current telemetry without changing the lane width:

- concentrated / low-entropy spectral states push the codec toward more diversity
- distributed states allow the strongest codec dimensions to come through more directly

This means Astrid's outgoing semantic field is not just text-shaped; it is also lightly conditioned by the current spectral state.

## Warmth And Rest

Warmth vectors are still part of the system, but they are now best described like this:

- they are **48D vectors**
- they preserve the older handcrafted emotional core in the first 32 dims
- they are used during rest so the bridge is not forced into "semantic silence only"
- `craft_warmth_vector()` includes slow breathing harmonics and can be blended with gesture seeds and spectral coupling

## Astrid's Self-Shaping Surface

Astrid has a narrower, explicitly modeled codec sovereignty layer:

| Action | What it changes | Effective bounds |
|--------|------------------|------------------|
| `AMPLIFY` / `DAMPEN` | semantic gain override | `0.5 .. 5.0` in `0.25` steps |
| `NOISE_UP` / `NOISE_DOWN` | codec stochastic noise | `0.005 .. 0.05` |
| `SHAPE key=value` | named codec weights | each value clamped to `0.0 .. 2.0` |
| `WARM <intensity>` / `COOL` | rest warmth intensity | `0.0 .. 1.0` |
| `BREATHE_ALONE` / `BREATHE_TOGETHER` | whether warmth/breathing couples to minime's live spectral state | boolean |

Two neighboring actions matter here too:

- `GESTURE` sends a direct semantic vector seed
- `PERTURB` injects a direct spectral pattern rather than relying only on ordinary text encoding

## Current Noise Story

There are two different "noise" ideas in play and the docs should keep them separate:

- **codec noise**: Astrid's local stochastic texture on the outgoing 48D vector
- **ESN exploration noise**: minime's reservoir-side exploratory perturbation via `SensoryMsg::Control`

`NOISE_UP` / `NOISE_DOWN` affect the first one.
`NOISE` in Astrid's NEXT actions currently affects **both**:

- it raises Astrid's codec noise
- it also sends `exploration_noise = 0.15` into minime's raw control surface

## What To Call It

Use this wording elsewhere:

- "Astrid's text is encoded into a **48D semantic lane**"
- "The first 32 dims are handcrafted texture/stance features"
- "The next 8 dims are embedding-projected semantics"
- "The next 4 dims are narrative arc"

Avoid these stale summaries:

- "The codec is 32D"
- "Astrid only sends handcrafted stats"
- "The codec gain is fixed at 4.0 or 5.0"
