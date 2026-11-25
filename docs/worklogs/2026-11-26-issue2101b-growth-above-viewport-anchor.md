# A height change above the viewport no longer moves the reader (#2101 part B)

`a 147-row room keeps mounting bounded while scrolling and measuring rows` failed on WebKit
under load: a row above the viewport grew by 180px and the reader's own row moved 170px, and
it never came back (`timeline-scroll-anchor-drift.spec.ts:283`, the 3px assertion).

## What the failing run actually shows

`#2103` left this to part B with the diagnosis that the commit publishes the new height into
the model before the observer sees it, so the observer's `delta` measures ~0 and no correction
is written. The trail from the failing run says something narrower: the `delta` was ~0 because
**the anchor row was the row that grew**.

Instrumented on a Linux arm64 Playwright container (`--cpus=2`, `--project=webkit`,
`--repeat-each`), one failing run, dump taken immediately after the growth:

```
RO delta=0.15625 anchor={"itemId":"$…-0065","offsetTop":-91.28125} active=none scrollTop=6022
measurePass … above=0 below=1 heldAnchor={"itemId":"$…-0065",…}
flush reason=idle … deltaNow=0.15625 active=none activity=idle
attempt delta=0.15625 anchor={"itemId":"$…-0065",…} scrollTop=6022
```

and, from the harness side at the same moment, the grown row is `$…-0065`, whose box is
`-91.12..-10.41`: **entirely above the container's top edge**, and its top is `-91.28` — the
recorded anchor offset, to the sub-pixel that the `0.15625` delta reports. The harness grows
the last row whose bottom is above the viewport; the anchor was that same row.

So the reader was anchored to a row they cannot see. A row's own height change never moves its
own top, so a top-based delta cannot see it — the compensation for every row below was dropped.
The rows the reader *does* see are one row lower and moved the full 170px.

Why the anchor could be there at all: `captureAnchor` takes the model's first visible row
(#2084). That row can end above the container's top edge — it is the boundary row `#2101` part A
allows the reported range to misname by one. On that boundary the model and the drawing disagree
by one row, and the anchor inherited the disagreement.

## What changed

- `captureAnchor` now decides "visible" from the row's own box as well as the model range: a
  candidate whose bottom is at or above the container's top edge is skipped and the walk
  continues to the next row in the model range. A row that ends exactly on the edge shows
  nothing, so it is skipped too. Each candidate costs the one rect the offset already needed, so
  the aligned path still reads one row (the unit test pins `reads() === 1`) and a capture that
  has to skip reads one more per skipped candidate (the shifted case in the same test pins 2).
  If no row in the model range crosses the edge (a window whose model lags its drawing, e.g.
  before the first measurement settles), the first such row is kept as the anchor, which is
  exactly what the capture returned before.
- The edge is the container's, not the sticky navigation bar's bottom: `timelineVisibleBand` uses
  the same edge, and the bar's coverage depends on whether its pill is showing, which is not
  geometry the capture has. A row hidden behind the bar therefore still counts as visible. It is
  the same one-band simplification the model already carries, and the remaining hole is a row no
  taller than the bar sitting entirely inside it.
- Nothing else moved. The compensation is still the transaction's DOM delta; a visible anchor
  makes that delta see every height change above it, including the growth of a row that ends
  above the edge.

## Tests

- New deterministic case in `TimelineView.viewport-transaction.test.tsx`: the fixture draws every
  row 80px above the offset its measured height gives it, so the model's first visible row ends
  above the container's top edge exactly as in the field. It grows the row the reader cannot see,
  then commits the height **with no `ResizeObserver` callback stored at all** — the observer
  cannot deliver, so the commit alone has to place the reader. Against `origin/main`'s
  `captureAnchor`: the reader's row moves to `180`; with this change: it stays at `0`.
- `TimelineViewportAnchors.test.ts` gains the capture-level cases behind that: the shifted window
  whose range starts on a row the drawing puts entirely above the edge (it picks the next row),
  the window whose rows all end above the edge (it keeps the first, as before), and the read cost
  of a skip. The older "excludes a row that ends exactly at the viewport's top edge" case is
  retitled — it exercises the range, not the box check.
- `e2e/timeline-scroll-anchor-drift.spec.ts` is unchanged: the 3px tolerance, the one-boundary-row
  viewport tolerance from `#2101` part A, and every case in the file are as they were.

## Evidence

- Linux arm64 Playwright container, `--cpus=2`, `--project=webkit`,
  `-g "a 147-row room keeps mounting bounded"`: **1 failed in 25** before, **0 failed in 120**
  after (two runs of 60).
- macOS, unchanged spec files: `timeline-scroll-anchor-drift`, `timeline-window-slide-anchor`,
  `timeline-viewport-transaction`, `timeline-touchpad-swipe-window` and `renderer-perf-budget`
  pass on chromium, and on webkit for the specs that opt in. No budget was raised.

## Left alone

- The sticky navigation bar's coverage (see above) is not modelled, so the anchor can still be a
  row the reader only "sees" through a translucent pill bar.
- `captureStableAnchor` transfers a residual measured on the *previous* anchor row to whichever
  row the new capture names. Two rows only share a displacement when the change is above both, so
  a capture that jumps onto a row whose top did not move stores an offset that is intentionally
  out of step with the DOM until the next settle applies it. It was visible in the instrumented
  run but it is not what the failing case depended on, and it is a separate change to the
  recapture rule.
