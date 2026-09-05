# Active prepend anchor preservation (#520)

Status: historical #520 design; superseded by
[the single-owner #837 migration](2026-09-05-issue837-viewport-transaction.md).
The constraints below describe the former localized fix, not the current
viewport ownership contract.

## Problem

`TimelineView` captures a DOM anchor before idle prepends, and it defers a *pure* prepend while scrolling is active. The event handler does not capture the pre-prepend anchor in the active branch: it sets `anchorRestorePendingRef` with `pendingAnchorRef == null`. The existing pure-prepend classifier often masks this by keeping the old rows rendered until idle, but a prepend combined with another projection change cannot use that deferral. React may then commit the changed projection, clear the empty restore transaction, and leave the unchanged `scrollTop` pointing at much older content.

## Existing ownership to reuse

The July #278 scroll-stability implementation already owns:

- free-scroll DOM anchors (`itemId` + pixel offset);
- active-input idle/max-defer release;
- pure-prepend render deferral;
- mounted DOM restoration plus one virtual-height fallback/follow-up frame;
- timeline-key/reset/jump/live-edge invalidation;
- backfill blocking while anchor restoration is pending;
- private-data-free scroll diagnostics and the 2px browser oracle.

Do not add a second scroll transaction type, timer, retry loop, or Rust/IPC state machine. The missing invariant is only that every prepend transaction must acquire its anchor before the event is applied.

## Minimal design

In the `ItemsUpdated` prepend handler, synchronously capture `captureFreeScrollAnchor(container)` before `setStore` can apply the batch, including while `scrollActivityRef` is active.

- If no restore is pending, store that anchor and mark restoration pending.
- If a prior prepend restoration is unresolved, retain its original anchor; a later batch must not overwrite it.
- Active pure prepends continue to render the committed projection until the existing idle/max-defer release.
- `releaseDeferredPrepend` reuses an already-held anchor and captures only when none exists, immediately before releasing the still-uncommitted pure prepend.
- While that prepend transaction is pending, the generic `ProjectionSnapshotBoundary` correction must not schedule a second anchor owner for the same commit; the prepend restore remains the sole correction path.
- Existing mounted restore, virtualized fallback, bounded follow-up, invalidation, diagnostics, and backfill release remain the sole settlement path.

This fixes mixed prepend/projection batches without changing the behavior already proven for pure prepends.

## Verify-first sequence

1. Add a Playwright regression before production changes:
   - initialize a virtualized 1,000-row timeline;
   - enter active upward free-scroll and record the first visible stable row and pixel offset;
   - deliver 100 mixed-height `PushFront` rows plus one projection-changing `Set` outside the viewport so the update is not classified as a pure prepend;
   - assert the original anchor is restored (including the virtual fallback when temporarily unmounted) within ±2px after layout/measurement settlement.
2. Run only that test and record RED on `origin/main`.
3. Apply the two localized capture/reuse changes in `TimelineView.tsx`.
4. Rerun the same test GREEN, then the existing active pure-prepend, variable-height settlement, and complete scrollback specs.
5. Run frontend test/typecheck/lint/build and full Playwright, perform preflight self-review, then obtain reviewer-gpt approval of the complete diff before PR.

## Acceptance

- Any prepend arriving during active input owns a pre-apply stable row/offset anchor.
- A large mixed-height virtualized prepend restores that row to within ±2px.
- A temporarily unmounted anchor uses the existing bounded virtual fallback and follow-up.
- A second batch cannot replace an unresolved anchor, and generic projection compensation cannot compete with it.
- Pure prepend deferral, variable-height settlement, user scroll ownership, live edge, jump-to-event, room/reset invalidation, and automatic-backfill blocking remain unchanged.
- No new identifier-bearing diagnostics, timers, polling, or duplicate viewport state are introduced.

## Review record

- Design review round 1: incomplete at timeout; not accepted.
- Design review round 2: reviewer-gpt `Correct-to-merge`.
- Implementation discovery: mixed projection reproduced a competing generic projection correction; amended the design to keep the prepend transaction as the sole anchor owner.
- Design amendment review: reviewer-gpt `Correct-to-merge`.
- Final diff review: required before PR creation; record the reviewer-gpt verdict in the PR body.
