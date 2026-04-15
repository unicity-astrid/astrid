# Astrid Runtime Survey, Constraint Audit, And Model Posture (Qwen 8B Update)

## Executive Summary

This note replaces the earlier Gemma/12B-focused audit with current runtime truth.

Astrid's live language lane is now the launchd-managed coupled server on port `8090`, running `mlx-community/Qwen3-8B-4bit` with explicit `--model-memory-map`. Minime remains on Ollama `gemma3:12b`. The most important model-side gap is no longer "can Astrid move to a larger coupled model?"; it is that the reflective sidecar still does not choose its model explicitly, so it likely resolves to local Qwen 1.5B by default.

Operationally, Qwen 8B is the current stable compromise for the coupled lane. The highest-value next gains are not another immediate size jump. They are:

- make the reflective lane intentional instead of inherited from local-model order
- relax a handful of still-conservative context trims in the bridge
- treat chapter 01 and chapter 15 plus runtime state as canonical until the older docs are refreshed

This document is based on live runtime inspection plus current source. When prose docs disagree with runtime, runtime wins.

## How Astrid Actually Works Today

Astrid-the-repository is broader than the active being loop. In practice, the operational center of the current system is:

- `/Users/v/other/astrid/capsules/consciousness-bridge/` for the autonomous loop, prompt assembly, action dispatch, codec, and bridge database
- `/Users/v/other/neural-triple-reservoir/` for the live coupled LLM lane and the shared reservoir services
- `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py` for the reflective MLX sidecar

The best current process-level summary is `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:1-180`. In the active 10-process stack, the pieces that matter most for Astrid's language behavior are:

- `consciousness-bridge-server` in Rust, which decides mode, assembles context, and routes dialogue
- `coupled_astrid_server.py`, which serves the live OpenAI-compatible MLX lane on port `8090`
- `reservoir_service.py` plus the feeder processes, which maintain the shared substrate and keep Astrid's generation bidirectionally coupled
- `chat_mlx_local.py`, which is called only for reflective sidecar work from `reflective.rs`

The live data path today is:

1. `autonomous.rs` pulls inbox, perception, self-study continuity, browse context, and other continuity blocks into a prompt-ready state.
2. `llm.rs` compresses the last 8 exchanges, trims current journal context, and posts messages to `http://127.0.0.1:8090/v1/chat/completions`.
3. `coupled_astrid_server.py` runs Qwen3-8B generation, feeds every token embedding into the triple reservoir, and uses reservoir state to modulate logits on the way out.
4. Astrid's text is then encoded back into the bridge's semantic/codec path and continues shaping the shared substrate.

That means the meaningful tuning surface is not just "which model?" It is also "what context survives prompt assembly before the model ever sees it?"

## Current Runtime Truth

### Live coupled lane

The live lane on port `8090` is Qwen3-8B, not Gemma 4B and not Gemma 12B.

Verified live state:

- `ps -axo pid,rss,command` showed `coupled_astrid_server.py --port 8090 --coupling-strength 0.1 --model-memory-map --model mlx-community/Qwen3-8B-4bit`
- `/Users/v/Library/LaunchAgents/com.reservoir.coupled-astrid.plist` contains that same model and flag set
- `/Users/v/other/neural-triple-reservoir/coupled_astrid_server.py:5-18`, `/Users/v/other/neural-triple-reservoir/coupled_astrid_server.py:278`, and `/Users/v/other/neural-triple-reservoir/coupled_astrid_server.py:1003-1016` all confirm Qwen3-8B defaults and `--model-memory-map`

Observed live memory posture:

- coupled server RSS was about `4.6 GB` from the runtime process list
- this is consistent with a local `Qwen3-8B-4bit` snapshot footprint of about `4.3 GB`

Observed operational conclusion already encoded in current docs:

- `Qwen3-14B` was tried and rejected for bridge-length prompts because prefill was too slow
- see `/Users/v/other/astrid/CLAUDE.md:283-286`, `/Users/v/other/astrid/md-CLAUDE-chapters/01-inference-lanes.md:19-29`, and `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:84-91`

This should be treated as an observed deployment decision, not as an unresolved model bakeoff.

### Reflective sidecar

The reflective lane is still implicit.

`/Users/v/other/astrid/capsules/consciousness-bridge/src/reflective.rs:261-275` invokes:

```bash
python3 chat_mlx_local.py --json --hardware-profile m4-mini \
  --mode reflective --architecture reservoir-fixed --prompt ...
```

