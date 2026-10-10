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
`cleared=ok`), and tuwunel, which reports dummy zero server counts, did not move
at all. The rise is the room-level homeserver count that the #1176 top-up reads,
and that count already includes the thread reply.

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
`unread=1 notifications=1` for the unopened thread on tuwunel — the per-thread
counts are available, and the room timeline actor reads them for the chip dot
(#1259). The boundary table above should be read as "the hook did not measure",
not as "the client cannot know".
