# Room Entry Gap Repair Pacing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep non-destructive automatic timeline repair while preventing room entry from materializing off-screen history or starting another repair batch before the desktop has rendered the previous projection.

**Architecture:** Core separates persisted-gap inspection from active-window projection, delays the first inspection until the Room `InitialItems` projection is acknowledged, and applies automatic/manual cache budgets. A generation-fenced render acknowledgement crosses the existing Core → Tauri → React boundary; Core admits the next repair only after the matching repair generation and timeline batch settle. TimelineView also uses its height model and projection transaction state to suppress false underfilled pagination during virtualization turnover.

**Tech Stack:** Rust/Tokio actors, matrix-rust-sdk event cache, Tauri 2 commands, TypeScript, React 19, Vitest/jsdom.

## Global Constraints

- Preserve the SDK-owned, token-free, non-destructive repair contract; no normal path may call `EventCacheStore::remove_room`.
- Automatic repair uses `event_limit: 64` and `cached_chunk_limit: 0`.
- Manual repair uses `event_limit: 64` and `cached_chunk_limit: 1`.
- An unprojected persisted gap produces no timeline row and no automatic repair.
- Core admits at most one unrendered repair projection per Room timeline actor.
- Render acknowledgements are fenced by timeline key, actor generation, timeline generation, room-local repair generation, and minimum timeline batch ID.
- Automatic pagination must wait until projection layout and anchor restoration settle.
- Diagnostics contain only coarse tokens, counts, generations, and batch IDs; never Matrix room/user/event IDs, display names, message content, tokens, or raw errors.
- Existing Rooms/People filters, Space-scoped DM membership, and room-label precedence remain unchanged.

---

## File Map

- `crates/koushi-core/src/timeline.rs`: gap projection, trigger policy, scheduler state, actor ordering, repair/render fences, and focused unit tests.
- `crates/koushi-core/src/command.rs`: typed render-acknowledgement command and request-ID/debug contracts.
- `crates/koushi-core/src/account.rs`: reliable routing from the account actor to the timeline manager.
- `crates/koushi-core/src/runtime.rs`: app-command admission/routing for render acknowledgements.
- `crates/koushi-core/tests/runtime_room_selection_scale.rs`: large-account command-pressure regression.
- `apps/desktop/src-tauri/src/commands/navigation.rs`: Tauri command endpoint.
- `apps/desktop/src-tauri/src/commands/mod.rs`: command builder and headless contract tests.
- `apps/desktop/src-tauri/src/lib.rs`: Tauri invoke registration.
- `apps/desktop/src/backend/browserFakeApi.ts`: `DesktopApi` contract and synchronous browser no-op.
- `apps/desktop/src/backend/client.ts`: native invoke implementation.
- `apps/desktop/src/backend/client.test.ts`: exact IPC argument test.
- `apps/desktop/src/App.tsx`: Room projection and rendered-batch transport methods.
- `apps/desktop/src/components/TimelineView.tsx`: post-layout acknowledgements and underfilled-backfill guard.
- `apps/desktop/src/components/TimelineView.test.tsx`: render-order, virtualization, and backfill regressions.
- `docs/architecture/state-machine.md`: actor-private awaiting-projection transition.
- `docs/policies/engineering-rules.md`: repair-specific application of the pagination/render-settlement rule.

---

### Task 1: Make Gap Projection And Repair Scheduling Bounded

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs:5062-5148`
- Modify: `crates/koushi-core/src/timeline.rs:5957-6580`
- Test: `crates/koushi-core/src/timeline.rs` module `timeline_gap_repair_tracker_tests`

**Interfaces:**
- Produces: `TimelineGapRepairTrigger::{Automatic, Manual}`.
- Produces: `timeline_gap_repair_budget(trigger) -> MatrixTimelineGapRepairBudget`.
- Produces: `projected_gap_insertion_index(newer_position, older_position) -> Option<usize>`.
- Produces: `TimelineGapRenderFence { actor_generation, timeline_generation, repair_generation, minimum_batch_id }`.
- Produces: tracker methods `queue_inspection`, `begin_pending_inspection`, `await_projection`, and `acknowledge_projection` used by Task 2.

- [ ] **Step 1: Write failing policy and scheduler tests**

Extend `timeline_gap_repair_tracker_tests` with these behaviors before changing production code:

```rust
#[test]
fn unlocated_gap_has_no_projection_position() {
    assert_eq!(projected_gap_insertion_index(None, None), None);
    assert_eq!(projected_gap_insertion_index(Some(7), None), Some(7));
    assert_eq!(projected_gap_insertion_index(None, Some(7)), Some(8));
}

