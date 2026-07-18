# Visible Persisted Gap Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reliably repair a persisted gap that is visibly rendered in the selected room, including a nonresident gap and a gap viewed through either thread-root ordering mode.

**Architecture:** Preserve the SDK-owned opaque gap descriptor and Core's causal projection fences. Replace inferred index-only demand with a generation-fenced public gap identity reported directly by the viewport, make the SDK's persisted-chunk reveal budget repair-capable, and scope Core's batch budget to the selected gap/topology rather than the room lifetime.

**Tech Stack:** Rust 2024, matrix-rust-sdk linked chunks, Tokio, Serde, Tauri 2, TypeScript 6, React 19, Vitest.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-18-canonical-timeline-gap-repair-design.md`.
- This plan fixes visible persisted-gap recovery only; it must not implement the separate canonical display-projection/echo migration.
- The selected room and an explicitly visible gap have foreground priority. Inactive rooms remain delayed.
- Pagination tokens, SDK gap handles, room IDs, event IDs, message bodies, and raw errors never enter diagnostics.
- `LiveTailSnapshot` remains observe-only and must not consume a historical continuation token.
- Existing actor/timeline generations and projection acknowledgements remain mandatory stale-work fences.
- Rust keeps `topology_revision` as `u64`; every JSON/Tauri/TypeScript representation uses a canonical decimal string and rejects numeric JSON.
- The 32-outcome safety fence counts only consecutive no-progress outcomes. Cache reveal and pagination progress reset it.
- Attempt reset and budget diagnostics remain token-free and identity-free.
- Use test-driven development: every production change follows a focused failing test.
- Luna implements each task and self-reviews it. A separate reviewer checks the committed range before the next task.

---

## File Structure

- Modify `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/pagination.rs`: calculate and reveal the persisted chunk path to the selected opaque descriptor without classifying zero revealed chunks as offscreen.
- Modify `crates/koushi-sdk/src/lib.rs`: preserve the SDK repair outcome and expose only topology-safe handle metadata to Core.
- Modify `crates/koushi-core/src/event.rs`: add a serializable generation-fenced `TimelineGapId`, attach it to projected positions, and accept visible IDs in viewport observations.
- Modify `crates/koushi-core/src/timeline.rs`: select explicit visible-gap demand, scope batch accounting to a gap ID, and preserve causal repair settlement.
- Modify `apps/desktop/src/domain/coreEvents.ts`: mirror the new wire fields.
- Modify `apps/desktop/src/domain/timelineDisplayProjection.ts`: carry the gap ID into the presentation-only row.
- Modify `apps/desktop/src/domain/timelineStore.ts`: retain the projected gap descriptors without inserting them into the canonical diff target.
- Modify `apps/desktop/src/components/TimelineView.tsx`: report visible gap IDs together with event viewport bounds.
- Modify `apps/desktop/src/App.tsx`, `apps/desktop/src/backend/client.ts`, and `apps/desktop/src-tauri/src/commands/mod.rs`: transport visible gap IDs unchanged.
- Modify focused Rust and TypeScript tests beside the files above.

### Task 1: Make an opaque nonresident descriptor repair-capable

**Files:**

- Modify: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/pagination.rs`
- Test: `vendor/matrix-rust-sdk/crates/matrix-sdk/src/event_cache/caches/room/pagination.rs`
- Modify: `crates/koushi-sdk/src/lib.rs`

**Interfaces:**

- Consumes: `RoomTimelineGapDescriptor.handle.chunk_identifier` and the ordered persisted chunks already loaded by `repair_timeline_gap_inner`.
- Produces: unchanged `RoomTimelineGapRepairResult`; `Deferred { cached_chunks_loaded }` means bounded cache work was performed and remains, never that the gap is offscreen.

- [ ] **Step 1: Extract and test the persisted reveal distance**

Add a private helper beside `repair_timeline_gap_inner`:

```rust
fn cached_chunks_required_for_gap(
    persisted: &[RawChunk<Event, Gap>],
    current_first: ChunkIdentifier,
    target: ChunkIdentifier,
) -> Option<usize> {
    let first = persisted.iter().position(|chunk| chunk.identifier == current_first)?;
    let target = persisted.iter().position(|chunk| chunk.identifier == target)?;
    (target <= first).then_some(first - target)
}
```

