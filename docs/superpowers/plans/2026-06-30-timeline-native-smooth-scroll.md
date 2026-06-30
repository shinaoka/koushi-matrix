# Timeline Native-Smooth Scroll Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make virtualized timeline scrolling native-smooth by measuring the current pipeline, reducing active-scroll React recomposition, deferring dynamic height commits, reserving media height, and choosing one scroll-anchor owner.

**Architecture:** Keep Matrix timeline semantics Rust-owned and keep this work inside the React presentation layer. Add private-data-free browser/headless diagnostics, then refactor `TimelineView` so raw `scrollTop` lives in refs while React state changes only for virtual range changes or committed height-model transactions. Apply measurement changes after a 100ms idle window and preserve anchors through reason-tagged scroll writes.

**Tech Stack:** React + TypeScript, Vitest + jsdom, Playwright headless Chromium, existing `TimelineView` local virtualizer, CSS `overflow-anchor` and media sizing.

---

## File Map

- Create `apps/desktop/src/domain/timelineScrollDiagnostics.ts`: private-data-free types, counters, row-kind buckets, and token formatting for timeline scroll diagnostics.
- Create `apps/desktop/src/domain/timelineScrollDiagnostics.test.ts`: unit coverage for diagnostic accumulation, row-kind bucketing, and private-data-free string output.
- Modify `apps/desktop/src/components/TimelineView.tsx`: add scroll diagnostics callback, render/range/height counters, raw metric refs, range state, scroll-write reasons, active-scroll tracking, deferred measurement flush, media box helpers, and exported test helpers where needed.
- Modify `apps/desktop/src/components/TimelineView.test.tsx`: jsdom tests for instrumentation, scroll-write reason classification, deferred measurement, and helper behavior that does not require real layout.
- Modify `apps/desktop/src/test/harnessMain.tsx`: expose `window.__harness.scrollDiagnostics()` and `window.__harness.resetScrollDiagnostics()` for Playwright.
- Modify `apps/desktop/e2e/timeline-scrollback.spec.ts`: add real-browser tests for active-scroll range stability, deferred measurement, media height reservation, and scroll anchoring.
- Modify `apps/desktop/src/styles.css`: reserve known-dimension media boxes and set the final `overflow-anchor` policy.
- Modify `apps/desktop/src/styles.contract.test.ts`: assert the selected `overflow-anchor` policy.
- Do not modify `apps/desktop/src/domain/diagnostics.ts` in this plan. Scroll diagnostics stay harness-only.
- Do not modify `apps/desktop/src/domain/qaTitle.ts` in this plan. Scroll diagnostics stay out of QA title tokens.

## Task 1: Private-Data-Free Scroll Diagnostics Model

**Files:**
- Create: `apps/desktop/src/domain/timelineScrollDiagnostics.ts`
- Create: `apps/desktop/src/domain/timelineScrollDiagnostics.test.ts`

- [ ] **Step 1: Write the failing diagnostics model tests**

Add `apps/desktop/src/domain/timelineScrollDiagnostics.test.ts`:

```ts
import { describe, expect, test } from "vitest";

import {
  createInitialTimelineScrollDiagnostics,
  recordTimelineScrollCommit,
  recordTimelineScrollEstimate,
  recordTimelineScrollFrame,
  recordTimelineScrollHeightCommit,
  recordTimelineScrollMeasurementFlush,
  recordTimelineScrollRangeCommit,
  recordTimelineScrollWrite,
  timelineScrollDiagnosticTokens,
  type TimelineScrollDiagnostics
} from "./timelineScrollDiagnostics";

describe("timelineScrollDiagnostics", () => {
  test("records private-data-free scroll counters and tokens", () => {
    let diagnostics: TimelineScrollDiagnostics = createInitialTimelineScrollDiagnostics();

    diagnostics = recordTimelineScrollCommit(diagnostics);
    diagnostics = recordTimelineScrollFrame(diagnostics, {
      scrollActivity: "active",
      viewportIntent: "anchored",
      userInputPending: true,
      virtualized: true,
      startIndex: 120,
      endIndex: 240,
      paddingTop: 8640,
      paddingBottom: 2800,
      anchorTopDeltaPx: 0,
      changedMeasuredRowCount: 0,
      heightDeltaAboveViewportPx: 0,
      heightDeltaInsideViewportPx: 0,
      heightDeltaBelowViewportPx: 0
    });
    diagnostics = recordTimelineScrollRangeCommit(diagnostics);
    diagnostics = recordTimelineScrollHeightCommit(diagnostics, "idleFlush");
    diagnostics = recordTimelineScrollMeasurementFlush(diagnostics, 3);
    diagnostics = recordTimelineScrollWrite(diagnostics, "measurementFlush");
    diagnostics = recordTimelineScrollEstimate(diagnostics, {
      rowKind: "media",
      estimatedPx: 72,
      measuredPx: 180
    });

    expect(timelineScrollDiagnosticTokens(diagnostics)).toEqual([
      "timeline_scroll_commits=1",
      "timeline_scroll_frames=1",
      "timeline_scroll_active_frames=1",
      "timeline_scroll_range_commits=1",
      "timeline_scroll_height_commits=1",
      "timeline_scroll_flushes=1",
      "timeline_scroll_pending_heights=0",
      "timeline_scroll_writes=1",
      "timeline_scroll_max_anchor_delta_px=0",
      "timeline_scroll_media_estimate_error_px=108"
    ]);

    const serialized = JSON.stringify(diagnostics);
    expect(serialized).not.toContain("!room");
    expect(serialized).not.toContain("$event");
    expect(serialized).not.toContain("@user");
    expect(serialized).not.toContain("mxc://");
    expect(serialized).not.toContain("message body");
  });
});
```

- [ ] **Step 2: Run the diagnostics model test and verify it fails**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/timelineScrollDiagnostics.test.ts
```

Expected: FAIL because `src/domain/timelineScrollDiagnostics.ts` does not exist.

- [ ] **Step 3: Implement the diagnostics model**

Create `apps/desktop/src/domain/timelineScrollDiagnostics.ts`:

```ts
export type TimelineScrollActivity = "idle" | "active";

export type TimelineViewportIntentKind =
  | "freeScroll"
  | "liveEdge"
  | "anchored"
  | "target"
  | "startupRestore"
  | "roomRestore"
  | "backfillCompensation"
  | "measurementFlush";

export type TimelineScrollWriteReason =
  | "liveEdge"
  | "jumpToEvent"
  | "jumpToBottom"
  | "roomRestore"
  | "backfillCompensation"
  | "measurementFlush";

export type TimelineScrollHeightCommitReason = "initial" | "idleFlush" | "timelineReset";

export type TimelineScrollRowKind =
  | "text"
  | "formatted"
  | "reply"
  | "reactionReceiptThread"
  | "linkPreview"
  | "media"
  | "redactedSystem";

export interface TimelineScrollFrameSample {
  scrollActivity: TimelineScrollActivity;
  viewportIntent: TimelineViewportIntentKind;
  userInputPending: boolean;
  virtualized: boolean;
  startIndex: number;
  endIndex: number;
  paddingTop: number;
  paddingBottom: number;
  changedMeasuredRowCount: number;
  heightDeltaAboveViewportPx: number;
  heightDeltaInsideViewportPx: number;
  heightDeltaBelowViewportPx: number;
  anchorTopDeltaPx: number;
}

