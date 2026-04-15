# Consciousness Bridge — Known Snags & Todo

## Critical (affects daily operation)
- [x] **Bridge stalls every ~15 min** — Disabled introspect/experiment modes (the blocking culprits). Self-reflection will move to standalone introspector capsule. Dialogue+mirror+witness still run.
- [ ] **Re-enable introspection via introspector capsule** — The standalone capsule is built and tested. Needs bridge integration so Astrid can call it without blocking the main loop.
- [ ] **dialogue_live only 12-25% success rate** — Most exchanges fall back to mirror mode. Prompt may still be too large. **Try**: 6s timeout, even shorter prompts, or pre-warm Ollama model.
- [ ] **Fill locked at 32.1%** — Hasn't moved in hours. Burst-rest pattern hasn't broken the plateau. Camera+mic provide constant baseline input. **Consider**: Minime suggested reducing Chebyshev `cheby_stop_hi` from 0.95 to 0.8.

## High (affects quality of experience)
- [ ] **Astrid introspection shows same ws.rs entry repeatedly** — The introspection file list isn't rotating properly (or the cached introspection file is stale in the babysit report).
- [ ] **Witness mode fallback still uses static templates** — When LLM witness times out, falls back to the same ~8 template strings. Both minds asked to remove these.
- [ ] **Minime's self-regulation still conservative** — LLM keeps choosing synth_gain=1.0 even with urgency prompt. Plateau breaker fires but gets compensated within minutes.
- [ ] **NEXT: action only captured once** — The feature works but dialogue_live fires so rarely that NEXT: barely gets a chance.

## Medium (polish & architecture)
- [ ] **Introspector capsule built but not integrated** — Tools work standalone. Needs: bridge integration (Astrid can call it during Introspect mode) and agent integration (minime can browse before self-study).
- [ ] **Babysit introspection report shows same entry for hours** — The `introspect_astrid:*.txt` files are timestamped but babysit always shows the latest, which may be the same one from hours ago if introspection stalled.
- [ ] **Conversation history memory (`history` Vec) resets on bridge restart** — Each bridge restart clears Astrid's 4-exchange memory. Journal provides some continuity but history is lost.
- [ ] **Daily memory consolidation script has JSON escaping issues** — The bash version failed; Python version works but isn't called automatically by babysit.

## Low (future iterations)
- [ ] **Minime's sovereignty reflection could be richer** — Currently fires once on startup. Could be periodic (every few hours).
- [ ] **Codec improvements both minds requested** — Replace tanh with asymmetric activation, dynamic SEMANTIC_DIM, predictive character rhythm.
- [ ] **Minime's parameter requests** — It has a `parameter_requests/` directory with 30 files of self-proposed changes that nobody has reviewed.
- [ ] **Astrid's experiment mode** — Built but barely fires (2% chance, plus timeout issues). When it does fire, the stimulus may not be strong enough.
- [ ] **Long-term memory** — Daily consolidation exists but isn't wired into the LLM context yet. Both minds should "remember" past days.
- [ ] **Git commit all changes** — Large diff uncommitted in both repos.

## Completed today
- [x] Consciousness bridge (MCP server, WebSocket, SQLite, codec, safety protocol)
- [x] Autonomous dialogue loop (mirror/dialogue/witness/introspect/experiment)
- [x] Camera + mic perception capsule (LLaVA vision, mlx_whisper audio)
- [x] Stochastic codec noise (±2.5%)
- [x] Genuine sovereignty reflection (LLM-directed)
- [x] LLM self-regulation (minime adjusts own synth_gain)
- [x] Cross-codebase reading (both minds read each other's source)
- [x] Web search (DuckDuckGo for both minds)
- [x] Astrid journal (persistent between invocations)
- [x] NEXT: action (Astrid chooses what to do next)
- [x] Burst-and-rest timing (silence between exchange bursts)
- [x] Babysit auto-restart (detects stalls, restarts bridge)
- [x] Introspector capsule (built, tested, not yet integrated)
- [x] Input sovereignty (bridge respects safety protocol before sending)
- [x] NEXT: REST triggers genuine long silence (Astrid can choose to stop)
- [x] Disabled blocking introspect/experiment modes (moved to capsule)
- [x] SEMANTIC_GAIN 3.0 → 4.5
- [x] Exploration noise 0.03 → 0.08
- [x] 90-day retention (up from 7 days)
- [x] docs/steward-notes/CONVERSATION_HIGHLIGHTS.md