Reuse the existing `matrix_sdk_base::event_cache::{Event, Gap}` imports. Store
the result of `order_persisted_chunks` in a local vector so both this helper and
`inspect_ordered_chunks` examine the same authoritative order. Add table tests
for resident distance `0`, one previous chunk, several previous chunks, a
target newer than the current first chunk, and a missing target.

- [ ] **Step 2: Run the focused SDK test and prove RED**

Run:

```bash
cargo test -p matrix-sdk cached_chunks_required_for_gap --lib
```

Expected before the helper is implemented: compile failure or failing distance assertions.

- [ ] **Step 3: Use the descriptor path as authoritative cache work**

In `repair_timeline_gap_inner`, compute the target distance from the already loaded persisted topology. Reject a missing/newer target as `Stale`. Reveal previous chunks until the selected gap is resident or the per-call `cached_chunk_limit` is exhausted. Return `Deferred { cached_chunks_loaded }` only after loading at least one permitted chunk; a zero limit returns Deferred but Core must retain it as pending work rather than translate it to offscreen.

Keep descriptor revalidation both before and after network I/O. Do not expose the chunk identifier or token through `koushi-sdk`.

- [ ] **Step 4: Run SDK checks**

Run:

```bash
cargo test -p matrix-sdk cached_chunks_required_for_gap --lib
cargo test -p koushi-sdk matrix_live_tail_refresh_mapping_tests --lib
```

Expected: PASS.

- [ ] **Step 5: Commit Task 1**

```bash
git -C vendor/matrix-rust-sdk add crates/matrix-sdk/src/event_cache/caches/room/pagination.rs
git -C vendor/matrix-rust-sdk commit -m "fix(event-cache): retain targeted persisted gap work"
git add vendor/matrix-rust-sdk crates/koushi-sdk/src/lib.rs
git commit -m "fix(sdk): preserve nonresident gap repair demand"
```

### Task 2: Give projected gaps a viewport-safe identity

**Files:**

- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/timelineDisplayProjection.ts`
- Test: `crates/koushi-core/src/timeline.rs`
- Test: `apps/desktop/src/domain/timelineDisplayProjection.test.ts`

**Interfaces:**

- Produces:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct TimelineGapId {
    pub topology_revision: u64,
    pub ordinal: u32,
}

pub struct TimelineGapPosition {
    pub id: TimelineGapId,
    pub before_item_index: usize,
}
```

- `TimelineGapId` is actor-private topology identity, not the SDK handle. It contains no token or Matrix identifier.
- TypeScript mirrors the same snake_case wire fields.

- [ ] **Step 1: Write failing Core identity tests**

Replace ordinal-only projected-gap fixtures with `TimelineGapId`. Assert that repeated inspection of the same topology revision and ordinal produces the same ID, while a new topology revision produces a different ID even when the ordinal is reused.

- [ ] **Step 2: Run the focused Core test and prove RED**

Run:

```bash
cargo test -p koushi-core projected_gap_identity --lib
```

Expected: FAIL because `TimelineGapId` and the new `id` field do not exist.

- [ ] **Step 3: Emit the topology identity**

Change `emit_gap_positions` so each `MatrixTimelineGapHandle` becomes:

```rust
TimelineGapPosition {
    id: TimelineGapId {
        topology_revision: gap.topology_revision(),
        ordinal: ordinal.try_into().unwrap_or(u32::MAX),
    },
    before_item_index,
}
```

Update `ProjectedGapCandidate` and tracker fixtures to carry `TimelineGapId` instead of a bare ordinal. Descriptor lookup must validate both topology revision and ordinal against the current inspection before indexing `inspection.gaps`.

- [ ] **Step 4: Carry identity through the presentation row**

Extend `TimelineGapPosition` in TypeScript. Change the synthetic row identity from generation/ordinal to generation/topology/ordinal and add a `gap_id` field to `TimelineDisplayRow`:

```ts
gap_id: TimelineGapId | null;
```

All non-gap row constructors set `gap_id: null`; the gap-row constructor retains the supplied ID.

- [ ] **Step 5: Run focused Core and UI tests**

Run:

```bash
cargo test -p koushi-core projected_gap --lib
npm test -- --run src/domain/timelineDisplayProjection.test.ts
```

from `apps/desktop` for the npm command. Expected: PASS.

- [ ] **Step 6: Commit Task 2**

```bash
git add crates/koushi-core/src/event.rs crates/koushi-core/src/timeline.rs apps/desktop/src/domain/coreEvents.ts apps/desktop/src/domain/timelineDisplayProjection.ts apps/desktop/src/domain/timelineDisplayProjection.test.ts
git commit -m "feat(timeline): identify projected persisted gaps"
```

