# #1238 thread-reply delivery: measured boundary

Investigation record for
[#1238](https://github.com/shinaoka/koushi-matrix/issues/1238). No product
behaviour changed: this note and a private-data-free diagnostic are the whole
deliverable, per the council decision to establish where an unopened thread's
replies stop reaching the client before designing anything on top of them.

## Question

The issue asks for a per-thread unread indicator for threads the user has not
opened, and for thread replies to raise the room badge. Before building that, we
need to know which of these boundaries actually hold on a real homeserver:

1. the homeserver advertises the thread-subscription capability;
2. a thread subscription request is sent and accepted;
3. the sync response delivers that thread's reply events;
4. the SDK thread cache reports the resulting unread counts;
5. Core consumes them.

## Method

`thread_delivery` diagnostic (temporary, private-data-free): Core records, once
per room, the capability advertisement, whether a subscription request was sent
and accepted, and whether the SDK thread cache yields unread counts; the QA prints
`thread_delivery=recorded` plus a booleans-and-counts detail line. The `thread` QA
stage also keeps A's room subscribed before B replies, so the reply arrives as a
live diff rather than only inside a re-subscription's initial window.

Command (both homeservers, disposable, local):

```bash
node scripts/desktop-headless-local-qa.mjs --run --server=<tuwunel|synapse> \
  --core --scenario=thread --cargo-profile=ci --timeout-ms=600000
```

## Environment

| Component | Value |
| --- | --- |
| tuwunel | 1.7.1 (stock QA fixture) |
| Synapse | v1.157.0 (stock QA fixture; also run once with `experimental_features.msc4306_enabled: true`) |
| Vendored matrix-rust-sdk | gitlink `55e51ffe` |
| Koushi | branch `research/1238-thread-delivery` from `16c78fb5` (v0.20.3) |

## Measurements

| Boundary | tuwunel 1.7.1 | Synapse v1.157.0 | Synapse + `msc4306_enabled` |
| --- | --- | --- | --- |
| Room timeline carries a thread-reply item | no (`threaded_items=0` of 16) | no (`threaded_items=0` of 17) | no |
| Server advertises `org.matrix.msc4306` | no | no | yes (the SDK reports the capability) |
| Subscription request/outcome | not reached | not reached | not reached |
| SDK thread cache unread | 0 | 0 | 0 |
| Room badge after a thread reply | unchanged | +1 | +1 |

The badge rise on Synapse is not evidence of thread delivery. Disabling the
thread subscription entirely produced the identical token sequence
(`thread_room_badge=ok before=0 after_reply=1`, `main_read=ok badge=1`,
`cleared=ok`), and tuwunel did not move at all in this experiment. The rise is
the room-level homeserver count that the #1176 top-up reads, and that count
already includes the thread reply.

> Correction (2026-10-10): the server roles above were misread. Measured, tuwunel
> reports real thread-inclusive `notification_count` values while Synapse
> v1.157.0 reports dummy zeros, so the "dummy zero server counts" attribution
> belongs to Synapse, not tuwunel. See `docs/architecture/state-machine.md` and
> #1176; do not reuse the original attribution.

## Boundary reached

The diagnostic did not fire on any configuration: the hook reads
`navigation_items`, which do not carry the thread-summary overlay, so no root is
found there. That is the first boundary to fix, and it is also why an earlier
per-root unread projection stayed dormant — its root discovery used the same
`navigation_items` and therefore never saw a root.

So the investigation has not yet reached boundary 1. The measurements above are
from room-timeline items and the room badge, not from the diagnostic.

## What this does not claim

- It does not claim that the homeservers cannot deliver thread events; only that
  this client did not observe them under these fixtures.
- It does not claim that the SDK extension is broken; the extension is enabled
  only when the server advertises `org.matrix.msc4306`, which the stock fixtures
  do not, and Synapse's experimental flag turns the capability on without (so
  far) delivering events.
- It does not claim a product decision. The room-badge composition question is
  recorded in the canon instead: a thread contribution must not be added to room
  totals without a proven non-overlapping decomposition.

## Next steps

1. Move the diagnostic hook to the overlaid items (or the thread projection
   service) so boundary 1 is actually measured.
2. Then measure boundaries 2-5 in order, recording the last confirmed and the
   next unconfirmed boundary.
3. Only after event delivery is confirmed, design the per-thread indicator, with
   the room-badge decomposition proven first.

## Correction (2026-10-10)

