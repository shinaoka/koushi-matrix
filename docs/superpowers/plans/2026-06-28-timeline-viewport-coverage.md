# Timeline Viewport Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make timeline startup restore, programmatic navigation, and user scrolling request exactly the history needed to render the current viewport plus buffer, without tying backfill to user scroll events.

**Architecture:** Introduce a pure viewport coverage evaluator that decides whether the loaded logical timeline covers the required render range. Route all viewport-settle points through one coverage evaluation path; `TimelineView` may read DOM metrics, but backfill policy and duplicate suppression live in domain/state-machine code.

**Tech Stack:** React, TypeScript, Vitest, existing `TimelineView`, `timelineViewportMachine`, `timelineStore`, and Matrix timeline pagination transport.

## Global Constraints

- Do not add ad hoc `maybeAutoBackfill()` calls at individual restore sites.
- Do not use "scrollTop near top" as the domain concept; use "required render range extends before loaded content".
- Preserve user scroll anchors and programmatic-scroll echo suppression.
- Keep implementation test-first. Every production behavior change must have a RED test first.
- Leave unrelated `scripts/run.sh` untracked and unstaged.
- Use CodeGraph before grep/file reads when locating code in this repository.

---

## File Structure

- Create `apps/desktop/src/domain/timelineViewportCoverage.ts`
  - Owns pure coverage/backfill decision logic.
  - Does not touch DOM, React refs, or transport.
- Create `apps/desktop/src/domain/timelineViewportCoverage.test.ts`
  - Covers backfill/no-backfill decisions independent of React.
- Modify `apps/desktop/src/domain/timelineViewportMachine.ts`
  - Owns request de-duplication and in-flight signatures for coverage-driven backfill.
- Modify `apps/desktop/src/domain/timelineViewportMachine.test.ts`
  - Covers request gating, completion, and reset behavior.
- Modify `apps/desktop/src/components/useTimelineViewportController.ts`
  - Exposes the new machine helpers.
- Modify `apps/desktop/src/components/TimelineView.tsx`
  - Reads DOM/virtualizer metrics and calls one `evaluateViewportCoverageAndMaybeBackfill(source)` function from defined viewport-settle points.
  - Removes the old scroll-only `maybeAutoBackfill` policy.
- Modify `apps/desktop/src/components/TimelineView.test.tsx`
  - Keep the current RED regression test: `paginates older history after startup restore lands near the loaded start`.
  - Add integration coverage for programmatic navigation and no duplicate requests.

---

### Task 1: Add Pure Viewport Coverage Evaluator

**Files:**
- Create: `apps/desktop/src/domain/timelineViewportCoverage.ts`
- Create: `apps/desktop/src/domain/timelineViewportCoverage.test.ts`

**Interfaces:**
- Consumes: `PaginationState` from `apps/desktop/src/domain/coreEvents.ts`
- Produces:
  - `TimelineViewportCoverageSource`
  - `TimelineViewportCoverageInput`
  - `TimelineViewportCoverageDecision`
  - `decideTimelineViewportCoverage(input: TimelineViewportCoverageInput): TimelineViewportCoverageDecision`

- [ ] **Step 1: Write the failing domain tests**

Add `apps/desktop/src/domain/timelineViewportCoverage.test.ts`:

```typescript
import { describe, expect, test } from "vitest";

import {
  decideTimelineViewportCoverage,
  type TimelineViewportCoverageInput
} from "./timelineViewportCoverage";

function input(
  overrides: Partial<TimelineViewportCoverageInput>
): TimelineViewportCoverageInput {
  return {
    source: "programmatic-restore",
    relativeScrollTopPx: 48,
    clientHeightPx: 500,
    loadedContentHeightPx: 1200,
    coverageMarginPx: 1000,
    backwardPaginationState: "Idle",
    autoLoadOlderMessages: true,
    forceUserBackfill: false,
    suppressPaginationUi: false,
    backfillInFlight: false,
    blockingAnchorWork: false,
    ...overrides
  };
}

describe("timeline viewport coverage", () => {
  test("requests backward pagination when required render range extends before loaded content", () => {
    expect(decideTimelineViewportCoverage(input({ relativeScrollTopPx: 48 }))).toEqual({
      kind: "request-backward",
      reason: "required-range-before-loaded-start"
    });
  });

  test("does not request when the required render range is covered", () => {
    expect(decideTimelineViewportCoverage(input({ relativeScrollTopPx: 1400 }))).toEqual({
      kind: "covered",
      reason: "required-range-within-loaded-content"
    });
  });

  test("does not request when backward pagination is exhausted or already running", () => {
    expect(
      decideTimelineViewportCoverage(input({ backwardPaginationState: "EndReached" }))
    ).toEqual({ kind: "blocked", reason: "backward-pagination-unavailable" });
    expect(decideTimelineViewportCoverage(input({ backfillInFlight: true }))).toEqual({
      kind: "blocked",
      reason: "backfill-in-flight"
    });
  });

  test("allows explicit user backfill even when automatic loading is disabled", () => {
    expect(
      decideTimelineViewportCoverage(
        input({
          source: "user-scroll",
          autoLoadOlderMessages: false,
          forceUserBackfill: true
        })
      )
    ).toEqual({
      kind: "request-backward",
      reason: "required-range-before-loaded-start"
    });
  });

  test("blocks automatic programmatic backfill when automatic loading is disabled", () => {
    expect(
      decideTimelineViewportCoverage(
        input({
          source: "programmatic-restore",
          autoLoadOlderMessages: false,
          forceUserBackfill: false
        })
      )
    ).toEqual({ kind: "blocked", reason: "automatic-backfill-disabled" });
  });
});
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
npm --prefix apps/desktop test -- --run timelineViewportCoverage.test.ts
```