export interface TimelineScrollEstimateSample {
  rowKind: TimelineScrollRowKind;
  estimatedPx: number;
  measuredPx: number;
}

export interface TimelineScrollEstimateBucket {
  count: number;
  maxAbsErrorPx: number;
  totalAbsErrorPx: number;
}

export interface TimelineScrollDiagnostics {
  renderCommits: number;
  scrollFrames: number;
  activeScrollFrames: number;
  rangeCommits: number;
  heightModelCommits: number;
  measurementFlushes: number;
  pendingMeasuredRows: number;
  scrollWrites: Record<TimelineScrollWriteReason, number>;
  maxAnchorTopDeltaPx: number;
  latestFrame: TimelineScrollFrameSample | null;
  estimateBuckets: Record<TimelineScrollRowKind, TimelineScrollEstimateBucket>;
}

const WRITE_REASONS: readonly TimelineScrollWriteReason[] = [
  "liveEdge",
  "jumpToEvent",
  "jumpToBottom",
  "roomRestore",
  "backfillCompensation",
  "measurementFlush"
];

const ROW_KINDS: readonly TimelineScrollRowKind[] = [
  "text",
  "formatted",
  "reply",
  "reactionReceiptThread",
  "linkPreview",
  "media",
  "redactedSystem"
];

function emptyWriteCounts(): Record<TimelineScrollWriteReason, number> {
  return Object.fromEntries(WRITE_REASONS.map((reason) => [reason, 0])) as Record<
    TimelineScrollWriteReason,
    number
  >;
}

function emptyEstimateBuckets(): Record<TimelineScrollRowKind, TimelineScrollEstimateBucket> {
  return Object.fromEntries(
    ROW_KINDS.map((kind) => [
      kind,
      {
        count: 0,
        maxAbsErrorPx: 0,
        totalAbsErrorPx: 0
      }
    ])
  ) as Record<TimelineScrollRowKind, TimelineScrollEstimateBucket>;
}

export function createInitialTimelineScrollDiagnostics(): TimelineScrollDiagnostics {
  return {
    renderCommits: 0,
    scrollFrames: 0,
    activeScrollFrames: 0,
    rangeCommits: 0,
    heightModelCommits: 0,
    measurementFlushes: 0,
    pendingMeasuredRows: 0,
    scrollWrites: emptyWriteCounts(),
    maxAnchorTopDeltaPx: 0,
    latestFrame: null,
    estimateBuckets: emptyEstimateBuckets()
  };
}

export function recordTimelineScrollCommit(
  diagnostics: TimelineScrollDiagnostics
): TimelineScrollDiagnostics {
  return { ...diagnostics, renderCommits: diagnostics.renderCommits + 1 };
}

export function recordTimelineScrollRangeCommit(
  diagnostics: TimelineScrollDiagnostics
): TimelineScrollDiagnostics {
  return { ...diagnostics, rangeCommits: diagnostics.rangeCommits + 1 };
}

export function recordTimelineScrollHeightCommit(
  diagnostics: TimelineScrollDiagnostics,
  _reason: TimelineScrollHeightCommitReason
): TimelineScrollDiagnostics {
  return { ...diagnostics, heightModelCommits: diagnostics.heightModelCommits + 1 };
}

export function recordTimelineScrollMeasurementFlush(
  diagnostics: TimelineScrollDiagnostics,
  changedRows: number
): TimelineScrollDiagnostics {
  return {
    ...diagnostics,
    measurementFlushes: diagnostics.measurementFlushes + 1,
    pendingMeasuredRows: Math.max(0, diagnostics.pendingMeasuredRows - changedRows)
  };
}

export function recordTimelineScrollWrite(
  diagnostics: TimelineScrollDiagnostics,
  reason: TimelineScrollWriteReason
): TimelineScrollDiagnostics {
  return {
    ...diagnostics,
    scrollWrites: {
      ...diagnostics.scrollWrites,
      [reason]: diagnostics.scrollWrites[reason] + 1
    }
  };
}

export function recordTimelineScrollFrame(
  diagnostics: TimelineScrollDiagnostics,
  sample: TimelineScrollFrameSample
): TimelineScrollDiagnostics {
  return {
    ...diagnostics,
    scrollFrames: diagnostics.scrollFrames + 1,
    activeScrollFrames:
      diagnostics.activeScrollFrames + (sample.scrollActivity === "active" ? 1 : 0),
    pendingMeasuredRows: Math.max(
      diagnostics.pendingMeasuredRows,
      sample.changedMeasuredRowCount
    ),
    maxAnchorTopDeltaPx: Math.max(
      diagnostics.maxAnchorTopDeltaPx,
      Math.abs(Math.round(sample.anchorTopDeltaPx))
    ),
    latestFrame: {
      ...sample,
      startIndex: Math.max(0, Math.trunc(sample.startIndex)),
      endIndex: Math.max(0, Math.trunc(sample.endIndex)),
      paddingTop: Math.max(0, Math.round(sample.paddingTop)),
      paddingBottom: Math.max(0, Math.round(sample.paddingBottom))
    }
  };
}

export function recordTimelineScrollEstimate(
  diagnostics: TimelineScrollDiagnostics,
  sample: TimelineScrollEstimateSample
): TimelineScrollDiagnostics {
  const absError = Math.abs(Math.round(sample.measuredPx - sample.estimatedPx));
  const bucket = diagnostics.estimateBuckets[sample.rowKind];
  return {
    ...diagnostics,
    estimateBuckets: {
      ...diagnostics.estimateBuckets,
      [sample.rowKind]: {
        count: bucket.count + 1,
        maxAbsErrorPx: Math.max(bucket.maxAbsErrorPx, absError),
        totalAbsErrorPx: bucket.totalAbsErrorPx + absError
      }
    }
  };
}

export function timelineScrollDiagnosticTokens(
  diagnostics: TimelineScrollDiagnostics
): string[] {
  const totalWrites = Object.values(diagnostics.scrollWrites).reduce(
    (sum, value) => sum + value,
    0
  );
  return [
    `timeline_scroll_commits=${diagnostics.renderCommits}`,
    `timeline_scroll_frames=${diagnostics.scrollFrames}`,
    `timeline_scroll_active_frames=${diagnostics.activeScrollFrames}`,
    `timeline_scroll_range_commits=${diagnostics.rangeCommits}`,
    `timeline_scroll_height_commits=${diagnostics.heightModelCommits}`,
    `timeline_scroll_flushes=${diagnostics.measurementFlushes}`,
    `timeline_scroll_pending_heights=${diagnostics.pendingMeasuredRows}`,
    `timeline_scroll_writes=${totalWrites}`,
    `timeline_scroll_max_anchor_delta_px=${diagnostics.maxAnchorTopDeltaPx}`,
    `timeline_scroll_media_estimate_error_px=${diagnostics.estimateBuckets.media.maxAbsErrorPx}`
  ];
}
```

- [ ] **Step 4: Run the diagnostics model test and verify it passes**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/timelineScrollDiagnostics.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/domain/timelineScrollDiagnostics.ts \
  apps/desktop/src/domain/timelineScrollDiagnostics.test.ts
git commit -m "test: add timeline scroll diagnostics model"
```

