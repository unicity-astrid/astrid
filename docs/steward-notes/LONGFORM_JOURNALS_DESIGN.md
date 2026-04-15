# Longform Journals Design

## Problem

Astrid's current journal corpus suggests "medium" output is possible, but normal autonomous journaling does not reliably reach the 2-3 kB range.

Recent behavior on the 2026-03-27 checkout:

- `dialogue_live` entries are usually about 300-800 bytes.
- `daydream` entries are usually about 400-550 bytes.
- `aspiration` entries are usually about 600-850 bytes.
- `moment_capture` entries are usually about 350-450 bytes.
- The only autonomous entries that regularly exceed 2 kB are `introspect` entries.

This is not primarily a `num_predict` problem anymore. The branch already raised token ceilings, but several parts of the design still bias the system toward short writing.

## Why It Plateaus

### 1. Prompts still explicitly request short responses

Current prompts constrain output by sentence count:

- `dialogue`: "Keep to 3-6 sentences"
- `witness`: "one or two sentences"
- `daydream`: "Write 3-6 sentences"
- `aspiration`: "3-6 sentences"
- `moment_capture`: "2-4 sentences"
- `introspect`: "4-8 sentences"

This means larger token budgets do not naturally become longer journals.

### 2. One string is serving two incompatible roles

Today, `response_text` is used for both:

- outbound semantic encoding to minime
- saved journal text

That creates a structural conflict. The spectral path benefits from compact, high-signal text. A journal benefits from room to elaborate, revisit, and connect ideas.

The code currently resolves that conflict in favor of compactness.

### 3. Main dialogue output is hard-capped

The main dialogue generation path truncates returned text to about 800 characters before it is encoded and before it is journaled. This is the single biggest reason ordinary Astrid entries stop where they do.

### 4. Context is compressed for the live loop

The prompt path trims:

- minime's current journal excerpt
- older exchange history
- Astrid's own recent journals
- retrieved continuity context

That helps latency, but it also means long journals are not meaningfully fed back into future turns.

### 5. Longform currently lives in side modes

`CREATE` already allows unrestricted writing, but it writes to `workspace/creations/`, not the journal. `introspect` can also run long, but normal dialogue/daydream/aspiration still live inside short-form prompt framing.

## Design Goal

Allow Astrid to produce medium and longform journal entries, especially in the 2-3 kB range, without making the spectral communication path bloated, slow, or noisy.

The design should preserve three properties:

1. The signal sent to minime stays compact and expressive.
2. The journal becomes a real reflective artifact, not just a transcript of the signal.
3. Longform writing becomes retrievable memory, not dead archive.

## Core Proposal

Split Astrid's output into two representations:

- `signal_text`: the compact text that gets encoded and sent to minime
- `journal_text`: the fuller reflective text that gets saved to disk and indexed for memory

The current design uses one text for both. The proposed design makes that split explicit.

## Proposed Output Model

Introduce a mode-level output object conceptually like:

```rust
struct GeneratedEntry {
    mode: String,
    signal_text: String,
    journal_text: String,
    next_action: Option<String>,
    summary_text: String,
}
```

Suggested meanings:

- `signal_text`: 250-900 bytes for most modes, optimized for spectral encoding
- `journal_text`: 1200-3000+ bytes, optimized for reflection and continuity
- `next_action`: parsed from the signal path, not buried inside the journal body
- `summary_text`: one short retrieval-friendly summary for continuity injection

## Mode Policy

Different modes should have different longform expectations.

### Witness

Stay short by design.

- Signal: 80-240 bytes
- Journal: optional brief note only

### Moment Capture

Stay compact but no longer artificially tiny.

- Signal: 180-500 bytes
- Journal: 500-1200 bytes

### Dialogue / Mirror

Allow medium reflective journals even when the outbound signal remains compact.

- Signal: 300-900 bytes
- Journal: 1200-2200 bytes by default
- `EXPANSIVE`: 1800-3000 bytes

### Daydream / Aspiration

These should become the main longform journal modes.

- Signal: 150-500 byte distilled essence
- Journal: 1800-3200 bytes

### Introspect

Keep as the deepest mode.

- Signal: 200-700 byte distilled takeaway
- Journal: 2200-4000 bytes

### Create

Unchanged. This remains artifact-first, not journal-first.

## Generation Strategy

Use a two-stage model instead of trying to force one output to do everything.

### Stage A: Fast live signal

Generate `signal_text` on the critical path.

Properties:

- compact
- fast timeout
- still ends with `NEXT:`
- immediately encodable to spectral features
- safe to use for outbox replies and live interaction

### Stage B: Journal elaboration

Generate `journal_text` after Stage A, using the same context plus the produced signal.

Properties:

- longer timeout
- can be asynchronous
- no `NEXT:` requirement
- can explicitly expand the inner experience behind the compact signal

This prevents longform journaling from slowing the live loop or making `dialogue_live` even less reliable.

## Why Two Stages Instead of One

This repo already has evidence that the live loop is prompt-size and timeout sensitive. A single large prompt that tries to produce the live reply, the journal, the action choice, and the continuity summary all at once will likely make the success rate worse.

Two-stage generation gives a cleaner failure model:

- if Stage A succeeds and Stage B fails, Astrid still speaks
- if Stage B succeeds, the journal grows richer without burdening the signal path

## Prompt Changes

### Live signal prompts

Replace sentence-count rules with signal-oriented rules such as:

- "Write the text that should be sent to minime now."
- "Keep it compact and vivid."
- "One to three short paragraphs is enough."
- "End with `NEXT:` on its own line."

