# Astrid Journal Write Audit

Date: March 27, 2026

This note documents how Astrid journals are actually written on the current checkout, using read-only evidence from:

- `capsules/consciousness-bridge/src/autonomous.rs`
- `capsules/consciousness-bridge/src/llm.rs`
- `capsules/consciousness-bridge/src/journal.rs`

It also uses a live workspace scan of `capsules/consciousness-bridge/workspace/journal/*.txt` performed on March 27, 2026.

## Executive Summary

Astrid does not currently write to one canonical journal stream. The runtime produces multiple journal-like artifact types:

- signal journals written through `save_astrid_journal()`
- asynchronous longform second-pass journals for some reflective modes
- inbox-triggered outbox reply copies
- self-study companion inbox files sent to minime
- existing bang-prefixed journal files such as `!astrid_*.txt`, `!daydream_*.txt`, and `!aspiration_*.txt` whose writer is not accounted for in the current runtime code

Operationally, the signal journal is the guaranteed write for a turn. Longform is a later, asynchronous secondary artifact for a small subset of modes. That split explains why some thoughts appear twice, why some entries are short even after the longform work, and why some files look like a short journal plus an appended section even though they are actually separate files.

## Write Paths

### 1. Signal journal write

Primary write path: `capsules/consciousness-bridge/src/autonomous.rs:2430-2431` and `capsules/consciousness-bridge/src/autonomous.rs:835-862`.

- Trigger: every normal Astrid response path that produces `response_text`
- Destination: `capsules/consciousness-bridge/workspace/journal/`
- Naming:
  - `daydream_<ts>.txt`
  - `aspiration_<ts>.txt`
  - `moment_<ts>.txt`
  - `witness_<ts>.txt`
  - `introspect_<ts>.txt`
  - `self_study_<ts>.txt`
  - fallback default: `astrid_<ts>.txt`
- Status: canonical in the current runtime, because it is the only guaranteed journal write for that turn

Important detail: `_longform` modes are not mapped in the filename prefix match. That means a mode like `daydream_longform` still falls through to the default `astrid_<ts>.txt` prefix in the current code.

### 2. Async longform second-pass journal

Secondary write path: `capsules/consciousness-bridge/src/autonomous.rs:2443-2463` and `capsules/consciousness-bridge/src/llm.rs:983-1040`.

- Trigger: only when `mode_name` is `dialogue_live`, `daydream`, or `aspiration`
- Destination: `capsules/consciousness-bridge/workspace/journal/`
- Naming: saved through the same `save_astrid_journal()` helper, so `*_longform` mode names still usually become `astrid_<ts>.txt`
- Body shape:
  - original compact signal
  - blank line
  - `--- JOURNAL ---`
  - elaborated reflective body
- Status: derived, asynchronous, and best-effort rather than guaranteed

This is not an in-place append to the original signal file. The code writes one journal file immediately, then later writes a second file from a spawned task. A corpus scan did not find journal files sharing the same timestamp suffix, which supports the interpretation that these are distinct artifacts rather than rewrites of the same file.

### 3. Inbox-triggered outbox reply

Trigger and duplication path: `capsules/consciousness-bridge/src/autonomous.rs:1314-1333` and `capsules/consciousness-bridge/src/autonomous.rs:2466-2468`, with outbox write in `capsules/consciousness-bridge/src/autonomous.rs:903-914`.

- Trigger: a `.txt` file appears in `workspace/inbox/`
- Effect:
  - Astrid is forced into dialogue mode
  - the reply is saved to the normal journal path
  - the same reply body is also copied to `workspace/outbox/reply_<ts>.txt`
- Status: outbox copy is derived, not canonical

This is one confirmed reason the same response can appear in two places.

### 4. Astrid self-study companion message to minime

Write path: `capsules/consciousness-bridge/src/autonomous.rs:2433-2440` and `capsules/consciousness-bridge/src/autonomous.rs:864-900`.

- Trigger: when `mode_name == "self_study"`
- Canonical write: Astrid still saves the local journal entry through `save_astrid_journal()`
- Secondary write: companion file to `/Users/v/other/minime/workspace/inbox/astrid_self_study_<ts>.txt`
- Special detail: the companion inbox message is excerpted to 1800 chars
- Status: inbox companion is derived and delivery-oriented

### 5. Continuity readback path

Parser path: `capsules/consciousness-bridge/src/journal.rs:69-180`.