## Task 2: Wire Scroll Diagnostics Into TimelineView And Harness

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/test/harnessMain.tsx`

- [ ] **Step 1: Write the failing component instrumentation test**

In `apps/desktop/src/components/TimelineView.test.tsx`, add this test near other diagnostics tests:

```ts
it("emits private-data-free scroll diagnostics for the mounted timeline", async () => {
  const onScrollDiagnosticsChange = vi.fn();
  let listener: ((payload: CoreEventPayload) => void) | null = null;
  const transport = baseTransport({
    listenCoreEvents(nextListener) {
      listener = nextListener;
      return () => undefined;
    }
  });

  render(
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      onReply={() => undefined}
      onScrollDiagnosticsChange={onScrollDiagnosticsChange}
    />
  );

  act(() => {
    listener?.({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: Array.from({ length: 700 }, (_, index) =>
            message(`$item${index}`, `message ${index}`)
          )
        }
      }
    });
  });

  await waitFor(() => expect(onScrollDiagnosticsChange).toHaveBeenCalled());
  const latest = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
  expect(latest.renderCommits).toBeGreaterThan(0);
  expect(JSON.stringify(latest)).not.toContain("!room:example.invalid");
  expect(JSON.stringify(latest)).not.toContain("$item");
});
```

- [ ] **Step 2: Run the component instrumentation test and verify it fails**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx -t "emits private-data-free scroll diagnostics"
```

Expected: FAIL because `TimelineView` does not accept `onScrollDiagnosticsChange`.

- [ ] **Step 3: Add the `TimelineView` diagnostics prop and counters**

In `apps/desktop/src/components/TimelineView.tsx`, import the diagnostics model:

```ts
import {
  createInitialTimelineScrollDiagnostics,
  recordTimelineScrollCommit,
  recordTimelineScrollFrame,
  recordTimelineScrollHeightCommit,
  recordTimelineScrollRangeCommit,
  type TimelineScrollDiagnostics,
  type TimelineViewportIntentKind
} from "../domain/timelineScrollDiagnostics";
```

Add the prop to `TimelineView`:

```ts
  onScrollDiagnosticsChange?: (diagnostics: TimelineScrollDiagnostics) => void;
```

Destructure it:

```ts
  onScrollDiagnosticsChange,
```

Add refs and helpers inside the component:

```ts
  const scrollDiagnosticsRef = useRef<TimelineScrollDiagnostics>(
    createInitialTimelineScrollDiagnostics()
  );

  const emitScrollDiagnostics = useCallback(() => {
    onScrollDiagnosticsChange?.(scrollDiagnosticsRef.current);
  }, [onScrollDiagnosticsChange]);

  const updateScrollDiagnostics = useCallback(
    (update: (current: TimelineScrollDiagnostics) => TimelineScrollDiagnostics) => {
      scrollDiagnosticsRef.current = update(scrollDiagnosticsRef.current);
      emitScrollDiagnostics();
    },
    [emitScrollDiagnostics]
  );
```

Add a post-commit counter:

```ts
  useEffect(() => {
    updateScrollDiagnostics(recordTimelineScrollCommit);
  });
```

After `virtualWindow` is computed, record a sanitized frame sample in an effect:

```ts
  useEffect(() => {
    const intentKind: TimelineViewportIntentKind =
      viewportIntentRef.current.kind === "live-edge" ? "liveEdge" : "freeScroll";
    updateScrollDiagnostics((current) =>
      recordTimelineScrollFrame(current, {
        scrollActivity: "idle",
        viewportIntent: intentKind,
        userInputPending: userScrollInputPendingRef.current,
        virtualized: virtualWindow.virtualized,
        startIndex: virtualWindow.startIndex,
        endIndex: virtualWindow.endIndex,
        paddingTop: virtualWindow.paddingTop,
        paddingBottom: virtualWindow.paddingBottom,
        changedMeasuredRowCount: 0,
        heightDeltaAboveViewportPx: 0,
        heightDeltaInsideViewportPx: 0,
        heightDeltaBelowViewportPx: 0,
        anchorTopDeltaPx: 0
      })
    );
  }, [
    updateScrollDiagnostics,
    virtualWindow.endIndex,
    virtualWindow.paddingBottom,
    virtualWindow.paddingTop,
    virtualWindow.startIndex,
    virtualWindow.virtualized
  ]);
```

When the measurement effect commits `setMeasuredHeightVersion`, wrap the counter:

```ts
    updateScrollDiagnostics((current) =>
      recordTimelineScrollHeightCommit(current, "initial")
    );
    setMeasuredHeightVersion((current) => current + 1);
```

When the range state is introduced in Task 4, `recordTimelineScrollRangeCommit`
will move to the range-state setter. For this task, do not count range commits.

- [ ] **Step 4: Expose diagnostics through the Playwright harness**

In `apps/desktop/src/test/harnessMain.tsx`, import the type:

```ts
import type { TimelineScrollDiagnostics } from "../domain/timelineScrollDiagnostics";
```

Extend the window shape:

```ts
      scrollDiagnostics(): TimelineScrollDiagnostics | null;
      resetScrollDiagnostics(): void;
```

Add storage:

```ts
let latestScrollDiagnostics: TimelineScrollDiagnostics | null = null;
```

Add methods:

```ts
  scrollDiagnostics: () => latestScrollDiagnostics,
  resetScrollDiagnostics: () => {
    latestScrollDiagnostics = null;
  }
```

Pass the callback to `TimelineView`:

```tsx
    onScrollDiagnosticsChange={(diagnostics) => {
      latestScrollDiagnostics = diagnostics;
    }}
```

- [ ] **Step 5: Run the instrumentation tests and verify they pass**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/domain/timelineScrollDiagnostics.test.ts src/components/TimelineView.test.tsx -t "scroll diagnostics|timelineScrollDiagnostics"
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/domain/timelineScrollDiagnostics.ts \
  apps/desktop/src/domain/timelineScrollDiagnostics.test.ts \
  apps/desktop/src/components/TimelineView.tsx \
  apps/desktop/src/components/TimelineView.test.tsx \
  apps/desktop/src/test/harnessMain.tsx
