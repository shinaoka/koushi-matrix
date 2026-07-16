# Live Catch-Up Gap Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair events missed while a room was unsubscribed by waiting for the current sliding-sync room-subscription checkpoint and repairing the exact persisted gap introduced by that response.

**Architecture:** The Matrix SDK event cache records a token-free observation after committing each room timeline response. RoomListService generation-fences those observations against room subscriptions and publishes retained checkpoints. Koushi routes the matching checkpoint to the active TimelineActor, which defers only `LiveEdge` gap acquisition until provenance arrives and selects the checkpoint-owned gap; all rendering and non-live-edge pagination remain unchanged.

**Tech Stack:** Rust, Tokio, Matrix Rust SDK event cache and RoomListService, Koushi actors and diagnostics, MatrixMockServer, local Conduit/Tuwunel headless QA.

---

### Task 1: Matrix SDK event-cache sync observation

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/pagination.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/state.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/mod.rs`
- Test: `vendor/matrix-rust-sdk/crates/matrix-sdk/tests/integration/event_cache/mod.rs`

- [ ] **Step 1: Write failing event-cache integration tests**

Add tests that sync a limited room timeline containing a normal event followed by a relation event and assert:

```rust
let observation = room_event_cache.latest_sync_observation().await.expect("observation");
assert_eq!(observation.sequence(), 1);
assert!(observation.limited());
assert_eq!(observation.event_count(), 2);
assert!(observation.prev_batch_present());
assert_eq!(observation.newest_event_id(), Some(relation_event_id));
let gap = observation.inserted_gap().expect("sync gap");
assert!(room_event_cache.inspect_timeline_gaps().await?.gaps.contains(gap));
assert!(!format!("{observation:?}").contains(relation_event_id.as_str()));
assert!(!format!("{observation:?}").contains("secret-prev-batch"));
```

Add a second sync with no timeline mutation and assert its room cache does not manufacture a new observation sequence.

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cargo test -p matrix-sdk --test integration event_cache::room_timeline_sync_observation -- --nocapture
```

Expected: compilation fails because `latest_sync_observation` and `RoomTimelineSyncObservation` do not exist.

- [ ] **Step 3: Implement the minimal observation**

Define `RoomTimelineSyncObservation` with private fields and closed accessors. Use a custom `Debug` containing only sequence, flags, counts, and gap presence. Make `inspect_ordered_chunks` `pub(super)` so `RoomEventCacheStateLockWriteGuard::handle_sync` can construct the exact descriptor after `post_process_new_events` and `shrink_to_last_chunk` have persisted the final topology. Return `Option<RoomTimelineSyncObservation>` from `handle_sync`; store it under an async lock in `RoomEventCacheInner`, incrementing the per-room sequence only for an actual timeline mutation. Expose:

```rust
pub async fn latest_sync_observation(&self) -> Option<RoomTimelineSyncObservation>;
```

Re-export the public type from `matrix_sdk::event_cache`.

- [ ] **Step 4: Run focused SDK tests and verify GREEN**

Run the Task 1 command and:

```bash
cargo test -p matrix-sdk --test integration event_cache -- --nocapture
```

Expected: all selected event-cache tests pass.

- [ ] **Step 5: Commit the SDK event-cache change**

```bash
git -C vendor/matrix-rust-sdk add crates/matrix-sdk
git -C vendor/matrix-rust-sdk commit -m "feat: observe committed room sync gaps"
```

