# State-Driven Timeline Backfill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make automatic timeline backfill progress whenever the current UI and Core state make it eligible, without requiring an incidental user scroll or a click on the live-edge arrow.

**Architecture:** `TimelineView` owns ordinary backward pagination through one pure state evaluator and one request epoch at a time. The Rust timeline actor keeps ownership of persisted gap repair, but a meaningful projected-gap candidate change caused by viewport observation becomes an explicit scheduler wake-up. Both sides remain event-driven: projection, layout, pagination terminal, viewport, and render-ACK transitions re-evaluate work; no polling timer or watchdog is introduced.

**Tech Stack:** React 19, TypeScript, Vitest, Testing Library, Rust, Tokio, matrix-sdk-ui timeline actors, Cargo tests.

## Global Constraints

- Preserve the existing ownership boundary: React calls `paginateBackwards`; Rust inspects and repairs persisted timeline gaps.
- Preserve exactly one in-flight backward pagination request per mounted timeline.
- Preserve Rust automatic repair budgets, including `cache_chunk_budget = 0`.
- Preserve projection ACK and rendered-batch ACK fences. A viewport observation must not bypass either fence.
- Never make raw scroll frequency equal repair frequency. Only a changed projected candidate may wake the gap-repair scheduler.
- Do not add interval polling, delayed retries, or a watchdog.
- Diagnostic payloads must not include message bodies, room names, user IDs, event IDs, or gap identifiers.
- Treat a resolved `paginateBackwards()` promise as command acceptance only. Completion is the matching timeline terminal event or a timeline reset/resync.

---

## Task 1: Add the Pure UI Backfill Policy

**Files:**

- Create: `apps/desktop/src/domain/timelineBackfillPolicy.ts`
- Create: `apps/desktop/src/domain/timelineBackfillPolicy.test.ts`

### Public contract

Add these exported types and function:

```ts
import type { PaginationState } from "./coreEvents";

export type TimelineBackfillEvaluationTrigger =
  | "initial_projection"
  | "layout_settled"
  | "room_anchor_settled"
  | "user_scroll"
  | "pagination_terminal"
  | "prepend_settled"
  | "resync_replayed"
  | "setting_changed"
  | "timeline_reset"
  | "live_edge_settled";

export type TimelineBackfillDemand =
  | "underfilled"
  | "near_top_prefetch"
  | "explicit_top_scroll";

export type TimelineBackfillBlocker =
  | "not_initialized"
  | "awaiting_resync"
  | "suppressed_ui"
  | "projection_unsettled"
  | "virtual_layout_unsettled"
  | "anchor_unsettled"
  | "request_in_flight"
  | "pagination_paginating"
  | "pagination_end_reached";

export type TimelineBackfillEvaluation =
  | { kind: "request"; demand: TimelineBackfillDemand }
  | { kind: "blocked"; demand: TimelineBackfillDemand; reason: TimelineBackfillBlocker }
  | { kind: "idle"; reason: "auto_load_disabled" | "no_demand" };

export interface TimelineBackfillSnapshot {
  trigger: TimelineBackfillEvaluationTrigger;
  initialized: boolean;
  awaitingResync: boolean;
  suppressPaginationUi: boolean;
  autoLoadEnabled: boolean;
  paginationState: PaginationState;
  requestInFlight: boolean;
  projectionSettled: boolean;
  virtualLayoutSettled: boolean;
  anchorSettled: boolean;
  itemCount: number;
  projectedContentHeight: number;
  clientHeight: number;
  scrollHeight: number;
  scrollTop: number;
  nearTopThreshold: number;
  genuineUserScroll: boolean;
}

export function evaluateTimelineBackfill(
  snapshot: TimelineBackfillSnapshot
): TimelineBackfillEvaluation;
```

Demand precedence must be deterministic:

1. `explicit_top_scroll` when `genuineUserScroll && scrollTop <= 0`. This demand is allowed even when automatic loading is disabled.
2. `underfilled` when automatic loading is enabled and either the projected content height or DOM scroll height does not fill the viewport.
3. `near_top_prefetch` when automatic loading is enabled and `scrollTop <= nearTopThreshold`.
4. Otherwise return `idle/no_demand`; if automatic loading is disabled and no explicit top-scroll demand exists, return `idle/auto_load_disabled`.