git commit -m "test: surface timeline scroll diagnostics"
```

## Task 3: RED Active-Scroll Recomposition Test

**Files:**
- Modify: `apps/desktop/e2e/timeline-scrollback.spec.ts`

- [ ] **Step 1: Write the failing active-scroll browser test**

In `apps/desktop/e2e/timeline-scrollback.spec.ts`, add this test after `large timelines keep only the viewport window in the DOM`:

```ts
test("active scroll inside mounted overscan does not recompose the virtual window", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 1_000);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-virtualized", "true");

  await container.evaluate((node) => {
    node.scrollTop = 20_000;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await waitAnimationFrames(page, 3);

  await page.evaluate(() => window.__harness.resetScrollDiagnostics());

  await container.evaluate((node) => {
    for (let index = 0; index < 12; index += 1) {
      node.scrollTop += 4;
      node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 4 }));
      node.dispatchEvent(new Event("scroll", { bubbles: true }));
    }
  });
  await waitAnimationFrames(page, 5);

  const diagnostics = await page.evaluate(() => window.__harness.scrollDiagnostics());
  expect(diagnostics).not.toBeNull();
  expect(diagnostics?.scrollFrames ?? 0).toBeGreaterThanOrEqual(1);
  expect(diagnostics?.rangeCommits ?? 0).toBe(0);
  expect(diagnostics?.heightModelCommits ?? 0).toBe(0);
  expect(diagnostics?.renderCommits ?? 0).toBeLessThanOrEqual(1);
});
```

- [ ] **Step 2: Run the active-scroll test and verify it fails on current behavior**

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts -g "active scroll inside mounted overscan" --workers=1
```

Expected: FAIL because scroll events currently update React `viewportMetrics.scrollTop`, causing render commits even when the virtual range remains inside overscan.

Do not commit the failing test by itself. Continue to Task 4 in the same working tree.

## Task 4: Refactor Virtual Range State Away From Raw ScrollTop State

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/e2e/timeline-scrollback.spec.ts`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`

- [ ] **Step 1: Add range and raw-metric types**

In `TimelineView.tsx`, replace the `TimelineViewportMetrics` state role with explicit raw metrics and range state:

```ts
type TimelineViewportMetrics = {
  scrollTop: number;
  clientHeight: number;
  listOffsetTop: number;
};

type TimelineVirtualRangeState = {
  virtualized: boolean;
  startIndex: number;
  endIndex: number;
  paddingTop: number;
  paddingBottom: number;
};

type TimelineVirtualWindow = TimelineVirtualRangeState & {
  items: readonly TimelineItem[];
};

const EMPTY_TIMELINE_RANGE: TimelineVirtualRangeState = {
  virtualized: false,
  startIndex: 0,
  endIndex: 0,
  paddingTop: 0,
  paddingBottom: 0
};
```

Add pure helpers near `timelineIndexAtOffset`:

```ts
function virtualRangeEquals(
  left: TimelineVirtualRangeState,
  right: TimelineVirtualRangeState
): boolean {
  return (
    left.virtualized === right.virtualized &&
    left.startIndex === right.startIndex &&
    left.endIndex === right.endIndex &&
    left.paddingTop === right.paddingTop &&
    left.paddingBottom === right.paddingBottom
  );
}

function calculateTimelineVirtualRange({
  visibleItemsLength,
  metrics,
  model
}: {
  visibleItemsLength: number;
  metrics: TimelineViewportMetrics;
  model: TimelineHeightModel;
}): TimelineVirtualRangeState {
  if (visibleItemsLength <= TIMELINE_VIRTUALIZATION_THRESHOLD) {
    return {
      virtualized: false,
      startIndex: 0,
      endIndex: visibleItemsLength,
      paddingTop: 0,
      paddingBottom: 0
    };
  }

  const viewportHeight = metrics.clientHeight || 600;
  const relativeScrollTop = Math.max(0, metrics.scrollTop - metrics.listOffsetTop);
  const firstVisibleIndex = timelineIndexAtOffset(model.offsets, relativeScrollTop);
  const lastVisibleIndex = timelineIndexAtOffset(
    model.offsets,
    relativeScrollTop + viewportHeight
  );
  const startIndex = Math.max(0, firstVisibleIndex - TIMELINE_VIRTUAL_OVERSCAN_ITEMS);
  const endIndex = Math.min(
    visibleItemsLength,
    Math.max(startIndex + 1, lastVisibleIndex + TIMELINE_VIRTUAL_OVERSCAN_ITEMS + 1)
  );

  return {
    virtualized: true,
    startIndex,
    endIndex,
    paddingTop: Math.round(model.offsets[startIndex] ?? 0),
    paddingBottom: Math.round(model.totalHeight - (model.offsets[endIndex] ?? 0))
  };
}
```

- [ ] **Step 2: Replace `viewportMetrics` state with refs and range state**

Inside `TimelineView`, replace:

```ts
  const [viewportMetrics, setViewportMetrics] = useState<TimelineViewportMetrics>({
    scrollTop: 0,
    clientHeight: 0,
    listOffsetTop: 0
  });
```

with:

```ts
  const viewportMetricsRef = useRef<TimelineViewportMetrics>({
    scrollTop: 0,
    clientHeight: 0,
    listOffsetTop: 0
  });
  const [virtualRange, setVirtualRange] =
    useState<TimelineVirtualRangeState>(EMPTY_TIMELINE_RANGE);
  const pendingScrollFrameRef = useRef<number | null>(null);
```

Add metric readers:

```ts
  const readViewportMetrics = useCallback((): TimelineViewportMetrics => {
    const container = containerRef.current;
    if (!container) {
      return viewportMetricsRef.current;
    }
    const next = {
      scrollTop: container.scrollTop,
      clientHeight: container.clientHeight,
      listOffsetTop: listRef.current?.offsetTop ?? 0
    };
    viewportMetricsRef.current = next;
    return next;
  }, []);
```

Add a range committer:

```ts
  const commitVirtualRangeForMetrics = useCallback(
    (metrics: TimelineViewportMetrics) => {
      const next = calculateTimelineVirtualRange({
        visibleItemsLength: visibleItems.length,
        metrics,
        model: timelineHeightModel
      });
      setVirtualRange((current) => {
        if (virtualRangeEquals(current, next)) {
          return current;
        }
        updateScrollDiagnostics(recordTimelineScrollRangeCommit);
        return next;
      });
    },
    [timelineHeightModel, updateScrollDiagnostics, visibleItems.length]
  );
```

Replace `updateViewportMetrics` with:

```ts
  const updateViewportMetrics = useCallback(() => {
    const metrics = readViewportMetrics();
    commitVirtualRangeForMetrics(metrics);
  }, [commitVirtualRangeForMetrics, readViewportMetrics]);
```

- [ ] **Step 3: Build `virtualWindow` from range state**

Replace the `virtualWindow` `useMemo` body with:

```ts
  const virtualWindow = useMemo<TimelineVirtualWindow>(() => {
    const range =
      visibleItems.length <= TIMELINE_VIRTUALIZATION_THRESHOLD
        ? {
            virtualized: false,
            startIndex: 0,
            endIndex: visibleItems.length,
            paddingTop: 0,
            paddingBottom: 0
          }
        : virtualRange;

    return {
      ...range,
      items: visibleItems.slice(range.startIndex, range.endIndex)
    };
  }, [virtualRange, visibleItems]);
```

Add an effect to recompute range when items or height model changes:

```ts
  useLayoutEffect(() => {
    commitVirtualRangeForMetrics(readViewportMetrics());
  }, [commitVirtualRangeForMetrics, readViewportMetrics]);
```

Replace every read of `viewportMetrics.listOffsetTop` with
`viewportMetricsRef.current.listOffsetTop`. For example:

```ts
const targetScrollTop =
  viewportMetricsRef.current.listOffsetTop + anchorTop - activeRoomAnchor.offset_px;
```

- [ ] **Step 4: rAF-throttle scroll processing**

