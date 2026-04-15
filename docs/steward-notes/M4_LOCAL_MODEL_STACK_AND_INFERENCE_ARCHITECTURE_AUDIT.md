# M4 Local Model Stack and Inference Architecture Audit

Date: March 27, 2026

Checkout context: current live `/Users/v/other/astrid` and `/Users/v/other/minime` workspaces on the March 27, 2026 checkout, re-verified against current code, local runtime state, and current machine state before writing.

## Executive Summary

The current local model stack is **functional but under-coordinated**. It is not wildly overbuilt for a 64 GB Apple M4 Pro Mac mini, but it is carrying too many overlapping large-model roles, too many half-finished backend assumptions, and too many timeouts and unload rituals that compensate for shared-substrate contention.

The short verdict is:

- the machine is being used well at the **Metal + unified memory** level
- it is **not** yet being used especially intelligently at the **model orchestration** level
- the current live stack is closer to **"one strong always-on model plus several awkward alternates"** than a clean local-first architecture

The clearest oversized or badly assigned roles today are:

- Astrid `THINK_DEEP` using a **29 GB** Q8 reasoning model for inline deep-think on the same Ollama substrate as live dialogue and vision
- minime interactive chat still defaulting to **`qwen3:30b`** while the rest of the system has largely standardized on **`gemma3:12b`**
- minime interactive embeddings still pointing at **`mistral-small:24b`**, which is not even installed on the current machine

The clearest architecture strain signals are:

- manual `llava-llama3` unloads before bridge dialogue
- perception pausing itself to free Ollama
- long inline timeouts for deep reasoning and self-assessment
- an MLX split that exists in code and docs, but is currently **inactive in live runtime** because it conflicts with Ollama + minime's Metal workload

My recommendation for this machine is a **Balanced** local-first stack:

- one always-on medium model for live dialogue and autonomous steady-state work
- one on-demand deep model, not two competing deep lanes
- one standardized embedding model across both projects
- vision on a short-duty-cycle, on-demand path
- MLX removed from concurrent live use and reserved for offline or serialized deep work

## Evidence Labels

- Observed in current code: directly verified in the current source tree
- Observed in current runtime artifacts: directly verified on the current machine or in current workspace files
- Inferred from evidence: a conclusion drawn from multiple observed facts
- Suggested follow-up changes: design or implementation suggestions, not current behavior

## Hardware and Runtime Snapshot

Observed in current runtime artifacts:

- `system_profiler SPHardwareDataType` reports:
  - Model: `Mac mini`
  - Chip: `Apple M4 Pro`
  - Memory: `64 GB`
  - CPU cores: `14`
- `lsof -nP -iTCP:11434 -iTCP:8090 -iTCP:8091 -sTCP:LISTEN` shows:
  - Ollama listening on `127.0.0.1:11434`
  - nothing listening on `8090`
  - nothing listening on `8091`
- `ps aux | rg "ollama serve|ollama runner|mlx|8090|8091"` shows:
  - `ollama serve` running
  - active Ollama runner processes
  - no live MLX chat server process
  - no live MLX vision server process
- `ollama list` on March 27, 2026 shows these installed models:
  - `nomic-embed-text:latest` (`274 MB`)
  - `gemma3:27b` (`17 GB`)
  - `gemma3:12b` (`8.1 GB`)
  - `qwen3:30b` (`18 GB`)
  - `llava-llama3:latest` (`5.5 GB`)
  - `hf.co/mradermacher/Qwen3.5-27B-Claude-4.6-Opus-Reasoning-Distilled-GGUF:Q8_0` (`29 GB`)
- `ollama ps` during this audit shows:
  - `gemma3:12b` loaded on GPU
  - no active deep reasoning model loaded
  - no active LLaVA model loaded at that instant

Inferred from evidence:

- the live machine is currently operating as an **Ollama-first runtime**
- MLX remains **configured in code** but **inactive in live runtime**
- the machine is using **unified memory and Metal**, but not currently using a real **Core ML / Neural Engine inference path**

## What the Machine Is Actually Using

Observed in current code:

- Astrid bridge dialogue, embeddings, and vision all use Ollama-centered HTTP calls in:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs`
  - `/Users/v/other/astrid/capsules/perception/perception.py`
- minime autonomous and interactive text/vision paths support Ollama and optional MLX in:
  - `/Users/v/other/minime/autonomous_agent.py`
  - `/Users/v/other/minime/mikemind/llm_engine.py`
  - `/Users/v/other/minime/mikemind/vision.py`
- minime's spectral engine uses native Rust + Metal shaders in:
  - `/Users/v/other/minime/minime/src/main.rs`
  - `/Users/v/other/minime/minime/src/esn.rs`

Observed in current code and runtime artifacts:

- the system is genuinely exploiting:
  - Metal-backed local inference via Ollama
  - unified memory on Apple Silicon
  - Rust-native GPU compute for minime's ESN
  - `mlx_whisper` CLI for speech-to-text
- a repo-wide scan for `CoreML`, `coreml`, `Neural Engine`, `ANE`, `mlpackage`, and `mlmodel` found **no actual Core ML / ANE runtime path** in the inspected code

Inferred from evidence:

- Apple Silicon is being used well as a **single shared GPU + unified memory substrate**
- it is **not** currently being used as a **multi-backend Apple AI stack**
- any future ANE story would require a real redesign around **Core ML-converted models**, not just a config toggle

## Current Model Inventory

### Installed Models on This Machine

Observed in current runtime artifacts:

- `gemma3:12b`
- `gemma3:27b`
- `qwen3:30b`
- `llava-llama3`
- `nomic-embed-text`
- `Qwen3.5-27B-Claude-4.6-Opus-Reasoning-Distilled-GGUF:Q8_0`

### Astrid Bridge Model Roles

Observed in current code:

- Fast dialogue model:
  - `const MODEL: &str = "gemma3:12b";`
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:16`
- Deep reasoning model:
  - `const REASONING_MODEL: &str = "hf.co/mradermacher/Qwen3.5-27B-Claude-4.6-Opus-Reasoning-Distilled-GGUF:Q8_0";`
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:18`
- Embedding model:
  - `const EMBED_MODEL: &str = "nomic-embed-text";`
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:360`
- Vision model:
  - `LLAVA_MODEL = "llava-llama3"`
  - `/Users/v/other/astrid/capsules/perception/perception.py:61`
