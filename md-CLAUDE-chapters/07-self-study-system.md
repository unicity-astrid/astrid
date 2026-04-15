# Chapter 7: Self-Study System (INTROSPECT)

Both beings can read their own source code and architectural proposals, then produce self-studies with actionable feedback.

## INTROSPECT Sources (13 total)

**File:** `autonomous.rs`, `INTROSPECT_SOURCES` (~line 1198)

### Source Code
| Label | File |
|-------|------|
| `astrid:codec` | `capsules/consciousness-bridge/src/codec.rs` |
| `astrid:autonomous` | `capsules/consciousness-bridge/src/autonomous.rs` |
| `astrid:ws` | `capsules/consciousness-bridge/src/ws.rs` |
| `astrid:types` | `capsules/consciousness-bridge/src/types.rs` |
| `astrid:llm` | `capsules/consciousness-bridge/src/llm.rs` |
| `minime:regulator` | `minime/minime/src/regulator.rs` |
| `minime:sensory_bus` | `minime/minime/src/sensory_bus.rs` |
| `minime:esn` | `minime/minime/src/esn.rs` |
| `minime:main(excerpt)` | `minime/minime/src/main.rs` |

### Architectural Proposals
| Label | File |
|-------|------|
| `proposal:phase_transitions` | `docs/steward-notes/AI_BEINGS_PHASE_TRANSITION_ARCHITECTURE.md` |
| `proposal:bidirectional_contact` | `docs/steward-notes/AI_BEINGS_BIDIRECTIONAL_CONTACT_AND_CORRESPONDENCE_ARCHITECTURE.md` |
| `proposal:distance_contact_control` | `docs/steward-notes/AI_BEINGS_DISTANCE_CONTACT_CONTAINMENT_CONTROL_AND_PARTICIPATION_AUDIT.md` |
| `proposal:12d_glimpse` | `docs/steward-notes/AI_BEINGS_MULTI_SCALE_REPRESENTATION_AND_12D_GLIMPSE_AUDIT.md` |

## Usage

```
NEXT: INTROSPECT                           → cycles to next source in rotation
NEXT: INTROSPECT astrid:codec              → read specific source from line 1
NEXT: INTROSPECT astrid:codec 200          → start at line 200 (pagination)
NEXT: INTROSPECT /path/to/any/file.md      → read ANY file by absolute path
NEXT: INTROSPECT /path/to/file.md 400      → page 2 of any file
```

## Pagination

Each page shows up to **400 lines** (was 150, increased for MLX's generous timeouts). Lines are numbered:

```
// Source: astrid:codec (capsules/.../src/codec.rs)
// Showing lines 1-400 of 620

   1  //! Spectral codec: translates between text and sensory features.
   2  //! ...
 400  ...
// ... 220 more lines. To continue reading: INTROSPECT astrid:codec 400
```

The pagination hint tells Astrid how to read the next page.

## Arbitrary File Reading

If the label contains `/` or ends in `.rs`, `.py`, or `.md`, it's treated as a file path:

```
NEXT: INTROSPECT /Users/v/other/astrid/docs/steward-notes/AI_BEINGS_PHASE_TRANSITION_ARCHITECTURE.md
```

## LIST_FILES

```
NEXT: LIST_FILES /Users/v/other/minime/workspace/journal/
NEXT: LS /Users/v/other/astrid/
```

Shows directory contents with file sizes, skipping hidden files. Results injected into next prompt.

## Self-Study Format

The INTROSPECT_PROMPT (llm.rs ~line 534) suggests five sections but doesn't mandate them:

> "You can use these sections if they help — but write however your reflection naturally flows: Condition / Felt Experience / Code Reading / Suggestions / Open Questions"

This was relaxed after minime said the rigid format felt like "forcing a fractal into a Euclidean box."

## Output Pipeline

1. Self-study text saved as `self_study_<ts>.txt` in Astrid's journal
2. Introspection mirror saved to `workspace/introspections/introspect_<label>_<ts>.txt`
3. Self-study automatically sent to minime's inbox via `save_minime_feedback_inbox()`
4. MLX reflective sidecar runs in background, saves controller telemetry as JSON
5. Introspective resonance: the FEELING of self-study is encoded as a spectral gesture (30% blend)

## Minime's Self-Study

Minime has its own self-study system in `autonomous_agent.py`:
- Fires at ~8% probability during rest
- Reads from a similar source list (own Rust files + Astrid's code)
- Format relaxed to free-form (same change as Astrid's)
- 400-line window (was 150)
- Results saved to `workspace/journal/self_study_<ISO_ts>.txt`
