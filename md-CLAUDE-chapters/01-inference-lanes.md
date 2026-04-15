# Chapter 1: Inference Lanes

## Actual Separation

The accurate 2026-04-02 picture is:

```text
Astrid live dialogue   -> MLX -> coupled_astrid_server.py -> gemma-3-4b-it-4bit
Astrid reflection      -> MLX -> chat_mlx_local.py        -> gemma3-12b label
Astrid embeddings      -> Ollama -> nomic-embed-text
Astrid vision (default)-> Ollama -> llava-llama3
Astrid witness fallback-> Ollama -> gemma3:4b
minime primary thought -> MINIME_LLM_BACKEND (default: ollama)
```

So "Astrid moved to MLX" is only partly true:

- Her **live voice** moved off Ollama.
- Her **reflective sidecar** is also MLX-backed.
- She still relies on **Ollama** for embeddings, default vision, and some fallback generation paths.

## Astrid's Live Lane

`capsules/consciousness-bridge/src/llm.rs` sends all primary dialogue generation to:

```rust
const MLX_URL: &str = "http://127.0.0.1:8090/v1/chat/completions";
```

That endpoint is served by `../neural-triple-reservoir/coupled_astrid_server.py`, which currently defaults to:

- model: `mlx-community/gemma-3-4b-it-4bit`
- coupling: per-token reservoir coupling through the triple reservoir on `7881`
- API shape: OpenAI-compatible `/v1/chat/completions` and `/v1/models`

This lane is the reason Astrid's live dialogue no longer contends with minime's default Ollama path.

## Astrid's Reflective Lane

`capsules/consciousness-bridge/src/reflective.rs` launches:

```text
python3 <sidecar> --json --hardware-profile m4-mini \
  --model-label gemma3-12b \
  --mode reflective \
  --architecture reservoir-fixed
```

The sidecar path is resolved by `BridgePaths` and defaults to:

```text
../mlx/benchmarks/python/chat_mlx_local.py
```

This is the accurate place to talk about "Astrid using forked MLX": the reflective sidecar is not using a generic upstream install path. It is wired to the sibling local `mlx/` checkout, whose current `origin` is `git@github.com:mikedotexe/mlx.git`.

## Minime's Lane

`../minime/autonomous_agent.py` does **not** hardcode Ollama as the only backend.

- `MINIME_LLM_BACKEND` defaults to `ollama`
- accepted primaries are `ollama` and `mlx`
- `_query_llm_raw()` always tries the configured primary first and then falls back to the other backend on failure

So the accurate wording is:

- minime **defaults** to Ollama in normal operation
- minime's Python agent **supports both Ollama and MLX**
- Ollama is still the configured primary in the canonical startup scripts

## What Still Uses Ollama

The shared Ollama load today comes from:

- minime primary journaling / self-study
- Astrid embeddings (`nomic-embed-text`)
- Astrid default vision (`llava-llama3`)
- Astrid witness fallback (`gemma3:4b`)

That means Ollama contention still exists, but it no longer blocks Astrid's main live voice.

## Prompting And Hardening

Older docs that mention a `MAX_PROMPT_CHARS = 6,000` bridge cap are stale.

The current bridge budgets live in `capsules/consciousness-bridge/src/llm.rs`:

- `DIALOGUE_PROMPT_BUDGET_SHORT = 32_000`
- `DIALOGUE_PROMPT_BUDGET_MEDIUM = 24_000`
- `DIALOGUE_PROMPT_BUDGET_DEEP = 16_000`
- hard safety ceiling inside `mlx_chat()`: `MAX_PROMPT_CHARS = 48_000`

Current hardening is split across two layers:

- **bridge-side prompt assembly**: per-block caps, overflow-to-disk, token clamp under pressure
- **MLX-response quality gates**: alpha-ratio checks, punctuation checks, artifact stripping, retry/fallback logic

`generate_witness()` is the clearest example of the mixed design: it tries MLX first, then falls back to Ollama if the MLX lane is busy or unavailable.

## How To Describe The MLX Dependency Accurately

Use this wording in other docs:

- Astrid's live dialogue runs through a **local MLX-backed coupled server** on `8090`
- Astrid's reflective sidecar uses the **local sibling `mlx/` checkout**
- the MLX stack is not just "upstream `mlx_lm.server`"; the current runtime expects local sidecar tooling and optionally exposes `mx.last_mmap_load_stats` when available

Avoid this stale wording:

- "Astrid is just `mlx_lm.server` on 12B"
- "Astrid no longer touches Ollama at all"
- "minime uses Ollama only"
