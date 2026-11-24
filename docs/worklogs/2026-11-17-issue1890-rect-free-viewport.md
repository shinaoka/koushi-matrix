# Rect-free viewport facts and anchors (2026-11-17, #1890 item 5)

#1890 item 5 asked for the `getBoundingClientRect` sweeps over the visible run to
go: the visible-range facts and the scroll anchor should come from the cached
height model plus one container read per frame. The window itself is unchanged
(item 1–4 own that), the perf budgets are unchanged, and #1892's invariants
(one observation per frame, only on a visible-range change, the anchor written at
settle) stay as they are.

Canon consulted: `docs/architecture/state-machine.md` (viewport facts, the scroll
anchor, #1892), `docs/policies/engineering-rules.md`, and the #1889 worklog
(`docs/worklogs/2026-11-16-issue1889-layout-budget-normalised.md`) whose binary
search this replaces.

## Where the answer comes from now

The height model already states where every row sits: the spacers the mounted
window is placed with are written from the same cumulative `offsets`. So:

- `timelineVisibleItemRange(model, metrics, itemCount)` binary-searches
  `offsets` at `scrollTop - listOffsetTop` and at the band's bottom edge, and
  trims a row that starts exactly on the bottom edge. It reads nothing.
- `timelineViewportFactsFromModel(view)` walks that range over the row list to
  name the first and last visible activity event id, the visible gap ids (deduped
  by `topology_revision\0ordinal`, as the DOM path did) and the last visible row's
  bottom offset, which is now `offsets[i + 1] - relativeScrollTop - viewportHeight`
  instead of a rect difference.
- `captureAnchor(container, view, { isEligible })` names the visible rows from the
  model, then reads exactly one row's rect — the anchor's own, for the pixel
  offset the transaction compares against. `isEligible` moved from a DOM
  attribute predicate to a row predicate, so eligibility no longer needs the
  node.
- `captureRoomScrollAnchor(container, view)` takes the event id from the model and
  the pixel offset from that row's rect, falling back to the modelled offset when
  the row is not mounted.
- `timelineCaptureMetrics(container, listOffsetTop)` is the one container read a
  capture makes (`timelineViewportBox` plus a fresh `scrollTop`/`clientHeight`, so
  a capture cannot lag the scroll it was taken on).

`readTimelineViewportFacts`, `visibleNodeSpan`, `linearVisibleNodeSpan`,
`parseCanonicalU32`, `eventIdForTimelineIdentity` and the DOM-based
`visibleTimelineViewportFacts` are gone; nothing else used them.

## Which rows and model a callback sees

`viewportModel()` reads refs, so a scroll callback costs no layout:
`rows`/`model` are the pair the *committed* window was built from
(`virtualRangeRowsRef`/`virtualRangeModelRef`, written together in
`commitVirtualRangeForMetrics`), and `metrics` is the last frame's read. The
committed pair is the one whose indices the mounted rows follow; the render's own
pair can be a commit ahead of the DOM, and using it made `captureAnchor` name
rows that were no longer mounted (terminals `reason=missing-anchor` in
`TimelineView.viewport-transaction.test.tsx`). A stale row that is not in the DOM
is skipped by the next candidate, and a range that lands outside the rows yields
no facts, so the failure mode is "no anchor this pass", never a wrong one.

Clamping replaces bailing when the model and the browser disagree: a scroll
position past `totalHeight - viewportHeight` is clamped, because the browser
clamps the same way and the window was placed with that clamp. Only a negative
relative position (scrolled into the list offset) yields nothing.

## Evidence

- `timeline/TimelineViewportAnchors.test.ts` (rewritten): `captureAnchor` reads
  one row rect for a 20-row window and one for a 600-row window (`reads() === 1`
  both times), the visible range and the facts read no row rect at all
  (`reads() === 0`), a row starting exactly on the bottom edge is excluded, a
  modelled gap set is deduped by topology revision and ordinal, and a scroll
  position past the modelled end clamps to the last rows.
- `e2e/timeline-model-viewport.spec.ts` (new): one open plus a scripted ten-frame
  scroll, row reads counted by `installLayoutReadProbe`. Before this change the
  120-row room read 1379 rows over 59 passes; after it, 42 over 87 — and the
  600-row room reads the same 42 over the same 87, so the read count no longer
  follows the item count. The same file pins that the observation names the rows
  on screen (±1 row, because rounded heights can move a boundary row) after the
  open, after a scroll, and after a jump re-slices the window.
- Three `TimelineView` fixtures were rewritten rather than special-cased, because
  a stubbed layout that is not a contiguous stack cannot be stated by a height
  model: `TimelineView.threads.test.tsx` ("preserves gap identity…", "covers
  selected-room persisted gap recovery…") and `TimelineView.viewport.test.tsx`
  ("does not re-request live mode…"). Their expectations now describe the band
  the model reports, and the gap-recovery fixture waits for the removed row's
  height commit before placing its final band.
- `npm run typecheck`, `npm run lint`, `npm run test -- --run` (214 files, 2695
  tests), `npx playwright test e2e/timeline-*.spec.ts --project=chromium` (90
  passed, 3 skipped), `e2e/renderer-perf-budget.spec.ts` and
  `e2e/room-switch-layout-budget.spec.ts` unchanged and green. No budget value
  was raised.

## Not covered

The model is a height model, not a position model: a row whose real height drifts
from the measured one (images, wrapping, code blocks) moves the modelled offset.
The design's STOP condition keeps that bounded to measuring the diverging row,
but nothing detects the drift yet, so the facts carry up to a pixel of rounding
per row inside the window and a boundary row can be off by one. The observation
tolerance (±1 row) is what the read-marker path can absorb. `atBottom` still
comes from the container's `scrollHeight`/`scrollTop`/`clientHeight`, which is a
container read, not a row read.

## The offset the band needs

`metrics.listOffsetTop` is how far the row list starts below the scroll container's
content origin, and the band depends on it: `scrollTop - listOffsetTop`. It was
`listRef.current.offsetTop`, which is only that distance when the container is the
list's own offset parent. In the real layout the list's offset parent is the app
grid, so the raw value carried the container's own offset as well (79 px instead
of 17 px on a 1280x800 window), and the model band started one row above the
physical band. `timeline-scroll-anchor-drift.spec.ts`'s "the observed viewport
matches the physically visible rows" — exact equality with the DOM — caught it;
the band now reads 17 px and the same spec passes unchanged. `listContentOffsetTop`
is one helper used by both `readViewportMetrics` and the backfill evaluation, so
placement and facts share one offset.