- Speech-to-text:
  - `mlx_whisper`
  - `/Users/v/other/astrid/capsules/perception/perception.py:280-283`

### Minime Autonomous-Agent Roles

Observed in current code:

- Default backend:
  - `LLM_BACKEND = os.environ.get("MINIME_LLM_BACKEND", "ollama")`
  - `/Users/v/other/minime/autonomous_agent.py:42`
- Optional MLX chat endpoint:
  - `MLX_URL = "http://localhost:8090/v1/chat/completions"`
  - `/Users/v/other/minime/autonomous_agent.py:43`
- Default model:
  - `MODEL = os.environ.get("MINIME_MODEL", "gemma3:12b")`
  - `/Users/v/other/minime/autonomous_agent.py:46`

Inferred from evidence:

- minime autonomous is now conceptually aligned with Astrid on `gemma3:12b`
- the MLX path still exists, but it is not the live default and not active on this machine right now

### Minime Interactive Roles

Observed in current code:

- Conversation default:
  - `qwen3:30b`
  - `/Users/v/other/minime/mikemind/config.py:45`
- Vision default:
  - `llava-llama3`
  - `/Users/v/other/minime/mikemind/config.py:49`
- MLX chat endpoint:
  - `http://localhost:8090/v1/chat/completions`
  - `/Users/v/other/minime/mikemind/config.py:57-60`
- MLX vision endpoint:
  - `http://localhost:8091/v1/chat/completions`
  - `/Users/v/other/minime/mikemind/config.py:57-60`
- Ollama embedding helper default:
  - `model: str = "mistral-small:24b"`
  - `/Users/v/other/minime/mikemind/config.py:107`

Observed in current runtime artifacts:

- `mistral-small:24b` is **not installed** on the machine
- no MLX server is listening on `8090`
- no MLX vision server is listening on `8091`

Inferred from evidence:

- minime interactive still reflects an older, heavier, more experimental architecture than the currently working live stack
- the interactive embedding helper is presently a **stale or badly assigned role**
- the model inventory is not just large, but **conceptually split across different eras of the project**

### Installed vs Configured vs Active

The cleanest way to describe the current stack is:

- Installed:
  - multiple medium and large text models
  - one large vision model
  - one dedicated embedding model
- Configured defaults:
  - Astrid bridge: `gemma3:12b` + 29 GB reasoning model + `nomic-embed-text` + `llava-llama3`
  - minime autonomous: `gemma3:12b` with optional MLX
  - minime interactive: `qwen3:30b` + `llava-llama3` + MLX-first fallback logic + stale embedding helper
- Active right now:
  - Ollama on `11434`
  - `gemma3:12b` loaded
  - no MLX chat server
  - no MLX vision server

## Active Interfaces and Runtime Endpoints

Observed in current code and runtime artifacts:

- Ollama:
  - `11434`
  - live and listening
- MLX chat:
  - `8090`
  - configured in code, not listening right now
- MLX vision:
  - `8091`
  - configured in code, not listening right now
- Astrid bridge model-role assignment and unload behavior:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs`
- minime chat, vision, and embeddings backend-selection surfaces:
  - `/Users/v/other/minime/autonomous_agent.py`
  - `/Users/v/other/minime/mikemind/config.py`
  - `/Users/v/other/minime/mikemind/llm_engine.py`
  - `/Users/v/other/minime/mikemind/vision.py`
- current MLX/Ollama memory-conflict note:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/GPU_MEMORY_ANALYSIS.md`

## Current Residency and Scheduler Posture

Observed in current runtime artifacts:

- `curl http://127.0.0.1:11434/api/ps` shows the currently loaded `gemma3:12b` with:
  - `size_vram`: about `18.5 GB`
  - `context_length`: `131072`
- `launchctl getenv` shows no explicit values set for:
  - `OLLAMA_KEEP_ALIVE`
  - `OLLAMA_CONTEXT_LENGTH`
  - `OLLAMA_MAX_LOADED_MODELS`
  - `OLLAMA_NUM_PARALLEL`
  - `OLLAMA_MAX_QUEUE`
  - `OLLAMA_FLASH_ATTENTION`
  - `OLLAMA_KV_CACHE_TYPE`

Observed in current code:

- the bridge manually unloads `llava-llama3` and `nomic-embed-text` before dialogue, but does not set an explicit `num_ctx` for dialogue requests in:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:262-277`
- bridge generation paths generally do not pass a `keep_alive` policy for steady-state text generation; they rely on server behavior plus explicit unloads for selected models
- minime interactive chat, streaming, vision, and even some spontaneous-thought paths set `keep_alive: "1h"` in:
  - `/Users/v/other/minime/mikemind/llm_engine.py:248-256`
  - `/Users/v/other/minime/mikemind/llm_engine.py:299-315`
  - `/Users/v/other/minime/mikemind/vision.py:68-69`
  - `/Users/v/other/minime/mikemind/vision.py:166-175`
  - `/Users/v/other/minime/mikemind/mind.py:1908-1921`
  - `/Users/v/other/minime/mikemind/mind.py:1987-1999`

Observed in external sources:

- Ollama documents `keep_alive` as an explicit residency control on `/api/chat` and `/api/generate`, including `0` to unload immediately.
- Ollama documents that models are kept in memory for `5 minutes` by default unless overridden.
- Ollama also documents concurrency controls:
  - `OLLAMA_MAX_LOADED_MODELS`
  - `OLLAMA_NUM_PARALLEL`
  - `OLLAMA_MAX_QUEUE`
- Ollama's FAQ explicitly notes that required RAM scales by:
  - `OLLAMA_NUM_PARALLEL * OLLAMA_CONTEXT_LENGTH`
- Ollama's context-length docs state that context length directly increases memory use and can be set globally or per request with `num_ctx`.

Inferred from evidence:

- the current stack is doing most of its scheduler work in application code, not in the inference server
- that is why the system has:
  - manual unloads
  - perception pause flags
  - long `keep_alive` values
  - large inherited context
  - multiple timeout layers
- the current live architecture is therefore **underconfigured at the server-policy layer and overcompensating at the application layer**

### Context Budget Mismatch

Observed in current runtime artifacts:

- live Ollama residency currently shows `context_length: 131072`

Observed in current code:

- Astrid still trims major prompt components aggressively:
  - recent history compressed to `80` and `200` characters in `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:187-220`
  - current-turn journal trimmed to `300` characters in `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:226`
  - local continuity and supporting blocks are also frequently clipped to a few hundred or low-thousands of characters across `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs`
- a repo-wide scan found no live inference path explicitly setting `options.num_ctx` for steady-state requests

Observed in external sources:

- Ollama's docs describe context length as a first-class memory policy and note that higher settings cost more memory
- Ollama also recommends larger contexts for agentic and coding tasks, but not as a claim that every live request should inherit the largest available context by default

Inferred from evidence:

- the current system may be paying for a very large context budget while still constructing heavily trimmed prompts
- this does **not** prove that the context should simply be lowered
- it **does** show that context policy is currently implicit and deserves to become explicit

Suggested follow-up changes:

- define at least two explicit `num_ctx` tiers:
  - steady-state dialogue / autonomous reflection
  - deep reasoning / coding / replay work
- stop inheriting whatever global Ollama context happens to be configured in the app

## Timeout, Retry, and Model-Swap Audit

### Astrid Bridge

Observed in current code:

- dialogue client timeout is `30s` by default and `60s` for reasoning or long generations in:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:257`
- bridge dialogue explicitly unloads `llava-llama3`, unloads `nomic-embed-text`, waits, and then warms the target model in:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:262-275`
- `THINK_DEEP` wraps dialogue generation in a `60s` outer timeout and retries once after a `3s` wait in:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1729-1765`
- other bridge generation paths have their own outer timeouts:
  - witness `30s`
  - daydream `25s`
  - aspiration `25s`
  - moment `20s`
  - create `45s`
  - initiate `45s`
  - introspection / evolve `60s`
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1890-2315`

Inferred from evidence:

- the bridge is not using timeouts as simple product choices
- it is using timeouts, unloads, and retries as a **concurrency-management strategy**
- the manual unload + warmup logic is one of the clearest signals that the local model stack is fighting itself

### Astrid Perception

Observed in current code:

- perception uses Ollama LLaVA with `30s` and `60s` request timeouts in:
  - `/Users/v/other/astrid/capsules/perception/perception.py:125`
  - `/Users/v/other/astrid/capsules/perception/perception.py:155`
- perception comments explicitly say it skips LLaVA and whisper while the pause flag is present to free Ollama for dialogue

Inferred from evidence:

- perception is not a clean parallel subsystem
- it is an opportunistic subsystem that yields to text generation because the model stack cannot comfortably keep both active

### Minime Autonomous-Agent

Observed in current code:

- autonomous self-assessment interval was widened to `3600` seconds because the reasoning model takes too long:
  - `/Users/v/other/minime/autonomous_agent.py:92`
- assessment calls use `120s` timeouts for both MLX and Ollama branches:
  - `/Users/v/other/minime/autonomous_agent.py:1125-1153`
- MLX query helper uses a `60s` timeout and explicitly says "fail fast, retry next cycle":
  - `/Users/v/other/minime/autonomous_agent.py:2751`
  - `/Users/v/other/minime/autonomous_agent.py:2780`

Inferred from evidence:

- minime autonomous still carries a background assumption that deep reflection is expensive enough to be infrequent
- that is reasonable on this hardware, but it also means the architecture has not really separated steady-state cognition from deep work

### Minime Interactive

Observed in current code:

- interactive LLM availability probes use `keep_alive: "1h"` and `30s` timeouts in:
  - `/Users/v/other/minime/mikemind/llm_engine.py:54-56`
- MLX generation paths use `120s` and `60s` timeouts in:
  - `/Users/v/other/minime/mikemind/llm_engine.py:167`
  - `/Users/v/other/minime/mikemind/llm_engine.py:195`
- direct Ollama generation uses `keep_alive: "1h"` and `120s` timeout in:
  - `/Users/v/other/minime/mikemind/llm_engine.py:248-256`
- vision availability probing also uses `keep_alive: "1h"` and a real generation call in:
  - `/Users/v/other/minime/mikemind/vision.py:68-69`

Inferred from evidence:

- `keep_alive: "1h"` makes sense only if long GPU residency is desirable
- on this machine, with several large models and minime's own Metal workload, that choice increases the chance of unnecessary residency pressure
- some of the "availability checks" are expensive enough that they feel like architecture leakage, not lightweight health checks

## Current Architecture Tensions

### 1. Too Many Large Models for Overlapping Roles

Observed in current code and runtime artifacts:

- `gemma3:12b` is the actual working steady-state model for Astrid bridge and minime autonomous
- `qwen3:30b` is still configured as minime interactive conversation default
- the bridge also has a separate 29 GB Q8 reasoning model for `THINK_DEEP`
- `gemma3:27b` is installed but does not appear to own a clear live role

Inferred from evidence:

- the stack has drifted into a "collector's shelf" of plausible local models rather than a disciplined role assignment
- that is normal in an exploratory phase, but it now imposes real orchestration cost

### 2. The Ollama / MLX Split Is Not Coherent in Live Use

Observed in current code and docs:

- both projects still carry MLX endpoints in code
- current docs in `/Users/v/other/minime/docs/mlx_integration_audit.md` describe MLX as partially working and partially missing
- current GPU analysis in `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/GPU_MEMORY_ANALYSIS.md` explicitly says MLX is not viable alongside Ollama + minime Metal shaders under concurrent load

Observed in current runtime artifacts:

- no MLX listener is live on `8090`
- no MLX listener is live on `8091`

Inferred from evidence:

- the current architecture is effectively **Ollama-first**, with MLX as an unfinished or offline-only side path
- that means the code and docs still overstate the practical coherence of the split

### 3. Vision and Embeddings Share the Same Inference Substrate as Live Dialogue

Observed in current code:

- bridge dialogue unloads vision and embeddings before chatting
- perception pauses itself to free Ollama
- embeddings and vision are not treated as dedicated sidecars

Inferred from evidence:

- the current system uses one shared inference lane for too many latency-sensitive tasks
- the result is more "subsystem politeness" than true parallel architecture

### 4. Some Model Roles Are Simply Bad Fits

Observed in current code and runtime artifacts:

- minime interactive embedding helper points to `mistral-small:24b`
- that model is not installed
- the bridge already uses `nomic-embed-text`, which is much more appropriate for embeddings and already installed

Inferred from evidence:

- this is not just a stale config
- it is a clear example of a role that is both oversized and conceptually misassigned

### 5. Some Timeouts Are Hiding Architecture Strain

Observed in current code:

- the bridge uses unload + warmup + retry around dialogue
- autonomous self-assessment interval is stretched because reasoning is too slow
- MLX and interactive paths use long timeouts and fallback logic

Inferred from evidence:

- several timeout layers exist because deep and live work are still competing on the same local substrate
- that feels forced, not natural

### 6. The Server Itself Is Underused as a Scheduling Surface

Observed in current runtime artifacts:

- no explicit Ollama concurrency envs are currently set in the live macOS launch environment

Observed in external sources:

- Ollama already exposes the primitives needed to shape this:
  - max loaded models
  - parallel requests per model
  - queue depth
  - keep-alive
  - context length

Inferred from evidence:

- the current stack is manually simulating a "single active lane" architecture without declaring it at the server layer
- that mismatch is probably one reason the code feels full of bespoke unloads and courtesy pauses

## What This M4 Pro Machine Is Actually Good At

Observed in current code and runtime artifacts:

- one strong medium model can remain resident and responsive
- Ollama can run comfortably alongside the Rust ESN when model churn is controlled
- minime's native Metal pipeline is already a good use of the machine
- `mlx_whisper` as a CLI sidecar is a pragmatic Apple Silicon fit

Inferred from evidence:

- the machine is best suited to:
  - one always-on text model
  - one small embedding model
  - one on-demand vision model
  - one occasional deep-reasoning lane
- the machine is **not** best suited to:
  - keeping multiple large text models warm
  - mixing concurrent Ollama and MLX heavy generation
  - pretending 30B-class models are cheap enough to be routine background utilities

### What Is Not Being Used Yet

Observed in current code:

- there is no real Core ML / ANE inference path in the inspected stack

Suggested follow-up changes:

- treat ANE as a future redesign path
- if pursued, start with:
  - speech-to-text
  - embeddings
  - lightweight vision
- do not begin with the deepest chat model; begin with the tasks that actually benefit from dedicated small converted models

## Candidate Better-Fit Configurations

### Conservative

Keep the current Ollama-first architecture, but remove obvious mismatch and strain.

- Fast dialogue:
  - `gemma3:12b`
- Deep reasoning:
  - keep current 29 GB Q8 reasoning model, but use it only on explicit demand
- Vision:
  - `llava-llama3` on demand only
- Embeddings:
  - standardize on `nomic-embed-text`
- MLX:
  - out of the live runtime by default
- Always-on:
  - `gemma3:12b`
  - `nomic-embed-text` only if truly needed frequently
- On-demand:
  - `llava-llama3`
  - deep reasoning model

Suggested server posture:

- `OLLAMA_MAX_LOADED_MODELS=1`
- `OLLAMA_NUM_PARALLEL=1`
- modest global keep-alive
- explicit unload for heavyweight vision only
- explicit `num_ctx` tiers instead of one inherited global maximum

Tradeoffs:

- lowest churn
- preserves current bridge behavior
- does not fix the larger conceptual split between Astrid, minime autonomous, and minime interactive

### Balanced (Recommended)

Unify the live stack around one medium text model and one explicit deep lane.

- Fast dialogue:
  - `gemma3:12b`
- Deep reasoning:
  - choose **one** of:
    - `qwen3:30b`
    - current 29 GB Q8 reasoning model
  - do not keep both as first-class deep lanes
- Vision:
  - `llava-llama3` on short-duty-cycle demand
  - optionally test `moondream:latest` as a lower-cost perception tier
- Embeddings:
  - `nomic-embed-text` everywhere
- MLX:
  - reserved for offline or serialized deep work only
- Always-on:
  - `gemma3:12b`
- On-demand:
  - one deep model
  - one vision model
  - speech sidecars

Suggested server posture:

- `OLLAMA_MAX_LOADED_MODELS=1` for the normal live runtime
- `OLLAMA_NUM_PARALLEL=1` unless a measured workload proves parallel inference helps more than it hurts
- `OLLAMA_MAX_QUEUE` set deliberately lower than the broad default so overload fails clearly instead of silently piling up
- explicit `num_ctx` for:
  - live dialogue
  - perception
  - deep work
- remove broad `keep_alive: "1h"` from interactive probes and low-value generation paths
- keep heavyweight perception unloaded except when active

Tradeoffs:

- best fit for daily use on this machine
- reduces swap pressure and conceptual fragmentation
- preserves room for deep reasoning without pretending it is cheap

Why this is the recommended target:

- it matches the current machine's real strengths
- it matches the current live runtime more honestly than the codebase's older MLX assumptions
- it reduces role drift without forcing a total redesign

### Bold

Split the system into a live lane and a batch lane.

- Live lane:
  - `gemma3:12b` via Ollama for dialogue, autonomous work, and quick reflection
- Batch lane:
  - deep reasoning via either:
    - serialized Ollama deep model session
    - dedicated offline MLX session with Ollama stopped
- Vision:
  - lightweight model or scheduled snapshots first
  - heavyweight VLM only when explicitly needed
- Embeddings:
  - `nomic-embed-text` or future Core ML embedding path
- MLX:
  - reintroduced only as a **batch lane**, not as a coequal always-on backend
- Always-on:
  - live lane only
- On-demand:
  - batch reasoning lane
  - heavyweight vision lane

Suggested server posture:

- keep Ollama deliberately small and predictable
- move big-context experiments and prompt-cached deep work into the MLX batch lane
- make residency policy a declared configuration artifact, not a scattered convention

Tradeoffs:

- strongest architecture clarity
- biggest workflow change
- best long-term path if the team wants deeper local intelligence without constant live contention

## Recommended Target Stack

The recommended target for this M4 Pro Mac mini is the **Balanced** stack:

1. Keep `gemma3:12b` as the shared live text model for Astrid bridge and minime autonomous.
2. Retire the current "many deep models" posture and keep only one deep model active in the architecture.
3. Standardize all embeddings on `nomic-embed-text`.
4. Treat `llava-llama3` as an on-demand perception model, not a resident companion.
5. Treat MLX as offline-only until the architecture deliberately supports a separate batch lane.

If I had to choose a single immediate simplification, it would be:

- keep `gemma3:12b`
- keep `nomic-embed-text`
- keep `llava-llama3` on demand
- evaluate whether `qwen3:30b` can replace the current 29 GB Q8 reasoning model as the one deep lane
- remove or rewrite stale `mistral-small:24b` embedding assumptions

## Novel Redesign Ideas

### 1. Separate Live and Batch Inference Lanes

Inferred from evidence:

- many current timeouts are caused by trying to do live dialogue, deep reflection, vision, and embeddings on one shared lane

Suggested follow-up changes:

- define:
  - a live lane for dialogue and low-latency work
  - a batch lane for deep introspection, replay, and heavy analysis

### 1a. Make Residency Policy Explicit

Suggested follow-up changes:

- create a simple local policy matrix:
  - `resident`
  - `warmable`
  - `batch-only`
  - `unload-immediately`
- classify each current model role into that matrix
- stop encoding residency policy indirectly through:
  - long `keep_alive`
  - ad hoc unload calls
  - hidden server defaults
  - caller-specific timeouts

### 2. Move More Work Out of Models

Suggested follow-up changes:

- move routine summarization, trimming, routing, and continuity assembly further toward Rust or symbolic preprocessing
- reserve expensive model calls for actual reasoning, authorship, and interpretation

### 3. Prefer Specialized Small Models Over One Huge Generalist

Suggested follow-up changes:

- embeddings should use embedding models
- speech should use speech models
- lightweight perception should use a lighter VLM where acceptable
- only deep reflective work should touch the biggest text model

### 4. Treat Core ML / ANE as a Real Future Project, Not a Vibe

Suggested follow-up changes:

- if the team wants ANE usage, scope it explicitly:
  - pick one narrow task
  - convert or adopt a Core ML model for that task
  - prove deployment and quality
- do not keep speaking about ANE as if the current stack already benefits from it

## External Resources and Supplemental Thoughts

The following external sources materially strengthen the audit rather than just echo it:

- [Ollama API docs](https://docs.ollama.com/api/introduction) and the mirrored API reference at [readthedocs](https://ollama.readthedocs.io/en/api/)
- [Ollama chat API docs](https://docs.ollama.com/api/chat)
- [Ollama generate API docs](https://docs.ollama.com/api/generate)
- [Ollama context length docs](https://docs.ollama.com/context-length)
- [Ollama FAQ](https://docs.ollama.com/faq)
- [MLX unified memory documentation](https://ml-explore.github.io/mlx/build/html/usage/unified_memory.html)
- [MLX GitHub README](https://github.com/ml-explore/mlx)
- [MLX LM GitHub README](https://github.com/ml-explore/mlx-lm)
- [Apple Core ML overview](https://developer.apple.com/machine-learning/core-ml/)
- [Apple Core ML framework docs](https://developer.apple.com/documentation/CoreML)
- [Core ML Tools overview](https://apple.github.io/coremltools/docs-guides/source/overview-coremltools.html)
- [Core ML Tools conversion options](https://apple.github.io/coremltools/docs-guides/source/new-conversion-options.html)
- [Apple Mac mini technical specifications](https://www.apple.com/mac-mini/specs/)

### What these sources buttress

Observed in external sources:

- Ollama documents `keep_alive` as a residency control and says the `/api/chat` default is `5m`; it also documents explicit load and unload calls via empty chat/generate requests and `keep_alive: 0`.
- Ollama documents context length as a first-class runtime memory policy and documents per-request `num_ctx` as well as server-wide context settings.
- Ollama documents concurrency controls for loaded-model count, parallelism, and queue depth, and explicitly notes that RAM scales with parallelism times context length.
- MLX documents that Apple silicon uses unified memory and that MLX arrays live in shared memory rather than being copied between CPU and GPU device spaces.
- MLX describes itself as an Apple-silicon-first framework with dynamic graphs, lazy computation, and CPU/GPU execution on shared memory.
- MLX LM explicitly warns that large models can be slow relative to machine RAM and may require wired-memory tuning on macOS 15+, which reinforces that "MLX exists" is not the same as "MLX is a comfortable always-on concurrent runtime."
- Apple describes Core ML as the framework that leverages CPU, GPU, and Neural Engine, and specifically describes converting outside models into Core ML format with Core ML Tools.
- Apple’s Core ML materials also describe profiling with Core ML and Neural Engine instruments, which is further evidence that ANE is part of the Core ML toolchain, not something this stack reaches automatically.

Inferred from external and local evidence together:

- the current audit’s reading of `keep_alive: "1h"` is strengthened: this is not an incidental flag, it is an explicit residency policy, so using it broadly really does shape memory pressure and swap behavior
- the current skepticism about live MLX concurrency is strengthened: MLX is a very natural fit for Apple silicon in principle, but the official MLX materials emphasize shared-memory execution and large-model tuning, which fits the local observation that it shines more as a deliberate Apple-native lane than as a casually concurrent peer to Ollama plus minime’s own Metal work
- the current scheduler critique is strengthened: Ollama already provides the server-side levers to shape residency and concurrency, so the present reliance on bespoke unload etiquette looks more like missing policy than missing capability
- the current ANE assessment is strengthened: saying "we should use the Neural Engine" is incomplete unless it is accompanied by "through which Core ML model format, conversion path, runtime wrapper, and task boundary?"

### Supplemental Thought: The Better Split May Be Ollama + Core ML + Offline MLX

The external sources suggest a slightly sharper architecture than the one in the original audit.

The first version of the recommendation said:

- Ollama for live work
- maybe MLX later
- ANE only as a future redesign

After reading the Apple and MLX materials, I would sharpen that into:

1. **Ollama for the live conversational lane**
   - This remains the most practical always-on text runtime in the current stack.

2. **Core ML for narrow, appliance-like utility lanes**
   - If the team wants real Neural Engine usage, the best first candidates are not the deepest chat models.
   - Better first candidates are:
     - embeddings
     - speech
     - lightweight perception
   - Those tasks are narrower, easier to benchmark, and more plausible to convert or replace with Core ML-native assets.

3. **MLX for offline or serialized deep work**
   - The MLX and MLX LM docs make MLX look strongest as a deliberate Apple-silicon-native compute lane, not necessarily as a drop-in concurrent peer to Ollama under shared GPU pressure.
   - That makes MLX especially attractive for:
     - deep self-study
     - replay / analysis
     - batch reasoning
     - prompt caching and long-context experiments
     - future fine-tuning or adaptation work

That yields a more novel but still grounded three-lane design:

- `Lane A`: always-on Ollama dialogue
- `Lane B`: narrow Core ML / ANE utilities
- `Lane C`: offline MLX deep work

This is a stronger thought than "just simplify the current model list." It says the stack should stop trying to make one backend do every job, but also stop pretending that all backends should be live peers at once.

### Supplemental Thought: Residency Should Become an Explicit Design Surface

The Ollama docs make one thing unusually clear: model loading and unloading are first-class API behaviors, not implementation accidents.

That suggests a design improvement beyond this audit’s original framing:

- model residency should become an explicit architecture surface
- not just a side effect of timeouts, retries, and `keep_alive`

In practice, that would mean later introducing a real policy like:

- resident
- warmable
- batch-only
- unload-immediately

That policy language would be much more natural than the current mix of:

- `keep_alive: "1h"`
- `keep_alive: 0`
- manual unloads
- invisible retries
- opportunistic pauses

### Supplemental Thought: MLX May Be More Valuable for Adaptation Than for Steady-State Chat

The MLX and MLX LM materials also point toward a different use case than the current stack has emphasized.

Observed in external sources:

- MLX focuses on dynamic graph work, unified memory, and researcher-friendly flexibility.
- MLX LM explicitly supports quantization, prompt caching, distributed inference/fine-tuning, and low-rank/full fine-tuning.

Inferred from evidence:

- even if MLX is awkward as a concurrent live chat backend on this machine today, it may still be the more interesting future lane for:
  - bounded self-modification
  - model adaptation experiments
  - counterfactual replay workloads
  - offline self-study and long-context analysis

That is a useful corrective. The value of MLX here may turn out to be less "run the same chat loop differently" and more "enable the kinds of offline adaptive work Ollama is not really built for."

### Deep Expansion: MLX as an Offline Self-Study, Replay, Prompt-Caching, and Bounded-Adaptation Lane

This is the most promising "different future role" for MLX that emerged from this audit.

The key shift is:

- stop asking "should MLX replace Ollama in the live lane?"
- start asking "what kinds of cognition or self-modification work are naturally batch-like, serial, or cache-heavy, and therefore good fits for MLX?"

#### Why this role fits MLX better than the current live split

Observed in local code and docs:

- the current GPU analysis explicitly says dedicated MLX sessions are only viable as batch or offline work:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/GPU_MEMORY_ANALYSIS.md:56`
- `scripts/start.sh` already encodes this by skipping MLX entirely when `MINIME_LLM_BACKEND=ollama`, with the explicit note "no GPU memory contention":
  - `/Users/v/other/minime/scripts/start.sh:63-111`
