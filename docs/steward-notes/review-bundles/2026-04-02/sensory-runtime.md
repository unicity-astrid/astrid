# Sensory / Runtime Bundle

## Why This Bundle Exists

This bundle groups the operational and runtime hardening changes:

- host versus physical sensory fallback
- launchd-backed startup surfaces
- perception and visual source routing
- canonical restart and shutdown scripts
- runtime contention measurement for the mixed Ollama / MLX stack

## Included Patch Files

- `sensory-runtime.minime.patch`
- `sensory-runtime.astrid.patch`

## Minime Scope

- `README.md`
- `scripts/start.sh`
- `visual_frame_service.py`
- `docs/ollama_mlx_contention_benchmark.md`
- `launchd/com.minime.autonomous-agent.plist`
- `launchd/com.minime.camera-client.plist`
- `launchd/com.minime.engine.plist`
- `scripts/launchd_autonomous_agent.sh`
- `scripts/launchd_minime_engine.sh`
- `tests/test_ollama_mlx_contention_bench.py`
- `tools/ollama_mlx_contention_bench.py`

## Astrid Scope

- `capsules/perception/perception.py`
- `md-CLAUDE-chapters/10-operations.md`
- `md-CLAUDE-chapters/15-unified-operations.md`
- `scripts/start_all.sh`
- `scripts/stop_all.sh`
- `scripts/restart_minime_launchd.sh`

## Review Questions

- Do the startup and shutdown surfaces behave sensibly across launchd, PTY, and
  headless Codex contexts?
- Is the host-sensory fallback coherent with the visual/perception runtime and
  source freshness checks?
- Should the Ollama / MLX contention benchmark stay in this runtime bundle, or
  be split into a later performance-investigation bundle after core runtime
  review passes?
- Are the operations docs now truthful enough to act as canonical operator
  guidance?