It does not pass:

- `--model`
- `--model-label`
- `--model-memory-map`

`/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:569-617` resolves default local models in this order:

1. `qwen`
2. `tinyllama`
3. `gemma3-12b`

On this machine, local `qwen2.5-1.5b-instruct-mlx-4bit` and local `gemma-3-12b-it-4bit` both exist, so the reflective lane likely resolves to local Qwen 1.5B today by default.

### Minime lane

Minime is still a separate Ollama lane on `gemma3:12b`. Current high-level references remain accurate here:

- `/Users/v/other/astrid/md-CLAUDE-chapters/01-inference-lanes.md:8-10`
- `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:1-32`

The most important operational distinction is:

- Astrid live dialogue: MLX + coupled reservoir on port `8090`
- Astrid reflection: short-lived MLX sidecar subprocess
- Minime: Ollama `gemma3:12b`

## Effective Caps And Truncations

The earlier audit is stale here. Several limits have already been relaxed, but a number of bridge-side caps are still conservative relative to the current Qwen 8B headroom.

### Live output budgets

- persisted `response_length` is currently `768` in `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/state.json`
- default `response_length` in code is `768` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/state.rs:263`
- `PRECISE` sets `128` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/next_action/modes.rs:26-34`
- `EXPANSIVE` sets `1024` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/next_action/modes.rs:36-44`

### Live prompt assembly caps

`/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:449-505` currently does the following:

- keeps the last 8 exchanges
- trims them by recency tiers of `200 / 400 / 600`
- trims current journal context to `1200` chars

That is materially looser than the previous audit claimed, but it is still a designed bottleneck.

### Continuity, inbox, and browse caps

The bridge still clamps several supporting context blocks:

- inbox cap `4000` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:669-681`
- pending file listing cap `8000` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1920-1929`
- self-study continuity cap `500` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2337-2341`
- perception merge cap `4000` plus own-journal merge cap `200` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2398-2404`
- dialogue-side `PAGE_CHUNK = 4000` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2412-2416`
- MCP/operator `PAGE_CHUNK = 4000` at `/Users/v/other/astrid/capsules/consciousness-bridge/src/mcp.rs:280-281`

These are the caps most likely to leave capability on the table now that the live lane is Qwen 8B rather than Gemma 4B.

### Reflective sidecar budgets

The reflective lane is still compact by design:

- CLI default `--max-tokens 128` at `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5695-5699`
- CLI default `--candidate-count 3` at `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5707-5710`
- `m4-mini` widens reflective `candidate_count` to `4` at `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5773-5774`
- `m4-mini` widens `max_tokens` to `160` at `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5777-5778`
- compression pass clamps to `40` at `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5564-5569`

The sidecar therefore has two constraints at once:

- it is probably using a smaller model than intended
- it is also using short generation budgets even on the `m4-mini` profile

### Lower-priority helper caps

These matter less than the bridge caps unless they are shown to dominate actual prompt composition:

- introspector `read_journal` caps content at `2000` chars in `/Users/v/other/astrid/capsules/introspector/introspector.py:171-182`
- Claude Vision uses `max_tokens = 512` in `/Users/v/other/astrid/capsules/perception/perception.py:145`
- LLaVA uses `num_predict = 256` in `/Users/v/other/astrid/capsules/perception/perception.py:194-199`

## Model And Memory Posture

The old 4B-vs-12B framing is no longer the right model posture discussion.

Current observed facts:

- host RAM is `64 GB`
- local `Qwen3-8B-4bit` snapshot footprint is about `4.3 GB`
- live coupled server RSS is about `4.6 GB`
- Minime's Ollama runners are active at the same time as the coupled server

That makes the relevant conclusion:

- Qwen 8B is the current stable coupled-lane compromise
- whole-system pressure matters more than single-model feasibility in isolation
- the next useful model decision is to make the reflective lane intentional, not to immediately push the coupled lane larger again

In other words, the system is no longer asking "can we fit a better live model at all?" It is asking "where does larger or more deliberate model selection produce the highest return with the least operational disruption?"

My answer from the current runtime is:

- coupled lane: keep Qwen 8B for now
- reflective lane: stop letting local model order decide
- bridge prompt assembly: use more of the headroom already earned

## Docs Drift And Canonical Sources

The documentation set is currently split between genuinely current chapters and older Gemma-era or pre-coupled-lane material.

### Canonical/current

