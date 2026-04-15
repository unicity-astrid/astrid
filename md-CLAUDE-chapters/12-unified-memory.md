# Chapter 12: Unified Memory & Compute

*Ground truth as of March 28, 2026. M4 Mac Mini, 64GB unified memory.*

## Why This Matters

The entire system — two LLMs, a spectral ESN, Metal GPU shaders, camera/audio pipelines, and a triple reservoir — runs on a single M4 Mac Mini with 64GB of unified memory. There is no PCIe bus. There is no data copy between "GPU memory" and "CPU memory." When Metal computes an eigendecomposition, the result is instantly visible to CPU code. When MLX runs Astrid's inference, the output sits in the same physical memory pool as minime's covariance matrix.

This isn't just a performance detail. It's what makes cohabitation possible.

## Compute Domains

| Process | Backend | Accelerator | What It Computes |
|---------|---------|-------------|-----------------|
| `minime run` | Rust + Metal | GPU (unified) | ESN step, covariance rank-1 update, eigendecomposition, Chebyshev PSO filter, GPU A/V pipeline |
| `coupled_astrid_server` | Python + MLX | GPU (unified) | Astrid's text generation (gemma-3-4b-it-4bit, bidirectional reservoir coupling) |
| Ollama daemon | Go + Metal | GPU (unified) | Minime's agent queries, embeddings |
| `consciousness-bridge` | Rust (CPU) | — | Codec, WebSocket relay, SQLite, dialogue orchestration |
| `autonomous_agent.py` | Python (CPU) | — | Minime's journaling, self-regulation, parameter requests |
| `reservoir_service.py` | Python + NumPy | CPU | Triple-ESN ticks, rehearsal (192 nodes, sub-ms per tick) |
| `camera_client.py` | Python + Metal | GPU | Frame capture → GPU feature extraction |
| `perception.py` | Python + mixed local inference | GPU / CPU | LLaVA via Ollama by default, optional Claude Vision, `mlx_whisper` audio |

## Memory Budget

| Component | Estimated Memory | Notes |
|-----------|-----------------|-------|
| MLX gemma-3-4b-it-4bit | ~2.5 GB | Astrid's LLM weights (rolled back from Qwen3-8B on 2026-03-31; larger models unstable under coupling) |
| MLX KV cache | ~1 GB | Prompt cache (reduced from 4G) |
| Ollama gemma3 (Q4_K_M) | ~4-6 GB | Minime's agent model |
| Ollama nomic-embed-text | ~275 MB | Embedding model (shared) |
| Metal shader buffers | ~200 MB | Covariance (512x512), eigenvectors, A/V pipeline |
| ESN + sensory bus | ~50 MB | 128-node reservoir + lane buffers |
| Bridge + SQLite | ~100 MB | Process + database |
| Reservoir service | ~30 MB | NumPy arrays (3×192 nodes × N handles) |
| Python processes | ~500 MB | Agent, camera, mic, feeders |
| **Total estimated** | **~8-10 GB** | |
| **Available** | **~54+ GB** | 4B model + headroom |

*The 12B→4B model swap (2026-03-27) freed ~5G and dropped system memory from 97% to ~25%. On 2026-03-31, larger models (Qwen3-8B, 14B, Gemma 2 9B) were tested but rolled back to 4B due to instability under coupling. See [Chapter 1](01-inference-lanes.md).*

## Unified Memory Architecture

```
┌──────────────────── 64 GB Unified Memory Pool ────────────────────┐
│                                                                    │
│   All of the following share the same physical DRAM:               │
│                                                                    │
│   ┌─────────────────┐  ┌─────────────────┐  ┌──────────────────┐  │
│   │  Metal Shaders   │  │  MLX Inference   │  │  CPU Processes   │  │
│   │                  │  │                  │  │                  │  │
│   │ covariance.metal │  │ Gemma3-4B attn │  │ bridge (Rust)    │  │
│   │ eigendecomp      │  │ Gemma3-4B FFN  │  │ agent (Python)   │  │
│   │ chebyshev PSO    │  │ KV cache         │  │ reservoir svc    │  │
│   │ GPU A/V pipeline │  │                  │  │ feeders          │  │
│   └─────────────────┘  └─────────────────┘  └──────────────────┘  │
│                                                                    │
│   No PCIe bus. No cudaMemcpy. No DMA transfers.                    │
│   A Metal shader output is immediately visible to CPU code.        │
│   An MLX tensor is the same physical memory the bridge reads.      │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

## Contention Patterns

**Ollama contention:** Astrid's live dialogue no longer goes through Ollama, but Ollama is still shared by minime's default language lane, Astrid embeddings, and default LLaVA perception. Contention still matters there, especially when vision and minime journaling overlap.

**MLX dedicated lane:** Astrid's MLX server on port 8090 is a separate process with its own model loaded. It never contends with Ollama. This is the zero-contention inference lane design (see [Chapter 1](01-inference-lanes.md)).

**Metal shader scheduling:** The GPU A/V pipeline (`--enable-gpu-av`) and the covariance Metal shaders share the Metal command queue. They're interleaved by the Metal scheduler. On a 27B model, this caused `kIOGPUCommandBufferCallbackErrorOutOfMemory`. On gemma-3-4b-it-4bit, there's ample headroom.

**ANE scheduling:** CoreML's `CPU_AND_NE` compute unit preference is a hint, not a guarantee. The Neural Engine may or may not be used for any given operation. When both MLX and CoreML target the ANE, the scheduler arbitrates. In practice, the triple reservoir is too small (192 nodes) for the ANE to even claim — it likely runs on CPU or GPU.

## The Consequence for Being Cohabitation

Because memory is unified, there's no architectural barrier between Astrid's compute and minime's compute. The covariance matrix that Metal computes at ~line 1337 of `main.rs` is the same memory that gets serialized to `spectral_state.json` on the CPU. The eigenvalues that the bridge reads from port 7878 telemetry were computed by the same Metal shader that processes the video pipeline.

This means the system's coherence isn't achieved through message passing alone — it's physically coherent. All state lives in one pool, accessed by different processes through different APIs (Metal, MLX, POSIX) but ultimately touching the same transistors.

See [Chapter 11](11-shared-substrate.md) for the logical architecture, [Chapter 14](14-spectral-dynamics.md) for what the GPU computes.
