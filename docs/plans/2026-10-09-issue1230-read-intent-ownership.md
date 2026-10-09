# #1230: manager-owned read intent, then bounded Room actor retention

Status: **revision 2 — reviewed and not yet implementable; nothing implemented.**
The first review rejected the retirement-policy design outright and prescribed
this ownership move; this revision is that ownership move, and the second review
answered "revise the design, not the architecture wholesale" with a concrete gap
list, recorded below. Four designs have now been reviewed for #1230; the memory
win they chase is real but bounded and still unmeasured, while the remaining
specification items include a normative teardown conflict that needs an explicit
maintainer decision. `docs/plans/2026-10-09-issue1230-cold-room-retirement.md`
holds the first rejected revision and its nine requirements.

Parent: [#1230](https://github.com/shinaoka/koushi-matrix/issues/1230), scoping the
retention half of [#1150](https://github.com/shinaoka/koushi-matrix/issues/1150).

## Review verdict on this revision (second review)

The review kept the architecture and named the remaining specification gaps. Of
the nine requirements from the first review it records: 2 met in principle,
4 met by elimination (its replacement contract still incomplete), 6 met, 7 met,
8 met as a limitation; 1, 3, 5 and 9 **not met**. The gaps, to be specified before
any implementation commit:

1. **A queued report from a retiring actor still loses its validation authority.**
   The ordering defect of the rejected revision survives in narrower form: an
   actor can enqueue `LocalReadBoundaryObserved` before or during removal, and
   sections 1-4 do not yet accept it. The suggested smallest fix is a cheap
   late-report validator instead of the full retained index: the report already
   carries its own target and position evidence, so an explicit provenance rule
   (incarnation/generation fencing plus the timeline generation the report was
   derived from) can replace retaining a whole `TimelinePositionIndex` per
   unresolved room. That contract must be written before the index cache is used.
2. **Sections 1-4 do not cover accepted non-read operations.** Queued replay
   controls, an explicit restore, or pagination on a non-foreground key are not
   part of the eligibility test.
3. **Convergence must be settlement-triggered, not selection-triggered.**
   Retiring the whole backlog in one foreground turn is also wrong: maintenance
   must be bounded and preemptible by the biased select, and must run again when a
   read settles rather than waiting for another room selection.
4. **A successful confirmation must survive until authoritative sync consumes
   it.** `local_server_confirmation_key` deliberately selects FullyRead for Room
   keys (`read_state.rs:1469-1488`), so moving settlement to the manager can
   regress the confirmation the user sees before sync, including across re-entry.
5. **Teardown.** Engineering rules "Async and Runtime" 2 requires a retained owner
   that cancels *and awaits* every spawned task; `stop()` awaits the actor and the
   handle's auxiliary tasks, but the actor-owned children (diff relay, relay
   restart, media, quote, pagination) are aborted without awaiting
   (`actor.rs:1037-1069`). Disclaiming full settlement is honest, but it is not an
   exception to the rule: either provide the settlement boundary for intentional
   retirement or obtain an approved, narrowly documented exception.
6. **More behavioural deltas than the design listed**: silently capacity-rejected
   observations, lost settled local boundary on re-entry above the bottom,
   possible confirmation regression before sync, thread-root projection clearing
   on Room retirement, and aborting non-read operations.
7. **Tests.** The five predicates cannot observe an in-flight unadmitted report,
   so the claimed "eligibility is false while a report is in flight" oracle is not
   expressible without ingress bookkeeping — and after ownership moves, the right
   assertion may instead be that retirement proceeds and the report still admits
   and settles. New narrow test-hooks support is needed for the testkit checks;
   existing read tests can drive real admission with controlled synthetic network
   futures but use positioned handles rather than real actors. Missing RED cases:
   a queued report with no correlation; a queued report with a Synced correlation;
   settlement with no further selection; re-entry with an old-generation
   correlation; reliable-action failure; a policy toggle while retired; delayed
   authoritative sync after actor-less FullyRead success; and retirement during an
   explicit restore or pagination.
8. **Canon.** Additionally: `state-machine.md:1209-1221` (correlations currently
   retire with actors, so the fencing text must distinguish presentation
   replacement from read-operation invalidation), `state-machine.md:1196-1198`
   (only a current actor may admit an observation, so accepting a retired
   incarnation needs an explicit provenance rule), and `state-ownership.md`
   "Local viewed and server-confirmed read state", which is the directly affected
   section and must be named in the amendment list.

Nothing was implemented. The design below stands as the current best starting
point and is not an instruction to land code.

## The one idea

`TimelineManagerActor` already owns the whole read lifecycle — admission,
retry, completion, persistence, and the `local_read_correlations` map
(`timeline/read_state.rs:224-250,669-720,926-1050,1212-1320,1365`). What it does
not own is the *intent*: the actor accepts a boundary into its own state before
the manager has admitted it (`TimelineActor::observe_local_viewed_boundary`,
`:1978-2030`, sets `local_viewed_boundary` and `read_state_sync = Pending` before
the `LocalReadBoundaryObserved` send at `actor.rs:2413-2423`), and the manager
refuses to serve a key whose actor is gone.

So the ownership move is three narrow changes, not a rewrite:

1. enumerate read work from the `TimelineKey`, so no correlation is needed to
   know what is outstanding;
2. let the actor report a boundary instead of accepting it, so no accepted intent
   is ever actor-owned;
3. let the manager settle an actor-less read on its own authority instead of
   converting it to a `Sdk` failure.

Only then does actor retention have a sound eligibility criterion.

## 1. Read work is enumerable from the key

Add one function next to the existing mapping
(`timeline/diagnostics.rs:390-413` `read_state_key_for_command`):

```rust
fn read_keys_for_timeline_key(key: &TimelineKey, send_read_receipts: bool) -> Vec<ReadStateKey>
```

- `TimelineKind::Room { room_id }` → `PublicUnthreaded { room_id }` when receipts
  are enabled, plus `FullyReadAndPrivateUnthreaded { room_id }`.
- `TimelineKind::Thread { room_id, root_event_id }` → `ThreadRead { .. }` when
  receipts are enabled.
- `TimelineKind::Focused { .. }` → none.

This is the same set `handle_local_read_boundary_observed` builds today
(`read_state.rs:1722-1750`); today it is only constructed once a boundary has
been accepted, which is exactly why "no correlation" could not mean "no work".
Both call sites then use this function, so the two can no longer drift.

Outstanding work for a key is then
`ReadWorkerSupervisor`-owned and correlation-free:

```
for read_key in read_keys_for_timeline_key(key, send_read_receipts):
    state.candidate_count(read_key) == 0          // no admitted-or-restored desired target
    && state.active_operation(read_key).is_none()
    && !queued.contains(read_key)
    && !reconciliation_pending.contains(read_key)
    && !scheduled_retries.contains(read_key)
```

This covers the counterexample the review found: an explicitly admitted room
receipt before any viewport observation is visible here as
`candidate_count(FullyReadAndPrivateUnthreaded) != 0`, with or without a
correlation.

## 2. The actor reports, the manager owns

`TimelineActor::observe_local_viewed_boundary` keeps deriving the target (it owns
`navigation_items`, `display_projection`, `viewport_observation` and the gap
projection, so the derivation cannot move cheaply), but stops writing
`local_viewed_boundary` and `read_state_sync`. It keeps a
`last_reported_boundary: Option<ReadTarget>` used only as a duplicate-suppression
cache so a stationary viewport does not re-report on every tick.

The actor's displayed read state becomes a pure projection: the manager already
projects through `TimelineActorControl::ReadStateProjection`
(`read_state.rs:1617-1650`, applied at `:2031-2054`), so the UI's boundary and
`Pending` state arrive one mailbox hop later than today instead of never for a
retired key. `server_confirmed_read_event_id` and `read_state_sync` follow the
same path.

Effect: there is no "accepted but unadmitted" intent anywhere in the actor, which
is the second hazard the review found. A report that never reaches the manager
leaves the manager unaware; a report that does reach it becomes manager-owned
work in the same turn.

## 3. Actor-less settlement

- `project_local_read_correlation` (`:1617-1650`) stops deleting the correlation
  when the key has no handle (`:1637-1640`). It skips the projection and leaves
  the correlation as the manager's authority.
- `handle_read_worker_completion` (`:1212-1320`) stops converting an actor-less
  completion into `ReadNetworkFailureKind::Sdk` (`:1243-1276`):
  - `PublicUnthreaded`: settle success and record the confirmed event in the
    correlation, instead of requiring `timelines.contains_key`.
  - `FullyReadAndPrivateUnthreaded`: the manager emits
    `AppAction::FullyReadMarkerUpdated { room_id, event_id }` itself, which is
    what the actor's `ReadActorApplyKind::FullyRead` branch does today
    (`read_state.rs:2104-2117`), then settles success.
  - `ThreadRead`: unchanged; Thread actors are never retired.
- `settle_read_operation`'s existing local-confirmation update (`:1376-1390`)
  already writes the correlation; with actor-less success it becomes the only
  writer for a retired key, which is correct.

## 4. Retention of the correlation, and of the position index

A correlation for a key with no handle is kept only while it has work or a
recorded failure, and is dropped once `local_read_sync == Synced` with no handle
present. That is the settlement-triggered convergence the review asked for, and
it bounds this map by rooms with *unsettled* reads rather than by rooms visited.

Late reports still need validation, and the actor-side `TimelinePositionIndex` is
the validator today (`:1694-1706`). The manager retains
`Arc<TimelinePositionIndex>` for a retired key exactly while that key keeps a
correlation, and drops it with the correlation. The index is
`HashMap<String, u64>` (`read_state.rs:694-697`), one entry per event, so this is
bounded by unresolved reads and is not retained for the warm set as a whole. A
report accepted through the retained index is treated exactly like a live one.

## 5. Retirement, now that it is sound

With sections 1-4 in place, retirement no longer has to reason about read intent:

- **Eligibility.** No outstanding read work per section 1, and no correlation
  with a failure. No drain, no two-phase protocol, and no requirement that the
  victim be quiescent for reads.
- **Recency.** Committed, generation-checked Room selection. The recency point is
  the projection-admission path (`timeline/navigation.rs:806-869`), not
  `EnsureSubscribed`; the latter has no navigation-generation or
  current-selection guard and is directly routable through `CoreCommand::Timeline`
  (`runtime.rs:4232-4278`).
- **Convergence.** Every eligible Room key beyond the warm set is retired in the
  same pass, not one per selection, so a backlog converges in the selection that
  first observes it rather than staying constant.
- **Warm set.** The foreground Room key plus `WARM_ROOM_TIMELINE_LIMIT` recently
  selected rooms. The number is a Core policy constant with its own
  justification; it is deliberately not the frontend's
  `TIMELINE_STORE_INACTIVE_RETAIN_LIMIT = 8`, which protects inactive keys of every
  timeline kind (`apps/desktop/src/domain/timelineStore.ts:436-475`).
- **Victims.** Room kind only. Thread and Focused keys are never victims, in-flight
  focused builds are excluded by the kind filter alone, and the live-tail
  foreground is excluded explicitly.
- **Mechanics**, per victim: fence via the existing path (Room unsubscribe already
  reaches `invalidate_and_quiesce` through
  `clear_thread_root_projections_for_room`, `thread_projection.rs:520-526`), settle
  the handle with the existing `TimelineActorHandle::stop().await`
  (`actor.rs:819-828`) rather than `Drop` (`:831-840`), and run
  `LiveTailRefreshCoordinator::forget(key)`.
- **`forget(key)`** must, per field: drop `states`; remove every occurrence from
  `delayed` and its `delayed_members` membership; clear `cancelled_active` only
  when it matches this key; clear `running` only when the running key matches,
  emitting the exact `CancelNetwork { key, operation_generation }` for it and then
  applying the `Start` actions of the next candidate, without re-queueing the
  victim (so `preempt_running`, `:386-410`, is the wrong model); leave `active`
  untouched, since the foreground guard already protects it. This is what closes
  the `schedule_next` early return (`:412-414`) that would otherwise strand every
  later refresh.

## 6. Re-entry

`AppEffect::SubscribeTimeline` → `EnsureSubscribed { replay_existing: false }` →
`handle_subscribe` builds a fresh actor for an absent key and publishes
`InitialItems` (`timeline/manager.rs:1906-1998`). A retained correlation is then
projected into the new actor, so a pending boundary reappears in the UI and the
read still settles; with no retained correlation, a fresh actor starts with no
boundary and the next at-bottom observation re-derives one. This is also why the
previous design's residual is gone: nothing depends on the retired actor to
re-derive a boundary, because the manager kept the intent.

## What this does not claim

- Complete task settlement. `stop()` awaits the handle's task and
  `auxiliary_tasks`, but the actor owns further children (diff relay, relay
  restart, media, quote, pagination) that `TimelineActor::drop` aborts without
  awaiting (`actor.rs:1037-1069`). The claim is structural removal: the key leaves
  `timelines`, and the SDK `Timeline` handle is dropped. Full SDK/task release is
  not asserted by any test here.
- Any byte measurement. Process RSS, allocator attribution and the event-ring
  payload budget have no instrumentation in this repository.
- Thread and Focused retention, which is already bounded by their own
  unsubscribe paths.

## Verification

RED before GREEN, headless, asserting on events and state rather than logs or
sleeps. The review's requirement 9 stands: the residency harness cannot express
these (`timeline/residency.rs:314-334,343-365,410-424` — no session, unavailable
read workers, discarded receivers, placeholder actors), so the checks have to be
built where real actors and real read admission exist.

- `crates/koushi-core/src/timeline/read_state.rs` tests: eligibility is false
  while an explicitly admitted receipt has no correlation (the reviewer's
  counterexample), becomes true once it settles, and is false while a boundary
  report is in flight.
- `crates/koushi-core/src/timeline/read_state.rs` tests: a completion for a key
  with no handle settles success and records the confirmed event instead of
  failing and retrying; the `FullyRead` case emits
  `AppAction::FullyReadMarkerUpdated` from the manager.
- `crates/koushi-core/src/timeline/read_state.rs` tests: the actor no longer
  writes `local_viewed_boundary`/`read_state_sync` on derivation, and does so on
  projection.
- `crates/koushi-core-testkit/tests/timeline_actor_retention.rs` (new, real
  session and mock server): committing more Room selections than the warm limit
  retains at most the warm set, keeps the most recent rooms, leaves session
  residency and the SDK active subscription set unchanged for every retired room,
  and republishes `InitialItems` on re-entry.
- Same file: a Room key with an outstanding read is not retired; a retired key
  leaves no `states`/`delayed` entry and the scheduler still refreshes a surviving
  room; the victim's retained correlation and position index are released when it
  settles.
- `crates/koushi-core/src/timeline/residency/tests.rs` residency invariants stay
  green, unchanged.
- Local gates: `cargo fmt --check`, the three clippy lanes,
  `cargo test --profile ci --workspace --exclude sidebar-composition --exclude
  key-management --locked`, and the headless `timeline_stress` scenario plus a
  room-switch scenario to show the issue's "without slowing recent-room
  switching" condition still holds.

## Canon to amend in the same change

- `docs/architecture/state-machine.md`: the read transition contract around
  `:1171-1221` gains "the manager owns the pending local-read intent; the actor
  reports and displays it", and Focused Context (`:3308-3318`) loses its
  "room/thread actors … untouched" sentence once the same committed-demand path
  retires cold Room actors. The Room-Subscription Residency machine keeps its
  ActorUnsubscribe self-loop.
- `docs/architecture/overview.md` item 7: record that Core owns an enforced
  bounded warm set as a backstop while the UI keeps the option to unsubscribe
  earlier, and reconcile the "keep-warm is decided by the UI" sentence with
  "the runtime never leaks timeline state in an unbounded map".
- `docs/agents/state-ownership.md` "Timeline items and the outbound send queue"
  and "Room-subscription residency": the read-intent owner, the bound, and that
  retirement still never removes residency; also correct the stale pointer to
  `crates/koushi-core/tests/room_subscription_residency.rs`.

## Known behavioural deltas to accept or reject

1. The UI's local-viewed boundary and `Pending` indicator update one mailbox hop
   later, because the actor no longer sets them optimistically.
2. An actor-less `FullyReadAndPrivateUnthreaded` success now emits
   `FullyReadMarkerUpdated` from the manager rather than from the actor. The
   reducer contract is unchanged; the emitter changes.
3. A room whose read keeps failing is not retired until the failure clears.
