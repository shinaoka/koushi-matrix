# Timeline Scroll Stability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge fixes for #277 and #278 so asynchronous timeline content and backward pagination cannot visibly move the user's local viewport anchor.

**Architecture:** Image and link-preview rows reserve their final outer geometry before content is ready. The virtual-height path captures a local DOM anchor before a height/prepend commit and applies only its measured relative delta in the commit's layout effect; active input releases deferred prepend and measurement work through the existing idle/max-defer timers.

**Tech Stack:** React 19, TypeScript, CSS, Vitest, Playwright Chromium.

## Global Constraints

- #277 changes only `TimelineMediaAttachment`, link-preview JSX, `styles.css`, and their tests; do not change the scroll engine for media loading.
- Do not change Matrix/Rust/Tauri product-state contracts or replace the timeline coordinate system.
- Known media metadata remains clamped to 420 by 260 pixels; missing/invalid dimensions reserve a fixed 347 by 260 pixel 4:3 box.
- Link-preview pending, ready, and failed cards have one fixed outer height.
- Free-scroll compensation uses a mounted local row and pixel delta; never derive the correction from global 72-pixel estimates.
- Active input uses the existing 100 ms idle and 500 ms maximum-defer timers.
- Near-top prefetch is two client viewports with the existing 80 pixel minimum.
- Add the browser-headless reproductions before implementation and run long integrated gates only after the coherent implementation is complete.

---

### Task 1: Stable media and link-preview boxes (#277)

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Modify: `apps/desktop/src/styles.css`
- Test: `apps/desktop/src/components/TimelineView.test.tsx`
- Test: `apps/desktop/e2e/timeline-scrollback.spec.ts`

**Interfaces:**
- Consumes: `TimelineItem.media`, `TimelineMediaDownloadState`, and Rust-projected `LinkPreview.state` entries.
- Produces: `timelineMediaDisplayBoxForTests(width, height)` which always returns `{ inlineSize, blockSize }` for image rows; stable `.message-media-figure` and `.link-preview-card` boxes.

- [ ] **Step 1: Extend geometry and real-DOM tests so missing metadata and preview transitions fail**

```ts
expect(timelineMediaDisplayBoxForTests(null, null)).toEqual({
  inlineSize: 347,
  blockSize: 260
});

test("missing-dimension media keeps row height stable across download completion", async ({ page }) => {
  // Seed an image whose event metadata has width/height null, record its frame
  // height, deliver MediaDownloadCompleted with real dimensions, and require
  // Math.abs(afterHeight - beforeHeight) <= ANCHOR_PIXEL_TOLERANCE.
});

test("pending link previews reserve the ready card height", async ({ page }) => {
  // Seed one pending preview, record the row/card height, replace it with one
  // ready preview through ItemsUpdated, and require both heights to stay equal.
});
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx
npm --prefix apps/desktop exec -- playwright test --config apps/desktop/playwright.config.ts apps/desktop/e2e/timeline-scrollback.spec.ts -g "missing-dimension media|pending link previews" --workers=1
```

Expected: fallback geometry returns `null`, and at least one real-DOM row-height assertion exceeds the 2-pixel tolerance.

- [ ] **Step 3: Render one immutable image figure and fixed preview cards**

```ts
const TIMELINE_MEDIA_FALLBACK_BOX = { inlineSize: 347, blockSize: 260 } as const;

function timelineMediaDisplayBox(width?: number | null, height?: number | null) {
  if (!width || !height || width <= 0 || height <= 0) {
    return TIMELINE_MEDIA_FALLBACK_BOX;
  }
  const scale = Math.min(420 / width, 260 / height, 1);
  return { inlineSize: Math.round(width * scale), blockSize: Math.round(height * scale) };
}
```

Render `.message-media-figure` for every image state using only the event
metadata display box. Put the icon/skeleton, retry/download control, progress,
or ready `<img>` inside that same figure; do not recompute the outer size from
download-result dimensions. Render pending previews with
`data-link-preview-state="pending"` and skeleton children inside the same
`.link-preview-card` used by ready/failed content.

