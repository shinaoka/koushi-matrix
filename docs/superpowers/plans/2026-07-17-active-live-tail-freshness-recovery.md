# Active Live-Tail Freshness Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove and restore the selected room's current live edge after a legacy-sync room-absent response, without mistaking an unrelated persisted historical gap for freshness.

**Architecture:** Add one authoritative, token-free live-tail refresh to the Rust SDK, then expose only coarse outcomes and causal projection generations through `koushi-sdk`. A focused Core freshness coordinator owns the active-room priority, epoch fencing, delayed queue, and phase-aware cancellation; existing viewport pagination remains the sole mechanism for older gaps.

**Tech Stack:** Rust 2024, Matrix Rust SDK event cache/linked chunks, matrix-sdk-ui timeline projections, Tokio cancellation tokens, Koushi reducer/timeline actors, `wiremock` Matrix test helpers, headless-core QA, Cargo and Node guard tests.

## Global Constraints

- The foreground request is room-scoped `/_matrix/client/v3/rooms/{roomId}/messages?dir=b&limit=128` with no `from` token.
- Core must never receive a Matrix pagination token, linked-chunk identifier, room ID in diagnostics, event ID in diagnostics, message body, or raw SDK error.
- Only an exact SyncService room checkpoint, exact legacy room update, or successful room-scoped snapshot may transition the active sync epoch to `Fresh`.
- A legacy `RoomAbsent` checkpoint transitions to `Unproven`; it must not select any persisted gap ordinal.
- At most one account-level live-tail refresh may wait on the network at once; the selected room preempts delayed rooms.
- Cancelling a room switch may abort the network phase, but an SDK cache commit that has started must finish persistence and observable publication atomically.
- When more than 128 events are missing, publish the latest 128 immediately and persist exactly one gap before that detached tail; ordinary viewport back-pagination owns continuation.
- Repeated room-absent responses in an already-proven epoch must not trigger another request.
- Preserve exact-response limited-update repair and viewport-driven historical gap repair.
- All implementation follows RED-GREEN-REFACTOR; do not weaken existing assertions to obtain GREEN.

---

## File Structure

- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/live_tail.rs`: private snapshot fence, cancellation-aware network phase, overlap/detached classification, and atomic linked-chunk commit.
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/mod.rs`: register and re-export room live-tail implementation.
- `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/pagination.rs`: public token-free `RoomPagination` entry point and coarse outcome/result types; retain normal historical pagination unchanged.
- `vendor/matrix-rust-sdk/crates/matrix-sdk/tests/integration/event_cache/mod.rs`: SDK integration regressions for unchanged, overlap, detached tail, stale fence, cancellation, and later historical pagination.
- `crates/koushi-sdk/src/lib.rs`: privacy boundary types and `MatrixClientSession::refresh_room_live_tail` adapter.
- `crates/koushi-sdk/tests/timeline_gap_adapter.rs`: adapter outcome/projection/privacy tests.
- `crates/koushi-core/src/live_tail_freshness.rs`: backend-neutral epoch state machine plus single-slot active/deferred scheduler.
- `crates/koushi-core/src/lib.rs`: register the focused Core module.
- `crates/koushi-core/src/timeline.rs`: actor integration, active-room hand-off, phase-aware cancellation, projection fencing, and diagnostics.
- `crates/koushi-core/src/live_catchup.rs`: remove `RepairPersistedLiveEdge` as the room-absent decision; keep exact limited-update and viewport policies.
- `crates/koushi-core/src/bin/headless-core-qa.rs`: production-shape stale-tail scenario and privacy-safe markers.
- `crates/koushi-core/tests/headless_core_qa.rs`: require the new live-tail recovery marker.
- `Cargo.toml`, `Cargo.lock`, `scripts/check-matrix-sdk-pin.mjs`, `scripts/test-matrix-sdk-pin-guard.mjs`, `vendor/matrix-rust-sdk`: advance and guard the SDK commit used by the superproject.

### Task 1: SDK authoritative live-tail snapshot and commit

**Worktree:** `/Users/hiroshi/projects/Element-dev/matrix-desktop/.worktrees/matrix-sdk-room-absent-fence`

**Files:**
- Create: `crates/matrix-sdk/src/event_cache/caches/room/live_tail.rs`
- Modify: `crates/matrix-sdk/src/event_cache/caches/room/mod.rs`
- Modify: `crates/matrix-sdk/src/event_cache/caches/room/pagination.rs`
- Modify: `crates/matrix-sdk/src/event_cache/caches/room/state.rs`
- Test: `crates/matrix-sdk/tests/integration/event_cache/mod.rs`

