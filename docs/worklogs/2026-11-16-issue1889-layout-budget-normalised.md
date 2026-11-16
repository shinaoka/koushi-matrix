# Layout-read budget: normalising by observation pass (2026-11-16, #1889)

PR #1918 went red on WebKit only, `e2e/room-switch-layout-budget.spec.ts:119`,
with `large.row (370) > ROW_READ_CEILING (300)`. Seven local WebKit runs of the
same tree were green and deterministic (large 192, small 228, ceiling 300), and
`main` passed on the same runner image (`43829232215`); #1918's diff does not
touch this path and the value is identical with and without #1914's row
memoization. The numerator was stable at 19.2 row reads per observation pass and
the *denominator* — how many passes fit in the post-seed two-frame window — grew
on the slower runner.

Canon consulted: `docs/agents/qa-lanes.md` (Browser-headless lane, and the
renderer budget's "count what the app does, not how long the runner took"),
`docs/policies/engineering-rules.md`, and PR #1918's triage.

## Change

- `e2e/support/roomSwitch.ts`: the layout-read probe gains `box`, counting
  `getBoundingClientRect` on the `timeline-view` container itself. Every
  observation pass reads that box (`timelineViewportBox`, reached from
  `readTimelineViewportFacts`), so the count tracks the pass count; it is not
  exactly equal to it, because the anchor paths (`captureAnchor`,
  `measureAnchorDelta`, `currentRoomScrollAnchorOffset`) read the container once
  each as well, and an added container read on the pass path would loosen the
  ratio by its factor. The spec mounts exactly one such container.
- `e2e/room-switch-layout-budget.spec.ts`: the switch budget becomes
  `row <= box * 32` for both the 100-row and the 30-row room, compared by
  multiplication so the float quotient cannot decide the result. A loose
  `row <= 600` stays as a guard against a runaway pass count, which the ratio
  alone would accept. The scroll-frame test keeps an absolute 300: one frame runs
  about two passes, and its measured 37 per pass is legitimately above the
  switch's 19.2 because the frame also re-measures what it scrolls into view, so
  a ratio there would assert a different fact rather than a stricter one.
- `docs/agents/qa-lanes.md`: the gate now has a paragraph next to the renderer
  budget, saying why it is normalised and why it must not go back to a total.

## Measured

WebKit, 1280×800, this worktree:

| Room | row reads | box reads | per pass | pass |
| --- | --- | --- | --- | --- |
| Budget Large (100 items) | 192 | 10 | 19.2 | 19 ≤ 32 |
| Budget Small (30 items) | 228 | 10 | 22.8 | 23 ≤ 32 |
| one scroll frame | 74 | 2 | 37.0 | absolute 74 ≤ 300 |

The CI number that failed, 370 over ~19 passes, is 19.5 per pass: under the new
assertion it is 370 ≤ 19 × 32 = 608 and 370 ≤ 600, so the same run passes while
the per-pass cost it really measured is unchanged.

## Proof the gate can still go red

Each mutation was applied, run, and reverted (nothing below is committed):

- `ROW_READS_PER_PASS = 1` → `Expected: <= 10, Received: 228` at the small-room
  assertion.
- `ROW_READS_PER_PASS = 22` → `Expected: <= 220, Received: 228`: the assertion
  binds on the measured 22.8, not on a number that always passes.
- `ROW_READ_GUARD = 100` → `Expected: <= 100, Received: 192` at the guard.

Per-row scanning would read ~100 rows per pass (the pre-#1898 behaviour), which
is 3.1× the ceiling of 32.

## Not covered

The hosted WebKit runner was not exercised from this branch; the local evidence
is WebKit (and Chromium) on macOS. Whether the ratio holds on a runner much
slower than the one in the triage is reasoned from the numbers — the numerator is
the pass cost, which does not depend on runner speed — not observed.
