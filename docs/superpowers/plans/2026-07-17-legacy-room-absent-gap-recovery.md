# Legacy Room-Absent Gap Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair a persisted newest timeline gap after legacy fallback even when the first committed incremental `/sync` response omits the active room, and expose privacy-safe diagnostics for every commit/gate decision.

**Architecture:** The SDK event cache publishes one retained global response-commit fence only after all room topology mutations for that response finish. TimelineManager snapshots the same-response per-room observations, routes exact checkpoints for updated rooms, and synthesizes a `room_absent` checkpoint only for active rooms omitted from the committed response. TimelineActor admits the newest persisted gap only for that explicit origin and keeps the existing bounded repair chain.

**Tech Stack:** Rust, Tokio watch channels, matrix-rust-sdk event cache, Koushi actor runtime, MatrixMockServer, Conduit Linux headless QA.

## Global Constraints

- Write the regression test first and observe the expected RED failure before production changes.
- The global response fence is published after event-cache topology mutation, never at room-update broadcast time.
- Do not scan or publish checkpoints for every known room on every response; synthesize only for active Koushi timelines.
- An explicit room update with no timeline gap closes LiveEdge; only `room_absent` may select the newest persisted gap.
- Preserve existing event, cached-chunk, relay, render, and retry bounds.
- Diagnostics must not contain room IDs, event IDs, gap identities, sync tokens, message bodies, credentials, or raw SDK errors.
- Do not overwrite unrelated worktree changes or the user's stale submodule checkout.
- Do not push the SDK or superproject branch until local RED/GREEN verification passes.

---

### Task 1: SDK committed-response fence

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/tests/integration/event_cache/mod.rs`

**Interfaces:**
- Produces: `CommittedRoomUpdatesResponse::response_sequence() -> u64`
- Produces: `CommittedRoomUpdatesResponse::{joined_room_count,left_room_count,invited_room_count}() -> usize`
- Produces: `EventCache::subscribe_to_committed_room_updates_responses() -> watch::Receiver<Option<CommittedRoomUpdatesResponse>>`
- Guarantees: the watch value changes only after `handle_room_updates` finishes all joined/left topology mutation for that response.

- [ ] **Step 1: Write the failing omitted-room fence test**

Add `test_committed_response_fence_advances_when_a_known_room_is_omitted` beside the existing committed-room observation integration test. Join rooms A and B, retain A's observation, subscribe to the new fence, then sync only B:

```rust
let mut responses = client
    .event_cache()
    .subscribe_to_committed_room_updates_responses();
let observations = client
    .event_cache()
    .subscribe_to_committed_room_timeline_observations();
let room_a_before = observations
    .borrow()
    .get(room_a)
    .expect("room A baseline")
    .response_sequence();

server.sync_room(&client, JoinedRoomBuilder::new(room_b)).await;
responses.changed().await.expect("committed response fence");
let committed = responses.borrow().clone().expect("committed response");

assert!(committed.response_sequence() > room_a_before);
assert_eq!(committed.joined_room_count(), 1);
assert_eq!(
    observations.borrow().get(room_a).map(|value| value.response_sequence()),
    Some(room_a_before),
    "an omitted room keeps its previous per-room observation",
);
assert_eq!(
    observations.borrow().get(room_b).map(|value| value.response_sequence()),
    Some(committed.response_sequence()),
    "the fence is visible only after the same-response room observation",
);
```

- [ ] **Step 2: Run the SDK test and verify RED**

Run:

```bash
cargo test -p matrix-sdk --test integration event_cache::test_committed_response_fence_advances_when_a_known_room_is_omitted -- --nocapture
```

Expected: compile failure because `subscribe_to_committed_room_updates_responses` and `CommittedRoomUpdatesResponse` do not exist.

- [ ] **Step 3: Add the retained fence type and sender**

Add the public token-free value near `CommittedRoomTimelineObservation`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommittedRoomUpdatesResponse {
    response_sequence: u64,
    joined_room_count: usize,
    left_room_count: usize,
    invited_room_count: usize,
}
```

Add accessors for all four fields. Add this sender to `EventCacheInner`:

```rust
committed_room_updates_response_sender:
    watch::Sender<Option<CommittedRoomUpdatesResponse>>,
```