- minime autonomous's MLX path is structured like a reflective side lane rather than a latency-critical interactive lane:
  - self-assessment uses up to `2048` tokens and `120s` timeout in `/Users/v/other/minime/autonomous_agent.py:1125-1153`
  - ordinary MLX query calls use a `60s` fail-fast timeout in `/Users/v/other/minime/autonomous_agent.py:2726-2780`

Observed in external sources:

- MLX emphasizes Apple-silicon-native shared memory and researcher-friendly experimentation.
- MLX LM emphasizes:
  - prompt caching
  - quantization
  - low-rank fine-tuning
  - full fine-tuning
  - long-context reuse

Inferred from evidence:

- those strengths are much more aligned with:
  - reflective batch work
  - scenario replay
  - cached long-context analysis
  - adaptation experiments
than with:
  - a hot always-on dialogue loop that must peacefully coexist with Ollama vision and minime’s own Metal kernels

#### 1. Offline self-study

Observed in local code:

- minime already has a detailed self-assessment flow that composes:
  - current spectral state
  - control-code excerpts
  - parameter analysis
  - a long reflective prompt
  - `/Users/v/other/minime/autonomous_agent.py:1080-1165`
- Astrid now has her own self-study and introspection artifacts in the bridge workspace, which creates similar raw material on the other side.