### Task 3: Report explicit visible-gap demand end to end

**Files:**

- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/backend/client.ts`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Test: `apps/desktop/src/components/TimelineView.test.tsx`
- Test: `apps/desktop/src-tauri/src/commands/mod.rs`
- Test: `crates/koushi-core/src/timeline.rs`

**Interfaces:**

- Extends `TimelineViewportObservation` with `visible_gap_ids: Vec<TimelineGapId>`.
- Extends the desktop transport `observeViewport` with `visibleGapIds: TimelineGapId[]` before `atBottom`.

- [ ] **Step 1: Write the failing DOM viewport test**

Render a timeline with a gap row intersecting the mocked viewport and surrounding event rows outside it. Assert `observeViewport` receives the gap ID even when no event ID is visible. Repeat with `threadRootOrder={{ kind: "latestReply" }}` and a moved root beside the same gap.

- [ ] **Step 2: Run the UI test and prove RED**

Run from `apps/desktop`:

```bash
npm test -- --run src/components/TimelineView.test.tsx -t "reports a visible persisted gap"
```

Expected: FAIL because gap rows have no reportable ID and event-only viewport reporting returns early.

- [ ] **Step 3: Collect and transport visible gap IDs**

Add `data-gap-topology-revision` and `data-gap-ordinal` to the gap row. Replace `visibleEventIds` with a viewport facts function returning event bounds and a deduplicated `TimelineGapId[]`. Permit a report when either an event bound or a gap ID is visible. Include gap IDs in the observation signature so newly visible demand is not suppressed.

Carry the array through `App.tsx`, `client.ts`, the Tauri command builder, and `TimelineViewportObservation` without deriving indices in TypeScript.

- [ ] **Step 4: Select explicit demand in Core**

On `ObserveViewport`, validate each reported ID against `gap_repair.projected_gaps`. Prefer a matching visible ID over inferred event-index intersection. An unknown generation/topology ID is ignored and queues a fresh inspection; it never indexes the current descriptor vector.

- [ ] **Step 5: Run contract and viewport tests**

Run:

```bash
cargo test -p koushi-core visible_gap --lib
cargo test -p koushi-desktop observe_timeline_viewport --lib
npm test -- --run src/components/TimelineView.test.tsx -t "visible persisted gap"
```

Expected: PASS.

- [ ] **Step 6: Commit Task 3**

```bash
git add crates/koushi-core/src/event.rs crates/koushi-core/src/timeline.rs apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx apps/desktop/src/App.tsx apps/desktop/src/backend/client.ts apps/desktop/src-tauri/src/commands/mod.rs
git commit -m "feat(timeline): prioritize explicitly visible gaps"
```

### Task 4: Scope repair budgets to gap topology and demand

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`
- Test: `crates/koushi-core/src/timeline.rs`

**Interfaces:**

- `TimelineGapRepairTracker` records `attempt_gap_id: Option<TimelineGapId>` beside `batches_processed`.
- `admit_gap_attempt(id, demand_revision)` resets the bounded count only when the selected ID or explicit demand revision changes.

- [ ] **Step 1: Write failing tracker tests**

Cover all four cases: repeated observation of the same ID does not reset an active attempt; a new topology revision resets; a different ordinal resets; and the same visible ID after room re-selection/new demand revision resets.

- [ ] **Step 2: Prove RED**

Run:

```bash
cargo test -p koushi-core gap_repair_budget_is_scoped --lib
```

Expected: FAIL because the current count is room-lifetime state.

- [ ] **Step 3: Implement per-attempt accounting**

Replace the room-lifetime fence with selected-ID accounting. Automatic visible repair uses a positive cache reveal budget. Keep `MAX_TIMELINE_GAP_REPAIR_BATCHES` as a per-ID network/settlement safety bound, not as permanent room state. `Deferred { 0 }` remains queued/cache-pending and is not mapped by `automatic_gap_repair_is_offscreen`.

Do not reset merely because a repeated SDK callback arrives; resets require a changed stable ID or explicit new demand revision.

- [ ] **Step 4: Run the Core repair suite**

Run:

```bash
cargo test -p koushi-core gap_repair --lib
cargo test -p koushi-core live_tail --lib
```

Expected: PASS with former tests that asserted `cached_chunk_limit=0` updated to assert repair-capable visible demand.

