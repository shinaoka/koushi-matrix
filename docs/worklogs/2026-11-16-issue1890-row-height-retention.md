# Retaining measured row heights outside the mounted window (2026-11-16, #1890 PR 1)

Design v2 for #1890 splits into three PRs; this is PR 1, which makes the height
model independent of mounting and leaves the virtualisation threshold at its
current 120 rows. The window shrinks in PR 2.

Canon consulted: `docs/architecture/overview.md` (scroll position, measured
heights and the virtual-list cache are UI state, and never cross into core),
`docs/architecture/frontend-ownership-inventory.md` (the mounted viewport
controller owns DOM measurement), `docs/policies/engineering-rules.md`, and the
v1 attempt (`fix/1890-perf`) whose diagnosis this replaces.

## Why retention first (v1's failure)

v1 shrank the window before making heights survive, and went red on
`e2e/timeline-viewport-transaction.spec.ts` and
`e2e/timeline-scroll-anchor-drift.spec.ts`. The map itself was not losing rows —
it was pruned only for rows that left residency — the coupling was elsewhere:

1. **Measuring requires mounting.** A windowed timeline never mounts the rows
   beyond the window, so they keep the 72 px estimate for good.
2. **A pending measurement is discarded when its row unmounts.**
   `flushPendingMeasurements` skipped any pending entry whose row was no longer
   mounted, so a reader wheeling through history measured rows and threw the
   results away; the spacer above kept estimates and every remount moved them.
3. **A stored measurement goes stale when its content changes**, because the
   model was keyed by `row_id` alone.

## Change

- New `apps/desktop/src/components/timeline/TimelineRowHeightCache.ts`:
  `isFresh(row)`, `record(row, height, width)`, `retainResident(rows)`,
  `heightsFor(rows)`, `clear()`, plus `rowHeightsUnpublished(next, committed)`. Keyed by `row_id`, the
  identity `buildTimelineHeightModel` already indexes. An entry is *fresh* only
  while `entry.item === row.item`, the list width matches and no font epoch has
  passed; a stale entry is still returned, because an old height is a better
  estimate than 72 px and the next mount re-measures it. A record only counts as
  a change when the stored height moves by more than a pixel, and re-recording
  moves the key to the recent end, so the 4,096-entry cap evicts the row the
  reader has been away from longest.
- `TimelineView` records on every mount pass and in the measurement flush, and
  only defers the height-model commit (`pendingMeasurementCountRef`, a bare
  counter now that the values are recorded — its epoch was dead, since a reset
  zeroes the count in the same block that bumps the epoch). Recording and
  committing stay separate in the model too: `committedHeightsRef` is what `buildTimelineHeightModel` reads,
  and only a commit (the flush, or the idle mount pass) publishes the cache into
  it. That is main's semantics exactly — main's map was mutated only at commit
  points, so the model could never move from an uncommitted measurement — and it
  is the property PR 2's anchor invariant needs.
- Publication is decided by comparing the resident heights with the committed
  snapshot (`rowHeightsUnpublished`), not by "did a record change a cache value".
  Those differ after a reset: a reset cancels the owed commit, and the next pass
  re-reads heights the cache already holds, so a cache-only comparison would
  return "nothing changed" and leave the model on its old estimates — the exact
  drift class this PR removes, kept alive around resets. Main healed it because
  its comparison was against the committed map.
- The `!mountedDomIds.has(domId)` filter is gone: a height read at this width, on
  the current item, stays valid after the row leaves the window.
  `resetActiveMeasurementDeferral` no longer clears the cache, and the cache is
  cleared only on a timeline-key change.
- A row whose rect height is not positive is not recorded at all. A zero rect
  means the row was never laid out (`display: none`, or a DOM without layout);
  charging it the 1 px floor would shrink the model, expand the window, and mount
  the next block of rows in a feedback loop. `TimelineView.media.test.tsx`'s
  off-window image case caught this: every row mounted and the window stopped
  being a window. Main never showed it because its record step required a move of
  *more* than one pixel, so a first reading of 0 px was treated as "no change"
  and never stored — an accident of the threshold, not a decision. A zero rect is
  not a measurement, so the cache now says so.
- The 72 px fallback and the `[36, 480]` clamp are unchanged; the running
  average is PR 2.