Set `.link-preview-card` to one fixed logical block size, clamp text, absolutely
or explicitly size its optional image region, and keep overflow inside the
card. Keep non-image file rows unchanged.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the two commands from Step 2. Expected: all selected tests pass.

- [ ] **Step 5: Commit the independently reviewable #277 fix**

```bash
git add apps/desktop/src/components/TimelineView.tsx apps/desktop/src/styles.css \
  apps/desktop/src/components/TimelineView.test.tsx apps/desktop/e2e/timeline-scrollback.spec.ts
git commit -m "fix: reserve stable timeline media heights"
```

### Task 2: Pure prepend and threshold policy (#278)

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Test: `apps/desktop/src/components/TimelineView.test.tsx`

**Interfaces:**
- Produces: `timelineRowsArePurePrependForTests(previousIds, nextIds): boolean`.
- Produces: `timelineBackfillThresholdForTests(clientHeight, enabled): number`.
- Consumed by: Task 3's projection deferral and backfill evaluation.

- [ ] **Step 1: Add failing pure-policy tests**

```ts
expect(timelineRowsArePurePrependForTests(["b", "c"], ["a", "b", "c"])).toBe(true);
expect(timelineRowsArePurePrependForTests(["b", "c"], ["b", "x", "c"])).toBe(false);
expect(timelineBackfillThresholdForTests(900, true)).toBe(1800);
expect(timelineBackfillThresholdForTests(20, true)).toBe(80);
expect(timelineBackfillThresholdForTests(900, false)).toBe(0);
```

- [ ] **Step 2: Run the unit test and verify RED**

Run: `npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx`

Expected: the two exported helpers do not exist.

- [ ] **Step 3: Implement the pure policies and wire the threshold**

```ts
function timelineRowsArePurePrepend(previousIds: readonly string[], nextIds: readonly string[]) {
  const added = nextIds.length - previousIds.length;
  return added > 0 && previousIds.every((id, index) => nextIds[added + index] === id);
}

function timelineBackfillThreshold(clientHeight: number, enabled: boolean) {
  return enabled ? Math.max(AUTO_BACKFILL_THRESHOLD_PX, clientHeight * 2) : 0;
}
```

Use the threshold helper in `evaluateAndMaybeRequestBackfill`; remove
`AUTO_BACKFILL_PREFETCH_ITEMS` and the 7,200-pixel estimate-derived path.

- [ ] **Step 4: Run the focused unit test and verify GREEN**

Run the command from Step 2. Expected: all `TimelineView.test.tsx` tests pass.

- [ ] **Step 5: Commit the policy change**

```bash
git add apps/desktop/src/components/TimelineView.tsx apps/desktop/src/components/TimelineView.test.tsx
git commit -m "fix: bound timeline backfill prefetch"
```

### Task 3: Same-pass anchor compensation and active-input deferral (#278)

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx`
- Test: `apps/desktop/e2e/timeline-scrollback.spec.ts`

**Interfaces:**
- Consumes: Task 2's pure-prepend classifier.
- Produces: a pending local `ScrollAnchor` transaction released with measured-height and deferred-prepend commits.
- Produces evidence in `TimelineScrollDiagnostics.latestFrame.heightDeltaAboveViewportPx` and `writesByReason.backfillCompensation`.

- [ ] **Step 1: Add failing real-DOM regression tests**

```ts
test("measurement commit compensates the local anchor before paint", async ({ page }) => {
  // Enter free-scroll, grow a mounted row above the anchor, wait for idle flush,
  // and assert anchor offset delta <= 2. Require a non-zero
  // heightDeltaAboveViewportPx and one backfillCompensation write.
});

test("active upward input defers prepend until idle", async ({ page }) => {
  // Dispatch wheel + scroll, push a prepend containing tall media rows, assert
  // the first new row is absent during the active interval, then poll until it
  // appears after idle with the prior anchor still within 2 pixels.
});
```

- [ ] **Step 2: Run the selected E2E tests and verify RED**

Run:

```bash
npm --prefix apps/desktop exec -- playwright test --config apps/desktop/playwright.config.ts apps/desktop/e2e/timeline-scrollback.spec.ts \
  -g "measurement commit compensates|active upward input defers" --workers=1