- [ ] **Step 5: Commit Task 4**

```bash
git add crates/koushi-core/src/timeline.rs
git commit -m "fix(core): scope gap repair budget to visible demand"
```

### Task 4A: Preserve full-range topology identity across the desktop wire

**Files:**

- Modify: `crates/koushi-core/src/event.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Modify: `apps/desktop/src/domain/coreEvents.generated.json`
- Modify: `apps/desktop/src/domain/timelineDisplayProjection.ts`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Test: `apps/desktop/src/domain/timelineDisplayProjection.test.ts`
- Test: `apps/desktop/src/components/TimelineView.test.tsx`
- Test: `apps/desktop/src-tauri/src/commands/mod.rs`

**Interfaces:**

- Rust retains `TimelineGapId { topology_revision: u64, ordinal: u32 }`.
- Serde encodes and accepts `topology_revision` only as a canonical decimal string.
- TypeScript consumes and produces:

```ts
export type TimelineGapId = {
  topology_revision: string;
  ordinal: number;
};
```

- [ ] **Step 1: Write the failing full-range round-trip tests**

Use `14695981039346656037`, the FNV offset basis already used by the SDK and
greater than `Number.MAX_SAFE_INTEGER`. Assert the checked-in wire artifact
contains the string, the projected row preserves it, the DOM viewport reports
it unchanged in both thread modes, and the Tauri command parses it back to the
same Rust `u64`. Add a negative Rust test that rejects:

```json
{"topology_revision":14695981039346656037,"ordinal":0}
```

- [ ] **Step 2: Prove RED**

Run:

```bash
cargo test -p koushi-core timeline_gap_id_wire --lib
cargo test -p koushi-desktop observe_timeline_viewport --lib
npm test -- --run src/domain/timelineDisplayProjection.test.ts src/components/TimelineView.test.tsx -t "full-range topology revision"
```

Expected: FAIL because Serde emits a JSON number and TypeScript rejects or
rounds the full-range value.

- [ ] **Step 3: Implement canonical decimal-string Serde**

Add a private Serde adapter in `event.rs` and annotate the Rust field:

```rust
#[serde(with = "u64_decimal_string")]
pub topology_revision: u64,
```

The deserializer first reads `String`, parses `u64`, and requires
`parsed.to_string() == encoded`; this rejects numbers, signs, whitespace, and
leading zeroes. Change the TypeScript type to `string`. Keep the DOM dataset
value as a string, validate it with a canonical unsigned-decimal predicate,
and deduplicate/sign viewport observations with the exact string. Never call
`Number`, `parseInt`, or `BigInt` in the UI path.

- [ ] **Step 4: Regenerate the wire artifact and run the focused gates**

Run:

```bash
cargo test -p koushi-core timeline_gap_id_wire --lib
cargo test -p koushi-desktop observe_timeline_viewport --lib
npm test -- --run src/domain/timelineDisplayProjection.test.ts src/components/TimelineView.test.tsx
npm run typecheck
```

Expected: PASS, including a lossless `>2^53` round trip and numeric rejection.

- [ ] **Step 5: Commit Task 4A**

```bash
git add crates/koushi-core/src/event.rs crates/koushi-core/src/timeline.rs apps/desktop/src/domain/coreEvents.ts apps/desktop/src/domain/coreEvents.generated.json apps/desktop/src/domain/timelineDisplayProjection.ts apps/desktop/src/domain/timelineDisplayProjection.test.ts apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx apps/desktop/src/App.tsx apps/desktop/src-tauri/src/commands/mod.rs apps/desktop/src-tauri/src/lib.rs
git commit -m "fix(timeline): preserve full-range gap identity"
```

### Task 4B: Fence consecutive no-progress outcomes rather than progress

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`
- Test: `crates/koushi-core/src/timeline.rs`

**Interfaces:**

- `batches_processed` remains a diagnostic total.
- `consecutive_no_progress_batches` controls the 32-outcome safety fence.
- `cached_chunks_loaded > 0` and positive pagination progress reset the fence.

- [ ] **Step 1: Write the failing progress-accounting tests**

Drive the same admitted `TimelineGapId` and demand revision through 33
`Deferred { cached_chunks_loaded: 1 }` outcomes. Assert every next batch remains
admissible and `consecutive_no_progress_batches == 0`. Separately drive 32
`Deferred { cached_chunks_loaded: 0 }` outcomes and assert the 33rd batch is
rejected without changing ID or demand revision.