## Evidence

- `timeline/TimelineRowHeightCache.test.ts` (new, 9 cases): a record survives
  while resident and drops when it leaves; a replaced item is stale but keeps its
  value; a width or font-epoch change is stale; the cap evicts the longest-unseen
  entry; a sub-pixel move is not a change; `clear()` empties it; a genuinely short
  row keeps its measured height; and `rowHeightsUnpublished` reports a value the
  model has not seen, including the resident-set changes on either side.
- `TimelineView.viewport.test.tsx`: "keeps a row's measured height when the row
  unmounts before the idle flush" asserts the rendered spacer plus the mounted
  rows add up to the true 160 × 40 px. **RED on main** (`expected 7552 to be
  6400`) because of the discarded pending measurements. The second case,
  "re-measures an edited row when it comes back into the window", asserts the
  model's total moves by the edit's exact delta (`before + 80`): the row was
  measured at 40 px, the store replaced its item while it was off-window, and
  coming back re-measures it at 120 px. It passes on main as well — re-measuring
  a mounted row is unconditional there too — so it is a forward guard, not a
  RED→GREEN pin: it fails if a later PR trusts a retained entry instead of
  re-reading the row, which is the trade PR 2 is allowed to make only with the
  freshness facts in hand.
- Mutation checks, each run and reverted: dropping `entry.item === row.item`,
  `entry.width === this.width` or `entry.fontEpoch === fontEpoch` from the
  freshness check fails the corresponding cache test; deleting the module makes
  its test file fail to resolve.
- `npm run typecheck`, `npm run lint` (no issues), `npm run test -- --run`
  (204 files, 2509 tests).
- `npx playwright test e2e/timeline-*.spec.ts --project=chromium
  --project=webkit --repeat-each=3`: 282 passed, 3 failed — all three in
  `e2e/timeline-viewport-transaction.spec.ts`. Those two WebKit 800-row cases
  fail on `origin/main` as well (main: 2 failures in an 18-run sample), so they
  are pre-existing flakiness, not a regression here; a later 2×-repeat run of the
  whole `timeline-*` set on this branch was 190/190. No spec
  was edited: `git diff origin/main --
  apps/desktop/e2e/timeline-viewport-transaction.spec.ts
  apps/desktop/e2e/timeline-scroll-anchor-drift.spec.ts
  apps/desktop/e2e/timeline-scrollback.spec.ts` is empty.

## Freshness audit (design §1)

The design asked for an audit of `domain/timelineDisplayProjection.ts` and, if
neither `item` nor `row_id` changes with a row's chrome, for the chrome flags to
join the freshness check. The audit ran; two chrome inputs are not in the row:

- **Sender continuation.** `isContinuation` is computed at render from the
  *previous* row, and in compact density it hides the sender line, so a prepend
  changes a row's height without touching its `item` or its `row_id`. It cannot
  leave a wrong height behind, though: the flip is computed while the row is
  rendered, that render changes the row's rect, and the ResizeObserver re-records
  it; a row that is off-window is re-measured at its next mount anyway.
- **Date dividers.** `dateDividerRow` builds a fresh `TimelineItem` on every
  projection rebuild, so divider entries read as stale as soon as anything
  changes — systematic over-staleness, never over-freshness.

So freshness is deliberately *estimates-only* here: main's re-measure-on-mount
behaviour is unconditional, `isFresh` is consumed by nothing in the product and
exists as the contract the design's deferred stale-hit diagnostic will read. The
caveat is recorded in the module docstring so a reader of the code sees it without
the worklog.

## Not covered

The window is still 120 rows, so nothing about the mounted footprint changes
here; PR 2 shrinks it. Stale-hit rate is not measured yet — the design's
"diagnostic counter for stale hits" is deferred, so a store that re-creates
`TimelineItem` objects on unrelated updates would show up as extra re-measures on
mount (never as a wrong height) rather than as a number. Nothing consumes
`isFresh` yet, for the reason in the audit above. An entry can also be evicted
while its row is resident if residency exceeds the 4,096-entry cap; the model then
falls back to 72 px for that row at the next commit — bounded to >4 096-row
timelines, and PR 2 reconciles it with the anchor invariant.