Expected: FAIL because `timelineViewportCoverage.ts` does not exist.

- [ ] **Step 3: Implement the pure evaluator**

Create `apps/desktop/src/domain/timelineViewportCoverage.ts`:

```typescript
import type { PaginationState } from "./coreEvents";

export type TimelineViewportCoverageSource =
  | "user-scroll"
  | "programmatic-restore"
  | "programmatic-navigation"
  | "layout-settle";

export type TimelineViewportCoverageInput = {
  source: TimelineViewportCoverageSource;
  relativeScrollTopPx: number;
  clientHeightPx: number;
  loadedContentHeightPx: number;
  coverageMarginPx: number;
  backwardPaginationState: PaginationState;
  autoLoadOlderMessages: boolean;
  forceUserBackfill: boolean;
  suppressPaginationUi: boolean;
  backfillInFlight: boolean;
  blockingAnchorWork: boolean;
};

export type TimelineViewportCoverageDecision =
  | { kind: "request-backward"; reason: "required-range-before-loaded-start" }
  | { kind: "covered"; reason: "required-range-within-loaded-content" }
  | {
      kind: "blocked";
      reason:
        | "pagination-ui-suppressed"
        | "blocking-anchor-work"
        | "backfill-in-flight"
        | "backward-pagination-unavailable"
        | "automatic-backfill-disabled";
    };

export function decideTimelineViewportCoverage(
  input: TimelineViewportCoverageInput
): TimelineViewportCoverageDecision {
  if (input.suppressPaginationUi) {
    return { kind: "blocked", reason: "pagination-ui-suppressed" };
  }
  if (input.blockingAnchorWork) {
    return { kind: "blocked", reason: "blocking-anchor-work" };
  }
  if (input.backfillInFlight) {
    return { kind: "blocked", reason: "backfill-in-flight" };
  }
  if (
    input.backwardPaginationState === "Paginating" ||
    input.backwardPaginationState === "EndReached"
  ) {
    return { kind: "blocked", reason: "backward-pagination-unavailable" };
  }
  if (!input.autoLoadOlderMessages && !input.forceUserBackfill) {
    return { kind: "blocked", reason: "automatic-backfill-disabled" };
  }

  const requiredTopPx = input.relativeScrollTopPx - input.coverageMarginPx;
  if (requiredTopPx <= 0) {
    return {
      kind: "request-backward",
      reason: "required-range-before-loaded-start"
    };
  }
  return { kind: "covered", reason: "required-range-within-loaded-content" };
}
```

- [ ] **Step 4: Run tests and verify GREEN**

Run:

```bash
npm --prefix apps/desktop test -- --run timelineViewportCoverage.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/domain/timelineViewportCoverage.ts apps/desktop/src/domain/timelineViewportCoverage.test.ts
git commit -m "Add timeline viewport coverage evaluator"
```

---

### Task 2: Move Coverage Request Gating Into the Viewport Machine

**Files:**
- Modify: `apps/desktop/src/domain/timelineViewportMachine.ts`
- Modify: `apps/desktop/src/domain/timelineViewportMachine.test.ts`
- Modify: `apps/desktop/src/components/useTimelineViewportController.ts`

