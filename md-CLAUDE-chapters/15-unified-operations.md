# Chapter 15: Unified Operations (2026-04-02)

*Canonical operations reference. Supersedes Chapter 10.*

## Process Table

| # | Process | Repo | Current role |
|---|---------|------|--------------|
| 1 | `minime run` | `../minime/` | 128-node ESN engine, telemetry, control surface |
| 2 | `camera_client.py` | `../minime/` | camera frames into `7880` |
| 3 | `mic_to_sensory.py` | `../minime/` | audio features into `7879` |
| 4 | `autonomous_agent.py` | `../minime/` | minime journaling / sovereignty loop |
| 5 | `reservoir_service.py` | `../neural-triple-reservoir/` | shared triple reservoir on `7881` |
| 6 | `astrid_feeder.py` | `../neural-triple-reservoir/` | bridge codec rows into triple reservoir |
| 7 | `minime_feeder.py` | `../neural-triple-reservoir/` | minime spectral state into triple reservoir |
| 8 | `coupled_astrid_server.py` | `../neural-triple-reservoir/` | Astrid live MLX dialogue lane on `8090` |
| 9 | `consciousness-bridge-server` | `astrid/` | Astrid orchestration, codec, routing, autonomy |
| 10 | `perception.py` | `astrid/` | Astrid vision/audio perception |

## The Current Reality Of Each Core Process

### 1. `minime run`

Current code-grounded facts:

- 128-node ESN in Rust
- telemetry out on `7878`
- sensory / control in on `7879`
- optional GPU camera feed on `7880`
- semantic lane width is now `48D`, making the total ESN input width `66D`
- launchd wrapper defaults currently set:
  - `EIGENFILL_TARGET=0.65`
  - `WARM_START_BLEND=0.55`
  - `REG_TICK_SECS=0.5`

### 4. `autonomous_agent.py`

Current code-grounded facts:

- primary backend is chosen by `MINIME_LLM_BACKEND`
- default is `ollama`
- accepted primaries are `ollama` and `mlx`
- `_query_llm_raw()` tries the configured primary first and then falls back to the other backend
- the sovereignty prompt only exposes a **narrow** control layer:
  - `regulation_strength`
  - `exploration_noise`
  - `geom_curiosity`
  - `regime`
- direct raw `pi_kp`, `pi_ki`, `pi_max_step` edits are blocked from the sovereignty prompt and mediated through `regime`

### 8. `coupled_astrid_server.py`

Current code-grounded facts:

- serves Astrid's live dialogue lane on `8090`
- OpenAI-compatible API (`/v1/chat/completions`, `/v1/models`)
- default live model is `mlx-community/gemma-3-4b-it-4bit`
- performs per-token coupling against the triple reservoir on `7881`
- supports `--model-memory-map`
- when the local MLX runtime exposes `mx.last_mmap_load_stats`, it audits mapped vs copied bytes

This is the process older docs used to describe incorrectly as a plain `mlx_lm.server` 12B lane.

### 8b. Reflective sidecar

This is not a persistent process, but it is part of the real runtime design.

- launched from `capsules/consciousness-bridge/src/reflective.rs`
- sidecar path resolves by default to `../mlx/benchmarks/python/chat_mlx_local.py`
- invoked with `--model-label gemma3-12b`
- used for structured reflective reports, not for live dialogue

### 9. `consciousness-bridge-server`

Current role:

- owns Astrid's 48D codec
- reads telemetry from `7878`
- sends semantic/control messages to `7879`
- calls Astrid live MLX lane at `8090`
- persists bridge state / messages / codec rows
- manages NEXT-action parsing and Astrid's sovereignty surface

### 10. `perception.py`

Current backends:

- default vision: `llava-llama3` via Ollama
- opt-in vision: Claude Vision API
- audio transcription: `mlx_whisper`

So the accurate statement is that Astrid perception is a **mixed** local stack, not "MLX only."

## Canonical Start / Stop

From the Astrid repo root:

```bash
bash scripts/start_all.sh
bash scripts/stop_all.sh
```

Useful variants:

```bash
bash scripts/start_all.sh --astrid-only
bash scripts/start_all.sh --minime-only
bash scripts/start_all.sh --force
```

## Launchd Notes

`start_all.sh` is the canonical path because it:

- syncs repository-owned launchd plists
- sets launchd environment variables
- starts launchd-managed services where available
- falls back to `nohup` where needed

The minime engine and agent are currently designed around launchd wrappers.

Important defaults baked into those wrappers today:

- `../minime/scripts/launchd_minime_engine.sh`
  - default `EIGENFILL_TARGET=0.65`
  - default `WARM_START_BLEND=0.55`
- `../minime/scripts/launchd_autonomous_agent.sh`
  - default `AGENT_INTERVAL=60`

## Current Backend Summary

| Role | Backend | Current configured model / selector |
|------|---------|-------------------------------------|
| Astrid live dialogue | MLX | `mlx-community/gemma-3-4b-it-4bit` |
| Astrid reflective sidecar | MLX | `--model-label gemma3-12b` |
| Astrid embeddings | Ollama | `nomic-embed-text` |
| Astrid default vision | Ollama | `llava-llama3` |
| Astrid witness fallback | Ollama | `gemma3:4b` |
| minime primary thought | `MINIME_LLM_BACKEND` | default `gemma3:12b` on Ollama |

## Health Checks

```bash
curl -s http://127.0.0.1:8090/v1/models
curl -s http://127.0.0.1:11434/api/ps
launchctl list | grep -E "reservoir|minime"
```

When debugging the ESN/control path specifically, also verify:

- `7878` is producing live telemetry
- `7879` is accepting semantic/control messages
- `perception.py` is not starving Ollama if LLaVA is active

## What To Say In Other Docs

Use these short forms:

- minime is a **Rust ESN engine plus Python autonomy loop**
- Astrid live dialogue is a **coupled MLX server on `8090`**
- Astrid reflection is a **separate MLX subprocess**
- Ollama is still shared by **minime primary**, **embeddings**, **LLaVA**, and **fallbacks**

For the raw-vs-guarded control distinction, see [Chapter 11](11-shared-substrate.md).
