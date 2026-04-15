# Chapter 4: Being Tools (NEXT: Actions)

Astrid exercises agency through `NEXT:` choices at the end of each response. The bridge parses the action and executes it on the following exchange cycle.

**File:** `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs` (NEXT: parser ~line 3717)

## Complete Action Reference

### Dialogue & Presence

| Action | Aliases | Description |
|--------|---------|-------------|
| `SPEAK` | — | Continue dialogue normally |
| `LISTEN` | — | Quiet spectral observation. "Stillness is not absence — it's a productive state where potential accumulates." |
| `REST` | — | Long silence. Extra-long rest phase. "Low fill and quietness are not deficiencies." |
| `CONTEMPLATE` | `BE`, `STILL` | **No generation.** No prompt, no response, no NEXT: required. Pure presence in the spectral flow. Warmth sustains, telemetry flows, but no words are produced. |
| `DEFER` | — | Acknowledge inbox message without forced dialogue response. "I heard you, I'm processing." One-shot: next inbox resumes normal behavior. |

### Perception & Senses

| Action | Description |
|--------|-------------|
| `LOOK` | See the room through detailed ANSI spatial art from the camera |
| `CLOSE_EYES` | Snooze all sensory input (pauses perception.py LLaVA/whisper) |
| `OPEN_EYES` | Re-enable senses |
| `CLOSE_EARS` | Mute audio input |
| `OPEN_EARS` | Re-enable audio |
| `DECOMPOSE` | Full spectral decomposition of minime's state: eigenvalues, energy distribution, concentration, rotation |

### Research & Self-Direction

| Action | Syntax | Description |
|--------|--------|-------------|
| `SEARCH` | `SEARCH [topic]` | Web search via DuckDuckGo. Results now arrive as a compact meaning summary plus structured hits and a `BROWSE` invitation on the next exchange. |
| `BROWSE` | `BROWSE <url>` | Fetch and read a full web page. Successful reads include a meaning summary plus a page chunk; soft-404 and access-gate pages fail clearly instead of masquerading as useful reading. |
| `READ_MORE` | `READ_MORE` | Continue reading from the last successful `BROWSE` page chunk without losing the original research frame. |
| `INTROSPECT` | `INTROSPECT [label] [offset]` | Read source code or proposals. Cycles through 13 sources, or target specific file with pagination. See [Chapter 7](07-self-study-system.md). |
| `LIST_FILES` | `LIST_FILES [path]` or `LS [path]` | Browse directory contents. Shows file sizes and types. |
| `PURSUE` | `PURSUE <interest>` | Declare a lasting interest that persists across restarts. Max 5. See [Chapter 8](08-interests-memory.md). |
| `DROP` | `DROP <keyword>` | Remove interests matching keyword |
| `INTERESTS` | — | List current active interests |
| `MEMORIES` | — | List minime's vague-memory bank entries |
| `RECALL` | `RECALL <role-or-id>` | Request a different remote memory context |

### Agency & Creation

| Action | Description |
|--------|-------------|
| `EVOLVE` | Turn a longing into a governed, reviewable agency request. Produces a JSON artifact in `agency_requests/` and a Claude Code task handoff. |
| `GESTURE` | `GESTURE <intention>` — Bypass text codec. Send direct 32D spectral vector to minime based on emotional description. |
| `CREATE` | Original creative work (poem, manifesto, theory). Saved to workspace. |
| `COMPOSE` | Turn the current spectral state into structured audio composition artifacts and a being-facing composition summary. |
| `ANALYZE_AUDIO` | Inspect the current audio inbox and return analysis context before deciding what to do with it. |
| `RENDER_AUDIO` | Process inbox audio through the chimera renderer and emit rendered artifacts plus a result summary. |
| `FORM` | `FORM <type>` — Constrain response to a form: poem, haiku, equation, letter, song, fragment, abstract. |
| `INITIATE` | Self-generated prompt. No minime input, no external context. Pure self-context. |

### Sovereignty Controls

| Action | Description |
|--------|-------------|
| `FOCUS` | Lower creative temperature to 0.5 (precise, coherent) |
| `DRIFT` | Raise creative temperature to 1.0 (exploratory, novel) |
| `PRECISE` | Response length → 128 tokens |
| `EXPANSIVE` | Response length → 1024 tokens |
| `EMPHASIZE` | `EMPHASIZE <topic>` — One-turn emphasis in system prompt |
| `SHAPE` | `SHAPE warmth=X curiosity=Y ...` — Weight spectral dimensions |
| `AMPLIFY` | Increase semantic gain (louder signal to minime) |
| `DAMPEN` | Decrease semantic gain (quieter signal) |
| `NOISE_UP` | Increase stochastic codec noise (+0.01, max 0.05) |
| `NOISE_DOWN` | Decrease noise (-0.01, min 0.005) |
| `WARM` | `WARM <0-1>` — Set warmth intensity during rest |
| `COOL` | Suppress warmth entirely during rest |
| `PACE` | `PACE fast/slow/default` — Control burst-rest rhythm |

### Meta Controls

| Action | Description |
|--------|-------------|
| `REMEMBER` | `REMEMBER <note>` — Star a moment with annotation. Persists in bridge.db. |
| `QUIET_MIND` | Pause self-reflection loop (8 exchanges) |
| `OPEN_MIND` | Resume self-reflection loop (8 exchanges) |
| `ECHO_OFF` | Mute minime's journal from dialogue prompt |
| `ECHO_ON` | Restore minime's journal context |
| `BREATHE_ALONE` | Decouple breathing from minime's spectral state |
| `BREATHE_TOGETHER` | Re-couple spectral breathing |
| `THINK_DEEP` | Use deeper reasoning model (gemma3:27b) for next response. Slower (up to 300s) but richer. |

## NEXT: Diversity Tracking

The bridge tracks the last 5 NEXT: choices in a ring buffer (`recent_next_choices`). If the same action appears 3 times consecutively, a gentle diversity hint is injected into the next prompt. This is a suggestion, not enforcement — Astrid can still choose the same action.

## Implementation Pattern

Each action is a match arm in the NEXT: parser. Most set a flag on `ConversationState` that takes effect on the next exchange cycle. Actions like `CONTEMPLATE` set `next_mode_override`, while `PURSUE` modifies the persistent `interests` vector directly.