**Interfaces:**
- Consumes: coverage decision kind from Task 1.
- Produces:
  - `timelineViewportCanRequestCoverageBackfill(state, signature): boolean`
  - machine events:
    - `{ type: "coverage-backfill-request-started"; signature: string }`
    - `{ type: "coverage-backfill-request-finished"; signature: string }`

- [ ] **Step 1: Write the failing machine tests**

Append to `apps/desktop/src/domain/timelineViewportMachine.test.ts`:

```typescript
it("deduplicates coverage-driven backfill requests by signature", () => {
  const started = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
    type: "coverage-backfill-request-started",
    signature: "room:!r:generation:1:backward"
  });

  expect(timelineViewportCanRequestCoverageBackfill(started, "room:!r:generation:1:backward")).toBe(
    false
  );
  expect(timelineViewportCanRequestCoverageBackfill(started, "room:!r:generation:2:backward")).toBe(
    true
  );
});

it("allows a coverage-driven backfill request after completion", () => {
  const started = reduceTimelineViewportMachine(createTimelineViewportMachineState(), {
    type: "coverage-backfill-request-started",
    signature: "room:!r:generation:1:backward"
  });
  const finished = reduceTimelineViewportMachine(started, {
    type: "coverage-backfill-request-finished",
    signature: "room:!r:generation:1:backward"
  });

  expect(timelineViewportCanRequestCoverageBackfill(finished, "room:!r:generation:1:backward")).toBe(
    true
  );
});
```

- [ ] **Step 2: Run test and verify RED**

Run:

```bash
npm --prefix apps/desktop test -- --run timelineViewportMachine.test.ts
```

Expected: FAIL because the new events/selectors are missing.

- [ ] **Step 3: Implement machine state and selector**

In `apps/desktop/src/domain/timelineViewportMachine.ts`, add state fields:

```typescript
coverageBackfillRequestSignature: string | null;
```

Initialize it in `createTimelineViewportMachineState()`:

```typescript
coverageBackfillRequestSignature: null
```

Add events to `TimelineViewportMachineEvent`:

```typescript
| { type: "coverage-backfill-request-started"; signature: string }
| { type: "coverage-backfill-request-finished"; signature: string }
```

Add selector:

```typescript
export function timelineViewportCanRequestCoverageBackfill(
  state: TimelineViewportMachineState,
  signature: string
): boolean {
  return state.coverageBackfillRequestSignature !== signature;
}
```

Add reducer cases:

```typescript
case "coverage-backfill-request-started":
  return { ...state, coverageBackfillRequestSignature: event.signature };
case "coverage-backfill-request-finished":
  return state.coverageBackfillRequestSignature === event.signature
    ? { ...state, coverageBackfillRequestSignature: null }
    : state;
```

Ensure `timeline-key-changed` and `resync-required` still reset via `createTimelineViewportMachineState()`.

- [ ] **Step 4: Expose selector through the controller**

In `apps/desktop/src/components/useTimelineViewportController.ts`, import and expose:

```typescript
timelineViewportCanRequestCoverageBackfill
```

Return:

```typescript
const canRequestCoverageBackfill = useCallback(
  (signature: string) =>
    timelineViewportCanRequestCoverageBackfill(stateRef.current, signature),
  []
);
```

Add it to the returned object.

- [ ] **Step 5: Run tests and verify GREEN**

Run:

```bash
npm --prefix apps/desktop test -- --run timelineViewportMachine.test.ts
npm --prefix apps/desktop test -- --run useTimelineViewportController
```

Expected: machine tests pass. If the second command finds no tests, continue; the controller is covered through `TimelineView.test.tsx` later.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/domain/timelineViewportMachine.ts apps/desktop/src/domain/timelineViewportMachine.test.ts apps/desktop/src/components/useTimelineViewportController.ts
git commit -m "Gate viewport coverage backfill in viewport machine"
```

---

### Task 3: Replace Scroll-Only Backfill With Viewport Coverage Evaluation

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`

**Interfaces:**
- Consumes:
  - `decideTimelineViewportCoverage`
  - `viewportController.canRequestCoverageBackfill(signature)`
  - `viewportController.dispatch({ type: "coverage-backfill-request-started" | "coverage-backfill-request-finished" })`
- Produces:
  - One local `evaluateViewportCoverageAndMaybeBackfill(source, options)` function in `TimelineView`.

- [ ] **Step 1: Keep the current RED integration test**

Keep the existing uncommitted test in `apps/desktop/src/components/TimelineView.test.tsx`:

```typescript
it("paginates older history after startup restore lands near the loaded start", async () => {
  // This test should stay RED until the coverage evaluator is integrated.
});
```

