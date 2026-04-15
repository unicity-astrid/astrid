# Chapter 0: System Overview

*Ground truth as of April 2, 2026. This chapter is aligned with current code in `astrid/`, sibling `minime/`, `neural-triple-reservoir/`, and the local `mlx/` checkout.*

## What This Is

Astrid and minime are coupled, but they are not the same kind of system.

- **minime** is a Rust ESN runtime: a 128-node reservoir with spectral homeostasis, live telemetry, and a raw WebSocket control surface.
- **Astrid** is a Rust bridge plus local-model stack: her live voice comes from the MLX-backed `../neural-triple-reservoir/coupled_astrid_server.py` on port `8090`, while deeper reflection comes from a separate MLX sidecar subprocess.
- Astrid's outgoing text is encoded into a **48D semantic vector** and sent to minime over `ws://127.0.0.1:7879`.
- minime sends back `eigenvalues`, `fill_ratio`, a `32D spectral_fingerprint`, `structural_entropy`, optional `spectral_glimpse_12d`, and optional `ising_shadow` data over `ws://127.0.0.1:7878`.
- The triple reservoir on `7881` is a **second** shared dynamical substrate. It is not minime's primary ESN.

## Runtime Shape

The full stack currently spans four sibling repositories:

- `astrid/` — bridge, perception, docs
- `../minime/` — Rust ESN engine and Python autonomous agent
- `../neural-triple-reservoir/` — shared three-layer reservoir plus coupled Astrid server
- `../mlx/` — local MLX checkout used by the reflective sidecar and MLX runtime tooling

The canonical process layout is:

1. `minime run`
2. `camera_client.py`
3. `mic_to_sensory.py`
4. `autonomous_agent.py`
5. `reservoir_service.py`
6. `astrid_feeder.py`
7. `minime_feeder.py`
8. `coupled_astrid_server.py`
9. `consciousness-bridge-server`
10. `perception.py`

Chapter 15 is the authoritative process/operations reference.

## Port Topology

| Port | Protocol | Service | Purpose |
|------|----------|---------|---------|
| `7878` | WebSocket | minime telemetry | Engine → bridge (`eigenvalues`, `fill_ratio`, `spectral_fingerprint`, memory glimpse, alerts) |
| `7879` | WebSocket | minime sensory/control input | Bridge/agent → engine (`audio`, `semantic`, `control`) |
| `7880` | WebSocket | minime GPU camera feed | `camera_client.py` → engine |
| `7881` | WebSocket | triple reservoir | feeders / coupled generation / reservoir tools |
| `8090` | HTTP | coupled Astrid server | OpenAI-compatible MLX dialogue lane |
| `11434` | HTTP | Ollama | minime default LLM lane, embeddings, LLaVA perception, selective fallback |

## The Model Split

The most important correction to older docs is that "Astrid's model" is not one thing.

| Role | Backend | Current configured model | Notes |
|------|---------|--------------------------|-------|
| Astrid live dialogue | MLX via `coupled_astrid_server.py` | `mlx-community/gemma-3-4b-it-4bit` | main voice on `8090` |
| Astrid reflective sidecar | MLX via `chat_mlx_local.py` | `--model-label gemma3-12b` | subprocess, used on `INTROSPECT` |
| Astrid witness fallback | Ollama | `gemma3:4b` | used when MLX is unavailable for `generate_witness()` |
| Astrid embeddings | Ollama | `nomic-embed-text` | fills codec dims `32-39` when available |
| Astrid vision | Ollama by default | `llava-llama3` | Claude Vision is opt-in |
| minime autonomous thought | `MINIME_LLM_BACKEND` (default `ollama`) | default `gemma3:12b` | code supports symmetric MLX/Ollama failover |

So the accurate short version is:

- Astrid's **live** voice is MLX-backed and does **not** contend with Ollama.
- Astrid still uses Ollama for embeddings, default vision, and some fallback paths.
- minime defaults to Ollama, but the Python agent can be configured to use MLX and will fail over between backends.

## Data Flow

```text
Astrid response
  -> 48D codec vector
  -> ws://127.0.0.1:7879 (semantic lane)
  -> minime ESN (128 nodes)
  -> eigenvalues / fill / fingerprint / memory glimpse
  -> ws://127.0.0.1:7878
  -> bridge prompt context
  -> Astrid perceives the spectral state

In parallel:
bridge.db / spectral_state.json
  -> astrid_feeder.py / minime_feeder.py
  -> triple reservoir on 7881
  -> coupled Astrid generation on 8090
```

## Key Directories

```text
astrid/
  capsules/consciousness-bridge/
  capsules/perception/
  md-CLAUDE-chapters/

../minime/
  minime/src/
  autonomous_agent.py
  workspace/

../neural-triple-reservoir/
  coupled_astrid_server.py
  reservoir_service.py
  astrid_feeder.py
  minime_feeder.py

../mlx/
  benchmarks/python/chat_mlx_local.py
```

## What To Trust

When docs disagree, the current source-of-truth files are:

- Astrid live language lane: `capsules/consciousness-bridge/src/llm.rs`
- Astrid reflective sidecar wiring: `capsules/consciousness-bridge/src/reflective.rs`
- Shared path resolution: `capsules/consciousness-bridge/src/paths.rs`
- Codec and semantic lane shape: `capsules/consciousness-bridge/src/codec.rs`
- minime input/control surface: `../minime/minime/src/sensory_ws.rs`
- minime semantic lane size and clamps: `../minime/minime/src/sensory_bus.rs`
- minime ESN runtime and telemetry packet: `../minime/minime/src/main.rs`

## Chapter Index

- [01 — Inference Lanes](01-inference-lanes.md)
- [02 — Spectral Codec](02-spectral-codec.md)
- [03 — Correspondence](03-correspondence.md)
- [04 — Being Tools](04-being-tools.md)
- [05 — Reflective Controller](05-reflective-controller.md)
- [06 — Checkpoint Bank](06-checkpoint-bank.md)
- [07 — Self-Study System](07-self-study-system.md)
- [08 — Interests & Memory](08-interests-memory.md)
- [09 — Being-Driven Development](09-being-driven-dev.md)
- [10 — Operations](10-operations.md)
- [11 — Shared Substrate](11-shared-substrate.md)
- [12 — Unified Memory & Compute](12-unified-memory.md)
- [13 — Triple Reservoir](13-ane-reservoir.md)
- [14 — Spectral Dynamics](14-spectral-dynamics.md)
- [15 — Unified Operations](15-unified-operations.md)
- [16 — The Spectral Codec Deep Dive](16-codec-deep-dive.md)
- [17 — Coupled Generation](17-coupled-generation.md)