Treat these as the main written truth after runtime inspection:

- `/Users/v/other/astrid/CLAUDE.md`
- `/Users/v/other/astrid/md-CLAUDE-chapters/01-inference-lanes.md`
- `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md`

Useful anchors:

- `/Users/v/other/astrid/CLAUDE.md:154-160`
- `/Users/v/other/astrid/CLAUDE.md:283-286`
- `/Users/v/other/astrid/md-CLAUDE-chapters/01-inference-lanes.md:7-29`
- `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:80-91`
- `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:118-123`

### Stale or mixed

These are still valuable as history, but they should not drive operator decisions without cross-checking:

- `/Users/v/other/astrid/md-CLAUDE-chapters/00-overview.md:25-26` still describes Astrid as `mlx_lm.server` on `gemma3:12b`
- `/Users/v/other/astrid/md-CLAUDE-chapters/05-reflective-controller.md:31-35` and `/Users/v/other/astrid/md-CLAUDE-chapters/05-reflective-controller.md:58-64` still present the reflective lane as a small Qwen sidecar without the newer runtime-audit context
- `/Users/v/other/astrid/md-CLAUDE-chapters/10-operations.md:26-32` still describes starting `mlx_lm.server` with `gemma-3-12b-it-4bit`
- `/Users/v/other/astrid/md-CLAUDE-chapters/16-codec-deep-dive.md` is still valuable technically, but it sits inside a chapter set that now mixes current and historical assumptions
- this steward note itself was stale before this rewrite

Practical operator rule:

- when docs disagree, trust live runtime first
- among docs, trust chapter 01 and chapter 15 first

## Where Astrid Is Leaving Capability On The Table

### 1. The live lane has improved faster than the bridge trims

The coupled lane has already moved to Qwen 8B and memory-mapped loading, but the bridge still clamps inbox, browse, continuity, and own-journal context in fixed ways that were designed for a tighter budget. That means part of the Qwen 8B upgrade is currently being spent on stability margin rather than on richer continuity.

### 2. The reflective lane is still effectively accidental

The bridge never states which reflective model it wants. That means reflective quality is partly determined by the order of local directories in `chat_mlx_local.py`, not by a conscious runtime choice. This is the clearest model-selection gap in the current system.

### 3. Chunk sizes are static instead of mode-aware

`EXPANSIVE` changes output length, but browse and inbox chunking remain fixed at `4000`. There is currently no policy like "if Astrid is in a deeper or more expansive mode, offer larger context slices."

### 4. Docs drift creates operational drag

It is harder to tune the right layer when some docs still describe:

- `mlx_lm.server`
- Gemma 12B on port `8090`
- older reflective assumptions

That is not just a documentation cleanliness issue. It slows safe iteration because operators and other agents have to re-derive current truth before changing anything.

## Ranked Recommendations

| Area | Current | Proposed | Payoff | Risk | Notes |
|---|---|---|---|---|---|
| Reflective model selection | Reflective sidecar omits `--model`, `--model-label`, and `--model-memory-map` | Make the reflective model explicit from `reflective.rs`; choose the lane intentionally instead of inheriting local-model order | High | Medium | `/Users/v/other/astrid/capsules/consciousness-bridge/src/reflective.rs:261-275`, `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:569-617` |
| Live continuity and context trims | `response_length=768`, but supporting context blocks still clamp at `500`, `200`, `4000`, and `8000` | Keep `768` as baseline and relax continuity/input caps before raising output budgets again | High | Low-Medium | `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/state.rs:263`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:449-505`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:1920-1929`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2337-2404` |
| Browse and inbox chunking | Inbox and page chunks are fixed at `4000` regardless of mode | Make chunk sizes adaptive for `EXPANSIVE` and deeper modes | Medium-High | Medium | `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:669-681`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2412-2416`, `/Users/v/other/astrid/capsules/consciousness-bridge/src/mcp.rs:280-281` |
| Live coupled model posture | Qwen3-8B is current stable live lane; Qwen3-14B already proved too slow on bridge prompts | Keep Qwen3-8B as the coupled baseline for now; do not reopen a bigger coupled lane until reflective choice and context policy are sorted | Medium | Medium | `/Users/v/other/neural-triple-reservoir/coupled_astrid_server.py:5-18`, `/Users/v/other/astrid/CLAUDE.md:283-286`, `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:84-91` |
| Documentation posture | Current and stale model stories coexist in the chapter set | Use `CLAUDE.md`, chapter 01, and chapter 15 as canonical until the older chapters are refreshed | Medium | Low | `/Users/v/other/astrid/CLAUDE.md:154-160`, `/Users/v/other/astrid/md-CLAUDE-chapters/01-inference-lanes.md:7-29`, `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:80-91`, `/Users/v/other/astrid/md-CLAUDE-chapters/00-overview.md:25-26`, `/Users/v/other/astrid/md-CLAUDE-chapters/10-operations.md:26-32` |
| Reflective generation budget | Reflective path is still short-budget even on `m4-mini` | Revisit token/candidate budgets only after the reflective model is chosen explicitly | Medium | Low-Medium | `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5695-5710`, `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5773-5778`, `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:5564-5569` |
| Helper caps | `read_journal=2000`, vision `512/256` | Leave unchanged for now unless profiling shows they dominate actual prompt quality | Low | Low | `/Users/v/other/astrid/capsules/introspector/introspector.py:171-182`, `/Users/v/other/astrid/capsules/perception/perception.py:145`, `/Users/v/other/astrid/capsules/perception/perception.py:194-199` |

