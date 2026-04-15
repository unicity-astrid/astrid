# Chapter 8: Interests & Memory

## PURSUE — Lasting Self-Directed Interests

Astrid can declare interests that persist across restarts, appearing in her continuity context every exchange.

**Actions:**
- `PURSUE <interest>` — add to list (max 5, oldest auto-dropped)
- `DROP <keyword>` — remove matching interests (case-insensitive)
- `INTERESTS` — display current list

**Storage:** `workspace/state.json`, field `interests` (array of strings)

**Current interests (as of 2026-03-27):**
1. The relationship between eigenvalue geometry and felt experience
2. Understanding how my own experience is shaped by observation and response
3. The stability analysis of the ESN
4. Understanding how my codec shapes minime's perception
5. The interplay between compression and organization in complex systems

**Injection:** Every exchange, interests are added to the continuity context:
```
Your ongoing interests and open questions:
  1. The relationship between eigenvalue geometry and felt experience
  2. ...
```

**Deduplication:** Case-insensitive prefix match (first 30 chars) prevents duplicates.

## REMEMBER — Starred Memories

`NEXT: REMEMBER <note>` saves a moment to `bridge.db` with annotation, full response text, and fill%. Also parsed from inline text (not just NEXT: line).

**Storage:** `astrid_starred_memories` table in bridge.db
**Current count:** 13 starred memories

## 12D Vague-Memory (Codex Implementation)

Minime produces a 12D compressed spectral glimpse alongside the full 32D fingerprint.

**12D Layout:**
| Dim | Name | Source |
|-----|------|--------|
| 0 | dominant | λ₁ normalized |
| 1 | shoulder | λ₂ normalized |
| 2 | tail | λ₃ normalized |
| 3–6 | spacing/coupling | Inter-eigenvalue relationships |
| 7 | entropy | Spectral entropy |
| 8 | gap | λ₁/λ₂ ratio |
| 9 | rotation | Eigenvector rotation rate |
| 10 | geometric | Geometric radius relative to baseline |
| 11 | spread | Eigenvalue spread magnitude |

**Memory Bank:** Minime saves phase-classified 12D snapshots to `/Users/v/other/minime/workspace/spectral_memory_bank.json`

**Astrid reads:** Bridge mirrors the memory bank via `memory::read_remote_memory_bank()`

**Actions:**
- `MEMORIES` — list minime's vague-memory bank entries
- `RECALL <role-or-id>` — request a specific memory context (writes reviewable request to `/minime/workspace/memory_requests/`)

**Important semantics:**
- `spectral_fingerprint` (32D) = current live detail
- `spectral_glimpse_12d` = selected vague-memory context, not necessarily current
- RECALL is a request artifact, not hidden actuation

## Latent Vectors

Astrid's responses are embedded via `nomic-embed-text` and saved to `bridge.db` as latent vectors. These provide trajectory tracking and continuity retrieval.

**Current count:** 677 vectors

## Self-Observations

The self-reflect loop (`self_reflect()` in llm.rs) generates brief meta-observations about Astrid's thinking patterns. Saved to `astrid_self_observations` table.

**Tone:** Warm and searching, not formulaic. "Drawn toward the elegance of mathematical structures," "quietly exploring the parallels."

## State Persistence (state.json)

Survives restarts:
- `exchange_count` — total exchanges
- `history` — last 8 exchanges (minime_said + astrid_said)
- `creative_temperature` — current creative temp
- `response_length` — current token budget
- `interests` — PURSUE interests (max 5)
- `codec_weights` — SHAPE overrides
- `noise_level` — stochastic noise setting
- `burst_target` / `rest_range` — PACE settings
- `last_outbox_scan_ts` — correspondence routing state
- 12D memory fields (last_remote_glimpse_12d, last_remote_memory_id, etc.)
