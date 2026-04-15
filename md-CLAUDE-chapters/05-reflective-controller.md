# Chapter 5: Reflective Controller

Two layers of reflective intelligence currently exist in Astrid's stack:

1. a fast Rust regime tracker that runs every exchange
2. a slower MLX sidecar subprocess used for deeper reflective passes

## Layer 1: `RegimeTracker`

**File:** `capsules/consciousness-bridge/src/reflective.rs`

This layer is pure Rust: no LLM, no subprocess, no network call.

It classifies the current spectral situation from:

- `fill_pct`
- short fill trajectory (`prev_fill`, `prev_prev_fill`)
- `lambda1_rel`

The current output labels are:

| Regime | Trigger shape |
|--------|----------------|
| `recovery` | critically low fill or low `lambda1_rel` at low fill |
| `escape` | sustained contraction while already low-fill |
| `consolidate` | repeated expansion into healthier fill |
| `sustain` | stable healthy band |
| `rebind` | high acceleration / basin shift |

The result is injected into Astrid's prompt context every exchange as a short explanatory string.

## Layer 2: MLX Reflective Sidecar

**Bridge wiring:** `capsules/consciousness-bridge/src/reflective.rs`

`query_sidecar()` launches a subprocess with the current spectral context:

```bash
python3 <sidecar> \
  --json \
  --hardware-profile m4-mini \
  --model-label gemma3-12b \
  --mode reflective \
  --architecture reservoir-fixed \
  --prompt "<spectral context>"
```

The sidecar path is resolved through `BridgePaths` and defaults to:

```text
../mlx/benchmarks/python/chat_mlx_local.py
```

That means the current reflective path is tied to the sibling local `mlx/` checkout rather than a generic system installation.

## What The Sidecar Returns

The bridge deserializes the subprocess output into `ReflectiveReport`.

Current structured fields include:

- `controller_regime`
- `controller_regime_reason`
- `observer_report`
- `change_report`
- `prompt_embedding_field`
- `reservoir_geometry`
- `condition_vector`
- `self_tuning`
- `text`
- `profiling`

This is the structured reflective surface Astrid currently gets, not just a prose blob.

## Model Wording

The safe wording in docs is:

- Astrid's **reflective** sidecar is invoked with `--model-label gemma3-12b`
- the bridge resolves the script path to the local sibling `mlx/` checkout
- `query_sidecar()` logs a model/load line from stderr when available

Avoid stronger claims unless you are re-verifying the runtime on that machine:

- exact wall-clock duration
- exact mapped model path on disk
- exact sidecar-internal architecture beyond what the sidecar itself reports

## Relationship To Astrid's Live Voice

The reflective sidecar is **not** the same thing as Astrid's live model lane.

- live dialogue: `8090`, `gemma-3-4b-it-4bit`, coupled server
- reflective sidecar: subprocess, `gemma3-12b` label, deeper structured report

The correct mental model is "two MLX roles," not "one Astrid model."