In `onTimelineScroll`, replace the direct `updateViewportMetrics()` call with:

```ts
    if (pendingScrollFrameRef.current === null) {
      pendingScrollFrameRef.current = window.requestAnimationFrame(() => {
        pendingScrollFrameRef.current = null;
        const metrics = readViewportMetrics();
        commitVirtualRangeForMetrics(metrics);
        updateScrollDiagnostics((current) =>
          recordTimelineScrollFrame(current, {
            scrollActivity: "active",
            viewportIntent:
              viewportIntentRef.current.kind === "live-edge" ? "liveEdge" : "freeScroll",
            userInputPending: userScrollInputPendingRef.current,
            virtualized: virtualWindow.virtualized,
            startIndex: virtualWindow.startIndex,
            endIndex: virtualWindow.endIndex,
            paddingTop: virtualWindow.paddingTop,
            paddingBottom: virtualWindow.paddingBottom,
            changedMeasuredRowCount: 0,
            heightDeltaAboveViewportPx: 0,
            heightDeltaInsideViewportPx: 0,
            heightDeltaBelowViewportPx: 0,
            anchorTopDeltaPx: 0
          })
        );
      });
    }
```

On component cleanup, cancel `pendingScrollFrameRef.current` if present.

- [ ] **Step 5: Run the active-scroll test and existing scrollback tests**

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts -g "active scroll inside mounted overscan|large timelines keep only the viewport window in the DOM|initial timeline load and remount start at the live edge" --workers=1
```

Expected: PASS.

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/components/TimelineView.tsx \
  apps/desktop/src/components/TimelineView.test.tsx \
  apps/desktop/e2e/timeline-scrollback.spec.ts
git commit -m "perf: decouple timeline scroll range from scrollTop state"
```

## Task 5: Deferred Measurement RED Test And Idle Flush

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`

- [ ] **Step 1: Write the failing deferred-measurement test**

In `TimelineView.test.tsx`, add this test:

```ts
it("defers virtual height commits during active scroll and flushes once after idle", async () => {
  vi.useFakeTimers();
  let listener: ((payload: CoreEventPayload) => void) | null = null;
  const onScrollDiagnosticsChange = vi.fn();
  const transport = baseTransport({
    listenCoreEvents(nextListener) {
      listener = nextListener;
      return () => undefined;
    }
  });

  const rects: Record<string, { top: number; height: number }> = {};
  for (let index = 0; index < 700; index += 1) {
    rects[`$item${index}`] = { top: index * 72, height: 72 };
  }
  const scrollContainerRef: { current: HTMLElement | null } = { current: null };
  mockTimelineRects(rects, { top: 0, height: 600 }, scrollContainerRef);

  render(
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      onReply={() => undefined}
      onScrollDiagnosticsChange={onScrollDiagnosticsChange}
      listRefCallback={(element) => {
        scrollContainerRef.current =
          element?.closest<HTMLElement>("[data-testid=timeline-view]") ?? null;
      }}
    />
  );

  act(() => {
    listener?.({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: Array.from({ length: 700 }, (_, index) =>
            message(`$item${index}`, `message ${index}`)
          )
        }
      }
    });
  });

  const timeline = await screen.findByTestId("timeline-view");
  Object.defineProperty(timeline, "scrollTop", { value: 3000, writable: true, configurable: true });
  Object.defineProperty(timeline, "scrollHeight", {
    value: 700 * 72,
    writable: true,
    configurable: true
  });
  Object.defineProperty(timeline, "clientHeight", { value: 600, writable: true, configurable: true });

  fireEvent.wheel(timeline, { deltaY: 40 });
  fireEvent.scroll(timeline);

  rects.$item50 = { top: 50 * 72, height: 180 };
  fireEvent.scroll(timeline);
  await act(async () => {
    await Promise.resolve();
  });

  const activeDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
  expect(activeDiagnostics.heightModelCommits).toBe(0);
  expect(activeDiagnostics.pendingMeasuredRows).toBeGreaterThan(0);

  act(() => {
    vi.advanceTimersByTime(100);
  });

  const idleDiagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
  expect(idleDiagnostics.measurementFlushes).toBe(1);
  expect(idleDiagnostics.heightModelCommits).toBe(1);
});
```

- [ ] **Step 2: Run the deferred-measurement test and verify it fails**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx -t "defers virtual height commits"
```

Expected: FAIL because the current measurement layout effect commits measured heights immediately.

- [ ] **Step 3: Add active-scroll tracking constants and refs**

In `TimelineView.tsx`, add constants near the virtualizer constants:

```ts
const TIMELINE_SCROLL_IDLE_FLUSH_MS = 100;
const TIMELINE_SCROLL_MAX_DEFER_MS = 500;
```

Add refs inside the component:

```ts
  const scrollActivityRef = useRef<"idle" | "active">("idle");
  const scrollIdleTimerRef = useRef<number | null>(null);
  const scrollMaxDeferTimerRef = useRef<number | null>(null);
  const pendingMeasuredHeightsRef = useRef<Map<string, number>>(new Map());
```

Add timer cleanup:

```ts
  const clearMeasurementTimers = useCallback(() => {
    if (scrollIdleTimerRef.current !== null) {
      window.clearTimeout(scrollIdleTimerRef.current);
      scrollIdleTimerRef.current = null;
    }
    if (scrollMaxDeferTimerRef.current !== null) {
      window.clearTimeout(scrollMaxDeferTimerRef.current);
      scrollMaxDeferTimerRef.current = null;
    }
  }, []);
```

- [ ] **Step 4: Add an idle measurement flush function**

In `TimelineView.tsx`, add:

```ts
  const flushPendingMeasurements = useCallback(
    (reason: "idle" | "maxDefer") => {
      const pending = pendingMeasuredHeightsRef.current;
      if (pending.size === 0) {
        clearMeasurementTimers();
        scrollActivityRef.current = "idle";
        return;
      }

      const nextHeights = new Map(itemHeightByDomIdRef.current);
      let changedRows = 0;
      for (const [domId, height] of pending) {
        if (Math.abs((nextHeights.get(domId) ?? 0) - height) > 1) {
          nextHeights.set(domId, height);
          changedRows += 1;
        }
      }
      pending.clear();
      clearMeasurementTimers();
      scrollActivityRef.current = "idle";

      if (changedRows === 0) {
        return;
      }

      const container = containerRef.current;
      const measuredAtBottom = Boolean(container && isScrolledToBottom(container));
      stickToBottomAfterMeasurementRef.current = measuredAtBottom;
      if (measuredAtBottom) {
        setViewportIntentToLiveEdge();
      }

      itemHeightByDomIdRef.current = nextHeights;
      updateScrollDiagnostics((current) =>
        recordTimelineScrollMeasurementFlush(
          recordTimelineScrollHeightCommit(current, "idleFlush"),
          changedRows
        )
      );
      setMeasuredHeightVersion((current) => current + 1);

      if (reason === "maxDefer") {
        emitDiagnosticLog("timeline.scroll", "measurement flush reason=max_defer");
      }
    },
    [
      clearMeasurementTimers,
      emitDiagnosticLog,
      setViewportIntentToLiveEdge,
      updateScrollDiagnostics
    ]
  );
```