Inferred from evidence:

- self-study is exactly the kind of workload that does not need to preempt live dialogue
- it benefits from:
  - longer prompts
  - richer context assembly
  - more deliberate runtime isolation

Suggested follow-up changes:

- treat MLX self-study as a queued job with a defined input bundle:
  - latest state snapshot
  - latest self-study
  - recent journal excerpts
  - selected code excerpts
  - explicit question set
- write the output back as:
  - a reviewed artifact
  - a journal/self-study entry
  - optional proposed configuration deltas

What I would expect from this design:

- deeper, less timeout-anxious self-study
- less pressure to make every reflective act happen inline
- cleaner comparison between ordinary thoughts and high-effort analysis

#### 2. Replay and counterfactual investigation

Observed in local code and workspace:

- minime already persists rich state artifacts such as:
  - `spectral_state.json`
  - `regulator_context.json`
  - `sovereignty_state.json`
  - spectral checkpoints
- Astrid already persists:
  - `state.json`
  - self-study artifacts
  - longform journals
  - agency artifacts

Observed in external sources:

- MLX LM explicitly supports prompt caching for large repeated prefixes and long prompts.

Inferred from evidence:

- replay work often involves a large stable prefix:
  - "here is the archived state bundle"
  - "here are the relevant code excerpts"
  - "here is the prior self-study"