```

Expected: diagnostics still report zero height delta and/or the prepended row
commits during active input.

- [ ] **Step 3: Add the local height-commit transaction**

Add a ref containing `{ timelineKeyHash, anchor }`. Before changing
`itemHeightByDomIdRef` in both idle-flush and immediate measurement paths,
capture `captureFreeScrollAnchor(container)` only in free-scroll. In a
`useLayoutEffect` keyed by the committed height version, locate the same row,
compute `currentOffset - anchor.offsetTop`, and synchronously add that delta to
`container.scrollTop` through `runWithScrollWriteReason("backfillCompensation", ...)`.

Update the diagnostic frame in that same effect:

```ts
recordTimelineScrollFrame(current, {
  ...current.latestFrame,
  changedMeasuredRowCount,
  heightDeltaAboveViewportPx: delta,
  anchorTopDeltaPx: delta
});
```

Clear the transaction on timeline-key reset/unmount and do not fall back to
the global height model when its row is absent.

- [ ] **Step 4: Defer pure prepends during active input and release on existing timers**

Keep a committed visible-row projection and a pending pure-prepend projection.
When `scrollActivityRef.current === "active"` and Task 2 classifies the raw
projection as a pure prepend, render the committed rows. On the existing idle
or maximum-defer callback, capture the current local anchor, set
`anchorRestorePendingRef`, mark activity idle, release the pending projection,
and commit pending heights in the same React update. Timeline-key changes,
resets, and unmount clear the pending projection.

Do not defer edits, redactions, live-edge appends, room switches, or resets.

- [ ] **Step 5: Run the selected E2E tests and verify GREEN**

Run the command from Step 2. Expected: both selected tests pass and the
diagnostic delta/write assertions are non-zero and same-commit.

- [ ] **Step 6: Run the complete scrollback spec once**

Run:

```bash
npm --prefix apps/desktop exec -- playwright test --config apps/desktop/playwright.config.ts apps/desktop/e2e/timeline-scrollback.spec.ts --workers=1
```

Expected: all scrollback tests pass with no retries.

- [ ] **Step 7: Commit the independently reviewable #278 fix**

```bash
git add apps/desktop/src/components/TimelineView.tsx apps/desktop/e2e/timeline-scrollback.spec.ts
git commit -m "fix: compensate timeline height commits synchronously"
```

### Task 4: Integrated verification, review, PR, and merge

**Files:**
- Review: all changes from `origin/main..HEAD`
- Modify only if a gate identifies a concrete defect.

**Interfaces:**
- Produces: one ready PR closing #277 and #278, with all required checks green and a verified merged state.

- [ ] **Step 1: Run fast static and frontend gates**

```bash
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop run test -- --run
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
git diff --check origin/main...HEAD
node scripts/check-sdk-submodule.mjs
```

Expected: every command exits 0.

- [ ] **Step 2: Review the finished diff against the design and repository rules**

Confirm each #277/#278 acceptance item has direct test evidence, no
source-text-only test substitutes for behavior, media changes do not enter the
scroll engine, and no unrelated files changed.

- [ ] **Step 3: Push and create a ready PR**

```bash
git push -u origin codex/issues-277-278-scroll-stability
gh pr create --base main --head codex/issues-277-278-scroll-stability \
  --title "Stabilize timeline media and back-pagination scrolling" \
  --body-file /tmp/issues-277-278-pr.md
```

The PR body includes `Closes #277` and `Closes #278`, the measured invariants,
and exact verification commands.

- [ ] **Step 4: Wait for and repair required CI checks**

Run: `gh pr checks --watch --interval 10 --fail-fast=false`

Expected: Frontend, Rust, and Windows overlay checks all complete successfully.
For a failure, inspect its failed step, reproduce with the closest focused
local command, commit only the evidence-driven fix, and wait for the new run.

- [ ] **Step 5: Merge and verify GitHub state**

```bash
gh pr merge --squash --delete-branch
gh pr view --json state,mergedAt,mergeCommit,url
```

Expected: `state` is `MERGED`, `mergedAt` is non-null, and `mergeCommit.oid` is
present.
