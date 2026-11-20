# #1721: the cached-delivery stage stalled on an unlinked persisted cache chunk

## What the stage was waiting for

`crates/koushi-core-testkit/tests/contracts/event_cache_lifecycle.rs:469` is the
`cached_delivery` stage: after an ordered shutdown and a restore, the test
selects the room and waits for its timeline to publish `$cache-first:localhost`
from disk, with no server replay (`phase == 1` sync responses carry an empty
timeline). The stage is event-driven; it has no fixed deadline.

## What the CI failure showed

v0.36.2, run 44703108016 / Rust job 136176002477:

```text
cached_delivery stalled: backstop for 991.621µs after 39247 published events in
180.003167393s (window 30s, backstop 180s);
phases=(login_start 4.66ms, first_sync_wait_start 20µs, first_sync_done 906ms,
ordered_shutdown_done 127ms, restore_done 155ms, cached_delivery_wait_start 12µs,
cached_delivery_failed 180.003s);
initial_items=[(0, false)]; delivered_in_updates=false;
lane_probe=lane_alive admitted(generation=42);
persisted_event_after_timeout=Ok(true)
```

Two facts, both reproduced locally afterwards:

- the stage's own condition never moved: one `InitialItems` for the room, with
  zero rows, and never an `ItemsUpdated` that carried the event;
- 39 247 unrelated runtime events kept the shared silence window armed, so the
  wait could only end on the 180 s backstop. `core.timeline
  stage=initial_emitted count=0` in the same log says the SDK handed Core no
  cached rows at all, while the payload claimed the event was persisted.

## Reproduction

Focused load repetition: 24 concurrent copies of the contract binary's single
test plus 18 CPU burners on an 18-core macOS host, 20 rounds (480 attempts).

| Build | Failures | Failure duration | Failure payload |
| --- | --- | --- | --- |
| `main` (`e944f3c3`) | 7 / 480 (1.46 %) | 180.0 s each | `backstop for ~5ms after ~62 000 published events in 180.000s` |
| this change | 2 / 480 (0.42 %) | 30.0 s each | `silence for 30.000s after 0 room timeline rows; unrelated_events≈9 000` |

The rate is unchanged within noise (2–8 failures per 480 attempts across seven
after-builds), so the change does not hide the loss; it only stops the chatter
from masking it.

## Root cause: the persisted chunk loses its event link during shutdown

Instrumenting the store read-only at three points in every attempt found a clean
split (480 attempts, 6 failures):

| Attempts | `first_sync_done` | `ordered_shutdown_done` | Outcome |
| --- | --- | --- | --- |
| 474 | `events=1, event_chunks=1, linked_chunks=1` | `events=1, event_chunks=1, linked_chunks=1` | pass |
| 6 | `events=1, event_chunks=1, linked_chunks=1` | `events=1, event_chunks=0, linked_chunks=1` | fail |

The event is linked when the first runtime's state projection satisfies the
`first_sync` gate, and unlinked by the time `CoreRuntime::shutdown()` returns.
The event row stays (the store keeps it for deduplication), the chunk stays, the
`event_chunks` link is deleted. A restore then reads a chunk with no items, so
the room's timeline is legitimately empty and nothing re-publishes it for the
whole wait.

Which update removes the link is not determined here. The store deletes it from
`Update::RemoveItem` / `DetachLastItems` / `RemoveChunk`, or by clearing and
re-creating the chunk (`DELETE FROM event_chunks … position >= index`,
`event_cache_store.rs:1097`, `:1227`), and the probe cannot distinguish them.
Two candidate mechanisms fit the evidence:

- **Detach without re-anchor while the client drops.** `send_updates_to_store`
  spawns each store write, and only the awaiting caller sees its failure
  (`persistence.rs:201-210`); `shutdown_owned_runtime` closes the stores
  (`close_session_stores` → `client.pause()` → store close) *before* dropping the
  client whose tasks are aborted (`account/runtime_children.rs:150-153`). A write
  or unload update that survives the abort then runs against a closed store and
  its error is dropped with the join handle.
- **Chunk replacement.** `RemoveChunk` + `NewItemsChunk`, or a `Clear` followed
  by a re-created chunk, produce the same `events=1, event_chunks=0,
  linked_chunks=1` shape, so the probe does not pin the update kind either.

The cheap next experiment is the teardown order (quiesce or flush the cache
worker before closing the stores), which is an ordering change rather than an
architecture decision.

This is a **product-side** defect: a quit at the wrong moment can persist a room
timeline whose newest chunk no longer links its events, so the next launch
restores less history than the store holds. Locally it needs load (1.3–1.5 % of
attempts). No wait this test can add prevents it — the transition is inside
shutdown, after the last gate the test observes — but whether the product could
expose a durable-cache quiescence point is exactly what the teardown-order
experiment above would answer.

The old payload's `persisted_event_after_timeout=Ok(true)` checked only the
`events` row, which is why this read as "the cache was there and the product
ignored it" — the misleading evidence #1721's follow-up asked to resolve.

## What changed here

`crates/koushi-core-testkit/tests/contracts/event_cache_lifecycle.rs` only:

- `cached_delivery_progress` classifies an event as this stage's progress: the
  selected room's timeline publishing a row (`InitialItems` with items,
  `ItemsUpdated` with a row-bearing diff). Sync chatter, other rooms, and an
  empty reset no longer re-arm the window, and the count of unrelated events is
  reported, so the next occurrence fails in 30 s on `silence` with the stage's
  own evidence.
- The wait reports how many ROOM timeline events and `ItemsUpdated` batches the
  room lane published, which is what separates "the cache is empty" from
  "Core published nothing".
- A row that only arrives in a later batch re-arms the window and the payload's
  `delivered_in_updates` says so; the accepted delivery is still the initial
  window carrying the row, so the oracle is unchanged.
- `read_cached_event` now distinguishes `Linked` / `Unlinked` / `Missing`
  instead of only the `events` row, so a persisted cache that a restore cannot
  use is not reported as intact.

The rest of the wait is untouched: the cached-event oracle, the empty-timeline
phase-1 sync, the ordered shutdown, and the 30 s window plus 180 s backstop are
as #2000 left them. The trade-off this narrowing costs is real and documented at
the classifier: this one stage now tolerates 30 s of its own lane's silence
instead of the runtime-wide 180 s backstop. A fixture experiment that only claimed `initial` on the first
response of a phase changed nothing (6 / 480 before and after), which is why it
is not part of the change.

## Canon

`docs/policies/engineering-rules.md` "Async and Runtime" item 1 gains one
sentence scoped to a stage whose window another lane dominates: it classifies
its own progress and says in its doc comment that its tolerance is the window of
its lane. The shared fence itself keeps counting any published event, so the
four stages that use `wait_for_state_event_at` unchanged remain conformant.

## Verification

```bash
cargo fmt -p koushi-core-testkit -- --check
cargo test -p koushi-core-testkit --test contracts --profile ci        # 192 passed
cargo clippy --profile ci -p koushi-core-testkit --all-targets --locked -- -D warnings
node scripts/check-agents-docs.mjs
```

RED→GREEN: with `cached_delivery_progress` replaced by "any event is progress",
`sync_chatter_cannot_mask_a_stalled_cached_delivery` fails with
`backstop for 42ns after 39247 room timeline rows in 181.000936875s`, the CI
shape; with the classifier it fails on `silence for` and `after 0 room timeline
rows`. `cache_failure_diagnostic_reads_hashed_event_without_writing` fails
without the link branch (`left: Ok(Linked), right: Ok(Unlinked)`).