### Task 2: RoomListService subscription checkpoints

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/src/room_list_service/mod.rs`
- Test: `vendor/matrix-rust-sdk/crates/matrix-sdk-ui/tests/integration/room_list_service.rs`

- [ ] **Step 1: Write failing generation/checkpoint tests**

Extend the existing room-subscription integration fixture to assert:

```rust
let generation = room_list.subscribe_to_rooms_with_generation(&[room_id]).await;
let mut checkpoints = room_list.room_subscription_checkpoints();
// Drive a limited response containing room_id.
let checkpoint = wait_for_generation(&mut checkpoints, room_id, generation).await;
assert_eq!(checkpoint.subscription_generation(), generation);
assert!(checkpoint.timeline().and_then(|t| t.inserted_gap()).is_some());
```

Add cases where a delayed generation G response completes after G+1, a room response has no newer timeline observation, and a receiver subscribes after publication. Assert respectively that G is rejected, `timeline()` is `None`, and the retained G+1 checkpoint is immediately available. Assert custom `Debug` omits room IDs, event IDs, and tokens.

- [ ] **Step 2: Run and verify RED**

```bash
cargo test -p matrix-sdk-ui --test integration room_list_service::test_room_subscription_checkpoint -- --nocapture
```

Expected: compilation fails because generation/checkpoint APIs do not exist.

- [ ] **Step 3: Implement generation-fenced retained checkpoints**

Add opaque `RoomSubscriptionGeneration`, redacted `RoomSubscriptionCheckpoint`, and private subscription state containing generation, active room set, and per-room observation baselines. Preserve `subscribe_to_rooms` as a wrapper and add:

```rust
pub async fn subscribe_to_rooms_with_generation(
    &self,
    room_ids: &[&RoomId],
) -> RoomSubscriptionGeneration;

pub fn room_subscription_checkpoints(
    &self,
) -> Subscriber<Arc<BTreeMap<OwnedRoomId, RoomSubscriptionCheckpoint>>>;
```

Snapshot the generation immediately before awaiting `sync.next()`. After a successful `UpdateSummary`, publish only rooms that are in `summary.rooms`, remain actively subscribed, and still belong to that generation. Attach a timeline observation only when its sequence exceeds the baseline. Retain only the latest checkpoint per room.

- [ ] **Step 4: Run focused and neighboring tests**

```bash
cargo test -p matrix-sdk-ui --test integration room_list_service -- --nocapture
cargo test -p matrix-sdk-ui --test integration timeline::sliding_sync -- --nocapture
```

Expected: all selected tests pass.

- [ ] **Step 5: Commit the SDK UI change**

```bash
git -C vendor/matrix-rust-sdk add crates/matrix-sdk-ui
git -C vendor/matrix-rust-sdk commit -m "feat: publish room subscription checkpoints"
```

### Task 3: Koushi token-free adapter and dependency pin

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-sdk/tests/timeline_gap_adapter.rs`
- Modify: `vendor/matrix-rust-sdk` (submodule pointer)

- [ ] **Step 1: Write failing adapter tests**

Add tests proving the wrapper exposes only generation, flags, counts, anchor presence, and opaque gap equality:

```rust
assert_eq!(checkpoint.subscription_generation(), expected_generation);
assert!(checkpoint.has_timeline_update());
assert!(checkpoint.has_inserted_gap());
assert!(checkpoint.inserted_gap_matches(&inspection.gaps[expected_ordinal]));
assert!(!format!("{checkpoint:?}").contains("secret"));
```

- [ ] **Step 2: Run and verify RED**

```bash
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
```

Expected: compilation fails because the checkpoint adapter is absent.

- [ ] **Step 3: Implement the adapter and pin the SDK revision**

Add `MatrixRoomSubscriptionGeneration`, `MatrixRoomSubscriptionCheckpoint`, and conversion/subscription helpers around RoomListService. Keep room and event identities actor-private and implement redacted custom `Debug`. Add `MatrixTimelineGapHandle::same_gap_as_checkpoint` using upstream descriptor equality. Update every Matrix SDK git `rev` in root `Cargo.toml` to the final SDK fork commit and record the same commit in the submodule pointer.

- [ ] **Step 4: Run adapter tests and compile consumers**

```bash
cargo test -p koushi-sdk --test timeline_gap_adapter -- --nocapture
cargo check -p koushi-core
```

Expected: both commands succeed.

- [ ] **Step 5: Commit the adapter change**

```bash
git add Cargo.toml Cargo.lock crates/koushi-sdk vendor/matrix-rust-sdk
git commit -m "feat: adapt live catch-up checkpoints"
```

### Task 4: Reproduce the core race as a failing behavior test

**Files:**
- Create: `crates/koushi-core/src/live_catchup.rs`
- Modify: `crates/koushi-core/src/lib.rs`
- Modify: `crates/koushi-core/src/timeline.rs`