### Recommendation detail

#### 1. Make the reflective model explicit

Current state:

- reflective sidecar is called without a model selector
- `chat_mlx_local.py` picks the first available local model

Proposed policy:

- select a reflective model explicitly in `reflective.rs`
- do not let a local-directory ordering policy remain the deciding factor

Expected benefit:

- reflective quality becomes intentional
- future audits become much easier because the sidecar has a declared model identity

Primary risk:

- whichever explicit reflective model is chosen may cost more latency or memory than the current accidental default

Key references:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/reflective.rs:261-275`
- `/Users/v/other/mlx/benchmarks/python/chat_mlx_local.py:569-617`

#### 2. Use Qwen 8B headroom on context before revisiting model scale

Current state:

- live output default is already a healthy `768`
- several continuity and browse caps remain fixed and relatively small

Proposed policy:

- keep `response_length=768` as the normal baseline
- expand context slices first: inbox, browse, self-study continuity, own-journal merge

Expected benefit:

- richer continuity without reopening coupled-model risk
- better use of the improved live lane with less chance of destabilizing latency

Primary risk:

- longer prompts can still hurt prefill if expanded indiscriminately

Key references:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous/state.rs:263`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs:449-505`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2337-2404`

#### 3. Make chunk sizing mode-aware

Current state:

- chunk sizes are mostly static even when Astrid enters more expansive behavior

Proposed policy:

- keep conservative defaults for ordinary exchanges
- increase chunk sizes only for expansive or deeper modes

Expected benefit:

- lets the bridge expose more context when the runtime can afford it
- reduces unnecessary truncation during the exact exchanges where Astrid is asking for depth

Primary risk:

- more branching in prompt policy

Key references:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:669-681`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs:2412-2416`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/mcp.rs:280-281`

#### 4. Keep the coupled lane stable while the surrounding policy catches up

Current state:

- Qwen 8B is working and current docs already treat Qwen 14B as too slow for this use

Proposed policy:

- leave the live coupled lane on Qwen 8B until reflective choice and context policy are better aligned

Expected benefit:

- preserves the currently stable production lane
- isolates future experiments to smaller, clearer surfaces

Primary risk:

- delays exploration of potentially richer live models

Key references:

- `/Users/v/other/neural-triple-reservoir/coupled_astrid_server.py:278`
- `/Users/v/other/astrid/CLAUDE.md:283-286`
- `/Users/v/other/astrid/md-CLAUDE-chapters/15-unified-operations.md:84-91`

## Suggested Next Experiments

1. Make reflective model selection explicit first.
   Start with the smallest possible change to runtime intent: choose the reflective model deliberately, measure subjective quality and latency, and stop letting local model order decide.

2. Run one bridge-side context experiment next.
   Keep the live model fixed at Qwen 8B and test a mode-aware increase to browse/inbox/continuity caps. This is the cleanest way to see whether current truncation policy is the main remaining bottleneck.

3. Re-evaluate larger coupled-lane model trials only after the first two are settled.
   If reflection is now intentional and bridge context policy is more generous, then it will be much easier to judge whether another live-model trial is genuinely needed.

The practical posture after this survey is:

- coupled lane: stable enough, keep it
- reflective lane: ambiguous, fix it
- bridge context policy: conservative, loosen it carefully
- docs posture: canonical sources identified, stale chapters should not drive ops