#[test]
fn automatic_and_manual_repair_use_separate_cache_budgets() {
    assert_eq!(
        timeline_gap_repair_budget(TimelineGapRepairTrigger::Automatic),
        MatrixTimelineGapRepairBudget { event_limit: 64, cached_chunk_limit: 0 }
    );
    assert_eq!(
        timeline_gap_repair_budget(TimelineGapRepairTrigger::Manual),
        MatrixTimelineGapRepairBudget { event_limit: 64, cached_chunk_limit: 1 }
    );
}

#[test]
fn subscription_inspection_waits_for_initial_projection_ack() {
    let mut tracker = TimelineGapRepairTracker::default();
    tracker.queue_inspection(TimelineGapRepairTrigger::Automatic);
    assert_eq!(tracker.begin_pending_inspection(false), None);
    assert!(tracker.has_pending_inspection());
    assert!(matches!(
        tracker.begin_pending_inspection(true),
        Some((_, TimelineGapRepairTrigger::Automatic))
    ));
}

#[test]
fn repair_continuation_requires_the_matching_render_fence() {
    let mut tracker = TimelineGapRepairTracker::default();
    let fence = TimelineGapRenderFence {
        actor_generation: 9,
        timeline_generation: TimelineGeneration(3),
        repair_generation: 11,
        minimum_batch_id: TimelineBatchId(5),
    };
    tracker.await_projection(fence);

    assert!(!tracker.acknowledge_projection(TimelineGapRenderFence {
        repair_generation: 10,
        ..fence
    }));
    assert!(!tracker.acknowledge_projection(TimelineGapRenderFence {
        minimum_batch_id: TimelineBatchId(4),
        ..fence
    }));
    assert!(tracker.acknowledge_projection(TimelineGapRenderFence {
        minimum_batch_id: TimelineBatchId(6),
        ..fence
    }));
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
```

Expected: compilation fails because `TimelineGapRepairTrigger`, `TimelineGapRenderFence`, `projected_gap_insertion_index`, and the new tracker methods do not exist.

- [ ] **Step 3: Add the minimal policy and tracker implementation**

Replace the single shared budget and extend the tracker with the following shapes:

```rust
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TimelineGapRepairTrigger {
    Automatic,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TimelineGapRenderFence {
    actor_generation: u64,
    timeline_generation: TimelineGeneration,
    repair_generation: u64,
    minimum_batch_id: TimelineBatchId,
}

fn timeline_gap_repair_budget(
    trigger: TimelineGapRepairTrigger,
) -> MatrixTimelineGapRepairBudget {
    MatrixTimelineGapRepairBudget {
        event_limit: 64,
        cached_chunk_limit: match trigger {
            TimelineGapRepairTrigger::Automatic => 0,
            TimelineGapRepairTrigger::Manual => 1,
        },
    }
}

fn projected_gap_insertion_index(
    newer_position: Option<usize>,
    older_position: Option<usize>,
) -> Option<usize> {
    newer_position.or_else(|| older_position.map(|index| index.saturating_add(1)))
}
```

Add `pending_trigger: Option<TimelineGapRepairTrigger>` and
`awaiting_projection: Option<TimelineGapRenderFence>` to
`TimelineGapRepairTracker`. Manual wins when duplicate triggers coalesce:

```rust
fn queue_inspection(&mut self, trigger: TimelineGapRepairTrigger) {
    self.pending_trigger = Some(self.pending_trigger.map_or(trigger, |pending| pending.max(trigger)));
}

fn begin_pending_inspection(
    &mut self,
    projection_acknowledged: bool,
) -> Option<(u64, TimelineGapRepairTrigger)> {
    if !projection_acknowledged || self.active_serial.is_some() || self.awaiting_projection.is_some() {
        return None;
    }
    let trigger = self.pending_trigger.take()?;
    let serial = self.begin_work()?;
    Some((serial, trigger))
}

fn await_projection(&mut self, fence: TimelineGapRenderFence) {
    self.awaiting_projection = Some(fence);
}

fn acknowledge_projection(&mut self, actual: TimelineGapRenderFence) -> bool {
    let Some(required) = self.awaiting_projection else { return false };
    if actual.actor_generation != required.actor_generation
        || actual.timeline_generation != required.timeline_generation
        || actual.repair_generation != required.repair_generation
        || actual.minimum_batch_id < required.minimum_batch_id
    {
        return false;
    }
    self.awaiting_projection = None;
    true
}
```

- [ ] **Step 4: Integrate projection eligibility and actor ordering**

Make these concrete changes in `TimelineActor`:

1. Remove the awaited `request_timeline_gap_inspection()` before the actor's select loop. Queue `Automatic` synchronously instead.
2. On the first accepted `AcknowledgeProjection`, start the pending inspection. Duplicate acknowledgements remain idempotent and do not create duplicate work.
3. Change `request_timeline_gap_inspection` to accept a trigger and retain it when another inspection, repair, or render settlement is active.
4. Make `emit_gap_positions` return the indices of projectable descriptors and use `filter_map`; remove `.unwrap_or(0)`.
5. For `Automatic`, select only a projectable descriptor. When none exists, keep continuity `Incomplete`, record `offscreen`, and stop.
6. For `Manual`, prefer the visible/projected descriptor, then the persisted descriptor nearest the live edge.
7. Pass `timeline_gap_repair_budget(trigger)` to the SDK.

The projection core must have this form:

```rust
let positions = gaps
    .iter()
    .enumerate()
    .filter_map(|(ordinal, gap)| {
        let newer = gap
            .newer_boundary_event_id()
            .and_then(|event_id| self.timeline_event_position(event_id));
        let older = gap
            .older_boundary_event_id()
            .and_then(|event_id| self.timeline_event_position(event_id));
        projected_gap_insertion_index(newer, older).map(|before_item_index| {
            (ordinal, TimelineGapPosition {
                ordinal: ordinal.try_into().unwrap_or(u32::MAX),
                before_item_index,
            })
        })
    })
    .collect::<Vec<_>>();
```

- [ ] **Step 5: Fence successful repair continuation**

At repair start, retain `minimum_batch_id = self.next_batch_id`. This field is
already the ID of the next batch that will be emitted, not the last emitted ID.
Treat these outcomes as requiring that rendered batch:

```rust
fn repair_outcome_expects_timeline_diff(outcome: &MatrixTimelineGapRepairOutcome) -> bool {
    match outcome {
        MatrixTimelineGapRepairOutcome::Deferred { cached_chunks_loaded } => *cached_chunks_loaded > 0,
        MatrixTimelineGapRepairOutcome::Progress { events }
        | MatrixTimelineGapRepairOutcome::BoundariesJoined { events }
        | MatrixTimelineGapRepairOutcome::StartReached { events } => *events > 0,
        MatrixTimelineGapRepairOutcome::Stale | MatrixTimelineGapRepairOutcome::Failed => false,
    }
}
```

For a successful diff-producing outcome, set `AwaitingProjection` before emitting
`TimelineGapRepairProgressed`; require `minimum_batch_id` (or a newer rendered
batch). Do not add one: doing so would wait forever when the repair publishes
exactly one diff batch.
For a successful no-diff outcome, re-inspect immediately. A relay diff arriving
after the SDK completion cannot unlock the scheduler until the desktop reports
the newer batch.

- [ ] **Step 6: Run focused Core tests and verify GREEN**

Run:

```bash
cargo test -p koushi-core --lib timeline_gap_repair_tracker_tests
```

Expected: all gap tracker/policy tests pass.

- [ ] **Step 7: Commit the bounded Core policy**

```bash
git add crates/koushi-core/src/timeline.rs
git commit -m "fix: bound automatic room gap repair"
```

---

### Task 2: Route A Generation-Fenced Render Acknowledgement

**Files:**
- Modify: `crates/koushi-core/src/command.rs`
- Modify: `crates/koushi-core/src/runtime.rs`
- Modify: `crates/koushi-core/src/account.rs`
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/tests/runtime_room_selection_scale.rs`
- Modify: `apps/desktop/src-tauri/src/commands/navigation.rs`
- Modify: `apps/desktop/src-tauri/src/commands/mod.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/backend/browserFakeApi.ts`
- Modify: `apps/desktop/src/backend/client.ts`
- Test: `apps/desktop/src/backend/client.test.ts`

**Interfaces:**
- Produces: `AppCommand::AcknowledgeTimelineBatchRendered` with `request_id`, `key`, `actor_generation`, `timeline_generation`, `repair_generation`, and `batch_id`.
- Produces: `DesktopApi.acknowledgeTimelineBatchRendered(...) -> Promise<void>`.
- Produces: Tauri command `acknowledge_timeline_batch_rendered` used by Task 3.

- [ ] **Step 1: Write failing Core/Tauri/client contract tests**

Add a Core request-ID/debug test for the new `AppCommand`, a Tauri builder test,
and this client IPC assertion:

```ts
test("acknowledges a rendered repair batch with every generation fence", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const api = createDesktopApi();
  await api.acknowledgeTimelineBatchRendered(
    { account_key: "account", kind: { Room: { room_id: "!room:example.invalid" } } },
    9,
    3,
    11,
    5
  );

  expect(invoke).toHaveBeenCalledWith("acknowledge_timeline_batch_rendered", {
    key: { account_key: "account", kind: { Room: { room_id: "!room:example.invalid" } } },
    actorGeneration: 9,
    timelineGeneration: 3,
    repairGeneration: 11,
    batchId: 5
  });
});
```

- [ ] **Step 2: Verify the contract tests are RED**

Run:

```bash
cargo test -p koushi-core --lib acknowledge_timeline_batch_rendered
cargo test -p koushi-desktop acknowledge_timeline_batch_rendered
npm test -- --run src/backend/client.test.ts
```

Expected: Rust compilation and TypeScript type checking fail because the
command, Tauri endpoint, and API method do not exist.

- [ ] **Step 3: Add the typed command through Core**

Add this variant beside `AcknowledgeTimelineProjection`:

```rust
AcknowledgeTimelineBatchRendered {
    request_id: RequestId,
    key: TimelineKey,
    actor_generation: u64,
    timeline_generation: TimelineGeneration,
    repair_generation: u64,
    batch_id: TimelineBatchId,
},
```

Carry the same fields through `runtime.rs`, `AccountMessage`, `TimelineMessage`,
and `TimelineActorMessage`. Route with reliable `send().await`; this is a
repair-state transition, not a lossy diagnostic hint. The actor builds an
`actual: TimelineGapRenderFence`, calls `tracker.acknowledge_projection(actual)`,
records either `render_acknowledged` or `stale_render_ack`, and starts the
retained pending inspection only after acceptance.

The command's `Debug` output may show generations and batch IDs but must render
the timeline key through the existing redacted key formatter.

- [ ] **Step 4: Add the Tauri and TypeScript API surface**

Add a Tauri command parallel to `acknowledge_timeline_projection`:

```rust
#[tauri::command]
pub async fn acknowledge_timeline_batch_rendered(
    key: TimelineKey,
    actor_generation: u64,
    timeline_generation: TimelineGeneration,
    repair_generation: u64,
    batch_id: TimelineBatchId,
    state: State<'_, CoreRuntimeState>,
) -> Result<(), String> {
    let request_id = next_request_id(state.inner()).await;
    submit_core_command(
        state.inner(),
        CoreCommand::App(AppCommand::AcknowledgeTimelineBatchRendered {
            request_id,
            key,
            actor_generation,
            timeline_generation,
            repair_generation,
            batch_id,
        }),
    )
    .await
}
```

Register it in `generate_handler!`, add the corresponding `DesktopApi` method,
implement the exact `invoke<void>` call from Step 1, and make BrowserFakeApi a
documented no-op because it has no actor lease.

- [ ] **Step 5: Add the large-account command-pressure regression**

Extend `runtime_room_selection_scale.rs` with a test that starts the existing
110-room/57-DM fixture, interleaves 40 full `RoomListUpdated` projections with
synthetic `AcknowledgeTimelineBatchRendered` commands, selects a deep room, and
then publishes a room summary whose `display_label` is `"Resolved target"`.
Assert both of these facts from the final snapshot:

```rust
assert_eq!(
    landed.navigation.active_room_id.as_deref(),
    Some(target.as_str())
);
assert_eq!(
    landed.rooms.iter().find(|room| room.room_id == target)
        .map(|room| room.display_label.as_str()),
    Some("Resolved target")
);
```

Use synthetic `example.test` identifiers and the current Ready account key.
The acknowledgement target may be absent from the local no-SDK timeline
manager; the contract under test is that reliable acknowledgement traffic does
not block or overwrite room-list/user-intent projections.

- [ ] **Step 6: Run the vertical contract tests and verify GREEN**

Run:

```bash
cargo test -p koushi-core --lib acknowledge_timeline_batch_rendered
cargo test -p koushi-core --test runtime_room_selection_scale
cargo test -p koushi-desktop acknowledge_timeline_batch_rendered
npm test -- --run src/backend/client.test.ts
```

Expected: all focused Rust and TypeScript tests pass.

- [ ] **Step 7: Commit the acknowledgement route**

```bash
git add crates/koushi-core/src/command.rs crates/koushi-core/src/runtime.rs crates/koushi-core/src/account.rs crates/koushi-core/src/timeline.rs crates/koushi-core/tests/runtime_room_selection_scale.rs apps/desktop/src-tauri/src/commands/navigation.rs apps/desktop/src-tauri/src/commands/mod.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src/backend/browserFakeApi.ts apps/desktop/src/backend/client.ts apps/desktop/src/backend/client.test.ts
git commit -m "feat: acknowledge rendered timeline repair batches"
```

---

### Task 3: Settle Room Projections In TimelineView Before Continuing Repair

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Test: `apps/desktop/src/components/TimelineView.test.tsx`

**Interfaces:**
- Consumes: `DesktopApi.acknowledgeTimelineProjection` and `DesktopApi.acknowledgeTimelineBatchRendered` from Task 2.
- Produces: optional `TimelineTransport.acknowledgeProjection(...)` and `TimelineTransport.acknowledgeRenderedBatch(...)` methods.
- Produces: one post-layout Room initial-projection ACK and one post-layout repair-generation ACK per settled signature.

- [ ] **Step 1: Write a failing Room initial-projection acknowledgement test**

Use the existing `baseTransport` and controlled animation-frame helper. Emit a
Room `InitialItems` event with a non-null request ID, flush the scheduled frame,
and assert:

```ts
expect(acknowledgeProjection).toHaveBeenCalledWith(
  { connection_id: 4, sequence: 8 },
  KEY,
  1
);
expect(acknowledgeProjection).toHaveBeenCalledTimes(1);
```

Before the frame is flushed, assert the spy has not been called. This keeps the
Room ACK at the render boundary; App's existing immediate ACK remains limited
to Focused and Thread projections.

- [ ] **Step 2: Write a failing rendered-repair acknowledgement test**

Emit `InitialItems`, then an `ItemsUpdated` batch with `actor_generation: 9`,
`generation: 3`, and `batch_id: 5`. Rerender with continuity
`{ kind: "repairing", generation: 11, gap_count: 1, batches_processed: 1 }`.
After flushing the projection/anchor frame, assert:

```ts
expect(acknowledgeRenderedBatch).toHaveBeenLastCalledWith(KEY, 9, 3, 11, 5);
```

Rerendering the same state must not send a duplicate. Changing only
`batches_processed` or `lastAppliedBatchId` must produce a new signature.

- [ ] **Step 3: Write the 3,234-item underfilled regression test**

Model the diagnostic failure exactly: set DOM `scrollHeight` and `clientHeight`
to 367, emit 3,234 canonical items so the height model predicts overflow, and
assert:

```ts
expect(screen.getByTestId("timeline-view")).toHaveAttribute("data-virtualized", "true");
expect(paginateBackwards).not.toHaveBeenCalled();
expect(
  onDiagnosticLogEntry.mock.calls
    .map(([entry]) => entry.message)
    .filter((message) => message.includes("trigger=underfilled_initial"))
).toEqual([]);
```

Keep the existing one-item underfilled test unchanged; it must still request
one page after settlement.

- [ ] **Step 4: Run TimelineView tests and verify RED**

Run:

```bash
npm test -- --run src/components/TimelineView.test.tsx
```

Expected: the new ACK spies remain untouched and the 3,234-item case incorrectly calls `paginateBackwards`.

- [ ] **Step 5: Implement post-layout acknowledgements**

Add these optional methods to `TimelineTransport` and implement them in
`appTimelineTransport` by calling the Task 2 API methods:

```ts
acknowledgeProjection?(
  projectionRequestId: RequestId,
  key: TimelineKey,
  generation: number
): Promise<void>;
acknowledgeRenderedBatch?(
  key: TimelineKey,
  actorGeneration: number,
  timelineGeneration: number,
  repairGeneration: number,
  batchId: number
): Promise<void>;
```

In TimelineView, store last-sent signatures in refs. Register the acknowledgement
frame after the existing projection-compensation layout effect so live-edge or
anchor restoration runs first. The repair signature is:

```ts
const repairAckSignature = [
  timelineKeyHash,
  timelineKeyState?.actorGeneration ?? 0,
  generation,
  continuity.kind === "repairing" ? continuity.generation : -1,
  continuity.kind === "repairing" ? continuity.batches_processed : -1,
  timelineKeyState?.lastAppliedBatchId ?? -1
].join("\u0000");
```

Only Room timelines send the initial ACK from TimelineView. Only a `repairing`
state with `batches_processed > 0` sends the rendered-batch ACK.

- [ ] **Step 6: Guard underfilled pagination with projection state and the height model**

Before consulting transient DOM overflow, return when the height model already
proves overflow:

```ts
const projectedOverflowPx = Math.max(0, timelineHeightModel.totalHeight - clientHeight);
if (projectedOverflowPx > SCROLL_EDGE_TOLERANCE_PX) {
  return;
}
```

Add `projection_settle` to `skipReasons` when
`pendingProjectionLayoutRef.current`, `projectionLayoutFrameRef.current`, Room
anchor restoration, or the current virtual range revision is unsettled. Include
the height-model total and settlement revision in the layout effect dependency
list so a genuinely short timeline re-evaluates after commit.

- [ ] **Step 7: Run TimelineView and client tests and verify GREEN**

Run:

```bash
npm test -- --run src/components/TimelineView.test.tsx src/backend/client.test.ts
npm run typecheck
```

Expected: all tests pass; the one-item case still backfills once, the 3,234-item
case does not backfill, and TypeScript reports no errors.

- [ ] **Step 8: Commit the desktop pacing behavior**

```bash
git add apps/desktop/src/App.tsx apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx
git commit -m "fix: pace timeline repair after rendered batches"
```

---

### Task 4: Update Canon And Run Regression Verification

**Files:**
- Modify: `docs/architecture/state-machine.md:784-824`
- Modify: `docs/policies/engineering-rules.md:309-330`
- Modify: `docs/superpowers/specs/2026-07-15-room-entry-gap-repair-pacing-design.md`
- Create: `docs/superpowers/plans/2026-07-15-room-entry-gap-repair-pacing.md`

**Interfaces:**
- Consumes: completed Core/Tauri/React contracts from Tasks 1-3.
- Produces: canonical documentation of `AwaitingProjection` and the exact verification record.

- [ ] **Step 1: Amend the state-machine canon**

Add an actor-private transition beside Room Timeline Continuity:

```text
Repairing -> AwaitingProjection: successful operation published timeline diffs
AwaitingProjection -> Inspecting: matching actor/timeline/repair/batch render ACK
AwaitingProjection -> terminal cleanup: unsubscribe / logout / actor replacement
```

State explicitly that `AwaitingProjection` does not enter `AppState`; React
only acknowledges completed presentation work and never decides continuity.

- [ ] **Step 2: Amend the async rule**

Extend Async and Runtime rule 4 with the concrete Room repair rule: automatic
gap repair may have only one unacknowledged projection batch, and
underfilled-initial pagination must use the settled height model rather than a
transient virtual DOM height.

- [ ] **Step 3: Run focused Rust verification**

Run:

```bash
cargo fmt --all -- --check
cargo test -p koushi-sdk --test timeline_gap_adapter
cargo test -p koushi-state --test timeline_gap_repair_state
cargo test -p koushi-core --lib timeline_gap_repair
cargo test -p koushi-core --test runtime_room_selection_scale
cargo test -p koushi-desktop acknowledge_timeline_batch_rendered
```

Expected: every command exits zero with no failed tests.

- [ ] **Step 4: Run focused desktop verification**

Run:

```bash
npm test -- --run src/backend/client.test.ts src/components/TimelineView.test.tsx
npm run typecheck
```

Expected: Vitest and TypeScript exit zero.

- [ ] **Step 5: Run repository safety checks**

Run:

```bash
git diff --check
rg -n "remove_room|reset_room_timeline_cache" crates apps
git status --short
```

Expected: `git diff --check` is empty; search results contain no normal repair
cache-deletion path; status contains only the intended task files.

- [ ] **Step 6: Commit canon and plan refinements**

```bash
git add docs/architecture/state-machine.md docs/policies/engineering-rules.md docs/superpowers/specs/2026-07-15-room-entry-gap-repair-pacing-design.md docs/superpowers/plans/2026-07-15-room-entry-gap-repair-pacing.md
git commit -m "docs: codify rendered gap repair pacing"
```

- [ ] **Step 7: Review the complete diff before handoff**

Run:

```bash
git diff --stat main...HEAD
git log --oneline main..HEAD
```

Then use `requesting-code-review` against `main...HEAD`. Resolve every verified
correctness, privacy, async-ordering, or test-coverage finding before declaring
the branch complete.