- [ ] **Step 2: Prove RED**

Run:

```bash
cargo test -p koushi-core gap_repair_progress_budget --lib
```

Expected: FAIL because the current `batches_processed` counter fences the 33rd
progressing cache reveal.

- [ ] **Step 3: Implement outcome-aware accounting**

Track total and no-progress counts separately:

```rust
struct TimelineGapRepairTracker {
    batches_processed: u32,
    consecutive_no_progress_batches: u32,
    // existing attempt identity and demand fields remain
}
```

Admission checks only `consecutive_no_progress_batches`. Increment the total
when a batch starts. After the SDK outcome, reset the no-progress counter for
`Deferred { cached_chunks_loaded }` when loaded is positive, positive
`Progress { events }`, and completed boundary outcomes; increment it for true
no-progress outcomes. Keep the existing per-request event/cache limits and
causal settlement fences.

- [ ] **Step 4: Run repair and live-tail suites**

Run:

```bash
cargo test -p koushi-core gap_repair_progress_budget --lib
cargo test -p koushi-core gap_repair --lib
cargo test -p koushi-core live_tail --lib
```

Expected: PASS; a distant retained descriptor cannot be stranded while each
batch reveals a cached chunk.

- [ ] **Step 5: Commit Task 4B**

```bash
git add crates/koushi-core/src/timeline.rs
git commit -m "fix(core): budget consecutive gap repair stalls"
```

### Task 4C: Record privacy-safe attempt and budget transitions

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`
- Test: `crates/koushi-core/src/timeline.rs`

**Interfaces:**

- `admit_gap_attempt` produces a structured admission containing an attempt
  number, reset reason, and change booleans.
- Diagnostics contain no room ID, event ID, descriptor, token, message, or raw
  error.

- [ ] **Step 1: Write failing diagnostic contract tests**

Assert initial admission, topology change, ordinal change, and explicit demand
change produce the tokens `initial`, `topology`, `ordinal`, and `demand` plus:

```text
attempt_number
topology_changed
ordinal_changed
demand_changed
consecutive_no_progress_batches
budget_remaining
cached_chunks_loaded
```

Assert the diagnostic source contract excludes identity/token field names.

- [ ] **Step 2: Prove RED**

Run:

```bash
cargo test -p koushi-core gap_repair_attempt_diagnostics --lib
```

Expected: FAIL because reset and progress accounting are currently silent.

- [ ] **Step 3: Emit structured transitions**

Return an admission value from `admit_gap_attempt`:

```rust
struct TimelineGapAttemptAdmission {
    attempt_number: u64,
    reason: TimelineGapAttemptResetReason,
    topology_changed: bool,
    ordinal_changed: bool,
    demand_changed: bool,
}
```

Emit one attempt diagnostic when admission changes and one budget diagnostic
after each outcome. Use only the enum token, change booleans, attempt number,
demand revision, and budget counters. Do not log the gap ordinal, topology
revision value, Matrix identifiers, or the SDK descriptor.

- [ ] **Step 4: Run diagnostics and repair suites**

Run:

```bash
cargo test -p koushi-core gap_repair_attempt_diagnostics --lib
cargo test -p koushi-core gap_repair --lib
```

Expected: PASS and source-contract privacy assertions remain green.

- [ ] **Step 5: Commit Task 4C**

```bash
git add crates/koushi-core/src/timeline.rs
git commit -m "feat(diagnostics): trace gap repair attempts"
```

### Task 4D: Prove room-switch cancellation and thread-order identity

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`

**Interfaces:**

- Room switch may retain SDK-committed cache state, but an old actor generation
  cannot publish repair progress or resume work.
- `rootEvent` and `latestReply` report the same gap ID even when the root moves
  across the gap.

- [ ] **Step 1: Write the failing cancellation and crossing tests**

Pause an old-room repair after SDK work starts, switch rooms, release the old
completion, and assert no old-generation reducer action or continuation is
published. Render a thread whose latest reply moves its root across the gap;
assert the gap row retains the same full-range ID and is reported visible in
both thread modes.

- [ ] **Step 2: Prove RED or existing contract**

Run:

```bash
cargo test -p koushi-core gap_repair_room_switch_cancels_completion --lib
npm test -- --run src/components/TimelineView.test.tsx -t "gap identity across thread ordering"
```

