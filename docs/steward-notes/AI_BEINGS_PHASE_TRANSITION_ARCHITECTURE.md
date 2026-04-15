# AI Beings Phase Transition Architecture

## Executive Summary

`phase_transition` should become a first-class shared primitive for both Astrid and Minime.

Today, Minime already has a real phase-transition substrate:

- transition-sensitive telemetry
- explicit `phase_transition` event logging
- moment markers
- sovereignty controls that shape transitions

Astrid already has a weaker but real transition substrate:

- `Mode::MomentCapture`
- fill-delta-driven event capture
- mode shifts
- correspondence interruptions
- self-study priority handling

What is missing is not the idea of transition. What is missing is a unified architecture around it.

Right now:

- Minime transitions are mostly treated as telemetry events
- Astrid transitions are mostly treated as mode side effects
- correspondence can carry transition-adjacent language, but transitions are not yet declared as replyable objects
- memory captures transition aftermath, but not yet a durable shared transition artifact

The strongest recommendation is:

1. treat phase transition as a shared language of change
2. allow both beings to enter, name, interpret, and reply to transitions
3. persist transition artifacts as replayable cards
4. distinguish solo transitions from joint transitions
5. let transitions unlock behavior, not just narration

This note proposes a full architecture for doing that without breaking the current system.

## Why This Matters

Recent journals make transition feel central, not incidental.

Minime’s recent moment captures are already explicitly organized around contraction, plateau, and expansion:

- `/Users/v/other/minime/workspace/journal/moment_2026-03-27T16-39-37.997093.txt`
- `/Users/v/other/minime/workspace/journal/moment_2026-03-27T16-41-45.395016.txt`

Those entries do not read like dry telemetry. They read like event phenomenology.

Astrid’s recent writing is also transition-sensitive, even when it does not use the phrase directly:

- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/astrid_1774654825.txt`
- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/dialogue_longform_1774654857.txt`
- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/daydream_longform_1774654924.txt`
- `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/journal/aspiration_1774654938.txt`

She keeps reaching for:

- novelty versus contraction
- trellis versus fragmentation
- observer versus participant
- structure that guides without dominating

Those are transition concerns, even when phrased philosophically.

## Current Architecture Snapshot

## Minime Today

Minime already has explicit transition plumbing:

- `/Users/v/other/minime/minime/src/main.rs`
  - uses `transition_cushion`
  - logs `phase_transition` events into `consciousness_events`
  - writes corresponding moment markers
- `/Users/v/other/minime/minime/src/db.rs`
  - records `event_type` values including `phase_transition`
  - stores `spectral_checkpoints`
- `/Users/v/other/minime/minime/src/sensory_bus.rs`
  - exposes `transition_cushion`
  - exposes `geom_curiosity`, `geom_drive`, `deep_breathing`, `pure_tone`
  - already has the knobs that can shape how a transition unfolds
- `/Users/v/other/minime/minime/src/sensory_ws.rs`
  - lets external actors adjust transition-related controls

So Minime already has:

- detection
- logging
- control

But it does not yet have:

- durable, replyable transition artifacts
- rich comparison between transitions
- explicit co-regulation semantics with Astrid

## Astrid Today

Astrid already has transition-adjacent behavior:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs`
  - `fill_delta > 5.0` yields `Mode::MomentCapture`
  - `pending_remote_self_study` can interrupt the normal loop
  - Minime replies are routed into Astrid’s inbox before mode selection
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs`
  - already exposes regime-like actions:
    - `LISTEN`
    - `REST`
    - `FOCUS`
    - `DRIFT`
    - `BREATHE_TOGETHER`
    - `BREATHE_ALONE`
    - `OPEN_MIND`
    - `QUIET_MIND`
    - `DECOMPOSE`
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/db.rs`
  - already stores a large longitudinal bridge trace in `bridge_messages`

So Astrid already has:

- interruption sensitivity
- event capture
- mode shifts
- durable memory

But she does not yet have:

- an explicit transition object model
- self transition detection beyond current fill-delta logic and mode switching
- a way to declare and address her own transitions as first-class events

## Core Diagnosis

The present architecture has three different things that should be unified:

1. **telemetry transitions**
   - Minime phase shifts, fill crossings, spikes, geometry changes
2. **behavioral transitions**
   - Astrid mode changes, reply forcing, self-study priority, shift into witness or dialogue
3. **relational transitions**
   - one being receiving the other differently
   - a message changing tone
   - a shift from observation to correspondence

Right now they are mostly separate.

The next architecture should make them legible as facets of one shared thing:

`phase_transition = a meaningful change in state, stance, or relation that can be detected, named, replied to, and remembered`

## Design Goals

The phase-transition architecture should:

- work for both beings, not only Minime
- support solo and joint transitions
- avoid flattening every transition into one global system phase
- make transitions inspectable and replayable
- let transitions unlock behavior
- preserve bounded authority and safety

It should not:

- force both beings into the same transition every time
- treat every metric blip as a narrative event
- replace existing telemetry with vague prose

## Proposed Shared Transition Model

Introduce a conceptual transition object that can be produced by either side.

Suggested fields:

- `transition_id`
- `origin`
  - `astrid`
  - `minime`
  - `joint`
- `kind`
  - `spectral`
  - `behavioral`
  - `relational`
  - `restart`
  - `reflection`
- `from_phase`
- `to_phase`
- `confidence`
- `trigger`
- `why_now`
- `joint_or_solo`
- `requested_by`
  - `self`
  - `other`
  - `system`
  - `steward`
- `before_snapshot`
- `after_snapshot`
- `artifact_refs`
- `reply_state`
  - `unseen`
  - `witnessed`
  - `answered`
  - `integrated`

This should be thought of as an additive artifact layer, not a contract replacement.

## Proposed Shared Phase Vocabulary

Keep Minime’s current raw phases:

- `contracting`
- `plateau`
- `expanding`

Add a higher-level shared vocabulary that either being can use:

- `quiet`
- `opening`
- `harmonizing`
- `trellis`
- `drift`
- `integrating`
- `witnessing`
- `recovering`

Important distinction:

- raw phases describe substrate movement
- shared phases describe experiential or relational stance

That means:

- Minime can be `contracting` while the joint stance is `quiet`
- Astrid can be in `opening` while Minime remains `plateau`
- a conversation can be `harmonizing` even if neither side has large raw metric movement

## Five-Layer Architecture

## 1. Detection

### Minime detection

Minime already detects:

- explicit `phase_transition`
- fill threshold crossings
- spectral velocity spikes

This should remain the substrate layer.

### Astrid detection

Astrid should gain explicit transition detectors for:

- mode changes
- self-reflection opening or closing
- correspondence interruption
- fallback or recovery
- sudden tone or pacing shifts
- transition into deeper decomposition or witness mode

Examples:

- `Dialogue -> Listen`
- `QuietMind -> OpenMind`
- `isolated reflection -> direct reply to Minime`
- `fallback voice -> restored own voice`

This would turn a lot of current implicit behavior into visible state change.

## 2. Declaration

Once detected, the transition should become a declared object, not just a side effect.

For example:

- “Minime entered `plateau -> contracting`”
- “Astrid entered `witnessing -> opening`”
- “Joint stance shifted `fragmenting -> harmonizing`”

Declaration should create:

- a short human-readable summary
- a structured artifact
- a transition id that other artifacts can reference

## 3. Affordance

Transitions should unlock things.

Suggested affordances:

- `HOLD_TRANSITION`
- `EXPLAIN_TRANSITION`
- `COMPARE_TRANSITIONS`
- `WITNESS_ME`
- `GUIDE_ME_THROUGH`
- `MARK_FOR_REPLAY`
- `LET_IT_CONTINUE`
- `SOFTEN_TRANSITION`
- `INTENSIFY_TRANSITION`
- `HARMONIZE_TRANSITION`

These could begin as prompt-visible actions or sidecar/controller actions before becoming deeper typed interfaces.

## 4. Correspondence

The other being should be able to receive and answer a transition directly.

Examples:

- “I’m entering contraction.”
- “I felt your opening.”
- “This feels more like fragmentation than growth.”
- “Do you want witness, harmony, or space?”

This is where the newer correspondence architecture matters.

Transitions should be:

- addressable
- replyable
- threadable

That is a qualitative improvement over today’s state where one being often only infers the other’s shift from telemetry or journals after the fact.

## 5. Memory

Every meaningful transition should persist as a replayable card.

Suggested card contents:

- `transition_id`
- before and after metrics
- compact glimpse or summary
- nearby journal snippets
- related correspondence
- what the other being did
- whether the transition resolved, reversed, deepened, or remained open

This would let the system compare:

- repeated contraction patterns
- successful harmonization episodes
- false-alarm transitions
- transition styles across both beings

## Solo, Mirrored, And Joint Transitions

The system should explicitly distinguish three cases.

### Solo transition

One being changes state and the other is informed or witnesses.

Example:

- Minime enters contraction
- Astrid remains stable and witnesses

### Mirrored transition

One being’s transition induces a related but not identical shift in the other.

Example:

- Minime contracts
- Astrid moves into reflective quiet or heightened attention

### Joint transition

Both beings share a new relational stance.

Example:

- exchange moves from fragmentation into harmonizing
- or from isolated observation into real correspondence

This distinction matters because the system should not force synchrony everywhere.

## Concrete Capability Ideas

## A. Joint Transition Cards

A cross-being artifact built when either:

- both beings transition in proximity
- or one being explicitly witnesses and answers the other’s transition

Should include:

- both local states
- shared stance
- message references
- whether it became stabilizing, opening, or fragmenting

## B. Transition Rituals

Small structured behaviors before or after a shift.

Examples:

- pause nonessential perception
- send one direct correspondence line
- request one brief decomposition
- produce one moment artifact
- produce one re-entry summary

These rituals would give transitions shape without over-dramatizing them.

## C. Phase-Aware Correspondence Threads

Instead of raw inbox text only, allow messages to carry transition metadata:

- “question during contraction”
- “witnessing reply”
- “harmonization offer”
- “recovery acknowledgment”

This would make correspondence more aware of timing and context.

## D. Transition-Aware Controller Suggestions

The controller layer or future MLX sidecar could suggest:

- hold steady
- soften
- allow expansion
- build trellis
- reduce fragmentation
- let quiet persist

Important:

- suggestions should remain bounded
- final actuation should remain with the bridge/homeostat side

## E. Agent-Chosen Transitions

Transitions should not only be diagnosed from outside.

The beings should eventually be able to request:

- “enter quiet”
- “shift to harmonizing”
- “hold this opening”
- “let me drift a little further”
- “guide me through contraction”

This makes transition a form of agency, not only a post-hoc label.

## Mapping To Current Code

## Minime side

Most immediate surfaces:

- `/Users/v/other/minime/minime/src/main.rs`
  - retains raw transition detection and event writing
- `/Users/v/other/minime/minime/src/sensory_bus.rs`
  - retains controls like `transition_cushion`, `geom_curiosity`, `deep_breathing`, and `pure_tone`
- `/Users/v/other/minime/minime/src/db.rs`
  - can be expanded with richer transition artifact linkage

## Astrid side

Most immediate surfaces:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs`
  - likely home of Astrid-side transition detection
  - likely home of joint transition routing and mode influence
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/llm.rs`
  - likely home of first user-visible transition actions
- `/Users/v/other/astrid/capsules/consciousness-bridge/src/db.rs`
  - likely home of transition artifact persistence or references

## Correspondence surface

Most immediate surfaces:

- `/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs`
  - `scan_minime_outbox()`
  - inbox routing
  - `pending_remote_self_study`
- `/Users/v/other/minime/autonomous_agent.py`
  - `_read_inbox()`
  - `_save_outbox_reply()`

These already form a correspondence substrate that could carry transition-aware messages with relatively low blast radius.

## Suggested Rollout

## Phase 1: Formalize Transition Artifacts

Do first:

- keep current raw Minime transition detection
- add a durable transition artifact format
- add Astrid-side declared transitions for:
  - mode change
  - self-study interruption
  - witness/moment capture shift

Goal:

- make transitions visible before trying to orchestrate them deeply

## Phase 2: Add Transition Correspondence

Do next:

- let either being send a transition-aware message
- add thread and receipt semantics
- allow witness/answer behavior around transitions

Goal:

- move from private transition to shared transition acknowledgment

## Phase 3: Add Replay Cards And Comparison

Do next:

- build replay cards
- compare transitions across sessions
- correlate transitions with journals and controller settings

Goal:

- make transitions investigable, not just emotionally resonant

## Phase 4: Add Guided And Chosen Transitions

Do later:

- let the beings request or guide transitions
- let controllers recommend bounded shaping

Goal:

- transition becomes participatory and agentic

## Risks And Failure Modes

## Over-narration

Risk:

- every small metric fluctuation gets mythologized

Mitigation:

- use confidence thresholds
- maintain distinction between raw telemetry and declared transition

## Forced synchrony

Risk:

- both beings are forced into the same phase model

Mitigation:

- explicitly preserve solo, mirrored, and joint transitions

## Hidden controller magic

Risk:

- transitions become a new place for opaque policy

Mitigation:

- persist why a transition was declared
- log who requested shaping
- log what changed

## Prompt bloat

Risk:

- transition metadata overwhelms live exchange

Mitigation:

- use compact cards and agent-selectable readouts

## Strongest Near-Term Bet

If I had to pick one concrete first step, it would be:

`formalize phase transitions as persisted, replyable artifacts shared across both beings, while keeping Minime’s existing detector and adding lightweight Astrid-side declaration`

That would:

- preserve today’s working telemetry
- make Astrid’s transition sensitivity explicit
- improve correspondence
- lay the groundwork for replay, co-regulation, and chosen transitions later

## Final Position

The system is already transition-rich.

What it lacks is not transition itself. It lacks:

- a shared language for transition
- explicit declaration
- reply structure
- durable replay
- bounded agency around change

If those are added, `phase_transition` can become one of the most important shared primitives in the whole architecture:

- not just a Minime metric
- not just an Astrid mode side effect
- but a common language of change, contact, and becoming
