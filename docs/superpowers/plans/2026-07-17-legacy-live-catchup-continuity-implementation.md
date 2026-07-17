# Legacy Live Catch-Up Continuity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make SyncService and legacy fallback repair the exact current-response live-edge gap, recover lost settlement fences, and report Running only after a successful backend response.

**Architecture:** The SDK event cache publishes retained committed-room observations after topology mutation. Koushi wraps SyncService and legacy observations in one backend-epoch-fenced checkpoint routed through TimelineManager; TimelineActor waits for it before exact `LiveEdge` selection. Relay/render correlations gain bounded recovery, and SyncActor promotes legacy lifecycle only on a first-success control message.

**Tech Stack:** Rust, Tokio, matrix-rust-sdk event cache, matrix-sdk-ui RoomListService, Koushi actors/reducers, MatrixMockServer, local Conduit/Tuwunel headless QA.

---

### Task 1: Canon and deterministic RED contract

**Files:**
- Modify: `docs/architecture/state-machine.md`
- Modify: `docs/policies/engineering-rules.md`
- Modify: `AGENTS.md`
- Test: `crates/koushi-core/src/timeline.rs`
- Test: `crates/koushi-core/src/sync.rs`

- [ ] **Step 1: Amend the normative state machine before behavior**

Replace the SyncService-only `AwaitingSubscriptionCheckpoint` rule with a
backend-neutral `AwaitingCommittedResponse` transition. Add timeout/overflow
transitions from `AwaitingRelay` and `AwaitingProjection` back through
authoritative resync/re-inspection, preserving queued `LiveEdge` intent.

- [ ] **Step 2: Write the failing real-actor regression**

Add a test named
`legacy_fallback_waits_for_committed_response_and_recovers_missing_interval`
using the existing fake session/timeline actor harness. Seed two gap descriptors
where the older descriptor is unrelated, queue `LiveEdge` before publishing the
legacy checkpoint, and assert no repair call occurs. Publish a checkpoint for
the newer descriptor, complete relay/render settlement, then assert the
expected interval is contiguous and each synthetic event ID occurs exactly
once.

- [ ] **Step 3: Write failing settlement tests**

Add tests named
`relay_overflow_clears_obsolete_gap_correlation_and_requeues_live_edge` and
`render_ack_timeout_clears_fence_and_requeues_live_edge`. Assert the tracker
returns to a runnable queued state instead of `awaiting_relay` or
`awaiting_render_ack`.

- [ ] **Step 4: Write failing lifecycle tests**

Add actor/control tests proving legacy task launch leaves status `Starting`, an
idempotent `Start` before proof does not emit `Running`, and the first successful
legacy response performs exactly one promotion/handoff.

- [ ] **Step 5: Run RED tests**

```bash
cargo test -p koushi-core --lib legacy_fallback_waits_for_committed_response_and_recovers_missing_interval -- --nocapture
cargo test -p koushi-core --lib relay_overflow_clears_obsolete_gap_correlation_and_requeues_live_edge -- --nocapture
cargo test -p koushi-core --lib render_ack_timeout_clears_fence_and_requeues_live_edge -- --nocapture
cargo test -p koushi-core --lib legacy_lifecycle -- --nocapture
```

Expected: each new assertion fails for the pre-fix behavior; existing tests
compile unchanged.

- [ ] **Step 6: Commit RED and canon**

```bash
git add AGENTS.md docs/architecture/state-machine.md docs/policies/engineering-rules.md crates/koushi-core/src/timeline.rs crates/koushi-core/src/sync.rs
git commit -m "test: reproduce legacy live catch-up gap"
```

### Task 2: SDK committed-room observation stream

**Files:**
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/mod.rs`
- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/tasks.rs`
- Test: `vendor/matrix-rust-sdk/crates/matrix-sdk/tests/integration/event_cache/mod.rs`

- [ ] **Step 1: Add failing SDK tests**

Cover a limited committed room response, an empty/no-timeline response, retained
latest-per-room replay for a late subscriber, monotonic sequence, and redacted
`Debug`. The public observation must expose only `sequence()`,
`has_timeline_update()`, `has_inserted_gap()`, and opaque gap matching; room ID
is a routing accessor but absent from `Debug`.

- [ ] **Step 2: Run SDK RED**

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml -p matrix-sdk --test integration event_cache::committed_room_observation -- --nocapture
```

Expected: compilation fails because the committed observation subscription API
does not exist.

- [ ] **Step 3: Implement retained publication**

Add `CommittedRoomTimelineObservation` and an EventCache-owned retained watch
map. Publish only after `RoomEventCacheInner::handle_joined_room_update`
finishes topology mutation. Publish an explicit no-timeline observation for a
joined-room response with no mutation rather than reusing the previous room
observation. Keep the opaque descriptor closed and `Debug` private-safe.

- [ ] **Step 4: Remove full-history lookup from observation construction if measured**

Instrument the synthetic SDK fixture with a private-data-free processed-room
count and elapsed duration. If observation construction dominates the fixture,
derive the just-inserted gap from the in-memory live chunk instead of calling
`load_all_chunks`; otherwise leave the optimization out of this issue.

- [ ] **Step 5: Run SDK GREEN**

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml -p matrix-sdk --test integration event_cache::committed_room_observation -- --nocapture
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml -p matrix-sdk-ui --test integration room_list_service -- --nocapture
```

