# #1230: retire cold Room timeline actors without losing read intent

Status: **rejected by the independent pre-implementation review; archived — not
planned.** Revision 2 records the verdict, the two BLOCKERs, the other findings, and
the one factual error this review corrected in the predecessor record. The
maintainer closed this line of work; see the successor record
[read-intent ownership](2026-10-09-issue1230-read-intent-ownership.md) and the
"Known deviation" note in engineering rules "Async and Runtime" 2 for where the
residual is tracked. The next attempt starts from the ownership move this issue's
title assumed, not from the retirement policy below.

Parent: [#1230](https://github.com/shinaoka/koushi-matrix/issues/1230), scoping
the retention half of [#1150](https://github.com/shinaoka/koushi-matrix/issues/1150).
Predecessor record:
[2026-10-09 issue1150 resident retention](2026-10-09-issue1150-resident-retention.md).

## Verdict

Revision 1 proposed retiring a cold Room key only when its read work had reached
its terminal network state (`local_read_sync == Synced`, or no correlation),
ordered as a mailbox message with a pre-removal drain. The review found that this
predicate cannot be made sound, because **read work exists independently of
`local_read_correlations`**:

- `route_read_command` requires an actor handle, then admits and persists a read
  operation through `read_workers.state.admit(...)` without creating any
  correlation (`crates/koushi-core/src/timeline/read_state.rs:926-1000`).
- Restored desired keys likewise begin without correlations, and reconstruction
  can rebuild only a subset of a room's keys.
- So "no correlation" and even "Synced correlation" can coexist with outstanding
  room work. The concrete counterexample: admit an explicit room receipt before
  any eligible viewport observation, retire the key because it has no
  correlation, then let the network write succeed —
  `handle_read_worker_completion` converts an actor-less success into
  `ReadNetworkFailureKind::Sdk` (`:1243-1276`) and `settle_read_operation`
  schedules another retry (`:1400-1407`), so a write can repeat without settling.

The stated residual was also not bounded as claimed: a fresh actor creates a
boundary only from an **at-bottom** observation
(`observe_local_viewed_boundary`, `:1978-1990`), so a user who re-enters an old
anchor above the bottom does not recreate a discarded boundary. And the discarded
message need not be a stale viewport command: the actor stores the boundary and
marks its read state `Pending` **before** awaiting the manager send
(`actor.rs:2413-2423`), so what retirement can discard is actor-accepted intent,
not just an unprocessed observation.

Review recommendation, which this record adopts: **move read-intent ownership
first**; the retirement policy is not the first step.

## What the ownership move has to cover

From the findings, stated as requirements rather than as a design:

1. Read work must be enumerable per `TimelineKey` **without** a
   `LocalReadCorrelation`: admitted and restored desired targets, active, queued
   and reconciliation-pending operations, scheduled retries, waiters, and
   recorded failures. Today these live per `ReadStateKey` in
   `ReadWorkerSupervisor` while the `TimelineKey` mapping lives on the actor side.
2. A completion for a key with no actor must settle as a real outcome instead of
   `ReadNetworkFailureKind::Sdk` with a retry loop. `handle_read_success` is
   actor-mediated and emits `AppAction::FullyReadMarkerUpdated`
   (`:2104-2117`), so either the manager takes over that emission or the
   semantics change explicitly.
3. Actor-accepted-but-unadmitted boundaries must not be actor-owned, or must be
   delivered before the actor can be retired.
4. The pre-removal drain needs a real contract, not an ad-hoc loop: a reusable
   all-message dispatcher, an explicit bound, explicit treatment of nested
   retirement and shutdown, and a self-enqueue that cannot wait on capacity while
   its sole receiver is busy.
5. Retirement needs settlement-triggered convergence, not "one victim per
   selection": accumulating excess actors would otherwise stay constant after
   their reads settle, and would stay forever with no further selection.
6. The recency point must be the reducer-committed, generation-checked projection
   admission, not `EnsureSubscribed`, which has no navigation-generation or
   current-selection guard and is directly routable through
   `CoreCommand::Timeline` (`runtime.rs:4232-4278`).
7. `LiveTailRefreshCoordinator::forget` must specify, per field: drop `states`;
   remove every occurrence from `delayed` and `delayed_members`; clear
   `cancelled_active` only when it matches this key; clear `running` only when the
   running key matches, never another key's operation; emit the exact
   `CancelNetwork { key, operation_generation }` when cancelling a live running
   refresh and then apply the `Start` actions of the next candidate; keep
   operation-generation uniqueness so a stale completion cannot resurrect. `active`
   ownership must be stated rather than left implicit.
8. `TimelineActorHandle::stop()` awaits the handle's task and `auxiliary_tasks`,
   but the actor owns further children (diff relay, relay restart, media, quote,
   pagination) that `TimelineActor::drop` aborts without awaiting
   (`actor.rs:1037-1069`). Handle teardown alone is not a full settlement barrier.
9. The safety tests need real actors, real position evidence, real read
   admission and controlled network/apply settlement, plus the production
   biased-selection loop. `RoomSubscriptionResidencyHarness` cannot supply them:
   it has no session, uses unavailable read workers, discards its action and
   event receivers, and constructs placeholder actors
   (`timeline/residency.rs:314-334,343-365,410-424`). A committed-selection probe
   alone would let every planned test pass while the read race stays broken.

## Correction carried into the predecessor record

Room retirement **does** have a quiescence fence. `unsubscribe_timeline` calls
`clear_thread_root_projections_for_room` for Room keys
(`manager.rs:1735-1741`), and that function's first step is
`timeline_actor_generations.invalidate_and_quiesce(key).await`
(`timeline/thread_projection.rs:520-526`). Revision 1 of this record and the
predecessor record both claimed otherwise; both are corrected. Adding a second
call is redundant. What is *not* fenced is read ingress: the boundary check
compares the handle's `Arc<TimelinePositionIndex>`
(`timeline/read_state.rs:1694-1706`), not the generation gate.

## Why the per-actor retention is still worth bounding

Unchanged from revision 1, and the reason this work exists at all: a retained
`TimelineActor` owns the full canonical item list for its room and nothing
truncates it.

- `navigation_items: Vec<TimelineItem>` (`timeline/actor.rs:905`) is only ever
  indexed into (`actor.rs:1280`, `item_projection.rs:645`,
  `navigation.rs:1757`), and it "deliberately has a wider lifetime than the UI's
  replay window" (`thread_projection.rs:1157-1159`) while that window is
  `ROOM_REPLAY_INITIAL_ITEMS_MAX = 120` (`navigation.rs:53`).
- `media_gallery_items` and `media_sources` grow with the same window
  (`actor.rs:891,911`, `media.rs:586-596`), and the SDK `Arc<Timeline>` holds its
  own copy (`actor.rs:849`).
- `TimelineManagerActor::timelines` (`timeline/manager.rs:493`) is never reduced
  for a Room key, and `LiveTailRefreshCoordinator::states`
  (`live_tail_freshness.rs:65`) is never pruned for a non-delayed room.

## Retired alternatives (do not retry)

- A "no pending intent" guard, checked before removal: rejected twice; the
  predicate cannot see supervisor work without correlations.
- A settled-only predicate: rejected in this revision, same reason.
- Two-phase retirement that stops the actor before deciding: stopping is
  irreversible, and a stopped actor cannot be re-admitted for a key that turns out
  to have pending work.
- Retiring with the pre-removal drain alone: the drain needs the contract in
  requirement 4, and cannot see actor-side sends.
- Keeping the actor and trimming `navigation_items`: the SDK `Arc<Timeline>` copy
  survives, and `navigation_items` is the index space that
  `DisplayProjectionState` and the position index address, so truncating it
  corrupts index translation.
- A byte bound on retained event-ring payloads: a `tokio::broadcast` retains
  cloneable values of arbitrary heap size; out of scope here.

## Canon that the eventual change must amend

- `docs/architecture/overview.md` item 7: its "keep-warm is decided by the UI"
  sentence conflicts with its own "the runtime never leaks timeline state in an
  unbounded map". Core owning a bounded backstop is the reading consistent with
  the rest of the canon, and the UI keeps the option to unsubscribe earlier.
  A drain or retirement that runs during foreground demand must also respect the
  polling order at overview `:1166-1177`.
- `docs/architecture/state-machine.md` Focused Context (`:3308-3318`): "room/thread
  actors … untouched" becomes false once the same committed-demand path retires
  cold Room actors. The read transition contract (`:1171-1221`) needs reconciling
  with accepted observations that currently disappear before admission. The
  Room-Subscription Residency machine keeps its ActorUnsubscribe self-loop.
- `docs/agents/state-ownership.md` "Timeline items and the outbound send queue"
  and "Room-subscription residency": record the bound and that retirement still
  never removes residency; also correct that section's stale pointer to
  `crates/koushi-core/tests/room_subscription_residency.rs` (the tests live in
  `crates/koushi-core-testkit/tests/room_subscription_residency.rs`).

## Verification, once a design survives review

RED before GREEN, headless, asserting on events and state rather than logs or
sleeps, in per-feature files rather than the `runtime/tests.rs` monolith. The
review named the cases a retirement policy must cover, and none of them is
expressible in the current residency harness:

- explicit or restored read work admitted with no correlation, and the
  network-success-before-actor-apply ordering;
- an actor-accepted boundary whose manager send is held across a drain;
- re-entry above the bottom of a room whose boundary was discarded;
- backlog convergence once pending reads settle, with no further selection;
- retirement admission against a full mailbox;
- a retirement after which the live-tail scheduler still refreshes a surviving
  room;
- unchanged session residency and SDK active subscription set for retired rooms;
- `InitialItems` on re-entry through a real actor.

## Not claimed

Process RSS, allocator attribution, the event-ring payload budget, and complete
SDK resource release. None has a check or instrumentation in this repository.
`crates/koushi-search/tests/search_memory.rs` is untouched.
