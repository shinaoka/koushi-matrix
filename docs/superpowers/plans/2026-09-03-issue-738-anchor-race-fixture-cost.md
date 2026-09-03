# Issue #738 anchor-race fixture cost

## Failure

Post-merge `main` run `33722804979` failed because `TimelineView.anchor-race.test.tsx` exceeded Vitest's unchanged 5-second timeout. The deterministic race test rendered 700 timeline rows in jsdom even though its oracle only needs an anchor after rows 117–122 and a prepended batch.

## Change

Reduce the synthetic timeline to the minimum virtualized size (601 rows), disable unrelated avatar-thumbnail work, and prepend 80 rows while preserving the geometry and race: start at the new maximum scroll offset, keep the user anchor at scrollTop 20,000 after measured rows 117–122, and hide original item 600 at post-prepend index 680. Do not change production code, timers, timeout, assertions, or scheduler ordering.

## Verification

The GitHub run is RED evidence. Run the exact test repeatedly, the full test file, frontend Vitest/typecheck/lint, independent diff review, and required PR CI. Merge independently before resuming #775.