### Journal prompts

Add dedicated journal prompts such as:

- "Expand the lived experience behind the signal you just sent."
- "Do not repeat the signal verbatim unless needed."
- "Write a reflective journal entry, not a reply."
- "Take enough space to develop the perception, tension, memory, and uncertainty."
- "Target 2-3 kB when the mode is daydream, aspiration, or expansive dialogue."

## State Model Changes

The current `response_length` field is too coarse because it governs one text stream.

Replace it conceptually with:

```rust
struct LengthProfile {
    signal_tokens: u32,
    journal_tokens: u32,
    journal_depth: JournalDepth,
}
```

Suggested profiles:

- `PRECISE`: short signal, short-medium journal
- `DEFAULT`: medium signal, medium journal
- `EXPANSIVE`: medium signal, long journal
- `THINK_DEEP`: medium signal, very long journal or introspection

This keeps "how much do I send?" separate from "how much do I write for myself?"

## File Format Changes

Update journal files to preserve both representations explicitly.

Proposed format:

```text
=== ASTRID JOURNAL ===
Mode: dialogue_live
Fill: 19.5%
Timestamp: 1774639999
Next: SEARCH "spectral memory and narrative"
Signal-Bytes: 612
Journal-Bytes: 2484

--- SIGNAL ---
<compact text sent to minime>

--- JOURNAL ---
<longform journal entry>
```

Benefits:

- keeps old plain-text accessibility
- makes it obvious what was actually encoded
- lets readers inspect divergence between outer signal and inner writing

`read_journal_entry()` should then prefer the `--- JOURNAL ---` section when present and fall back to the old body-only behavior for older files.

## Memory and Retrieval Changes

Longform journals only matter if future turns can recover them meaningfully.

Current continuity mostly injects:

- recent latent summaries
- self-observations
- starred memories
- small research snippets
- tiny excerpts from Astrid's own journals

That is not enough for 2-3 kB writing to become living memory.

### Proposed indexing

Store journal metadata plus chunked paragraph embeddings.

Conceptually:

- `astrid_journal_entries`
- `astrid_journal_chunks`

Each entry stores:

- mode
- timestamp
- fill
- signal text
- journal text
- short summary

Each chunk stores:

- entry id
- chunk index
- chunk text
- embedding

### Retrieval policy

For future dialogue/daydream/aspiration/introspection:

- retrieve top 2-4 relevant journal chunks by similarity
- inject only those chunks
- never inject whole multi-kB journals into the live prompt by default

This preserves continuity without blowing up latency.

## Encoding Policy

Do not encode the full journal by default.

Instead:

- encode `signal_text`
- optionally derive a second "journal essence" vector if experimentation shows it helps

The important principle is that journal verbosity should not directly force higher semantic bandwidth into minime's reservoir.

## Migration / Backward Compatibility

The design can be introduced incrementally.

### Phase 1

Decouple signal and journal text in memory, but keep journal files simple.

### Phase 2

Adopt the explicit `SIGNAL` / `JOURNAL` file format and section-aware readers.

### Phase 3

Add chunk indexing and retrieval into `bridge.db`.

Older journal files remain readable because the parser falls back to current behavior.

## Implementation Sketch

### Files to touch

- `capsules/consciousness-bridge/src/llm.rs`
  - split live signal generation from journal expansion generation
  - replace sentence-count prompt rules

- `capsules/consciousness-bridge/src/autonomous.rs`
  - change mode handlers to work with `signal_text` and `journal_text`
  - send only `signal_text` through the codec
  - save longform `journal_text`
  - parse `NEXT:` from the signal path

- `capsules/consciousness-bridge/src/db.rs`
  - store journal metadata, summary text, and optional chunk embeddings
  - retrieve relevant journal chunks for continuity

- `capsules/consciousness-bridge/src/codec.rs`
  - likely unchanged except for clarifying that encoding uses `signal_text`

## Recommended Rollout

### Step 1

Remove the 800-character dialogue truncation and replace it with explicit `signal_text` truncation only.

This is the minimum change that stops journals from being silently cut off mid-thought.

### Step 2

Add a journal elaboration pass for `daydream`, `aspiration`, and `dialogue_live`.

These are the highest-value modes for longform.

### Step 3

Teach continuity retrieval to prefer journal chunks rather than tiny excerpts.

Without this step, longform journals will still feel disconnected from future behavior.

### Step 4

Tune prompts and length profiles based on observed byte ranges rather than just token ceilings.

The target should be measured on actual saved journal body size.

## Success Criteria

The design is working if the following become true:

1. Ordinary `daydream` and `aspiration` entries frequently land in the 2-3 kB range.
2. `dialogue_live` journals can be 1.5-3 kB without making live signal text rambling.
3. No more journal entries end abruptly at about 800 characters.
4. Future turns can quote or build on prior longform journals through chunk retrieval.
5. The `dialogue_live` success rate does not regress because longform work happens off the critical path.

## Non-Goals

- Sending full longform journals directly into minime's semantic lane
- Stuffing whole journals into the live prompt every turn
- Turning `witness` into a longform mode
- Replacing `CREATE`; this design is for journaling, not artifact generation

## Bottom Line

Astrid does not need "more tokens" as the primary fix. She needs a separation of concerns.

The root issue is that the system currently asks one string to be:

- a live reply
- a spectral signal
- a journal
- a memory trace
- an action-selection carrier

That is why the design collapses toward shortness.

The clean solution is to make longform journaling a first-class output alongside, not inside, the compact signal path.