Run:

```bash
npm --prefix apps/desktop test -- --run TimelineView.test.tsx -t "paginates older history after startup restore"
```

Expected: FAIL with `paginateBackwards` call count 0.

- [ ] **Step 2: Add one local coverage evaluation function**

In `apps/desktop/src/components/TimelineView.tsx`, add imports:

```typescript
import { decideTimelineViewportCoverage, type TimelineViewportCoverageSource } from "../domain/timelineViewportCoverage";
```

Inside `TimelineView`, add:

```typescript
const canRequestCoverageBackfill = viewportController.canRequestCoverageBackfill;

const evaluateViewportCoverageAndMaybeBackfill = useCallback(
  (source: TimelineViewportCoverageSource, options?: { forceUserBackfill?: boolean }) => {
    const container = containerRef.current;
    if (!container) {
      return;
    }
    const relativeScrollTopPx = Math.max(
      0,
      container.scrollTop - (listRef.current?.offsetTop ?? 0)
    );
    const coverageMarginPx = Math.max(
      virtualItemHeight * TIMELINE_VIRTUAL_OVERSCAN_ITEMS,
      container.clientHeight * 2
    );
    const decision = decideTimelineViewportCoverage({
      source,
      relativeScrollTopPx,
      clientHeightPx: container.clientHeight,
      loadedContentHeightPx: timelineHeightModel.totalHeight,
      coverageMarginPx,
      backwardPaginationState: getPaginationState(store, timelineKeyRef.current, "Backward"),
      autoLoadOlderMessages,
      forceUserBackfill: options?.forceUserBackfill === true,
      suppressPaginationUi,
      backfillInFlight: backfillInFlightRef.current,
      blockingAnchorWork: viewportHasBlockingAnchorWork()
    });
    if (decision.kind !== "request-backward") {
      return;
    }
    const signature = [
      timelineStoreKeyId(timelineKeyRef.current),
      generation,
      Math.floor(relativeScrollTopPx),
      "backward"
    ].join("\u0000");
    if (!canRequestCoverageBackfill(signature)) {
      return;
    }
    dispatchViewportMachine({
      type: "coverage-backfill-request-started",
      signature
    });
    backfillInFlightRef.current = true;
    void transport.paginateBackwards(timelineKeyRef.current)
      .finally(() => {
        backfillInFlightRef.current = false;
        dispatchViewportMachine({
          type: "coverage-backfill-request-finished",
          signature
        });
      });
  },
  [
    autoLoadOlderMessages,
    canRequestCoverageBackfill,
    dispatchViewportMachine,
    generation,
    store,
    suppressPaginationUi,
    timelineHeightModel.totalHeight,
    transport,
    virtualItemHeight,
    viewportHasBlockingAnchorWork
  ]
);
```

- [ ] **Step 3: Route viewport-settle points through the function**

Call `evaluateViewportCoverageAndMaybeBackfill(...)` only after viewport position is settled:

```typescript
// In scheduled scroll work after reportViewportObservation()
evaluateViewportCoverageAndMaybeBackfill("user-scroll", { forceUserBackfill: true });

// After room anchor restore succeeds or virtualized anchor scrollTop is assigned and metrics updated
evaluateViewportCoverageAndMaybeBackfill("programmatic-restore");

// After prepend anchor restoration completes and metrics are updated
evaluateViewportCoverageAndMaybeBackfill("programmatic-restore");

// After event target navigation completes
evaluateViewportCoverageAndMaybeBackfill("programmatic-navigation");

// After layout-settle corrections that change scrollTop
evaluateViewportCoverageAndMaybeBackfill("layout-settle");
```

Do not pass booleans such as `autoBackfill: true` at call sites. The evaluator owns that policy.

- [ ] **Step 4: Remove old scroll-only backfill policy**

Delete the old `maybeAutoBackfill` function from `TimelineView.tsx`.

Keep user-triggered top scroll behavior by using:

```typescript
evaluateViewportCoverageAndMaybeBackfill("user-scroll", { forceUserBackfill: true });
```

inside the scroll work frame.

- [ ] **Step 5: Run integration tests and verify GREEN**

Run:

```bash
npm --prefix apps/desktop test -- --run TimelineView.test.tsx -t "paginates older history after startup restore"
npm --prefix apps/desktop test -- --run TimelineView.test.tsx -t "paginates older history when the user scrolls to the top"
```

Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx
git commit -m "Backfill timeline history by viewport coverage"
```

---

### Task 4: Add Regression Coverage for Duplicate Suppression and Non-Backfill Cases

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`