Initialize it with `watch::channel(None)` and expose:

```rust
pub fn subscribe_to_committed_room_updates_responses(
    &self,
) -> watch::Receiver<Option<CommittedRoomUpdatesResponse>> {
    self.inner.committed_room_updates_response_sender.subscribe()
}
```

- [ ] **Step 4: Publish the fence after topology commit**

At the beginning of `handle_room_updates`, capture the three counts before consuming the maps. After the joined/left loops and immediately before `Ok(())`, publish:

```rust
self.committed_room_updates_response_sender.send_replace(Some(
    CommittedRoomUpdatesResponse {
        response_sequence,
        joined_room_count,
        left_room_count,
        invited_room_count,
    },
));
```

Do not publish from `Client::latest_room_updates_response_sequence`; that sequence proves broadcast publication, not event-cache commit.

- [ ] **Step 5: Run focused and neighboring SDK tests GREEN**

Run:

```bash
cargo test -p matrix-sdk --test integration event_cache::test_committed_response_fence_advances_when_a_known_room_is_omitted -- --nocapture
cargo test -p matrix-sdk --test integration event_cache::test_committed_room_observation_is_retained_and_advances_for_empty_responses -- --nocapture
```

Expected: both tests pass with zero failures.

---

### Task 2: Koushi checkpoint origin and gate policy

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-sdk/tests/timeline_gap_adapter.rs`
- Modify: `crates/koushi-core/src/live_catchup.rs`

**Interfaces:**
- Produces: `MatrixCommittedRoomTimelineOrigin::{RoomUpdate,RoomAbsent}`
- Produces: `MatrixCommittedRoomUpdatesResponse::from_sdk(&matrix_sdk::event_cache::CommittedRoomUpdatesResponse)`
- Produces: `MatrixCommittedRoomTimelineCheckpoint::from_legacy_room_absent(response, room_id)`
- Produces: `MatrixCommittedRoomTimelineCheckpoint::origin()` and `is_room_absent()`
- Produces: `LiveCatchupGate::RepairPersistedLiveEdge`

- [ ] **Step 1: Write adapter and gate RED tests**

Add compile/runtime adapter assertions proving an absent checkpoint is legacy, has no timeline mutation, carries the response generation, is routed to the requested room, and reports `RoomAbsent`. Extend `live_catchup.rs` tests:

```rust
assert_eq!(
    classify_live_catchup_gate(None, Some((19, false, false, true))),
    LiveCatchupGate::RepairPersistedLiveEdge,
);
assert_eq!(
    classify_live_catchup_gate(None, Some((19, false, false, false))),
    LiveCatchupGate::NoTimelineUpdate,
);
```

- [ ] **Step 2: Run focused tests and verify RED**

Run with the local SDK worktree supplied through Cargo patch configuration:

```bash
cargo test -p koushi-core live_catchup -- --nocapture
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
```

Expected: compile failure because the origin, response adapter, constructor, and fourth classifier input do not exist.

- [ ] **Step 3: Implement the token-free adapters**

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixCommittedRoomTimelineOrigin {
    RoomUpdate,
    RoomAbsent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixCommittedRoomUpdatesResponse {
    generation: u64,
    joined_room_count: usize,
    left_room_count: usize,
    invited_room_count: usize,
}
```

Store `origin` in `MatrixCommittedRoomTimelineCheckpoint`. Existing SyncService and committed-room constructors set `RoomUpdate`; `from_legacy_room_absent` sets `LegacySync`, the committed response generation, no observation sequence, no timeline mutation, and no inserted gap. Keep custom `Debug` limited to backend, origin, generation, and booleans/counts.

- [ ] **Step 4: Add the explicit persisted-live-edge gate**

Change the classifier input to `(generation, has_timeline, has_gap, room_absent)`. Evaluate stale generation first, then `room_absent`, then the existing no-timeline/no-gap/exact-gap cases. Add token `repair_persisted_live_edge`.

- [ ] **Step 5: Run adapter and policy tests GREEN**

Run:

```bash
cargo test -p koushi-core live_catchup -- --nocapture
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
```

Expected: all focused tests pass.

---