Expected: all selected tests pass with no identifiers or tokens in output.

- [ ] **Step 6: Commit and publish the SDK fork change**

```bash
git -C vendor/matrix-rust-sdk add crates/matrix-sdk
git -C vendor/matrix-rust-sdk commit -m "fix(event-cache): publish committed room observations"
```

Push the SDK branch and update all root Cargo revisions plus the submodule to
the final SDK commit before Koushi verification.

### Task 3: Backend-neutral Koushi checkpoint adapter and routing

**Files:**
- Modify: `crates/koushi-sdk/src/lib.rs`
- Modify: `crates/koushi-sdk/tests/timeline_gap_adapter.rs`
- Modify: `crates/koushi-core/src/sync.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/src/live_catchup.rs`

- [ ] **Step 1: Add the closed adapter contract**

Define `MatrixCommittedRoomTimelineCheckpoint` with closed constructors from a
RoomListService checkpoint and SDK committed observation. Its custom `Debug`
prints only backend kind, epoch/generation counters, and boolean/count facts.

- [ ] **Step 2: Generalize the classifier**

Replace `LiveCatchupGate::Unsupported` with pending/stale/no-update/no-gap/exact
gap decisions for both backend kinds. Add pure tests showing a missing legacy
checkpoint waits and can never select an unrelated descriptor.

- [ ] **Step 3: Route one backend-epoch-fenced stream**

TimelineManager increments a backend epoch on every sync handoff/replacement,
aborts the previous observation task, clears retained checkpoints, and routes a
checkpoint only when backend epoch, room key, and actor generation match. Fast
checkpoint replay after actor registration remains required.

- [ ] **Step 4: Select only exact checkpoint gaps**

TimelineActor defers only `LiveEdge` inspection while pending. On an exact gap,
use the opaque descriptor to start the bounded repair directly and retain it
until admission so rapid later responses cannot starve it. After a progressing
batch, follow only the newest updated gap until boundaries join. No-update/no-gap
closes without fallback; missing/stale provenance records a closed coarse
decision and releases scheduler ownership.

- [ ] **Step 5: Run adapter/core tests**

```bash
cargo test -p koushi-sdk --test timeline_gap_adapter
cargo test -p koushi-core --lib live_catchup -- --nocapture
cargo test -p koushi-core --lib timeline_actor_waits_for_current_subscription_checkpoint_before_live_edge_repair -- --nocapture
cargo test -p koushi-core --lib legacy_fallback_waits_for_committed_response_and_recovers_missing_interval -- --nocapture
```

Expected: all tests pass and the existing SyncService regression remains
unchanged in behavior.

- [ ] **Step 6: Commit checkpoint routing**

```bash
git add crates/koushi-sdk crates/koushi-core
git commit -m "fix: fence legacy live catch-up to committed sync"
```

### Task 4: Recover relay and render settlement fences

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `docs/architecture/state-machine.md`

- [ ] **Step 1: Add actor-owned settlement deadlines**

Add generation-tagged `GapRelaySettlementDue` and `GapRenderSettlementDue`
messages. Store the originating trigger with each pending correlation/fence.
Schedule one cancellable actor timer per phase using the existing executor
timeout patterns; never poll or sleep in the actor loop.

- [ ] **Step 2: Centralize obsolete-settlement recovery**

Add one helper that clears the matching correlation, pending projection, and/or
render fence; preserves the maximum-priority queued trigger; emits a coarse
failure/recovery action; and requests authoritative resync/re-inspection.
Stale timer messages are ignored by actor/timeline/repair generation guards.

- [ ] **Step 3: Integrate overflow and replacement**

During `handle_relay_overflow`, settle any correlation not found in the
authoritative replacement snapshot through the recovery helper. A timeline
generation change also invalidates an old render fence and requeues its trigger.

- [ ] **Step 4: Run settlement tests**

```bash
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests -- --nocapture
cargo test -p koushi-core --lib relay_overflow -- --nocapture
cargo test -p koushi-core --lib relay_overflow_clears_obsolete_gap_correlation_and_requeues_live_edge -- --nocapture
cargo test -p koushi-core --lib render_ack_timeout_clears_fence_and_requeues_live_edge -- --nocapture
```

Expected: all tests pass; no phase can remain permanently pending after its
correlation becomes impossible.

- [ ] **Step 5: Commit settlement recovery**

```bash
git add crates/koushi-core/src/timeline.rs docs/architecture/state-machine.md
git commit -m "fix: recover stalled timeline gap settlement"
```