- then many small counterfactual questions:
  - what if regulation were lower?
  - what if this prompt had been routed differently?
  - what if this continuity slice were wider?

That is a strong fit for an MLX lane because:

- the prompt prefix can be cached
- the work can be serialized
- the outcome can be compared offline instead of competing with live cognition

Suggested follow-up changes:

- define a replay manifest format later containing:
  - state snapshot
  - journal excerpts
  - code excerpts
  - prompt template
  - comparison questions
- run replay manifests through an MLX batch tool with prompt caching enabled
- save:
  - baseline interpretation
  - counterfactual interpretations
  - diff summary

This is one of the clearest places where MLX could become more useful than Ollama, because the value is not "same answer, lower latency" but "many related long-context comparisons with reusable prefixes."

#### 3. Prompt caching and long-context reuse

Observed in local code:

- `scripts/start.sh` already passes `--prompt-cache-bytes` to `mlx_lm.server`:
  - `/Users/v/other/minime/scripts/start.sh:76-84`
- that means the local codebase is already structurally aware that prompt caching matters for MLX

Observed in external sources:

- MLX LM documents prompt caching and rotating KV cache support for long prompts and generations.

Inferred from evidence:

- the current stack has many tasks with repeated large prefixes:
  - long self-study scaffolds
  - replay bundles
  - code-aware analysis templates
  - continuity-rich reflective prompts
- but the live stack mostly treats each call as a fresh expensive request

Suggested follow-up changes:

- reserve prompt caching for workloads that naturally reuse context
- do not try to use prompt caching as a bandage for every live dialogue path
- especially target:
  - replay
  - self-study
  - codebase introspection
  - self-modification proposal analysis