- [ ] **Step 1: Add the deterministic real-actor regression test**

Using `MatrixMockServer`, seed an older persisted gap, start a room subscription whose limited response is held behind a barrier, build the room TimelineActor, and acknowledge its initial projection. Assert the required behavior: rendering and its ACK complete, but no gap repair request is made before the response. Release the response and require the same actor generation to repair the checkpoint-owned gap. The test name is:

```rust
timeline_actor_waits_for_current_subscription_checkpoint_before_live_edge_repair
```

- [ ] **Step 2: Run and verify RED**

```bash
cargo test -p koushi-core --lib timeline_actor_waits_for_current_subscription_checkpoint_before_live_edge_repair -- --nocapture
```

Expected before the fix: FAIL because the actor requests the older persisted gap before the current subscription response is released. Preserve this output as RED evidence; Task 5 must make this exact test pass without weakening it.

- [ ] **Step 3: Add pure shadow classifier and privacy tests**

In `live_catchup.rs`, define the closed decisions from the design spec and table-driven tests for awaiting, no timeline, anchored, exact match, different gap, missing topology, stale generation, and unsupported backend. Add a diagnostic builder test that rejects private field names and emits only closed tokens/counts.

- [ ] **Step 4: Run the pure tests without weakening the RED behavior test**

```bash
cargo test -p koushi-core --lib live_catchup -- --nocapture
```

Expected: the classifier tests pass; rerunning the behavior test still fails for the same early repair request. Do not commit the failing behavior test separately—carry it directly into Task 5 so the repository is committed only after the same test turns GREEN.

### Task 5: Fix LiveEdge acquisition and exact gap selection

**Files:**
- Modify: `crates/koushi-core/src/live_catchup.rs`
- Modify: `crates/koushi-core/src/timeline.rs`

- [ ] **Step 1: Confirm the unchanged regression test still expresses the required behavior**

Before releasing the response, it asserts no gap repair request is made and rendering/initial projection ACK completes. After releasing it, it asserts the actor selects the checkpoint-owned newest gap, repairs the missing interval, preserves repair budgets and render fences, and emits no duplicate timeline rows. Do not rename, skip, or relax the test written in Task 4.

- [ ] **Step 2: Re-run and verify the same RED failure**

```bash
cargo test -p koushi-core --lib timeline_actor_waits_for_current_subscription_checkpoint_before_live_edge_repair -- --nocapture
```

Expected: FAIL because current code starts repair before the checkpoint.

- [ ] **Step 3: Implement minimal manager/actor routing**

Add a manager-owned checkpoint observer task and `TimelineMessage::RoomSubscriptionCheckpoint`. Abort and replace the observer on sync backend replacement and shutdown. Pass the subscription generation from `build_timeline_actor_handle` to room actors; use an unsupported marker for legacy sync, threads, and focused timelines. Route and replay checkpoints only when room ID, subscription generation, and active actor generation match.

- [ ] **Step 4: Implement the LiveEdge wait and exact selection**

For room `LiveEdge` only:

```rust
match live_catchup.checkpoint_for(actor_generation, subscription_generation) {
    Pending => return without acquiring gap-repair scheduler ownership,
    NoTimelineUpdate | Anchored => finish_live_edge_without_gap(),
    Gap(checkpoint_gap) => select_the_inspected_gap_matching(checkpoint_gap),
    Stale => ignore_without_mutating_repair_state(),
}
```

On checkpoint arrival, schedule the deferred `LiveEdge` inspection. Do not delay timeline construction, initial item emission, projection ACK, automatic projected-gap repair, or manual pagination. If the checkpoint gap is already absent, re-inspect once and classify it as joined/stale without falling back to another gap.

- [ ] **Step 5: Run focused core regression tests**

```bash
cargo test -p koushi-core --lib timeline_actor_waits_for_current_subscription_checkpoint_before_live_edge_repair -- --nocapture
cargo test -p koushi-core --lib live_catchup -- --nocapture
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests -- --nocapture
```

Expected: all tests pass; the old repair-before-checkpoint expectation no longer exists.

- [ ] **Step 6: Commit the behavioral fix**