### Task 5: Promote legacy lifecycle on first successful response

**Files:**
- Modify: `crates/koushi-core/src/sync.rs`
- Modify: `docs/architecture/state-machine.md`

- [ ] **Step 1: Add a lossless internal success control**

Make the legacy task report `FirstResponseCommitted` to SyncActor through a
dedicated control path. Task launch remains `Starting`; reconnecting after a
previous success remains `Reconnecting`.

- [ ] **Step 2: Move promotion and dependent handoff**

On the first success control only, set actor lifecycle `Running`, notify room
and timeline dependents, then project `SyncLifecycleStatus::Running` and emit
`SyncEvent::Running`. Remove task-launch promotion from direct legacy start and
runtime fallback. Idempotent `Start` before proof re-emits `Started`/mode but not
`Running`.

- [ ] **Step 3: Run lifecycle tests**

```bash
cargo test -p koushi-core --lib legacy_lifecycle -- --nocapture
cargo test -p koushi-state --test session_state sync -- --nocapture
```

Expected: Running is impossible before first success and is emitted once after
proof.

- [ ] **Step 4: Commit lifecycle fix**

```bash
git add crates/koushi-core/src/sync.rs docs/architecture/state-machine.md
git commit -m "fix: prove legacy sync before running"
```

### Task 6: Production-faithful Linux headless regression

**Files:**
- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`
- Modify: `scripts/desktop-headless-local-qa.mjs` only if token routing changes
- Modify: `AGENTS.md`

- [ ] **Step 1: Add `timeline_legacy_fallback` and preserve `timeline_reconnect`**

Use `QaTcpProxy` to advertise SyncService, fail it before connectivity, and let
Core choose legacy automatically. Create more than 128 offline events for one
closed room, release legacy sync, send one later event, reopen, and assert the
entire expected set exactly once. Assert state never reports Running before the
first legacy success and finishes outside relay/render pending phases.

Keep the existing `timeline_reconnect` scenario and its SyncService checkpoint
assertions unchanged. The new scenario must use the probed backend; the existing
forced-legacy lane remains a separate direct backend check and is not evidence
for automatic fallback.

- [ ] **Step 2: Keep output closed and private-safe**

Print only:

```text
legacy_fallback_checkpoint=ok
legacy_fallback_gap_repaired=ok
legacy_fallback_settled=ok
legacy_fallback_lifecycle=ok
```

- [ ] **Step 3: Run forced fallback and existing SyncService lanes**

```bash
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=timeline_legacy_fallback --core --core-backend=probed --timeout-ms=240000
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=timeline_reconnect --core --core-backend=probed --timeout-ms=240000
PATH=/tmp/koushi-desktop-local-qa-bin:$PATH npm --prefix apps/desktop run qa:headless-local -- --server=conduit --scenario=timeline_reconnect --core --core-backend=legacy --timeout-ms=240000
```

Expected: all three lanes exit zero with their required closed tokens.

- [ ] **Step 4: Commit headless coverage**

```bash
git add crates/koushi-core/src/bin/headless-core-qa.rs scripts/desktop-headless-local-qa.mjs AGENTS.md
git commit -m "test: cover legacy fallback timeline continuity"
```

### Task 7: Final verification, review, and publication

**Files:**
- Modify only for verified review findings.

- [ ] **Step 1: Update and verify final SDK pin**

Ensure `Cargo.toml`, `Cargo.lock`, and `vendor/matrix-rust-sdk` reference the
same pushed SDK commit.

- [ ] **Step 2: Run focused repository gates**

```bash
cargo fmt --all -- --check
cargo test -p koushi-sdk --test timeline_gap_adapter
cargo test -p koushi-core --lib live_catchup
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
cargo test -p koushi-core --lib relay_overflow
cargo test -p koushi-core --lib legacy_lifecycle
cargo check -p koushi-core
```

Expected: every command exits zero.

- [ ] **Step 3: Run both fresh Linux headless lanes**

Repeat Task 6 Step 3 with clean QA data directories. Confirm only closed tokens
are emitted.

- [ ] **Step 4: Run the required external diff reviews**

Generate separate SDK and Koushi diffs including root pin/module files and new
files. Feed each to `codex review -` with the repository-rule, architecture,
state-machine, Rust/Tauri, security/privacy, and relevant-plan priorities from
`AGENTS.md`. Verify every finding before changing code.

- [ ] **Step 5: Publish SDK then Koushi PRs**

Push the SDK dependency first and wait for its required checks. Push the Koushi
branch and open a ready-for-review PR with `Fixes #275`, RED→GREEN evidence,
closed-token headless results, and the final SDK commit. Do not close the issue
manually before merge.

- [ ] **Step 6: Preserve macOS validation handoff**

Install the candidate build into the preserved macOS reproduction profile and
verify the historical interval repairs, new events continue, room switching
does not recreate the hole, and diagnostics settle without private content.
This attended validation is final production evidence and is not replaced by a
Linux-only success claim.