**Interfaces:**
- Consumes: `RoomPagination`, `RoomTimelineGapProjectionId`, the room cache write lock, `Room::messages(MessagesOptions::backward())`, and the same deduplication/post-processing/publication machinery used by a limited sync update.
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomLiveTailRefreshOutcome {
    Cancelled,
    Unchanged,
    Advanced { events: usize },
    Detached { events: usize, historical_gap_remaining: bool },
    Stale,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomLiveTailRefreshResult {
    pub outcome: RoomLiveTailRefreshOutcome,
    pub returned_events: usize,
    pub last_projection_batch: Option<u32>,
}

#[derive(Clone, Default)]
pub struct RoomLiveTailRefreshCancellation { /* private CancellationToken */ }

impl RoomLiveTailRefreshCancellation {
    pub fn new() -> Self;
    pub fn cancel(&self);
}

impl RoomPagination {
    pub async fn refresh_live_tail_with_projection(
        &self,
        event_limit: u16,
        projection: RoomTimelineGapProjectionId,
        cancellation: RoomLiveTailRefreshCancellation,
    ) -> Result<RoomLiveTailRefreshResult>;
}
```

- `RoomTimelineGapProjectionId` is reused only as the existing causal publication envelope (`actor_generation`, `repair_generation`); the new algorithm never resolves or chooses a persisted gap descriptor.

- [ ] **Step 1: Add failing integration tests for overlap, unchanged, and detached snapshots**

Add named tests to `crates/matrix-sdk/tests/integration/event_cache/mod.rs` using the existing `MatrixMockServer` room/event-cache fixtures:

```rust
#[async_test]
async fn live_tail_refresh_uses_tokenless_messages_and_appends_newest_events() {
    // Seed cached events old-1, edge; also seed two unrelated historical gaps.
    // Mock exactly one backwards messages request with no `from` and limit 128.
    // Return [new-2, new-1, edge] in Matrix backwards order.
    // Assert Advanced { events: 2 }, chronological publication edge,new-1,new-2,
    // no request used either historical gap token, and no duplicate edge exists.
}

#[async_test]
async fn unchanged_live_tail_is_a_successful_proof_without_projection() {
    // Return only cached edge events.
    // Assert Unchanged, returned_events > 0, last_projection_batch == None,
    // and the room subscriber receives no duplicate diff.
}

#[async_test]
async fn detached_live_tail_publishes_latest_page_and_one_navigable_gap() {
    // Seed a stale cached edge that is absent from the returned 128 events.
    // Return a non-empty `end` token.
    // Assert Detached { events: 128, historical_gap_remaining: true }, latest
    // events are visible immediately, and inspect_timeline_gaps reports exactly
    // one additional gap directly before the new live tail.
}
```

- [ ] **Step 2: Run the new tests and capture RED**

Run:

```bash
cargo test -p matrix-sdk --test integration event_cache::live_tail_refresh -- --nocapture
```

Expected: compilation fails because `RoomLiveTailRefreshCancellation`, `RoomLiveTailRefreshOutcome`, and `refresh_live_tail_with_projection` do not exist.

- [ ] **Step 3: Define the public token-free types and cancellation contract**

In `pagination.rs`, add the exact public types above. Implement cancellation as a private `tokio_util::sync::CancellationToken`; `Debug` must print only `RoomLiveTailRefreshCancellation(..)`. Re-export the public types through the same event-cache module path used by existing gap repair types.

The entry point must reject `event_limit == 0` with `EventCacheError::InvalidLinkedChunkMetadata`, capture a private cache fence, and execute these phases:

```rust
let fence = LiveTailSnapshotFence::capture(&state)?;
let response = tokio::select! {
    _ = cancellation.cancelled() => {
        return Ok(RoomLiveTailRefreshResult::cancelled());
    }
    response = room.messages(MessagesOptions::backward().limit(event_limit.into())) => response?,
};
// No cancellation branch below this line: commit is atomic once it starts.
self.commit_live_tail_response(fence, response, projection).await
```

- [ ] **Step 4: Implement the private fence and limited-tail commit**

In `live_tail.rs`, define a private fence containing `gap_snapshot_id`, `gap_topology_generation`, and the newest cached event identity. Revalidate all three under the room cache write lock before mutation; return `Stale` with no diff when any changes.

Apply the response with this exact classification:

```rust
match (previous_edge, response_contains_previous_edge, new_event_count) {
    (_, _, 0) => Unchanged,
    (Some(_), true, count) => Advanced { events: count },
    (Some(_), false, count) => Detached {
        events: count,
        historical_gap_remaining: response.end.is_some(),
    },
    (None, _, count) => Advanced { events: count },
}
```

Reverse the backwards response into chronological order, deduplicate through existing event-cache identity helpers, post-process with the same relation/redaction path as limited sync, and publish with `EventsOrigin::GapRepair { actor_generation, repair_generation, projection_batch }` so Core can retain the already-tested causal fence. For `Detached`, retain older chunks and insert `response.end` as exactly one gap immediately before the refreshed tail. For `Advanced`, join through the overlapping edge and do not manufacture a gap between already-contiguous cached events.

- [ ] **Step 5: Add RED tests for stale fencing and phase-aware cancellation**

Add:

```rust
#[async_test]
async fn live_tail_refresh_discards_response_when_sync_moves_cache_fence() {
    // Block /messages, commit a sync event to the room cache, release response.
    // Assert Stale, no refresh-produced event, and sync event remains canonical.
}

#[async_test]
async fn live_tail_refresh_cancels_network_but_finishes_started_commit() {
    // First cancel while /messages is blocked: assert Cancelled and no mutation.
    // Then pause at a test-only commit hook, cancel after the write lock is held,
    // release hook, and assert persisted chunks and subscriber diff complete once.
}

#[async_test]
async fn detached_live_tail_gap_continues_with_standard_back_pagination() {
    // After Detached, call ordinary paginate_backwards(50).
    // Assert its request uses the persisted end token and renders the next page.
}
```

- [ ] **Step 6: Run the SDK tests to GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p matrix-sdk --test integration event_cache::live_tail_refresh -- --nocapture
cargo test -p matrix-sdk event_cache::caches::room --lib
```

Expected: all new live-tail tests pass; existing room cache unit tests pass.

- [ ] **Step 7: Commit the SDK deliverable**

```bash
git add crates/matrix-sdk/src/event_cache/caches/room/live_tail.rs \
  crates/matrix-sdk/src/event_cache/caches/room/mod.rs \
  crates/matrix-sdk/src/event_cache/caches/room/pagination.rs \
  crates/matrix-sdk/src/event_cache/caches/room/state.rs \
  crates/matrix-sdk/tests/integration/event_cache/mod.rs
git commit -m "fix(event-cache): refresh authoritative room live tail"
```

Record the resulting SDK commit SHA; Task 2 pins exactly that SHA.

### Task 2: Koushi SDK privacy adapter and SDK pin

**Worktree:** `/Users/hiroshi/projects/Element-dev/matrix-desktop/.worktrees/issue-275-room-absent-recovery`

**Files:**
- Modify: `vendor/matrix-rust-sdk`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `scripts/check-matrix-sdk-pin.mjs`
- Modify: `scripts/test-matrix-sdk-pin-guard.mjs`
- Modify: `crates/koushi-sdk/src/lib.rs`
- Test: `crates/koushi-sdk/tests/timeline_gap_adapter.rs`

**Interfaces:**
- Consumes: Task 1 `RoomLiveTailRefresh*` types and `RoomTimelineGapProjectionId` causal envelope.
- Produces:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixLiveTailRefreshOutcome {
    Cancelled,
    Unchanged,
    Advanced { events: usize },
    Detached { events: usize, historical_gap_remaining: bool },
    Stale,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatrixLiveTailRefreshResult {
    pub outcome: MatrixLiveTailRefreshOutcome,
    pub returned_events: usize,
    pub last_projection_batch: Option<u32>,
}

#[derive(Clone, Default)]
pub struct MatrixLiveTailRefreshCancellation { /* private SDK handle */ }

impl MatrixLiveTailRefreshCancellation {
    pub fn new() -> Self;
    pub fn cancel(&self);
}

impl MatrixClientSession {
    pub async fn refresh_room_live_tail(
        &self,
        room_id: &str,
        event_limit: u16,
        actor_generation: u64,
        operation_generation: u64,
        cancellation: MatrixLiveTailRefreshCancellation,
    ) -> MatrixLiveTailRefreshResult;
}
```

- [ ] **Step 1: Pin the Task 1 SDK commit before compiling adapter tests**

Update the `vendor/matrix-rust-sdk` gitlink to Task 1's SHA. Replace every prior room-absent-fence SHA in `Cargo.toml`, `Cargo.lock`, and the two pin guard scripts with the same exact SHA; do not use a branch name or floating revision.

- [ ] **Step 2: Write failing adapter and privacy tests**

Add to `timeline_gap_adapter.rs`:

```rust
#[test]
fn live_tail_refresh_debug_is_privacy_safe() {
    let cancellation = MatrixLiveTailRefreshCancellation::new();
    let debug = format!("{cancellation:?}");
    assert_eq!(debug, "MatrixLiveTailRefreshCancellation(..)");
    assert!(!debug.contains("!room"));
    assert!(!debug.contains("token"));
}

#[test]
fn live_tail_refresh_outcomes_are_token_free_and_coarse() {
    let outcome = MatrixLiveTailRefreshOutcome::Detached {
        events: 128,
        historical_gap_remaining: true,
    };
    assert_eq!(format!("{outcome:?}"),
        "Detached { events: 128, historical_gap_remaining: true }");
}
```

Extend the existing mock-client test to call `refresh_room_live_tail` and assert that invalid/unavailable room and SDK failures all return the coarse `Failed` outcome without raw error text.

- [ ] **Step 3: Run adapter tests and capture RED**

Run:

```bash
cargo test -p koushi-sdk --test timeline_gap_adapter live_tail -- --nocapture
```

Expected: compilation fails because the Matrix adapter types are undefined.

- [ ] **Step 4: Implement the minimal adapter**

Add the exact types above next to `MatrixTimelineGapRepairResult`. `refresh_room_live_tail` parses and resolves the room internally, obtains `room.event_cache().await`, calls Task 1 with limit/projection/cancellation, maps every SDK outcome one-for-one, and maps parse, room lookup, cache, and SDK errors to:

```rust
MatrixLiveTailRefreshResult {
    outcome: MatrixLiveTailRefreshOutcome::Failed,
    returned_events: 0,
    last_projection_batch: None,
}
```

Implement manual `Debug` for the cancellation wrapper. Do not add accessors for its SDK handle, room IDs, events, or pagination data.

- [ ] **Step 5: Run adapter and pin guards to GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
node --test scripts/test-matrix-sdk-pin-guard.mjs
node scripts/check-matrix-sdk-pin.mjs
```

Expected: adapter suite passes and both guards report the same Task 1 SDK SHA.

- [ ] **Step 6: Commit the adapter and pin**

```bash
git add vendor/matrix-rust-sdk Cargo.toml Cargo.lock \
  scripts/check-matrix-sdk-pin.mjs scripts/test-matrix-sdk-pin-guard.mjs \
  crates/koushi-sdk/src/lib.rs crates/koushi-sdk/tests/timeline_gap_adapter.rs
git commit -m "feat(sdk): expose token-free live-tail refresh"
```

### Task 3: Core freshness state machine and single-slot scheduler

**Files:**
- Create: `crates/koushi-core/src/live_tail_freshness.rs`
- Modify: `crates/koushi-core/src/lib.rs`
- Test: `crates/koushi-core/src/live_tail_freshness.rs`

**Interfaces:**
- Consumes: account-local opaque `TimelineKey`, sync epoch `u64`, actor generation `u64`, operation generation `u64`.
- Produces:

```rust
pub(crate) const FOREGROUND_LIVE_TAIL_LIMIT: u16 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveTailFreshnessState {
    Unproven { epoch: u64 },
    Refreshing { epoch: u64, operation_generation: u64 },
    Fresh { epoch: u64 },
    Deferred { epoch: u64 },
    Retryable { epoch: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LiveTailSchedulerAction<K> {
    Start { key: K, epoch: u64, operation_generation: u64, limit: u16 },
    CancelNetwork { key: K, operation_generation: u64 },
}

pub(crate) struct LiveTailRefreshCoordinator<K> { /* active, running, FIFO delayed */ }
```

The coordinator methods are `activate(key, epoch)`, `mark_unproven(key, epoch)`, `mark_fresh(key, epoch)`, `finish(key, epoch, operation_generation, outcome)`, and `invalidate_epoch(key, epoch)`, each returning zero or more `LiveTailSchedulerAction<K>`.

- [ ] **Step 1: Write the pure state-machine RED tests**

Add tests in the new module:

```rust
#[test]
fn active_unproven_room_starts_once_and_same_epoch_does_not_retry_after_fresh() { /* ... */ }

#[test]
fn activating_b_preempts_a_and_delays_a_before_starting_b() { /* ... */ }

#[test]
fn late_old_epoch_finish_cannot_prove_replacement_epoch() { /* ... */ }

#[test]
fn failed_active_refresh_is_retryable_without_busy_loop() { /* ... */ }

#[test]
fn delayed_rooms_run_one_at_a_time_in_fifo_order() { /* ... */ }
```

Use opaque test keys `A`, `B`, `C`; assert full ordered action vectors, including `CancelNetwork(A)` before `Start(B)` and limit 128.

- [ ] **Step 2: Run the module tests and capture RED**

Run:

```bash
cargo test -p koushi-core live_tail_freshness::tests --lib -- --nocapture
```

Expected: compilation fails because the module and coordinator do not exist.

- [ ] **Step 3: Implement the minimal deterministic coordinator**

Use `VecDeque<K>` plus a membership set to prevent duplicate delayed entries. Increment the operation generation only when emitting `Start`. `activate(B)` must emit cancellation before a B start; `finish` must ignore mismatched `(key, epoch, operation_generation)`. `Failed`, `Cancelled`, and `Stale` return an active room to `Retryable` or an inactive room to `Deferred`, but do not immediately self-retry; the next checkpoint, selection, or explicit retry tick may start it.

Exact proof mapping:

```rust
match outcome {
    Unchanged | Advanced { .. } | Detached { .. } => Fresh { epoch },
    Cancelled if is_active => Unproven { epoch },
    Cancelled => Deferred { epoch },
    Stale | Failed if is_active => Retryable { epoch },
    Stale | Failed => Deferred { epoch },
}
```

- [ ] **Step 4: Run Core state tests to GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-core live_tail_freshness::tests --lib -- --nocapture
```

Expected: all five coordinator tests pass.

- [ ] **Step 5: Commit the coordinator**

```bash
git add crates/koushi-core/src/live_tail_freshness.rs crates/koushi-core/src/lib.rs
git commit -m "feat(core): schedule active live-tail freshness"
```

### Task 4: Timeline actor integration, cancellation, and diagnostics

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/src/live_catchup.rs`
- Test: `crates/koushi-core/src/timeline.rs`
- Test: `crates/koushi-core/src/live_catchup.rs`

**Interfaces:**
- Consumes: Task 2 `MatrixLiveTailRefresh*`; Task 3 `LiveTailRefreshCoordinator` and actions; existing `GapRepairProjectionId` extraction for causally tagged diffs.
- Produces: active selection hand-off to TimelineManager, one actor-owned cancellation handle per live-tail operation, epoch-fenced proof transitions, and privacy-safe diagnostic records.

- [ ] **Step 1: Replace the production-shape regression with a live-tail RED test**

Replace the room-absent assertion that chooses the last persisted gap with:

```rust
#[tokio::test]
async fn legacy_room_absent_refreshes_active_live_tail_not_persisted_gap() {
    // Seed four historical gaps and a stale rendered edge.
    // Commit a legacy response whose room membership is Absent.
    // Mock tokenless /messages with recent events.
    // Assert refresh called with limit 128 and no gap handle/ordinal.
    // Publish the matching causal batch and assert recent events render.
    // Assert Fresh for the response epoch and no repeated call in that epoch.
}
```

Add actor/manager tests for preemption, late epoch result, and commit-boundary publication. The preemption test must assert the ordered fake-session log:

```rust
[
    "start:A:epoch=7:limit=128",
    "cancel-network:A:epoch=7",
    "start:B:epoch=9:limit=128",
]
```

- [ ] **Step 2: Run targeted Core tests and capture RED**

Run:

```bash
cargo test -p koushi-core legacy_room_absent_refreshes_active_live_tail_not_persisted_gap --lib -- --nocapture
cargo test -p koushi-core live_tail_preemption --lib -- --nocapture
```

Expected: failures show the existing `RepairPersistedLiveEdge` path still selects a persisted gap and there is no live-tail scheduler action handling.

- [ ] **Step 3: Remove room-absent persisted-gap selection**

In `live_catchup.rs`, change room-absent evaluation from `RepairPersistedLiveEdge` to a freshness signal:

```rust
pub(crate) enum LiveCatchupDecision {
    None,
    ProveLiveTailFreshness,
    RepairExactLimitedUpdate,
}
```

Delete the mapping from `ProveLiveTailFreshness` to `inspection.gaps.last()`. Keep the exact limited checkpoint path, descriptor matching, and viewport projection logic unchanged. Update unit tests to assert room absence never consumes `MatrixTimelineGapInspection`.

- [ ] **Step 4: Integrate active selection and operation ownership**

TimelineManager owns `LiveTailRefreshCoordinator<TimelineKey>`. On every committed `SelectRoom`, call `activate(new_key, current_sync_epoch)` before dispatching the new room subscribe effect. Apply emitted actions in order:

```rust
match action {
    CancelNetwork { key, operation_generation } => {
        actor.cancel_live_tail_network(operation_generation);
    }
    Start { key, epoch, operation_generation, limit } => {
        actor.start_live_tail_refresh(epoch, operation_generation, limit);
    }
}
```

Each actor stores `Option<(u64, MatrixLiveTailRefreshCancellation, JoinHandle<()>)>`. Cancelling calls the handle's `cancel()`; it does not abort the task, so Task 1's post-response commit can finish. Actor drop cancels the token and drops the join handle only during account shutdown.

`RoomAbsent` calls `mark_unproven`. Exact SyncService and exact legacy room updates call `mark_fresh`. Backend/sync-run replacement calls `invalidate_epoch`. A task completion is accepted only when actor generation, sync epoch, operation generation, and optional final projection batch all match.

- [ ] **Step 5: Add structured privacy-safe diagnostics**

Emit existing structured diagnostic records with only these fields:

```text
timeline_live_tail_state: from, to, sync_epoch
timeline_live_tail_queue: priority, queue_depth, preempted
timeline_live_tail_refresh: outcome, requested_limit, returned_events,
                            historical_gap_remaining, operation_generation,
                            duration_ms
timeline_live_tail_commit: phase(started|completed), operation_generation
```

Never attach timeline key, room ID, event ID, pagination token, message content, or formatted SDK error. Add a diagnostic serialization test that scans the JSON for `!private`, `$event`, `token-secret`, `message-secret`, and asserts all are absent.

- [ ] **Step 6: Run actor and policy tests to GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-core live_tail --lib -- --nocapture
cargo test -p koushi-core live_catchup --lib -- --nocapture
```

Expected: live-tail, epoch fence, preemption, commit-boundary, and diagnostics tests pass; existing exact gap repair tests still pass.

- [ ] **Step 7: Commit actor integration**

```bash
git add crates/koushi-core/src/timeline.rs crates/koushi-core/src/live_catchup.rs
git commit -m "fix(core): prioritize active room live-tail recovery"
```

### Task 5: Headless production-shape regression and 128+ continuation

**Files:**
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`
- Modify: `crates/koushi-core/tests/headless_core_qa.rs`

**Interfaces:**
- Consumes: the real Core manager/actor and SDK adapter from Tasks 2–4.
- Produces: deterministic `legacy_live_tail_room_absent=ok`, `live_tail_detached_gap=ok`, and `live_tail_historical_continuation=ok` QA markers.

- [ ] **Step 1: Add failing headless marker assertions**

In `headless_core_qa.rs`, require:

```rust
assert!(stdout.contains("legacy_live_tail_room_absent=ok"));
assert!(stdout.contains("live_tail_detached_gap=ok"));
assert!(stdout.contains("live_tail_historical_continuation=ok"));
```

Remove the obsolete `legacy_persisted_gap_room_absent=ok` assertion.

- [ ] **Step 2: Run the headless test and capture RED**

Run:

```bash
cargo test -p koushi-core --test headless_core_qa -- --nocapture
```

Expected: FAIL because the three new markers are absent.

- [ ] **Step 3: Implement the production-shape scenario**

In the QA binary, seed a stale active DM plus multiple unrelated historical gaps, commit a legacy room-absent response, and serve recent events only from tokenless `/messages`. Assert request query is exactly `dir=b&limit=128` with no `from`, then print `legacy_live_tail_room_absent=ok` after the recent event is projected.

Run a second response containing 128 events and `end`; assert the newest event projects immediately and one additional gap exists, then print `live_tail_detached_gap=ok`. Trigger ordinary backward pagination, assert the next request contains the prior `end` token and its older event projects, then print `live_tail_historical_continuation=ok`.

- [ ] **Step 4: Run headless QA to GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-core --test headless_core_qa -- --nocapture
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=timeline_legacy_persisted_gap --core --core-backend=probed --timeout-ms=300000
```

Expected: tests pass and the real-sync run prints all three new `=ok` markers.

- [ ] **Step 5: Commit QA coverage**

```bash
git add crates/koushi-core/src/bin/headless-core-qa.rs \
  crates/koushi-core/tests/headless_core_qa.rs
git commit -m "test(core): cover stale active live-tail recovery"
```

### Task 6: Full verification, review, and app handoff

**Files:**
- Modify only files required by concrete failures or reviewer findings from Tasks 1–5.

**Interfaces:**
- Consumes: all previous commits.
- Produces: a clean SDK and superproject worktree with all supported macOS/Linux verification green and a review-ready commit range.

- [ ] **Step 1: Run complete SDK verification**

In `/Users/hiroshi/projects/Element-dev/matrix-desktop/.worktrees/matrix-sdk-room-absent-fence`:

```bash
cargo fmt --all -- --check
cargo test -p matrix-sdk event_cache --lib
cargo test -p matrix-sdk --test integration event_cache -- --nocapture
```

Expected: all event-cache unit and integration tests pass.

- [ ] **Step 2: Run complete superproject verification**

In `/Users/hiroshi/projects/Element-dev/matrix-desktop/.worktrees/issue-275-room-absent-recovery`:

```bash
cargo fmt --all -- --check
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
cargo test -p koushi-core --lib -- --nocapture
cargo test -p koushi-core --test headless_core_qa -- --nocapture
node --test scripts/test-matrix-sdk-pin-guard.mjs
node scripts/check-matrix-sdk-pin.mjs
```

Expected: all commands exit 0; no ignored existing test is newly enabled or disabled.

- [ ] **Step 3: Run Linux amd64 headless verification**

Run:

```bash
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=timeline_legacy_persisted_gap --core --core-backend=probed --timeout-ms=300000
```

Expected: exit 0 with `legacy_live_tail_room_absent=ok`, `live_tail_detached_gap=ok`, and `live_tail_historical_continuation=ok`; this command launches the existing Linux amd64 Conduit lane and executes the real Core restart scenario rather than only compiling it.

- [ ] **Step 4: Inspect privacy and old-policy removal mechanically**

Run:

```bash
rg -n "RepairPersistedLiveEdge|legacy_persisted_gap_room_absent" crates/koushi-core
rg -n "timeline_live_tail_(state|queue|refresh|commit)" crates/koushi-core
git diff --check 9d6c5cf..HEAD
git status --short
```

Expected: first search has no matches; second finds the four diagnostic families; diff check prints nothing; status is clean.

- [ ] **Step 5: Request two-stage review and fix concrete findings**

The spec reviewer checks every invariant and regression in `docs/superpowers/specs/2026-07-17-active-live-tail-freshness-design.md`. The code-quality reviewer checks cancellation races, linked-chunk atomicity, duplicate projection, privacy, and test realism. Any fix gets its own RED reproducer, GREEN run, and focused commit.

- [ ] **Step 6: Build and launch the corrected macOS app for user validation**

Build from the superproject worktree with the same development configuration used by the current Koushi diagnostics build. Confirm diagnostics report the pinned SDK SHA and the new live-tail event families. Open the affected DM and verify the recent live edge appears; do not claim the production symptom fixed until this manual observation or an equivalent real-account diagnostic is available.

## Self-Review Record

- Spec coverage: Tasks 1 and 5 cover tokenless latest-tail retrieval, overlap, detached 128+ behavior, and historical continuation; Tasks 3 and 4 cover proofs, epochs, single-slot priority, preemption, delayed recovery, cancellation phases, and diagnostics; Task 4 preserves exact/viewport repairs while removing the persisted-gap substitute.
- Placeholder scan: no `TBD`, `TODO`, “implement later”, or unnamed error/edge-case step remains. Test names, interfaces, commands, expected RED/GREEN outcomes, and commit boundaries are explicit.
- Type consistency: Task 1 SDK outcomes map one-for-one to Task 2 Matrix outcomes; Task 3 consumes those same outcome variants; `operation_generation` maps to the existing projection envelope's `repair_generation` solely at the SDK boundary.
- Design clarification: an `end` token becomes a new navigable gap only for a detached bounded tail. When the prior edge overlaps the response, the cache joins at that edge instead of introducing a false gap through already-contiguous events.