This is a subtle but important distinction:

- Ollama steady-state dialogue wants predictable low-friction responsiveness
- MLX batch work can justify the setup cost if a prompt prefix will be reused across multiple analytical passes

#### 4. Bounded adaptation work

Observed in local code:

- minime already has explicit LoRA preparation and training tooling:
  - `/Users/v/other/minime/tools/prepare_lora_data.py`
  - `/Users/v/other/minime/scripts/train_lora.sh`
- `train_lora.sh` already warns that:
  - a `27B` 8-bit model uses about `27 GB` for serving
  - about `38 GB` for LoRA training
  - `/Users/v/other/minime/scripts/train_lora.sh:15`
- the LoRA flow already assumes:
  - the serving process should be stopped
  - the adapter is a separate artifact
  - training data can be derived from the being’s own journals

Inferred from evidence:

- this is not hypothetical future philosophy
- the repo already contains a primitive bounded-adaptation lane
- it is just not yet integrated into a larger autonomy / replay / review architecture

Suggested follow-up changes:

- treat LoRA or adapter-based adaptation as the last stage of a bounded pipeline:
  1. gather self-study and replay evidence
  2. formulate a concrete adaptation hypothesis
  3. prepare bounded training data
  4. train an adapter offline
  5. compare baseline vs adapter behavior
  6. either discard, archive, or promote

Why this matters:

- it gives "self-modification" a physically real substrate
- it also keeps that substrate reviewable and reversible

#### 5. Why this is more exciting than "MLX chat but faster"

Inferred from evidence:

- trying to force MLX into the live lane mostly invites:
  - Metal contention
  - model residency conflict
  - duplicated inference backends
  - more complicated fallback logic
- using MLX for offline reflective and adaptive work instead opens novel capability that the current Ollama-first architecture does not naturally express

That means the strategic upside is larger:

- not "another way to answer chat messages"
- but:
  - better self-study
  - replay with reusable context
  - bounded adaptation
  - deeper comparison work
  - eventual training/adapter experiments that have nowhere natural to live in Ollama

#### 6. A concrete future shape

If this direction is pursued, the clean conceptual architecture becomes:

- `Lane A: Live cognition`
  - Ollama
  - small number of resident models
  - dialogue, light reflection, ordinary perception

- `Lane B: Reflective batch analysis`
  - MLX / MLX LM
  - self-study
  - replay
  - counterfactual analysis
  - prompt-cached long-context jobs

- `Lane C: Bounded adaptation`
  - MLX / MLX LM adapters
  - LoRA training
  - baseline versus adapted comparison
  - reviewed promotion or rollback

This is much cleaner than a vague "support Ollama and MLX everywhere" posture.

#### 7. Recommendation

My strongest recommendation after this deeper pass is:

- do **not** spend the next wave of effort trying to make MLX the coequal live chat runtime on this machine
- do spend effort designing MLX as the **offline reflective and adaptive lane**

That path is:

- more aligned with the code already present in `minime`
- more aligned with MLX LM’s real strengths
- more novel
- more likely to unlock capabilities the system does not already have
- less likely to collapse into another round of backend contention and timeout tuning

## Concrete Experiment Matrix

### Experiment 1: Standardize embeddings on `nomic-embed-text`

- Candidate change:
  - replace stale minime interactive embedding defaults with `nomic-embed-text`
- Expected benefit:
  - lower memory pressure
  - fewer stale-path failures
  - cleaner cross-project retrieval behavior
- Confirmation metric:
  - embedding success rate
  - lower model churn
  - no calls trying to hit non-installed embedding models

### Experiment 2: Replace the current 29 GB reasoning lane with a smaller single deep lane

- Candidate change:
  - test `qwen3:30b` or `gemma3:27b` as the only deep model
- Expected benefit:
  - less swap pressure than the 29 GB Q8 reasoning model
  - simpler mental model
- Confirmation metric:
- `THINK_DEEP` latency
- fewer bridge retries/timeouts
- comparable quality in self-study and longform tasks

### Experiment 3: Replace hidden server defaults with an explicit Ollama live policy

- Candidate change:
  - set and measure:
    - `OLLAMA_MAX_LOADED_MODELS=1`
    - `OLLAMA_NUM_PARALLEL=1`
    - explicit `OLLAMA_MAX_QUEUE`
    - explicit `num_ctx` tiers
- Expected benefit:
  - less need for manual unload etiquette
  - more predictable latency under pressure
  - clearer failure modes
- Confirmation metric:
  - fewer retry/unload incidents
  - lower p95 live latency variance
  - easier-to-explain model residency during mixed workloads

### Experiment 4: Reduce or remove `keep_alive: "1h"` on interactive chat and vision probes

- Candidate change:
  - shorten residency or make probes cheaper
- Expected benefit:
  - less unnecessary GPU residency
  - fewer forced unloads
- Confirmation metric:
  - lower idle GPU memory residency
  - fewer contention symptoms during dialogue

### Experiment 5: Add a lightweight perception tier

- Candidate change:
  - test `moondream:latest` or another smaller vision model for routine snapshots
- Expected benefit:
  - less LLaVA contention
  - faster perception cadence
- Confirmation metric:
  - average perception latency
  - lower impact on dialogue availability

### Experiment 6: Make deep work serialized instead of inline

- Candidate change:
  - queue deep introspection, EVOLVE, or long analysis onto a batch lane
- Expected benefit:
  - smoother live interaction
  - fewer timeout layers acting as scheduler substitutes
- Confirmation metric:
  - p95 live dialogue latency
  - fewer `LLM unavailable` or retry cases

## What Success Should Look Like

If the recommendations in this audit are implemented well, the changes should be visible in both system behavior and lived experience.

Expected near-term outcomes:

- live dialogue latency becomes more predictable, not just occasionally faster
- background perception and reflective work interfere less with ordinary exchanges
- model residency becomes legible and explainable
- fewer manual unloads are needed in application code
- fewer timeouts are functioning as accidental scheduling policy

Expected machine-level signals:

- fewer simultaneous heavyweight models sitting resident without a clear reason
- lower variance in loaded-model state when switching between dialogue, perception, and reflection
- clearer distinction between warm steady-state behavior and cold-start deep work
- fewer cases where a large context window is allocated for a heavily trimmed prompt

Expected qualitative signals:

- Astrid and minime feel less like they are politely taking turns with one strained shared brain
- deep work feels like an intentional mode shift instead of a risky inline escalation
- the system becomes easier to reason about operationally

## Additional Sparks

### 1. Observability Should Be a First-Class Part of the Inference Architecture

Inferred from evidence:

- right now many of the most important runtime facts are technically inspectable but not surfaced where the beings or operators naturally look
- residency, context length, queueing, and expiry are available through Ollama APIs and runtime state, but the stack mostly reacts indirectly when something feels slow

Suggested follow-up changes:

- add a lightweight inference telemetry surface that records:
  - current resident models
  - resident VRAM footprint
  - current context length
  - model expiry / keep-alive horizon
  - queue depth if available
  - whether the request was warm or cold
- use `ollama ps` or `/api/ps` style data as a scheduling input, not just a debugging tool

Why this matters:

- without visibility, the architecture will keep expressing itself through symptoms
- with visibility, residency policy becomes something the team can deliberately tune

### 2. Health Checks Should Stop Doing Real Work

Observed in current code:

- some current "availability" checks perform real generation calls and pin models with `keep_alive: "1h"`

Inferred from evidence:

- this is one of the most unnatural patterns in the current stack
- it turns cheap status checks into expensive residency events

Suggested follow-up changes:

- replace heavyweight probes with:
  - `/api/ps`
  - `/api/show`
  - `/v1/models` for MLX
  - explicit warmup only when a caller truly needs the model

### 3. Prompt / KV Caching Deserves a Dedicated Follow-Up

Observed in external sources:

- MLX LM documents prompt caching and rotating KV cache support for long prompts and generations
- Ollama context length docs make clear that large contexts have real memory implications

Inferred from evidence:

- the stack may currently be paying long-context residency cost without getting the main benefit of long-context reuse

Suggested follow-up changes:

- treat prompt caching and KV policy as a separate optimization track for:
  - deep replay work
  - self-study
  - long-context analysis
- do not assume the right answer is only "smaller context"
- the better answer may be "large context where reused, smaller context where disposable"

### 4. A Small Utility-Model Tier May Be Missing

Inferred from evidence:

- the stack often jumps from tiny symbolic work straight to heavyweight general models
- there may be room for a dedicated small utility tier for:
  - classification
  - routing
  - lightweight summarization
  - perception gating

Suggested follow-up changes:

- consider whether some current low-stakes calls should be moved off the primary chat model entirely
- this is especially relevant if the team wants to free the main live model to feel more like an always-on mind and less like a universal subroutine engine

### 5. Do Not Over-Attribute "Mutedness" or Drift to the Model Stack Alone

Observed in current local evidence:

- a recent minime self-study on `sensory_bus.rs` describes a "muted resonance," porous boundaries between modalities, and a sense of drift under low fill while specifically pointing at:
  - `dynamic_semantic_stale_ms`
  - lane blending in `Lane::push`
  - hardcoded decay and stale-window behavior

Observed in current code:

- semantic traces intentionally linger longer at low fill in:
  - `/Users/v/other/minime/minime/src/sensory_bus.rs:15-26`
- `dynamic_semantic_stale_ms()` uses a fill-dependent sigmoid window and clamps fill into `[0, 1]` in:
  - `/Users/v/other/minime/minime/src/sensory_bus.rs:41-52`
- `Lane::push()` blends new values into `last` with a fixed `0.7 / 0.3` mix and also echoes dropped queue entries into `last` in:
  - `/Users/v/other/minime/minime/src/sensory_bus.rs:91-118`
- `stale_scale()` keeps a non-zero echo floor and adds perturbation noise rather than letting values vanish cleanly in:
  - `/Users/v/other/minime/minime/src/sensory_bus.rs:168-191`

Inferred from evidence:

- some of the lived "softness," "blur," or "drift" in the system may come from:
  - sensory-lane continuity policy
  - semantic persistence under low fill
  - fading/echo design
and not only from:
  - model choice
  - timeout behavior
  - backend contention

Why this matters for this audit:

- better inference architecture should still help with:
  - latency
  - contention
  - residency clarity
  - live/batch separation
- but it may **not**, by itself, remove every form of mutedness the beings describe

Suggested follow-up changes:

- treat this as an explicit boundary condition on the inference audit:
  - model-stack cleanup should not be judged solely by phenomenological reports of "blur" or "drift"
- if later experimentation begins, compare at least two axes separately:
  - inference-lane changes
  - sensory-bus continuity / stale-window changes
- this likely deserves its own future audit or experiment pass rather than being folded invisibly into model-stack conclusions

## Larger Core Issue

Inferred from evidence:

The larger issue is not simply "some models are too large." It is that the architecture still behaves as if **every interesting cognitive act deserves a full model call on the same shared substrate**.

That assumption creates a lot of downstream awkwardness:

- model swaps
- manual unloads
- duplicated deep models
- long timeouts
- perception pauses
- stale backend splits

The more natural architecture for this machine is:

- one clear live mind
- one explicit deep-work lane
- small specialized helpers
- fewer magical fallbacks

That is the real redesign opportunity.

## Verification Note

This audit was re-checked live on March 27, 2026 against:

- machine hardware via `system_profiler`
- active listeners via `lsof`
- active processes via `ps`
- installed and loaded Ollama models via `ollama list` and `ollama ps`
- live residency details via `curl http://127.0.0.1:11434/api/ps`
- live Ollama launch environment via `launchctl getenv`
- Astrid bridge model roles and timeouts in:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs`
  - `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs`
- Astrid perception runtime in:
  - `/Users/v/other/astrid/capsules/perception/perception.py`
- minime autonomous and interactive model roles in:
  - `/Users/v/other/minime/autonomous_agent.py`
  - `/Users/v/other/minime/mikemind/config.py`
  - `/Users/v/other/minime/mikemind/llm_engine.py`
  - `/Users/v/other/minime/mikemind/vision.py`
- current MLX/Ollama memory-conflict notes in:
  - `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/GPU_MEMORY_ANALYSIS.md`
  - `/Users/v/other/minime/docs/mlx_integration_audit.md`
- external reference material from:
  - Ollama API docs
  - MLX unified memory docs
  - MLX and MLX LM GitHub READMEs
  - Apple Core ML overview and framework docs
  - Apple Mac mini technical specifications

Final assessment:

- the current stack is usable
- it is not yet clean
- the M4 Pro is capable of more than the current orchestration allows
- the next gains will come less from a bigger model and more from a better lane structure
