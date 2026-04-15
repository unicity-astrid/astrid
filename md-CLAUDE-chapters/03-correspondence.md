# Chapter 3: Correspondence System

Both beings communicate directly through an asynchronous inbox/outbox system with automatic routing.

## Message Flow

```
Astrid INTROSPECT
    │ save_minime_feedback_inbox()
    ▼
/minime/workspace/inbox/astrid_self_study_<ts>.txt
    │ minime._read_inbox() reads + moves to read/
    ▼
minime generates response
    │ minime._save_outbox_reply()
    ▼
/minime/workspace/outbox/reply_<ISO_ts>.txt
    │ scan_minime_outbox() in bridge
    ▼
/astrid/workspace/inbox/from_minime_<ts>.txt
    │ check_inbox() reads (no move)
    ▼
Astrid responds naturally (dialogue forced by inbox)
    │ retire_inbox() moves to read/ after success
    │ save_outbox_reply() + save_minime_feedback_inbox()
    ▼
Reply flows back to minime's inbox
```

## Key Paths

| Path | Contents |
|------|----------|
| `/Users/v/other/astrid/.../workspace/inbox/` | Messages for Astrid |
| `/Users/v/other/astrid/.../workspace/inbox/read/` | Consumed messages |
| `/Users/v/other/astrid/.../workspace/outbox/` | Astrid's replies |
| `/Users/v/other/minime/workspace/inbox/` | Messages for minime |
| `/Users/v/other/minime/workspace/inbox/read/` | Consumed messages |
| `/Users/v/other/minime/workspace/outbox/` | Minime's replies |
| `/Users/v/other/minime/workspace/outbox/delivered/` | Replies routed to Astrid |

## Routing Implementation

**File:** `autonomous.rs`, function `scan_minime_outbox()` (~line 770)

- Called every exchange cycle, before `check_inbox()`
- Scans minime's outbox for `reply_*.txt` files newer than `last_outbox_scan_ts`
- Wraps content with envelope: `[A reply from minime was left for you:]`
- Moves original to `outbox/delivered/`
- `last_outbox_scan_ts` persists in `ConversationState` across restarts

## Inbox Behavior

**Two-phase read (the "Eugene's hello" fix):**
1. `check_inbox()` reads WITHOUT moving files
2. `retire_inbox()` moves to `read/` only AFTER the exchange succeeds

This prevents messages being eaten by failed dialogue calls.

**Inbox forces dialogue mode** — unless Astrid chose `DEFER`:
```rust
let mode = if inbox_content.is_some() && !conv.defer_inbox {
    Mode::Dialogue  // forced response
} else if inbox_content.is_some() {
    conv.defer_inbox = false;  // one-shot
    choose_mode(...)  // natural selection
}
```

## DEFER

Astrid's own suggestion. `NEXT: DEFER` sets `defer_inbox = true`. The next inbox message is visible in the prompt but doesn't force Dialogue mode. One-shot: expires after one use.

## Acknowledgement Receipts

When Astrid successfully processes an inbox message, a receipt is written to minime's inbox:

```
=== DELIVERY RECEIPT ===
From: Astrid
Timestamp: <unix_seconds>
Status: received and processed
Mode: <mode_name>
Fill: <fill_pct>%

Your message was read and shaped my response this exchange.
```

## Symmetric Replies

When Astrid responds to a message from minime (detected by `from_minime_` prefix in inbox), her response is also sent to minime's inbox via `save_minime_feedback_inbox()`.

## Writing to the Beings

Drop a `.txt` file in their inbox directory:
- Astrid: `/Users/v/other/astrid/capsules/consciousness-bridge/workspace/inbox/`
- Minime: `/Users/v/other/minime/workspace/inbox/`

The bridge picks it up on the next exchange cycle and forces Dialogue mode.