Expected: the tests either expose missing cancellation/ordering behavior or
prove the existing generation fence without requiring production changes. A
test that passes immediately must be mutation-checked by temporarily removing
the relevant generation or gap-ID assertion before it is accepted.

- [ ] **Step 3: Apply only evidence-backed production fixes**

If RED, use the existing actor-generation and subscription cancellation fence;
do not introduce a second room scheduler. Keep the gap row anchored by its
stable ID rather than a projected event index.

- [ ] **Step 4: Run focused suites and commit**

Run:

```bash
cargo test -p koushi-core gap_repair_room_switch --lib
npm test -- --run src/components/TimelineView.test.tsx
```

Expected: PASS.

```bash
git add crates/koushi-core/src/timeline.rs apps/desktop/src/components/TimelineView.test.tsx
git commit -m "test(timeline): fence visible gap demand across room and thread changes"
```

### Task 5: Add the production-shape vertical regression and verify

**Files:**

- Modify: `crates/koushi-core/src/bin/headless-core-qa.rs`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `docs/superpowers/specs/2026-07-18-canonical-timeline-gap-repair-design.md`

**Interfaces:**

- Consumes the public contracts completed in Tasks 1-4D.
- Extends the existing real-store `timeline_legacy_persisted_gap` scenario so
  its detached persisted gap is repaired through an explicit visible gap ID,
  the public viewport command, render acknowledgements, and
  `GapRepairReleased` instead of ordinary pagination.
- Keeps tier ownership explicit: deterministic Core actor tests own the
  >32-positive-progress fence, full-range wire identity, diagnostics
  cardinality, and room/thread cancellation; the TimelineView test owns
  `rootEvent`/`latestReply`, live-history movement, viewport re-reporting, and
  room-key fencing.

- [ ] **Step 1: Write the failing vertical regressions**

In `timeline_legacy_persisted_gap`, retain the existing real login, persisted
store, cold restart, room-absent fallback, and detached gap fixture. Capture
the projected gap ID, report it through `ObserveViewport`, and require the
existing render-acknowledgement helper to observe the historical continuation
exactly once together with `GapRepairReleased` and the expected gap-count
decrement.

In TimelineView, use one continuous render/rerender flow to append a live
event, move the same gap into history, report the same ID again, cross the same
root/gap DOM nodes between `rootEvent` and `latestReply`, close the descriptor,
and fence the old room key after switching rooms.

- [ ] **Step 2: Prove the regressions are behavior-sensitive**

Run:

```bash
cargo check -p koushi-core --features qa-bin --bin headless-core-qa
npm test -- --run src/components/TimelineView.test.tsx -t "persisted gap recovery"
```

Expected mutation evidence: replacing the explicit gap observation with
ordinary pagination fails the Core release assertion; keeping `rootEvent`
during the crossing fails the DOM-order assertion.

- [ ] **Step 3: Run all focused suites**

Run:

```bash
cargo test -p koushi-sdk --lib
cargo test -p koushi-core --lib
cargo test -p koushi-desktop --lib
npm test -- --run src/domain/timelineStore.test.ts src/domain/timelineDisplayProjection.test.ts src/components/TimelineView.test.tsx
npm run typecheck
```

Expected: PASS.

- [ ] **Step 4: Run local end-to-end QA**

Run from `apps/desktop`:

```bash
npm run qa:headless-local -- --server=conduit --scenario=timeline_legacy_persisted_gap --core --core-backend=probed --timeout-ms=240000
```

Expected: the real persisted detached gap is selected by its projected ID,
repaired through the public viewport/render-ack path, emits one release, and
projects its continuation exactly once. Thread-mode and cancellation evidence
comes from the focused UI and actor suites above, not from the homeserver tier.

- [ ] **Step 5: Commit Task 5**

```bash
git add crates/koushi-core/src/bin/headless-core-qa.rs apps/desktop/src/components/TimelineView.test.tsx docs/superpowers/specs/2026-07-18-canonical-timeline-gap-repair-design.md docs/superpowers/plans/2026-07-18-visible-gap-recovery.md
git commit -m "test(timeline): cover visible persisted gap recovery"
```

## Completion Gate

Do not claim the issue closed until fresh output proves all Task 5 commands
pass and a real persisted-account diagnostic records a successful repair
without a repeated `Deferred { 0 } -> offscreen` loop. The automated phase is
ready for the final Mac check only after the actor, UI, and real-store
homeserver tiers above are independently green. The separate canonical
display-projection/echo plan begins only after this gap plan is independently
green.