Apply blockers to an existing demand in this order: initialization, resync, UI suppression, projection settlement, virtual-layout settlement, anchor settlement, local request epoch, SDK `Paginating`, SDK `EndReached`. This order makes diagnostics stable and testable.

### TDD steps

- [ ] Add a table-driven test named `evaluates demand and blockers without event-order state` covering every demand, every blocker, disabled-auto-load exact-top behavior, and no-demand behavior.
- [ ] Add a test named `does not let projected height hide a DOM underfill` for a zero/unknown projected height and a short DOM.
- [ ] Add a test named `prefers explicit user top-scroll over automatic demand`.
- [ ] Run the focused test and confirm RED because the module does not exist:

```bash
cd apps/desktop
npm test -- src/domain/timelineBackfillPolicy.test.ts
```

Expected failure: Vitest cannot resolve `./timelineBackfillPolicy`.

- [ ] Implement `evaluateTimelineBackfill` as a pure function with no time, refs, DOM, React, or transport dependencies.
- [ ] Re-run the focused test and confirm GREEN.
- [ ] Run TypeScript checking:

```bash
cd apps/desktop
npm run typecheck
```

- [ ] Commit:

```bash
git add apps/desktop/src/domain/timelineBackfillPolicy.ts apps/desktop/src/domain/timelineBackfillPolicy.test.ts
git commit -m "feat: define timeline backfill policy"
```

---

## Task 2: Route `TimelineView` Through One Evaluator

**Files:**

- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/domain/diagnostics.ts` only if an exported diagnostic category/type is required by the existing diagnostics API.

### Integration shape

Replace `autoBackfillRequiresUserScrollRef` and all direct ordinary-pagination calls with one callback:

```ts
type TimelineBackfillRequestEpoch = {
  id: number;
  timelineKeyHash: string;
  demand: TimelineBackfillDemand;
};

const backfillRequestEpochRef = useRef<TimelineBackfillRequestEpoch | null>(null);
const nextBackfillRequestEpochRef = useRef(1);

const evaluateAndMaybeRequestBackfill = useCallback(
  (trigger: TimelineBackfillEvaluationTrigger, genuineUserScroll = false) => {
    const evaluation = evaluateTimelineBackfill(buildSnapshot(trigger, genuineUserScroll));
    recordTimelineBackfillEvaluation(/* private-safe, deduplicated fields */);
    if (evaluation.kind !== "request") return;

    const epoch = {
      id: nextBackfillRequestEpochRef.current++,
      timelineKeyHash,
      demand: evaluation.demand
    };
    backfillRequestEpochRef.current = epoch;
    void transport.paginateBackwards(timelineKey).catch(() => {
      if (backfillRequestEpochRef.current?.id === epoch.id) {
        backfillRequestEpochRef.current = null;
        scheduleBackfillEvaluation("pagination_terminal");
      }
    });
  },
  [/* every snapshot input */]
);
```

Use the existing `timelineBackfillCompletionReason(event)` matcher to clear the epoch only for a matching terminal transition. Timeline reset/key change/resync may also invalidate it. Do not clear the epoch in `.finally()` after a successful command promise.

Create one post-commit scheduler, using the component's existing animation-frame/layout machinery rather than a timer, so a terminal transition is evaluated only after a prepend and anchor/layout restoration have settled:

```ts
const pendingBackfillEvaluationTriggerRef =
  useRef<TimelineBackfillEvaluationTrigger | null>(null);