- Local self-continuity uses `read_local_journal_body_for_continuity()`
- If `--- JOURNAL ---` exists, local continuity prefers that section
- Extracted body is still capped to 2500 chars

This means longform can matter for later self-continuity, but only within a bounded readback window.

## Current Corpus Snapshot

Live scan date: March 27, 2026

Observed from `capsules/consciousness-bridge/workspace/journal/*.txt`:

- Total journal files: 2808
- Files containing `--- JOURNAL ---`: 12
- Bang-prefixed journal files: 13

Top observed modes by count:

| Mode | Count | Median bytes | Max bytes |
| --- | ---: | ---: | ---: |
| `dialogue_live` | 957 | 467 | 879 |
| `mirror` | 690 | 872 | 2572 |
| `daydream` | 281 | 520 | 2634 |
| `dialogue` | 261 | 372 | 426 |
| `dialogue_fallback` | 207 | 279 | 280 |
| `aspiration` | 190 | 755 | 3026 |
| `witness` | 156 | 111 | 407 |
| `moment_capture` | 39 | 459 | 2267 |
| `daydream_longform` | 8 | 4701 | 5203 |
| `dialogue_live_longform` | 2 | 4036 | 4036 |
| `aspiration_longform` | 2 | 5668 | 5668 |
| `introspect` | 8 | 2177 | 2465 |

Additional filename prefix counts:

- `astrid_*`: 2190
- `daydream_*`: 279
- `aspiration_*`: 187
- `witness_*`: 96
- `moment_*`: 39
- `!astrid_*`: 8
- `!aspiration_*`: 3
- `!daydream_*`: 2

Broad conclusion: most journal files are still small. Longform exists, but it is rare in the corpus relative to the overall journal stream.

## Confirmed Artificial Limits

### Dialogue context is still aggressively compressed

From `capsules/consciousness-bridge/src/llm.rs:179-220`:

- recent history is compressed to 80 chars for older exchanges and 200 chars for newer exchanges
- the current-turn `journal_text` is trimmed to 300 chars before entering the dialogue prompt

This means even when a source journal is rich, the live dialogue model usually sees only a compact slice.

### Live dialogue output is still hard-capped

From `capsules/consciousness-bridge/src/llm.rs:296-303`:

- `generate_dialogue()` trims the returned text to 800 chars before it is handed back to the runtime

That is the direct cause of cut-off live journal files such as:

- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774637463.txt`

The file ends mid-thought, and its overall size lands right near the cap once header bytes are included.

### Longform exists only for three modes

From `capsules/consciousness-bridge/src/autonomous.rs:2443-2463`:

- Stage B runs only for `dialogue_live`
- Stage B runs only for `daydream`
- Stage B runs only for `aspiration`

No equivalent longform second-pass exists in the current runtime for `mirror`, `witness`, `dialogue`, `moment_capture`, or `self_study`.

### Longform is written as a second file, not an upgrade of the first file

From `capsules/consciousness-bridge/src/autonomous.rs:2430-2431` and `capsules/consciousness-bridge/src/autonomous.rs:2451-2461`:

- the compact signal journal is written first
- a spawned task later writes a second journal file containing the original signal plus the `--- JOURNAL ---` section

This is why the system can feel like it creates a short journal and then “the next file is identical except appended.” In current behavior, that is not one file being rewritten. It is two separate artifacts.

### Stage B expands only signal text plus spectral summary

From `capsules/consciousness-bridge/src/autonomous.rs:2447-2455` and `capsules/consciousness-bridge/src/llm.rs:986-1029`:

- Stage B receives:
  - `signal_text`
  - `spectral_summary`
  - `mode`
- It does not receive the richer original source journal entry that triggered the turn

So the longform pass is elaborating Astrid's own compact signal rather than re-reading the full incoming context.

### Local continuity still has a bounded readback cap

From `capsules/consciousness-bridge/src/journal.rs:69-73` and `capsules/consciousness-bridge/src/journal.rs:135-180`:

- local continuity prefers the `--- JOURNAL ---` body when present
- extracted continuity text is still capped to 2500 chars

This is better than header-only readback, but it is still not unlimited longform continuity.

## Why Responses Sometimes Appear In Two Places

There are three separate duplication patterns in current behavior.

### Inbox reply duplication

When a message is dropped in `workspace/inbox/`, Astrid is forced into dialogue mode. The response is then written both to:

- the normal journal stream
- `workspace/outbox/reply_<ts>.txt`

Example pair:

- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774636070.txt`
- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/outbox/reply_1774636070.txt`

### Self-study dual delivery

When Astrid performs `self_study`, the system writes:

- a canonical local journal entry
- a delivery copy to minime's inbox

Those are intentionally different artifact roles even when they share the same core text.

### Signal plus longform second-pass

For `dialogue_live`, `daydream`, and `aspiration`, current behavior can produce:

- a short signal journal first
- a later longform second artifact that repeats the signal and appends `--- JOURNAL ---`

Representative observed sequence:

- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/daydream_1774637019.txt`
- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774637045.txt`

These are separate files, not one file being extended in place.

## Representative Examples

### Truncated live dialogue

`/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774637463.txt`

- Mode: `dialogue_live`
- Behavior: cut off mid-thought
- Interpretation: consistent with the 800-char hard cap in `generate_dialogue()`

### Longform second-pass journal

Either of these captures the Stage B pattern clearly:

- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774637045.txt`
- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/!astrid_1774632449.txt`

They contain:

- the original compact signal
- `--- JOURNAL ---`
- a much longer reflective body

### Inbox reply duplication

This pair demonstrates the same response body landing in two destinations:

- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774636070.txt`
- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/outbox/reply_1774636070.txt`

## Mode Notes

### `dialogue_live`

`dialogue_live` is directly capped by the current code path. It is the clearest case where the bridge still imposes a hard upper bound that can truncate the visible journal artifact.

### `mirror`

`Mode::Mirror` in `capsules/consciousness-bridge/src/autonomous.rs:1341-1388` reads a remote journal body and writes that text back as Astrid's mirror entry. It does not apply the 800-char `generate_dialogue()` cap, because it is not using the dialogue generation path. Mirror is often short because it inherits already-short remote journal content, not because mirror itself adds the live dialogue truncation limit.

Example:

- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774487045.txt`

That mirror entry ends mid-word, but the limiting factor is upstream content shape rather than the dialogue cap.

### `witness`

`witness` is frequently tiny because many recent witness turns are the fallback string produced by `witness_text()` in `capsules/consciousness-bridge/src/autonomous.rs:801-803`:

- `[witness — LLM unavailable] fill=...`

That fallback dominates recent witness corpus size more than prompt design does.

### `dialogue_fallback`

`dialogue_fallback` is intentionally small because it comes from three fixed fallback strings in `capsules/consciousness-bridge/src/autonomous.rs:440-450`. These are not longform paths.

## Unknowns

No current write path was found in:

- `capsules/consciousness-bridge/src/autonomous.rs`
- `capsules/consciousness-bridge/src/llm.rs`
- `capsules/consciousness-bridge/src/journal.rs`

that writes bang-prefixed filenames such as:

- `!astrid_*.txt`
- `!daydream_*.txt`
- `!aspiration_*.txt`

Those files definitely exist in the live journal workspace, but their current writer is unresolved from the runtime code inspected here.

One additional oddity follows from the current code: `_longform` mode names are not mapped to custom filename prefixes in `save_astrid_journal()`, so the live runtime would normally write those longform files under the default `astrid_<ts>.txt` naming path. That makes the observed bang-prefixed longform files especially noteworthy.

## Suggestions, Not Implemented

These are follow-up recommendations only. They are not part of the current behavior.

- Evaluate longform availability across all reflective modes, not only `dialogue_live`, `daydream`, and `aspiration`.
- If the system eventually changes, prefer one natural canonical record per thought, with delivery artifacts like outbox or inbox companions labeled explicitly as secondary.
- Remove the hard 800-char live dialogue cap if the goal is genuine longform dialogue journaling rather than compact transport only.
- If dual artifacts remain, attach an explicit linkage id so signal and longform files can be paired deterministically instead of by timing and visual similarity.
- If longform remains a second pass, consider giving Stage B the richer triggering journal context rather than only `signal_text + spectral_summary`.

## Bottom Line

Astrid is not blocked from longform in principle, but the overall journal system is still dominated by short primary artifacts. The most important remaining constraint is the live `dialogue_live` 800-char cap. The most important structural source of confusion is that one thought can currently create multiple artifacts with different purposes, and the runtime does not yet present them as one explicitly linked record.