### Task 3: TimelineManager response routing and deterministic actor regression

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`

**Interfaces:**
- Consumes: the SDK committed-response watch from Task 1.
- Consumes: checkpoint origin and `RepairPersistedLiveEdge` from Task 2.
- Produces: `TimelineMessage::LegacyResponseCommitted { service_epoch, response, checkpoints }`
- Produces: process-local retained latest legacy response for fast replay to replacement/new room actors.

- [ ] **Step 1: Write the real-path RED regression**

Add `legacy_fallback_repairs_persisted_gap_when_room_is_absent_from_first_response`. Use `MatrixMockServer` and the real event cache/TimelineManager path. Seed a limited persisted timeline gap for room A and an unrelated older gap, subscribe the manager from the current response baseline, commit a response containing only room B, then create/rebuild room A. Do not construct or send `TimelineActorMessage::RoomSubscriptionCheckpoint` in the test. Assert:

```rust
assert_eq!(messages_before_committed_fence, 0);
assert!(event_ids(&rendered).contains(&missing_id.as_str()));
assert_eq!(
    event_ids(&rendered)
        .iter()
        .filter(|event_id| **event_id == missing_id.as_str())
        .count(),
    1,
);
assert_eq!(session.inspect_room_timeline_gaps(room_a.as_str()).await?.gaps.len(), 1);
assert_eq!(actor_gap_scheduler_phase, "idle");
```

The remaining gap must be the unrelated older gap; the newest persisted gap is the only admitted candidate.

- [ ] **Step 2: Run the regression and verify RED**

Run:

```bash
cargo test -p koushi-core --lib legacy_fallback_repairs_persisted_gap_when_room_is_absent_from_first_response -- --nocapture
```

Expected: timeout or assertion failure because no per-room observation/checkpoint is produced for room A.

- [ ] **Step 3: Drive legacy routing from the committed-response fence**

Replace the independent legacy per-room observation loop with one loop driven by `subscribe_to_committed_room_updates_responses`. On every retained response at or after `committed_from_response_sequence`, snapshot retained committed-room observations and include only checkpoints whose `response_sequence()` equals that fence. Send the response and vector in one manager message so mailbox ordering is atomic.

- [ ] **Step 4: Route exact and absent checkpoints to active actors**

When `LegacyResponseCommitted` matches the active `service_epoch`:

1. Replace the retained current-response checkpoint map with the supplied same-response checkpoints.
2. Retain the response adapter for actors created after the fence.
3. For each active room actor, route its exact checkpoint when present; otherwise create `from_legacy_room_absent` for that room.
4. During fast replay, use the retained exact checkpoint or synthesize the retained `room_absent` checkpoint.

Stale epochs and response sequences below the baseline remain ignored.

- [ ] **Step 5: Admit only the newest persisted gap for room-absent**

Pass `checkpoint.is_room_absent()` into `classify_live_catchup_gate`. In `handle_timeline_gap_inspection_finished`, map `RepairPersistedLiveEdge` to:

```rust
inspection
    .gaps
    .len()
    .checked_sub(1)
    .map_or(GapRepairSelection::None, |ordinal| {
        GapRepairSelection::LiveEdgeFallback { ordinal }
    })
```

Do not consult viewport projection for this first admission. After progress, retain the existing bounded live-edge continuation behavior.

- [ ] **Step 6: Run the new and existing actor regressions GREEN**

Run:

```bash
cargo test -p koushi-core --lib legacy_fallback_repairs_persisted_gap_when_room_is_absent_from_first_response -- --nocapture
cargo test -p koushi-core --lib legacy_fallback_waits_for_committed_response_and_recovers_missing_interval -- --nocapture
cargo test -p koushi-core live_catchup -- --nocapture
```

Expected: all tests pass; the old exact-response test still performs no `/messages` request before its commit checkpoint.

---

### Task 4: Privacy-safe diagnostics

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-sdk/src/lib.rs`
- Test: `crates/koushi-sdk/tests/timeline_gap_adapter.rs`

**Interfaces:**
- Produces diagnostic stage `core.timeline legacy_response_commit`.
- Extends `core.live_catchup checkpoint` with `checkpoint_origin`, `candidate`, and `scheduler_phase`.

- [ ] **Step 1: Write RED privacy and field assertions**

Assert the checkpoint `Debug` output contains `origin: RoomAbsent`, generation, and boolean state, while excluding the room ID and all opaque descriptor/token material. Add a focused diagnostic capture assertion for fields:

```text
response_sequence
commit_fence_sequence
active_timeline_count
rooms_updated_count
checkpoint_origin
candidate
scheduler_phase
batches_processed
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
cargo test -p koushi-core --lib legacy_fallback_repairs_persisted_gap_when_room_is_absent_from_first_response -- --nocapture
```

Expected: field assertions fail before diagnostics are added.

- [ ] **Step 3: Record response and actor decisions**

Add one manager diagnostic when a committed fence is accepted, with response/fence sequence, active actor count, and current-response checkpoint count. Extend gate diagnostics with origin and scheduler phase. Record candidate `exact_response_gap`, `newest_persisted`, or `none` at inspection selection. Use only `DiagnosticField::count`, `boolean`, and closed token values.

- [ ] **Step 4: Run diagnostics tests GREEN**

Run the two focused commands from Step 2. Expected: pass, and captured diagnostics contain no private identifiers or content.

---

### Task 5: Linux restart acceptance scenario

**Files:**
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`
- Modify: the matching local QA scenario registration/test assertions in the same file.

**Interfaces:**
- Produces scenario `timeline_legacy_persisted_gap`.
- Emits typed evidence markers `legacy_persisted_gap_room_absent=ok` and
  `legacy_persisted_gap_unrelated_gap_retained=ok`, followed by the fence,
  repair, and settlement markers.

- [ ] **Step 1: Add the failing restart scenario**

Create a profile/store for A, produce an offline limited timeline and persist its gap without opening the room, stop Core cleanly, restart Core with the same store and legacy fallback, allow a first response that does not update the target room, then open it. Assert the missing interval is contiguous, every expected event appears once, the unrelated older gap remains, and the scheduler settles.

- [ ] **Step 2: Run Linux headless RED**

Run:

```bash
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=timeline_legacy_persisted_gap --core --core-backend=probed --timeout-ms=300000
```

Expected before Tasks 1-4: timeout while the actor remains `awaiting_subscription_response`.

- [ ] **Step 3: Complete scenario wiring without test-only production bypasses**

Reuse the real credential store, Core restart, automatic fallback, event cache, and `/messages` repair path. Do not inject a checkpoint or call a test-only repair function.

- [ ] **Step 4: Run Linux headless GREEN**

Run the command from Step 2. Expected: exit 0 and all three markers equal `ok`.

---

### Task 6: Dependency pin and final verification

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Update: `vendor/matrix-rust-sdk` gitlink

**Interfaces:**
- Consumes: the verified SDK commit from Task 1.
- Produces: a superproject build pinned to the exact verified SDK revision.

- [ ] **Step 1: Commit and push the verified SDK branch**

Commit only Task 1 SDK files on `codex/issue-275-room-absent-fence`, push that branch, and record the exact commit SHA. Do not force-push.

- [ ] **Step 2: Pin all Matrix SDK dependencies and the submodule**

Replace every `590d9469041407178047eb633d309a5ea75a01c1` Matrix SDK `rev` in `Cargo.toml` with the verified SHA, update `Cargo.lock`, and set the superproject gitlink to the same SHA. The QA submodule synchronization guard must report clean.

- [ ] **Step 3: Run formatting and focused suites**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
cargo test -p koushi-core live_catchup -- --nocapture
cargo test -p koushi-core --lib legacy_fallback_repairs_persisted_gap_when_room_is_absent_from_first_response -- --nocapture
cargo test -p koushi-core --lib legacy_fallback_waits_for_committed_response_and_recovers_missing_interval -- --nocapture
```

Expected: exit 0 for every command.

- [ ] **Step 4: Run repository guards**

Run the SDK submodule synchronization guard and the applicable dependency/release structural checks from the repository scripts. Expected: all exit 0 with the submodule, manifest, and lockfile on one SDK SHA.

- [ ] **Step 5: Re-run Linux headless acceptance**

Run the Task 5 GREEN command using the pinned default dependency configuration, without a local Cargo path patch. Expected: exit 0 and all acceptance markers equal `ok`.

- [ ] **Step 6: Review the final diff**

Confirm `git diff --check` is empty, no private identifiers/content entered tests or diagnostics, and unrelated user files are absent from the diff.