const scheduleBackfillEvaluation = useCallback((trigger) => {
  pendingBackfillEvaluationTriggerRef.current = trigger;
  scheduleTimelineFrame();
}, [scheduleTimelineFrame]);
```

The frame/layout commit consumes the latest trigger and calls the evaluator. A programmatic scroll echo must call it with `genuineUserScroll = false`; `onTimelineScroll` passes `true` only when the existing scroll-write classifier says the scroll was user-originated.

Call the same evaluator from these transitions:

- initial projection commit: `initial_projection`
- ordinary layout settlement: `layout_settled`
- restored room anchor settlement: `room_anchor_settled`
- genuine scroll: `user_scroll`
- matching pagination terminal: `pagination_terminal`
- prepend/anchor settlement: `prepend_settled`
- resync replay completion: `resync_replayed`
- automatic-loading setting change: `setting_changed`
- timeline key/reset: `timeline_reset`
- jump-to-bottom completion: `live_edge_settled`

Keep the empty-thread special case only if the policy cannot observe a real scroll container. It must still share the same request epoch, terminal clearing, and diagnostics helper; it must not retain a second in-flight boolean.

### Diagnostics

Record a deduplicated `timeline.backfill_evaluation` entry containing only:

```ts
{
  trigger,
  decision: evaluation.kind,
  demand: evaluation.kind === "idle" ? null : evaluation.demand,
  reason: evaluation.kind === "request" ? null : evaluation.reason,
  item_count: items.length,
  projected_height_bucket,
  client_height_bucket,
  scroll_height_bucket,
  scroll_top_bucket,
  pagination_state,
  request_epoch: backfillRequestEpochRef.current?.id ?? null
}
```

Bucket pixel metrics before recording, and deduplicate consecutive identical decision/trigger/reason/state tuples. Do not record timeline keys or event identities.

### TDD steps

- [ ] Change the existing restored-anchor test so it expects automatic backfill after anchor settlement without a user scroll. Run it and confirm RED with zero calls.
- [ ] Add `continues underfill backfill after each terminal and settled prepend` using deferred pagination promises/events and assert one call at a time, then a second call only after the terminal plus layout settlement. Confirm RED.
- [ ] Add `does not treat a programmatic restore scroll as explicit top scroll when auto-load is disabled`. Confirm RED if the current scroll path calls pagination.
- [ ] Add `jump to bottom re-evaluates backfill after live-edge settlement`. Model the reported screenshot path: the visible list is underfilled/stale, click the header arrow, and assert ordinary pagination is requested without a subsequent user scroll. Confirm RED.
- [ ] Add `keeps the request epoch active after the command promise resolves until a terminal event`. Confirm RED with two evaluation transitions and only one transport call expected.
- [ ] Add `records the last automatic backfill blocker without private identifiers`.
- [ ] Run the focused component tests:

```bash
cd apps/desktop
npm test -- src/components/TimelineView.test.tsx
```

- [ ] Implement the single callback, epoch lifecycle, scheduler hook, and diagnostic entry; delete `autoBackfillRequiresUserScrollRef`, `backfillInFlightRef`, `maybeAutoBackfill`, and duplicate blocker lists.
- [ ] Re-run the component test until GREEN.
- [ ] Verify removed state cannot reappear:

```bash
rg -n "autoBackfillRequiresUserScrollRef|backfillInFlightRef|maybeAutoBackfill" apps/desktop/src
```

Expected output: none.

- [ ] Run the policy and component tests together plus typecheck:

```bash
cd apps/desktop
npm test -- src/domain/timelineBackfillPolicy.test.ts src/components/TimelineView.test.tsx
npm run typecheck
```

- [ ] Commit:

```bash
git add apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx apps/desktop/src/domain/diagnostics.ts
git commit -m "fix: drive timeline backfill from settled state"
```

If `diagnostics.ts` is unchanged, omit it from `git add`.

---

## Task 3: Add Candidate-Aware Core Wake-Up Policy

**Files:**

- Modify: `crates/koushi-core/src/timeline.rs`

### Private policy contract

Add a private, identity-free scheduler observation:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProjectedGapCandidate {
    ordinal: usize,
    relation: ProjectedGapRelation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectedGapRelation {
    IntersectsViewport,
    NearestLiveEdge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GapRepairWakeDecision {
    Wake { candidate: ProjectedGapCandidate },
    IdleNoCandidate,
    IdleUnchangedCandidate { candidate: ProjectedGapCandidate },
}

fn evaluate_gap_repair_viewport_wake(
    projected_gaps: &[ProjectedTimelineGap],
    viewport_range: Option<(usize, usize)>,
    previous: Option<ProjectedGapCandidate>,
) -> GapRepairWakeDecision;
```

Use the actual existing projected-gap type name when implementing; do not introduce a duplicate DTO. The candidate fingerprint may contain only an ordinal and relation, not an event ID or gap identifier.

Extend `TimelineGapRepairTracker` with `last_projected_candidate: Option<ProjectedGapCandidate>`. Reset it on actor generation/timeline reset and whenever the projected gap set becomes empty. Update it when projection changes or viewport observation selects a different candidate.

In `TimelineActorMessage::ObserveViewport`:

1. Store the observation and emit existing navigation state as today.
2. Compute the current projected candidate from the actor's canonical navigation items and the new viewport range.
3. If the candidate changed, call the existing `request_timeline_gap_inspection(TimelineGapRepairTrigger::Automatic)` path.
4. Let the existing tracker coalesce the request when an inspection, projection ACK, or rendered-batch ACK is active.
5. If the candidate did not change, do not inspect.

Do not call SDK pagination directly from the viewport handler. Do not modify automatic budgets or manual repair behavior.

### Diagnostics

Extend the existing `record_timeline_gap_repair` use with stage `evaluation` (or add a small typed wrapper) and private-safe fields:

- trigger: `viewport`, `projection`, `terminal`, or existing trigger label
- decision: `wake`, `blocked`, `idle_unchanged`, `idle_no_candidate`
- blocker/scheduler phase
- projected gap count
- candidate changed boolean

Do not log candidate ordinal if the existing diagnostics policy considers even positional information sensitive; the ordinal is not required for acceptance.

### TDD steps

- [ ] Add a unit test `viewport_wake_requests_inspection_when_projected_candidate_changes`.
- [ ] Add a unit test `viewport_wake_ignores_repeated_observation_for_same_candidate`.
- [ ] Add a unit test `viewport_wake_requests_again_when_viewport_selects_another_gap`.
- [ ] Add a unit test `viewport_wake_preserves_pending_trigger_while_render_ack_is_outstanding` against `TimelineGapRepairTracker`.
- [ ] Add or extend an actor test `observe_viewport_wakes_automatic_gap_inspection_after_projection_ack` that sends `ObserveViewport` and observes the existing inspection path. Confirm no inspection occurs before the required ACK.
- [ ] Run the focused tests and confirm RED because the wake evaluator does not exist:

```bash
cargo test -p koushi-core viewport_wake -- --nocapture
cargo test -p koushi-core observe_viewport_wakes_automatic_gap_inspection_after_projection_ack -- --nocapture
```

- [ ] Implement the evaluator, tracker field, reset/update logic, viewport wake, and evaluation diagnostics.
- [ ] Re-run the focused tests and confirm GREEN.
- [ ] Run all gap repair tests:

```bash
cargo test -p koushi-core timeline_gap_repair -- --nocapture
cargo test -p koushi-core projected_gap -- --nocapture
```

- [ ] Confirm automatic cache budget is still zero:

```bash
rg -n "cache_chunk_budget" crates/koushi-core/src/timeline.rs
```

- [ ] Format and commit:

```bash
cargo fmt --check
git add crates/koushi-core/src/timeline.rs
git commit -m "fix: wake gap repair for changed viewport candidates"
```

---

## Task 4: Cover Event-Order Permutations

**Files:**

- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `crates/koushi-core/src/timeline.rs`

### UI permutation table

Add a table-driven test `backfill eligibility is independent of settlement event order` for at least:

1. terminal → prepend commit → anchor settlement
2. prepend commit → terminal → anchor settlement
3. anchor settlement → terminal → layout frame
4. command promise resolution → terminal → layout frame
5. resync reset → initial projection → anchor settlement

For each sequence, assert:

- no request while a blocker is present;
- exactly one request when all blockers clear and demand remains;
- no second concurrent request;
- no genuine user scroll is required.

### Core permutation table

Add a table-driven tracker/actor test `gap repair wake is retained across ack and inspection order` for:

1. viewport candidate change before initial projection ACK;
2. candidate change while inspection is active;
3. candidate change while a repair-produced batch awaits render ACK;
4. same candidate repeated during all three phases;
5. candidate disappears before a pending inspection starts.

Assert the existing scheduler runs at most one inspection at a time, retains a meaningful changed-candidate wake, drops a vanished candidate, and never grants a nonzero automatic cache-chunk budget.

### TDD and commit

- [ ] Add both permutation tables and run them first against the current Task 2/3 implementation.
- [ ] If any row fails, make the smallest policy/state transition fix; do not add sleeps or retry timers.
- [ ] Run focused suites:

```bash
cd apps/desktop
npm test -- src/components/TimelineView.test.tsx src/domain/timelineBackfillPolicy.test.ts
cd ../..
cargo test -p koushi-core gap_repair_wake -- --nocapture
```

- [ ] Commit tests and any fixes:

```bash
git add apps/desktop/src/components/TimelineView.test.tsx crates/koushi-core/src/timeline.rs
git commit -m "test: cover timeline backfill event ordering"
```

---

## Task 5: Update Architecture and Diagnostic Documentation

**Files:**

- Modify: `docs/architecture/overview.md`
- Modify: `docs/policies/diagnostics.md` if it exists and owns diagnostic-event documentation; otherwise update the nearest existing diagnostics policy document discovered through CodeGraph.
- Modify: `docs/superpowers/specs/2026-07-15-state-driven-timeline-backfill-design.md` only if implementation reveals a necessary approved clarification.

### Documentation requirements

- [ ] Document the evaluator inputs, demand precedence, blocker precedence, and request-epoch terminal rule.
- [ ] Document that programmatic restore scrolls are not genuine top-scroll demand.
- [ ] Document that layout/terminal/projection transitions are explicit event-driven wake-ups.
- [ ] Document candidate-aware Core viewport wake-up and preserved ACK/budget fences.
- [ ] Document `timeline.backfill_evaluation` and `core.timeline_gap_repair` private-safe fields.
- [ ] State explicitly that no timer, polling, or watchdog participates.
- [ ] Verify links and formatting:

```bash
git diff --check
rg -n "timeline.backfill_evaluation|core.timeline_gap_repair|request epoch|candidate" docs
```

- [ ] Commit:

```bash
git add docs/architecture/overview.md docs/policies/diagnostics.md docs/superpowers/specs/2026-07-15-state-driven-timeline-backfill-design.md
git commit -m "docs: define timeline backfill wake-up contract"
```

Only stage files that actually exist and changed.

---

## Task 6: Full Verification and Review

### Frontend verification

- [ ] Run all desktop tests:

```bash
cd apps/desktop
npm test
npm run typecheck
npm run lint
```

- [ ] Build the Rust workspace and run the complete Core package tests:

```bash
cd ../..
cargo build
cargo test -p koushi-core
cargo fmt --check
cargo clippy -p koushi-core --all-targets -- -D warnings
```

- [ ] Inspect the final patch for accidental scope expansion and privacy leaks:

```bash
git diff origin/main...HEAD --stat
git diff --check origin/main...HEAD
rg -n "setInterval|setTimeout|event_id|room_id|user_id" apps/desktop/src/domain/timelineBackfillPolicy.ts apps/desktop/src/components/TimelineView.tsx crates/koushi-core/src/timeline.rs
```

The identifier search is a manual-review aid: existing legitimate fields may match, but new diagnostic payloads must not include them.

- [ ] Request code review specifically for missed wake-ups, duplicate pagination, ACK-fence violations, budget changes, private diagnostic data, and test event ordering.
- [ ] Apply only verified review findings, rerun the affected focused suites, and commit review fixes separately.
- [ ] Re-run the complete verification commands after the last code change.

---

## Task 7: Publish, Pass CI, and Merge

- [ ] Confirm the branch is clean and based on current `origin/main`:

```bash
git fetch origin
git status --short
git log --oneline --decorate origin/main..HEAD
```

- [ ] Rebase onto updated `origin/main` if necessary, then re-run all verification affected by conflict resolution.
- [ ] Push `codex/fix-timeline-backfill-wakeups`.
- [ ] Open a draft PR with:

  - the intermittent room-entry/underfill symptom;
  - the sticky UI wake-up root cause and Core viewport wake-up gap;
  - the state-driven evaluator/request epoch and candidate-aware scheduler design;
  - focused and full verification evidence;
  - an explicit statement that automatic cache chunk budget remains zero and no polling was added.

- [ ] Mark the PR ready after the patch and checks are reviewable.
- [ ] Monitor every required GitHub Actions check. For any failure, inspect the exact failing job/log, reproduce locally where possible, fix on the same branch, rerun local verification, push, and wait for the new run.
- [ ] Address actionable review threads and resolve them only after the corresponding fix or evidence is present.
- [ ] Merge only after required checks pass and required approvals are present.
- [ ] Verify the merge commit is reachable from fresh `origin/main`:

```bash
git fetch origin
git merge-base --is-ancestor "$(git rev-parse HEAD)" origin/main
git log -1 --oneline origin/main
```

- [ ] Report the PR URL, merge commit, final verification results, and any pre-existing warnings that remain outside this change.