- [ ] **Step 5: Schedule measurement flushing from scroll activity**

Add:

```ts
  const markScrollActivityActive = useCallback(() => {
    scrollActivityRef.current = "active";
    if (scrollIdleTimerRef.current !== null) {
      window.clearTimeout(scrollIdleTimerRef.current);
    }
    scrollIdleTimerRef.current = window.setTimeout(
      () => flushPendingMeasurements("idle"),
      TIMELINE_SCROLL_IDLE_FLUSH_MS
    );
    if (scrollMaxDeferTimerRef.current === null) {
      scrollMaxDeferTimerRef.current = window.setTimeout(
        () => flushPendingMeasurements("maxDefer"),
        TIMELINE_SCROLL_MAX_DEFER_MS
      );
    }
  }, [flushPendingMeasurements]);
```

Call `markScrollActivityActive()` in `onTimelineScroll` before scheduling rAF.

- [ ] **Step 6: Defer measurement commits while active**

In the measurement `useLayoutEffect`, replace the immediate assignment block:

```ts
    itemHeightByDomIdRef.current = nextHeights;
    setMeasuredHeightVersion((current) => current + 1);
```

with:

```ts
    if (scrollActivityRef.current === "active") {
      for (const [domId, height] of nextHeights) {
        if (Math.abs((itemHeightByDomIdRef.current.get(domId) ?? 0) - height) > 1) {
          pendingMeasuredHeightsRef.current.set(domId, height);
        }
      }
      updateScrollDiagnostics((current) =>
        recordTimelineScrollFrame(current, {
          scrollActivity: "active",
          viewportIntent:
            viewportIntentRef.current.kind === "live-edge" ? "liveEdge" : "freeScroll",
          userInputPending: userScrollInputPendingRef.current,
          virtualized: virtualWindow.virtualized,
          startIndex: virtualWindow.startIndex,
          endIndex: virtualWindow.endIndex,
          paddingTop: virtualWindow.paddingTop,
          paddingBottom: virtualWindow.paddingBottom,
          changedMeasuredRowCount: pendingMeasuredHeightsRef.current.size,
          heightDeltaAboveViewportPx: 0,
          heightDeltaInsideViewportPx: 0,
          heightDeltaBelowViewportPx: 0,
          anchorTopDeltaPx: 0
        })
      );
      return;
    }

    itemHeightByDomIdRef.current = nextHeights;
    updateScrollDiagnostics((current) =>
      recordTimelineScrollHeightCommit(current, "initial")
    );
    setMeasuredHeightVersion((current) => current + 1);
```

On timeline key reset, clear `pendingMeasuredHeightsRef.current` and call
`clearMeasurementTimers()`.

- [ ] **Step 7: Run deferred measurement checks**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx -t "defers virtual height commits"
```

Expected: PASS.

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts -g "active scroll inside mounted overscan" --workers=1
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add apps/desktop/src/components/TimelineView.tsx \
  apps/desktop/src/components/TimelineView.test.tsx
git commit -m "fix: defer timeline measurement during active scroll"
```

## Task 6: Reason-Tagged Scroll Writes

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`

- [ ] **Step 1: Write the failing scroll-write reason test**

In `TimelineView.test.tsx`, add:

```ts
it("classifies programmatic scroll writes by reason and suppresses their scroll echo", async () => {
  const onScrollDiagnosticsChange = vi.fn();
  let listener: ((payload: CoreEventPayload) => void) | null = null;
  const transport = baseTransport({
    listenCoreEvents(nextListener) {
      listener = nextListener;
      return () => undefined;
    }
  });

  render(
    <TimelineView
      timelineKey={KEY}
      roomId="!room:example.invalid"
      transport={transport}
      onReply={() => undefined}
      onScrollDiagnosticsChange={onScrollDiagnosticsChange}
    />
  );

  act(() => {
    listener?.({
      kind: "Timeline",
      event: {
        InitialItems: {
          request_id: null,
          key: KEY,
          generation: 1,
          items: Array.from({ length: 700 }, (_, index) =>
            message(`$item${index}`, `message ${index}`)
          )
        }
      }
    });
  });

  const timeline = await screen.findByTestId("timeline-view");
  Object.defineProperty(timeline, "scrollTop", { value: 1000, writable: true, configurable: true });
  Object.defineProperty(timeline, "scrollHeight", { value: 700 * 72, configurable: true });
  Object.defineProperty(timeline, "clientHeight", { value: 600, configurable: true });

  act(() => {
    listener?.({
      kind: "Timeline",
      event: {
        NavigationUpdated: {
          key: KEY,
          snapshot: {
            can_jump_to_bottom: true,
            first_unread_event_id: null,
            newer_event_count: 4,
            read_marker_display_event_id: null,
            read_marker_event_id: null,
            unread_event_count: 0,
            unread_position: "none"
          }
        }
      }
    });
  });

  fireEvent.click(screen.getByRole("button", { name: /Jump to bottom/ }));
  fireEvent.scroll(timeline);

  const diagnostics = onScrollDiagnosticsChange.mock.calls.at(-1)?.[0];
  expect(diagnostics.scrollWrites.jumpToBottom).toBe(1);
  expect(diagnostics.latestFrame?.userInputPending).toBe(false);
});
```

- [ ] **Step 2: Run the scroll-write reason test and verify it fails**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx -t "classifies programmatic scroll writes"
```

Expected: FAIL because programmatic scroll writes are not reason-tagged.

- [ ] **Step 3: Replace signature-only suppression with reason tokens**

In `TimelineView.tsx`, import:

```ts
  recordTimelineScrollWrite,
  type TimelineScrollWriteReason
```

Replace `programmaticScrollSignatureRef` with:

```ts
  const programmaticScrollSignatureRef = useRef<{
    scrollHeight: number;
    scrollTop: number;
    reason: TimelineScrollWriteReason;
    token: number;
  } | null>(null);
  const programmaticScrollTokenRef = useRef(0);
```

Replace `runWithSuppressedScrollAnchorCapture` with:

```ts
  const runWithScrollWriteReason = useCallback(
    (reason: TimelineScrollWriteReason, action: () => void) => {
      const asyncGeneration = anchorAsyncGenerationRef.current;
      suppressScrollAnchorCaptureRef.current = true;
      const container = containerRef.current;
      const beforeScrollTop = container?.scrollTop ?? 0;
      action();
      if (container && container.scrollTop !== beforeScrollTop) {
        const token = programmaticScrollTokenRef.current + 1;
        programmaticScrollTokenRef.current = token;
        programmaticScrollSignatureRef.current = {
          scrollHeight: container.scrollHeight,
          scrollTop: container.scrollTop,
          reason,
          token
        };
        updateScrollDiagnostics((current) => recordTimelineScrollWrite(current, reason));
      }
      requestAnimationFrame(() => {
        if (anchorAsyncGenerationRef.current !== asyncGeneration) {
          return;
        }
        suppressScrollAnchorCaptureRef.current = false;
        programmaticScrollSignatureRef.current = null;
      });
    },
    [updateScrollDiagnostics]
  );
```

Replace call sites:

```ts
runWithScrollWriteReason("liveEdge", () => {
  container.scrollTop = targetScrollTop;
});

runWithScrollWriteReason("roomRestore", () => {
  container.scrollTop = Math.max(0, targetScrollTop);
});

runWithScrollWriteReason("backfillCompensation", () => {
  restoreAnchor(container, anchor);
});

runWithScrollWriteReason("jumpToEvent", () => {
  row.scrollIntoView({ block: "center", inline: "nearest" });
});

runWithScrollWriteReason("jumpToBottom", () => {
  scrollContainerToBottom(container);
});

runWithScrollWriteReason("measurementFlush", () => {
  container.scrollBy(0, topDiff);
});
```

For call sites that currently use `runWithSuppressedScrollAnchorCapture`, choose the reason matching the branch. Do not add a generic catch-all reason.

- [ ] **Step 4: Keep echo classification deterministic**

In `onTimelineScroll`, keep the current fuzzy scrollTop/scrollHeight echo check but include `sig.reason` in diagnostics and ensure user input is not consumed for an echo:

```ts
    const isProgrammaticEcho =
      sig !== null &&
      container !== null &&
      Math.abs(container.scrollTop - sig.scrollTop) <= SCROLL_EDGE_TOLERANCE_PX &&
      Math.abs(container.scrollHeight - sig.scrollHeight) <= SCROLL_EDGE_TOLERANCE_PX;
    const isUserDrivenScroll = userScrollInputPendingRef.current && !isProgrammaticEcho;
```

When recording frame diagnostics, set:

```ts
userInputPending: isUserDrivenScroll
```

- [ ] **Step 5: Run tests**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx -t "classifies programmatic scroll writes|defers virtual height commits"
```

Expected: PASS.

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts -g "active scroll inside mounted overscan|prevents repeated auto-backfill while restoring the prepend anchor" --workers=1
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/components/TimelineView.tsx \
  apps/desktop/src/components/TimelineView.test.tsx
git commit -m "fix: tag timeline scroll writes by reason"
```

## Task 7: Known-Dimension Media Height Reservation

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/components/TimelineView.test.tsx`
- Modify: `apps/desktop/src/styles.css`
- Modify: `apps/desktop/e2e/timeline-scrollback.spec.ts`

- [ ] **Step 1: Write helper unit tests for media box sizing**

In `TimelineView.test.tsx`, add:

```ts
it("computes a stable clamped media box for known image dimensions", () => {
  expect(timelineMediaDisplayBoxForTests(2048, 1188)).toEqual({
    inlineSize: 420,
    blockSize: 244
  });
  expect(timelineMediaDisplayBoxForTests(800, 1600)).toEqual({
    inlineSize: 130,
    blockSize: 260
  });
  expect(timelineMediaDisplayBoxForTests(null, 1600)).toBeNull();
  expect(timelineMediaDisplayBoxForTests(800, null)).toBeNull();
});
```

- [ ] **Step 2: Add the test helper to the existing TimelineView import**

In `TimelineView.test.tsx`, update the existing import from `./TimelineView`:

```ts
import {
  MessageSourceDialog,
  TimelineView,
  clearTimelineViewportSessionMemoryForTests,
  timelineMediaDisplayBoxForTests,
  type TimelineTransport
} from "./TimelineView";
```

- [ ] **Step 3: Run the helper test and verify it fails**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx -t "computes a stable clamped media box"
```

Expected: FAIL because `timelineMediaDisplayBoxForTests` does not exist.

- [ ] **Step 4: Add the media display box helper**

In the React import list in `TimelineView.tsx`, add `type CSSProperties`:

```ts
  type CSSProperties,
  type Dispatch,
  type SetStateAction
```

In `TimelineView.tsx`, near `formatDimensions`, add:

```ts
const TIMELINE_MEDIA_MAX_INLINE_PX = 420;
const TIMELINE_MEDIA_MAX_BLOCK_PX = 260;

function timelineMediaDisplayBox(
  width: number | null | undefined,
  height: number | null | undefined
): { inlineSize: number; blockSize: number } | null {
  if (!width || !height || width <= 0 || height <= 0) {
    return null;
  }
  const scale = Math.min(
    TIMELINE_MEDIA_MAX_INLINE_PX / width,
    TIMELINE_MEDIA_MAX_BLOCK_PX / height,
    1
  );
  return {
    inlineSize: Math.round(width * scale),
    blockSize: Math.round(height * scale)
  };
}

export const timelineMediaDisplayBoxForTests = timelineMediaDisplayBox;
```

- [ ] **Step 5: Render a stable media frame before and after ready state**

In `TimelineMediaAttachment`, compute:

```ts
  const displayBox = timelineMediaDisplayBox(media.width, media.height);
  const readyDisplayBox =
    downloadState?.kind === "ready"
      ? timelineMediaDisplayBox(downloadState.width ?? media.width, downloadState.height ?? media.height)
      : displayBox;
  const mediaFrameStyle =
    readyDisplayBox === null
      ? undefined
      : ({
          inlineSize: `${readyDisplayBox.inlineSize}px`,
          blockSize: `${readyDisplayBox.blockSize}px`
        } satisfies CSSProperties);
```

For the ready image branch, wrap the image:

```tsx
<span className="message-media-image-frame" style={mediaFrameStyle}>
  <img
    className="message-media-image"
    src={sourceUrl}
    alt={media.filename}
    width={downloadState.width ?? undefined}
    height={downloadState.height ?? undefined}
    loading="lazy"
  />
</span>
```

For non-ready image media with known dimensions, render the same frame before the icon/metadata:

```tsx
{media.kind === "Image" && displayBox ? (
  <span
    className="message-media-image-frame message-media-image-frame-reserved"
    style={mediaFrameStyle}
    aria-hidden="true"
  >
    <Icon className="message-media-icon" size={22} aria-hidden="true" />
  </span>
) : (
  <Icon className="message-media-icon" size={18} aria-hidden="true" />
)}
```

Keep filename and download controls in the same `message-media` row. The frame is visual reservation only and must not expose filenames or MXC values in diagnostics.

- [ ] **Step 6: Add CSS for the stable image frame**

In `styles.css`, update the media rules:

```css
.message-media-image-frame {
  display: inline-grid;
  place-items: center;
  overflow: hidden;
  max-inline-size: min(100%, 420px);
  max-block-size: 260px;
  border-radius: 6px;
  background: var(--surface);
}

.message-media-image-frame-reserved {
  color: var(--text-muted);
  border: 1px dashed var(--line);
}

.message-media-image-frame .message-media-image {
  inline-size: 100%;
  block-size: 100%;
}
```

Keep the existing `.message-media-image` object-fit and border-radius behavior:

```css
.message-media-image {
  max-width: min(100%, 420px);
  max-height: 260px;
  object-fit: contain;
  border-radius: 6px;
  background: var(--surface);
}
```

- [ ] **Step 7: Add the Playwright media stability test**

In `timeline-scrollback.spec.ts`, add:

```ts
function makeImageItem(id: string) {
  return {
    ...makeItem(id, "image body"),
    media: {
      kind: "Image",
      filename: "synthetic.png",
      source: {
        mxc_uri: "mxc://example.invalid/synthetic",
        encrypted: false,
        encryption_version: null
      },
      mimetype: "image/png",
      size: 12345,
      width: 2048,
      height: 1188,
      thumbnail: null
    }
  };
}

test("known-dimension media keeps row height stable across download completion", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 650 }, (_, index) =>
        index === 620 ? makeImageItem("$image620") : makeItem(`$m${index}`, `message ${index}`)
      )
    }
  );

  const frame = page.locator('[data-frame-item-id="$image620"]');
  await expect(frame).toBeVisible();
  const beforeHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);

  await page.evaluate(({ key }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        MediaDownloadCompleted: {
          request_id: "media-request",
          key,
          event_id: "$image620",
          source_url: "appmedia://synthetic-image",
          byte_count: 12345,
          mimetype: "image/png",
          width: 2048,
          height: 1188
        }
      }
    } as any);
  }, { key: timelineKey() });

  await waitAnimationFrames(page, 3);
  const afterHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);
  expect(Math.abs(afterHeight - beforeHeight)).toBeLessThanOrEqual(1);
});
```

- [ ] **Step 8: Run media tests**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx -t "stable clamped media box"
```

Expected: PASS.

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts -g "known-dimension media keeps row height stable" --workers=1
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add apps/desktop/src/components/TimelineView.tsx \
  apps/desktop/src/components/TimelineView.test.tsx \
  apps/desktop/src/styles.css \
  apps/desktop/e2e/timeline-scrollback.spec.ts
git commit -m "fix: reserve known media height in timeline"
```

## Task 8: Browser Scroll Anchoring Policy

**Files:**
- Modify: `apps/desktop/src/styles.css`
- Modify: `apps/desktop/src/styles.contract.test.ts`
- Modify: `apps/desktop/e2e/timeline-scrollback.spec.ts`

- [ ] **Step 1: Write the style contract test for one anchoring owner**

In `apps/desktop/src/styles.contract.test.ts`, update the existing overflow-anchor test to assert:

```ts
test("timeline uses Koushi-owned event anchoring rather than browser scroll anchoring", () => {
  const timelineBlock = selectorBlock(".timeline-view");
  const spacerBlock = selectorBlock(".timeline-virtual-spacer");

  expect(timelineBlock).toContain("overflow-anchor: none;");
  expect(spacerBlock).toContain("overflow-anchor: none;");
});
```

- [ ] **Step 2: Run the style contract test and verify it fails**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/styles.contract.test.ts -t "timeline uses Koushi-owned event anchoring"
```

Expected: FAIL because `.timeline-view` currently uses `overflow-anchor: auto`.

- [ ] **Step 3: Set the timeline scroller policy**

In `styles.css`, change:

```css
.timeline-view {
  min-inline-size: 0;
  max-inline-size: 100%;
  overflow-anchor: auto;
  overflow-x: hidden;
}
```

to:

```css
.timeline-view {
  min-inline-size: 0;
  max-inline-size: 100%;
  overflow-anchor: none;
  overflow-x: hidden;
}
```

Keep:

```css
.timeline-virtual-spacer {
  inline-size: 100%;
  overflow-anchor: none;
  pointer-events: none;
}
```

- [ ] **Step 4: Add a controlled row-growth browser test**

In `timeline-scrollback.spec.ts`, add:

```ts
test("manual anchor correction is the only row-growth correction path", async ({ page }) => {
  await page.goto("/harness.html?variableHeights=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 1_000);

  const container = page.locator("[data-testid=timeline-view]");
  await container.evaluate((node) => {
    node.scrollTop = 20_000;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await waitAnimationFrames(page, 3);

  const anchor = page.locator('[data-item-id="$m420"]');
  await expect(anchor).toBeVisible();
  const beforeTop = await anchor.evaluate((node) => node.getBoundingClientRect().top);

  await page.addStyleTag({
    content: `
      [data-frame-item-id="$m410"] .message-body::after {
        content: "";
        display: block;
        block-size: 96px;
      }
    `
  });
  await waitAnimationFrames(page, 5);

  const afterTop = await anchor.evaluate((node) => node.getBoundingClientRect().top);
  expect(Math.abs(afterTop - beforeTop)).toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
});
```

- [ ] **Step 5: Run anchoring tests**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/styles.contract.test.ts -t "timeline uses Koushi-owned event anchoring"
```

Expected: PASS.

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts -g "manual anchor correction is the only row-growth correction path" --workers=1
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/styles.css \
  apps/desktop/src/styles.contract.test.ts \
  apps/desktop/e2e/timeline-scrollback.spec.ts
git commit -m "fix: make timeline event anchoring explicit"
```

## Task 9: Final Regression Sweep And Documentation Sync

**Files:**
- Modify: `docs/superpowers/specs/2026-06-30-timeline-native-smooth-scroll-design.md`

- [ ] **Step 1: Update the spec status after implementation**

Change the spec header from:

```md
Status: written design pending user review
```

to:

```md
Status: implemented
```

Do this only after Tasks 1 through 8 are implemented and the checks in this task pass.

- [ ] **Step 2: Run focused unit tests**

Run:

```bash
npm --prefix apps/desktop run test -- --run \
  src/domain/timelineScrollDiagnostics.test.ts \
  src/components/TimelineView.test.tsx \
  src/styles.contract.test.ts
```

Expected: PASS.

- [ ] **Step 3: Run focused browser-headless scrollback tests**

Run:

```bash
npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts --workers=1
```

Expected: PASS.

- [ ] **Step 4: Run typecheck**

Run:

```bash
npm --prefix apps/desktop run typecheck
```

Expected: PASS.

- [ ] **Step 5: Review staged diff for private-data leaks**

Run:

```bash
rg -n "mxc://|!room|@user|message body|source_url|event_id|room_id" \
  apps/desktop/src/domain/timelineScrollDiagnostics.ts \
  apps/desktop/src/domain/timelineScrollDiagnostics.test.ts \
  apps/desktop/src/components/TimelineView.tsx \
  apps/desktop/src/components/TimelineView.test.tsx \
  apps/desktop/e2e/timeline-scrollback.spec.ts
```

Expected: matches are limited to synthetic test fixtures and existing typed command/event field names. No real account data, raw SDK errors, Matrix tokens, local paths, or message content appears in diagnostics output.

- [ ] **Step 6: Commit final docs status**

```bash
git add docs/superpowers/specs/2026-06-30-timeline-native-smooth-scroll-design.md
git commit -m "docs: mark native-smooth timeline scrolling implemented"
```

## Notes For Execution

- Use a fresh worktree or clean branch at execution time. Do not execute this plan on top of unrelated local changes in `TimelineView.tsx`, `styles.css`, or the Playwright scrollback specs.
- Preserve existing behavior around deep anchor restore, prepend restoration, live-edge startup, room-switch memory, read-signal observation, and timeline store ordering.
- Keep diagnostics private-data-free. Diagnostic strings may include counts, booleans, reason enum names, and pixel numbers only.
- Do not add TanStack Virtual or Virtuoso in this implementation pass. A remaining local-virtualizer failure after Tasks 1 through 8 requires stopping and writing a separate evaluation design using the same measurement tests as acceptance gates.
