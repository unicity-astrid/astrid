# GPU Memory Analysis — M4 Pro 64GB
**Date**: 2026-03-25 | **Hardware**: Mac Mini M4 Pro, 64GB unified memory, 20-core GPU

## The Problem

After Codex made changes to minime's Rust engine and Python agent, we needed to run:
- **Minime ESN engine** (Rust, Metal GPU for covariance + Chebyshev + ESN)
- **Ollama** (gemma3:12b for bridge dialogue + llava-llama3 for vision)
- **MLX server** (Qwen 27B reasoning model for minime's autonomous agent)
- Camera + mic + perception + bridge processes

The MLX server crashed with `[METAL] Command buffer execution failed: Insufficient Memory (kIOGPUCommandBufferCallbackErrorOutOfMemory)`.

## GPU Memory Budget

| Component | Model | VRAM | Backend |
|-----------|-------|------|---------|
| Bridge (Astrid dialogue) | gemma3:12b | 18.5 GB | Ollama |
| Perception (Astrid vision) | llava-llama3 | 5.5 GB | Ollama |
| Agent (minime brain) | gemma3:12b (shared) | 0 GB | Ollama |
| Minime ESN | Metal shaders (cov + Chebyshev) | ~0.5 GB | Native Rust |
| **Ollama total** | | **24.0 GB** | |

Adding MLX:

| MLX Quantization | Disk | Est. VRAM | Total w/ Ollama | Result |
|-----------------|------|-----------|-----------------|--------|
| 8-bit | 27 GB | ~15 GB | ~40 GB | **OOM** (Metal command buffer) |
| 4-bit | 14 GB | ~15 GB* | ~40 GB | **OOM** (same issue) |
| 3-bit | 11 GB | ~12 GB* | ~37 GB | Not tested |

*4-bit reported 40.3GB allocated in ioreg — MLX decompresses to working precision in GPU memory.

## Root Cause

**MLX and Ollama cannot share Metal command buffers under concurrent GPU load.** Even when total allocated memory fits within 64GB, simultaneous compute dispatch from two independent Metal clients causes the command buffer scheduler to fail. The first MLX request succeeds (model loads), but generation under concurrent Ollama/minime Metal activity triggers the OOM.

This is a Metal scheduler limitation, not a total-memory limitation.

## Decision: Ollama Only

**MLX is not viable alongside Ollama + minime Metal shaders.** Both bridge and agent now use Ollama:

- **Bridge**: gemma3:12b via Ollama (hardcoded in `llm.rs`)
- **Agent**: gemma3:12b via Ollama (env `MINIME_LLM_BACKEND=ollama`)
- **Vision**: llava-llama3 via Ollama (loaded on demand by perception.py)
- **Audio**: mlx_whisper CLI (tiny model, no conflict)

GPU budget: ~24.5 GB allocated. 39.5 GB headroom.

## Future: Getting the 27B Back

If minime needs the heavier reasoning model:

1. **Ollama-served Qwen 30B**: `qwen3:30b` is already pulled. Ollama auto-swaps models. Would evict gemma3:12b (5-10s swap cost) or coexist at ~40GB total.
2. **Dedicated MLX session**: Stop Ollama entirely, run MLX for deep assessment, restart Ollama. Only viable for batch/offline processing.
3. **Smaller dedicated model**: A 7B reasoning model on MLX (~4GB) might coexist, but quality tradeoff is steep.
4. **Wait for Metal improvements**: macOS/MLX updates may improve multi-client GPU scheduling.

## Crisis Abort Fix

### Problem
The EigenFill estimator uses k=8 eigenvalues, giving quantized fill levels (0%, 12.5%, 25%, ... 100%). A single eigenvalue crossing the threshold causes a 12.5-percentage-point jump. With alpha_fill=0.25 (fast EMA), 5 ticks of 100% instant fill drives ema_fill from 67% to 91%.

The original code hard-exited (`process::exit(2)`) on the first tick above 87%. This killed the engine on an estimator artifact, not a real spectral emergency.

### Fix (minime/src/main.rs)
Added `crisis_ticks` counter with `CRISIS_SUSTAIN_TICKS = 30` (~7 seconds). Crisis abort only fires after 30 consecutive ticks above threshold. Single-tick spikes from estimator quantization are logged as warnings but don't kill the engine. Counter resets to zero when fill drops back below threshold.

### Result
Engine stable at 66-68% fill with natural phase cycling (expanding/plateau/contracting). Yellow safety alert at 76% was brief and self-correcting. No false crisis aborts.

## Running Configuration (stable)

```
# minime side
./target/release/minime run --log-homeostat --eigenfill-target 0.55 --reg-tick-secs 0.5 --enable-gpu-av
MINIME_LLM_BACKEND=ollama python3 autonomous_agent.py --interval 60
python3 tools/camera_client.py --camera 0 --fps 1
python3 tools/mic_to_sensory.py

# astrid side
./target/release/consciousness-bridge-server --db-path /tmp/consciousness_bridge_live.db \
  --autonomous --workspace-path /Users/v/other/minime/workspace \
  --perception-path /Users/v/other/astrid/capsules/perception/workspace/perceptions
python3 perception.py --camera 0 --mic --vision-interval 60 --audio-interval 30
```