```bash
git add crates/koushi-core
git commit -m "fix: anchor live catch-up repair to room sync"
```

### Task 6: Production-faithful local headless regression

**Files:**
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`
- Modify: `apps/desktop/scripts/run-headless-local.mjs` if token routing requires it
- Modify: `AGENTS.md`
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`

- [ ] **Step 1: Extend `timeline_reconnect` before changing QA production hooks**

Add an unsubscribed-room stage: A and B join, A seeds one known anchor, A closes/unsubscribes the room timeline, A's proxy is disabled, B sends more than 20 messages, A reconnects and opens the room. Assert the received event set is contiguous and print only:

```text
live_catchup_checkpoint=ok
live_catchup_gap_repaired=ok
```

Do not print room IDs, event IDs, bodies, server names, tokens, or raw errors.

- [ ] **Step 2: Run the local QA stage**

```bash
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=timeline_reconnect --core --core-backend=sliding-sync --timeout-ms=240000
```

Expected before any necessary harness adjustment: the stage either fails on missing catch-up events or passes with both new closed tokens. Any failure must be fixed in product code unless it is demonstrably a deterministic harness contract error.

- [ ] **Step 3: Update canon**

Document that current room-subscription checkpoints are Rust-owned, `LiveEdge` repair waits for matching provenance, exact checkpoint gaps are selected without fallback, legacy sync remains explicitly unsupported for this gate, and diagnostics/QA evidence are private-data-free.

- [ ] **Step 4: Run focused headless and privacy checks**

Run the Task 6 QA command again and:

```bash
rg -n "live_catchup_(checkpoint|gap_repaired)=ok" crates/koushi-core/src/bin/headless-core-qa.rs
```

Expected: QA exits zero with both tokens and source contains no interpolated identifier payload for those tokens.

- [ ] **Step 5: Commit QA and canon**

```bash
git add crates/koushi-core/src/bin/headless-core-qa.rs apps/desktop/scripts/run-headless-local.mjs AGENTS.md docs/architecture/state-machine.md docs/policies/engineering-rules.md
git commit -m "test: cover unsubscribed room live catch-up"
```

### Task 7: Full verification, external review, and PR publication

**Files:**
- Modify as required only when a verified review finding is actionable.

- [ ] **Step 1: Run formatting and focused repository gates**

```bash
cargo fmt --all -- --check
cargo test -p koushi-sdk --test timeline_gap_adapter
cargo test -p koushi-core --lib live_catchup
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
cargo test -p koushi-core --lib timeline_actor_waits_for_current_subscription_checkpoint_before_live_edge_repair
cargo check -p koushi-core
```

Expected: every command exits zero.

- [ ] **Step 2: Run SDK fork gates**

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml -p matrix-sdk --test integration event_cache
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml -p matrix-sdk-ui --test integration room_list_service
```

Expected: all selected tests pass.

- [ ] **Step 3: Run full local headless proof**

Run the Task 6 command fresh. Expected: exit zero and both closed tokens.

- [ ] **Step 4: Run the required private external diff reviews**

Generate separate SDK and Koushi diffs, include untracked files and root pin/module files, and feed them to `codex review -` with the repository-rule, architecture, state-machine, security/privacy, and contract priorities from `AGENTS.md`. Verify every finding before adopting or rejecting it.

- [ ] **Step 5: Publish the SDK fork PR**

Push the SDK branch, open a ready-for-review PR against `shinaoka/matrix-rust-sdk-work`, and wait for required checks. The Koushi dependency pin must reference the final pushed SDK commit.

- [ ] **Step 6: Re-run final Koushi verification after the final SDK pin**

Repeat Steps 1 and 3 after updating the final SDK commit in all Cargo entries and submodule pointer. Confirm `git status --short` contains no unintended changes.

- [ ] **Step 7: Publish the Koushi PR**

Push `codex/issue-271-live-catchup-shadow`, open a ready-for-review PR against `main`, include `Fixes #271`, root cause, SDK dependency PR/commit, RED→GREEN evidence, privacy guarantees, and all verification commands. Wait for required CI and leave the issue closable by merge; do not manually close it before merge.