**Interfaces:**
- Consumes Task 3 integration.
- Produces tests that prevent eager or repeated pagination.

- [ ] **Step 1: Add a duplicate suppression test**

Add to `TimelineView.test.tsx`:

```typescript
it("does not duplicate coverage backfill requests while one is in flight", async () => {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  let resolvePagination: () => void = () => undefined;
  const paginateBackwards = vi.fn(
    () => new Promise<void>((resolve) => {
      resolvePagination = resolve;
    })
  );
  const transport = baseTransport({
    listenCoreEvents(nextListener) {
      emit = nextListener;
      return () => undefined;
    },
    paginateBackwards
  });

  render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />);
  const timeline = await screen.findByTestId("timeline-view");
  Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
  Object.defineProperty(timeline, "scrollHeight", { value: 1200, configurable: true });
  Object.defineProperty(timeline, "scrollTop", { value: 0, writable: true, configurable: true });

  act(() => {
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: [message("$first", "First")]
        }
      }
    });
  });
  fireEvent.scroll(timeline);
  fireEvent.scroll(timeline);

  await waitFor(() => expect(paginateBackwards).toHaveBeenCalledTimes(1));
  act(() => resolvePagination());
});
```

- [ ] **Step 2: Add a no-request test when coverage is sufficient**

Add to `TimelineView.test.tsx`:

```typescript
it("does not request older history when the restored viewport has sufficient loaded coverage", async () => {
  let emit: (payload: CoreEventPayload) => void = () => undefined;
  const paginateBackwards = vi.fn(async () => undefined);
  const transport = baseTransport({
    listenCoreEvents(nextListener) {
      emit = nextListener;
      return () => undefined;
    },
    paginateBackwards
  });
  const items = Array.from({ length: 80 }, (_, index) =>
    message(`$covered-${index}:example.invalid`, `Covered message ${index}`)
  );

  render(<TimelineView timelineKey={KEY} roomId="!room:example.invalid" transport={transport} onReply={vi.fn()} />);
  const timeline = await screen.findByTestId("timeline-view");
  Object.defineProperty(timeline, "clientHeight", { value: 500, configurable: true });
  Object.defineProperty(timeline, "scrollHeight", { value: 80 * 72, configurable: true });
  Object.defineProperty(timeline, "scrollTop", { value: 3000, writable: true, configurable: true });

  act(() => {
    emit({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items
        }
      }
    });
  });

  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(paginateBackwards).not.toHaveBeenCalled();
});
```

- [ ] **Step 3: Run TimelineView tests**

Run:

```bash
npm --prefix apps/desktop test -- --run TimelineView.test.tsx
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/components/TimelineView.test.tsx
git commit -m "Cover viewport coverage pagination edge cases"
```

---

### Task 5: Cleanup and Verification

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/domain/timelineStore.ts` only if `shouldSuppressAutoBackfill` becomes unused.

**Interfaces:**
- Consumes all previous tasks.
- Produces final reviewed branch.

- [ ] **Step 1: Remove obsolete scroll-only names**

If `shouldSuppressAutoBackfill` is unused after Task 3, delete it from `apps/desktop/src/domain/timelineStore.ts` and remove its tests from `apps/desktop/src/domain/timelineStore.test.ts`.

Run:

```bash
rg "shouldSuppressAutoBackfill|maybeAutoBackfill" apps/desktop/src
```

Expected: no matches, unless a test intentionally references old behavior during refactor and is removed in this step.

- [ ] **Step 2: Run verification**

Run:

```bash
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run lint
cargo fmt --check
git diff --check
```

Expected:
- Vitest: all tests pass.
- TypeScript: no errors.
- ESLint: no errors.
- Rust formatting unchanged.
- `git diff --check`: no whitespace errors.

- [ ] **Step 3: Commit cleanup**

```bash
git add apps/desktop/src
git commit -m "Clean up scroll-only timeline backfill logic"
```

- [ ] **Step 4: Push branch and update PR**

```bash
git status --short --branch
git push
```

Expected: branch `codex/perf-read-marker` updates PR #154.

---

## Self-Review

- Spec coverage: The plan implements viewport-data coverage rather than scroll-top-only behavior, preserves the existing RED startup-restore regression, and adds duplicate/no-request tests.
- Placeholder scan: No TBD/TODO/implement-later placeholders remain.
- Type consistency: The planned evaluator input names match the integration code snippets; machine events use one signature string for request de-duplication.