The "no data" reading above was a measurement error. The diagnostic hook reads
`navigation_items`, where a thread summary usually is not, so it rarely fired and
the printed `cache_read=no unread=0` values were untouched defaults, not
observations. In a run where it did fire, the SDK thread cache reported
`unread=1 notifications=1` for the unopened thread on tuwunel.

The #1259 chip dot reads the SDK thread cache directly (`EventCache::thread` ->
`read_receipts`) and does not need the diagnostic: on both tuwunel and Synapse the
headless thread QA now shows and clears the dot while the diagnostic reports
`subscribe=no cache_read=no`, so no subscription this client makes is what fills
the cache — the thread receipts arrive with ordinary room sync. The boundary table
above should be read as "the hook did not measure", not as "the client cannot know".

Two consequences the hook's silence hid:

- The room timeline actor must re-read the cache *after* a threaded read: the SDK
  learns the read only from the sync echo of its receipt, which lands after the
  send success that triggers the refresh. The actor therefore retries the refresh a
  bounded number of times, which is what makes the dot actually clear.
- The value is per root and still contributes to nothing room-level; the canon
  boundary above stands as written.

## Product change (2026-10-10): the proven decomposition

The canon now carries the proof, so the badge half of the issue is implemented
rather than deferred.

### Decomposition

The proof is the SDK event-cache boundary, not a badge:

- `RoomReadReceiptEventFilter` (`vendor/matrix-rust-sdk/crates/matrix-sdk/src/
  event_cache/caches/read_receipts.rs`) filters an event out when
  `extract_thread_root` finds a thread root, and also filters edits/reactions whose
  relation target is a known thread reply; `receipt_thread_matches` accepts only
  `ReceiptThread::Unthreaded | Main`.
- `ThreadReadReceiptEventFilter` is built from one thread's own cache state and
  accepts only `ReceiptThread::Thread(thread_id)` for that thread.

The room counters (`num_unread_messages` / `num_unread_notifications` /
`num_unread_mentions`, computed by `compute_unread_counts` with the room filter)
and the per-thread counters therefore describe disjoint scopes: every reply
belongs to exactly one thread and to no room-cache unread. A mirror of the
per-thread counters can be stale and undercount a thread, but it can never invent
or duplicate a reply.

### Composition

`koushi_state::room_activity_unread_count` is the single badge helper:

```
max(unread_count + thread_unread_count,
    notification_count,
    highlight_count + thread_highlight_count)
```

with `marked_unread` still the zero-count fallback. `notification_count` is
max'd, never summed: a homeserver whose own room counter already includes thread
replies (the Synapse measurement above) must not double count, while a server
with dummy zeros (Synapse Simplified Sliding Sync, tuwunel before a client
receipt is anchored) leaves the client decomposition in charge.

`RoomSummary.unread_count` stays the main-only navigation value; read markers,
"Read up to here" and first-unread keep excluding thread replies. The per-room
`thread_unread_count` / `thread_highlight_count` are reducer-derived sums of the
per-root `ThreadUnreadObserved` values the room timeline actors already read for
the Threads-list chip, so React renders them and never computes them.

### Evidence

Focused unit RED (`cargo test -p koushi-state --test thread_badge_state`), with
the badge fold temporarily reverted to the pre-#1238 formula: 4 of 8 tests fail,
including "main unread 1 plus thread unread 3 badges 4" (observed 1). Restored:
8 pass. The same file pins the max-not-sum server counter, the muted display
count, mention styling, the main-read-never-clears-thread rule, and that a
room-list snapshot re-derives rather than drops the thread totals.

Headless QA RED/GREEN (`--server=tuwunel --core --scenario=thread`), with the
tuwunel room badge measured on a room whose main counters are all zero:

| Token | Meaning | Observed |
| --- | --- | --- |
| `thread_room_badge=ok` | a remote thread reply alone raises the badge 0 -> 1 | `badge=1 thread_unread=1 notifications=0` |
| `thread_room_badge_cleared=ok` | the threaded read takes it back to 0 | `badge=0 thread_unread=0` |
| `thread_room_badge_main_read_kept=ok` | a second reply re-arms it, then a room-scoped receipt leaves it at 1 | `badge=1 thread_unread=1` |

The stage anchors one main-timeline read receipt before the thread reply, so
`latest_active` is set and the #1176 server top-up cannot contribute; the
`notifications=0` column is that check. With the badge fold reverted, the same
scenario fails at `thread_room_badge` because `unread_count` never moves. All
diagnostics print counts and booleans only.
