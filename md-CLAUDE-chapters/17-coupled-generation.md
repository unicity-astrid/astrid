# Chapter 17: Coupled Generation

*Ground truth as of April 2, 2026. Verified against `../neural-triple-reservoir/coupled_astrid_server.py`, `../neural-triple-reservoir/mlx_reservoir.py`, and `capsules/consciousness-bridge/src/llm.rs`.*

This chapter describes Astrid's **live** dialogue lane, not her reflective sidecar.

## What Is Coupled

Astrid's live generation path currently works like this:

```text
bridge request
  -> coupled_astrid_server.py on 8090
  -> current reservoir handle pulled from 7881
  -> prompt prefill through MLX model
  -> per-token loop:
       token embedding
       -> 32D projection
       -> triple reservoir tick
       -> reservoir-aware logit modulation
  -> updated handle pushed back to 7881
  -> response returned to bridge
```

The live coupled server is currently the thing behind:

```text
http://127.0.0.1:8090/v1/chat/completions
```

## Current Live Model

The currently configured live model is:

```text
mlx-community/gemma-3-4b-it-4bit
```

That is the right answer to "what LLM Astrid has" for **live dialogue**.

It is not the same answer for reflection:

- live dialogue: `gemma-3-4b-it-4bit`
- reflective sidecar: `--model-label gemma3-12b`

## Reservoir Input Width Here Is Still 32D

Do not confuse Astrid's **48D codec into minime** with the **32D projection into the triple reservoir**.

These are different links in the system:

- Astrid -> minime ESN semantic lane: **48D**
- Astrid live coupled generation -> triple reservoir: **32D**

The coupled server's projection path is still a 32D reservoir input space and is configured by:

- `--input-dim 32` in `coupled_astrid_server.py`
- `ReservoirLogitProcessor` / embedding projection logic in `mlx_reservoir.py`

## OpenAI-Compatible Surface

The coupled server exposes:

- `POST /v1/chat/completions`
- `GET /v1/models`

The bridge treats it as a normal OpenAI-compatible backend. The coupling is internal to the server.

## Local MLX Runtime / Fork Wording

The accurate way to describe the MLX dependency is:

- the coupled server uses the local MLX runtime available in the environment
- it explicitly checks for `mx.last_mmap_load_stats`
- with `--model-memory-map`, it can audit mapped vs copied load behavior when the runtime supports it

That means "Astrid uses a local MLX checkout/fork" is reasonable shorthand, but the concrete code fact is the memory-map/audit hook and the sibling `mlx/` reflective tooling, not just the word "fork."

## Prompt Budget Correction

Older docs claimed the coupled path had a `MAX_PROMPT_CHARS = 6,000` safety net.

That is stale.

The current prompt budgeting that matters for live Astrid requests lives in `capsules/consciousness-bridge/src/llm.rs`:

- short budget: `32_000`
- medium budget: `24_000`
- deep budget: `16_000`
- hard `mlx_chat()` ceiling: `48_000`

So the correct description is:

- **bridge-side prompt assembly is now much roomier than the old 6k note**
- the coupled server itself speaks an OpenAI-compatible API and does not define the live bridge budget story on its own

## What The Server Is For

This server is only for Astrid's live coupled voice.

It is **not**:

- the reflective sidecar
- the embedding path
- the perception path
- minime's primary language path

Those are separate components and should be documented separately.
