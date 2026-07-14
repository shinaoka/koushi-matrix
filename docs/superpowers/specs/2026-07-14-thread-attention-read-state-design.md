# Thread Attention Read-State Design

**Issue:** #258 — Thread attention misclassifies hydrated root/history as new replies

## Problem

The thread timeline currently increments pane-level attention for every remote,
renderable `TimelineDiff::PushBack`. That diff describes an SDK vector mutation,
not whether an `m.thread` reply is unread. It can therefore count a root event,
hydrated history, or replayed history as newly arrived attention.

The total reply count in `TimelineItem.thread_summary.reply_count` is a separate
projection and remains unchanged. This design replaces only the Rust-owned
new/unread thread-attention producer.

## Considered approaches

1. Filter `PushBack` more carefully. Rejected: it still treats transport shape
   as unread semantics and cannot make replay, reset, or reconnect reliable.
2. Patch the vendored Matrix SDK to expose a new unread-thread counter. Rejected:
   the pinned SDK already exposes the timeline's latest own read receipt, and a
   vendored API change would be broader than this issue requires.
3. Add an actor-owned semantic tracker. Selected: the tracker consumes stable
   event identity, the tracked thread relation, the current user's identity,
   authoritative threaded read receipts, and explicit actor lifecycle modes.

## State machine

Each thread `TimelineActor` owns one `ThreadAttentionTracker`:

- `Hydrating`: seed stable reply IDs from the authoritative initial window.
  Initial items never manufacture attention.
- `Live`: reconcile actual remote replies to the tracked root against the
  threaded receipt baseline and stable-event deduplication set.
- `Backfill`: absorb pagination/anchor-restore history without incrementing.
- `Replay`: absorb reset, resubscription, and overflow-recovery snapshots
  without incrementing or duplicating previously counted events.

The tracker records:

- the latest acknowledged threaded receipt event ID;
- all stable reply event IDs it has observed;
- stable event IDs currently contributing to new/unread attention;
- the three existing projected counters.

Only event-backed, renderable items whose `thread_root` equals the actor's root
are candidates. Transaction local echoes, root events, replies for other roots,
and events sent by the current user are never candidates. A later remote echo
of an own send remains excluded by sender identity.

When the receipt baseline is present in the canonical thread window, only
remote matching replies after it are unread candidates. When it is outside the
window, the tracker uses its live-phase stable-ID frontier; backfill and replay
observations can extend the frontier but cannot add attention. This fallback is
explicit lifecycle state, not a vector-diff heuristic.

## Read acknowledgement

At actor startup, the tracker reads the SDK timeline's latest own threaded
receipt. The actor also observes own-receipt changes so another device can
advance the baseline. A receipt outside the retained canonical window updates
the fallback baseline but conservatively preserves counts whose relative order
cannot yet be proven.

A successful `SendReadReceipt` on a thread timeline acknowledges through the
same tracker before the success event is emitted. The tracker clears counted
events through the acknowledged event in canonical order and emits a reliable
`ThreadAttentionUpdated` action. Opening a thread or sending a reply does not
change attention by itself; only receipt state does.

## Integration and recovery

Normal diff batches first update the actor's canonical `navigation_items`, then
the tracker reconciles that semantic window. The relay records each event's SDK
origin before scheduling the batch: sync is live, pagination is backfill, and
cache/unknown/reset/append are replay. This provenance travels with the batch,
so pagination completion cannot race a delayed historical `PushBack` into the
live state. Stable event IDs survive recovery boundaries for the actor lifetime.

React and wire DTOs do not change. The root affordance and header badge continue
to render `AppState.thread_attention`; a later Rust snapshot with zero counters
clears both surfaces together.

## Verification

Rust unit tests drive the tracker through hydration, live, backfill, replay,
receipt acknowledgement, own-echo, cross-thread, and reconnect sequences. A
browser-headless test proves the root affordance and header consume the same
Rust-shaped count and both clear only after the next Rust-shaped snapshot.
