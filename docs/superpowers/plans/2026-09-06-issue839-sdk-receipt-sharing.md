# #839/#840: share SDK receipt snapshots

Status: implemented and reviewed; SDK commit published; root CI pending.

## Approved representation and bounded deliverable

The earlier Sol high read-only representation review approved a persistent receipt
collection preserving SDK placement and the existing IndexMap accessor/order. That
approval is recorded in the scoped-publication working contract; it was not an
approval of the entire scoped API or Core consumer lifecycle migration.

Use the SDK's existing imbl dependency: OrdMap keyed by reader with its ordering
slot and Receipt, plus Vector for IndexMap-compatible insertion/swap-remove order.
An opaque read-only ReadReceiptSnapshot exposes len/is_empty/get/iter/cheap Clone.
SDK-only mutators invalidate a lazy OnceLock compatibility IndexMap. Cloning a
snapshot starts with an empty compatibility cache and shared tree/vector storage.
No previous-version chain, additional event stream, dependency or placement owner.

The existing EventTimelineItem::read_receipts signature/order remains unchanged.
The new read_receipt_snapshot accessor and Core's sole collector avoid constructing
that full compatibility map. SDK hidden-event redistribution also retains a cheap
snapshot rather than cloning its compatibility map. Explicit legacy full-map reads
still materialize all entries; this is not a claim that legacy enumeration/drop is
constant-time. Ordinary direct iteration remains full enumeration (O(n log n) with
indexed ordered lookup), not a sparse update API.

## Evidence and limits

- Existing live-receipt test strengthened BEFORE representation replacement:
  event-item clones must share receipt records even after legacy full-map access.
  Failed with that exact assertion, then passed with the shared representation.
- A 1,500-reader owner test checks IndexMap-equivalent serialized values/order,
  remove/reinsert/update, slot invariants, cache invalidation and immutable old
  snapshots. Direct iteration must not initialize the compatibility cache.
- SDK UI library: 371 passed; new public API doctests: 2 passed.
- Core existing tests: 944 passed, 8 existing ignored; two separately added
  architectural regressions remain RED in the integration worktree. They are not
  changed or claimed by this representation step.
- Logs: `/tmp/issue839-sdk-sharing-red.log`, `/tmp/issue839-sdk-ui.log`,
  `/tmp/issue839-sdk-receipt-doc.log`, `/tmp/issue839-sdk-core-existing.log`.

Core still enumerates/materializes all reader DTOs and looks up 1,500 profiles.
No sparse changed-reader accessor, bounded compact view, full-reader window,
avatar-demand cutover, network proof or whole-epic completion is claimed here.
Ruma Receipt lacks PartialEq; a future diff boundary must compare receipt fields
and filter slot-only changes, not assume arbitrary value equality.

Parent has audited the entire SDK diff and the one-line Core caller migration.
Final cross-model review: DeepSeek V4 Flash medium, read-only — **Correct-to-merge
subject to root integration CI**, no Critical/Important findings. Parent and reviewer
read the full 359-line SDK/Core patch. The invariant expects are intentional internal
consistency checks, not a new fallback or externally supplied condition.

Reviewed SDK commit `35ed65c95bcef777d471b778177014a90af6a37b` was pushed to
`shinaoka/matrix-rust-sdk-work:koushi/shared-receipt-snapshots` and fetched afresh by
the clean root task worktree. This PR pins that commit and changes only Core's
receipt iterator entry point plus documentation. Clean root Core suite: 944 passed,
8 existing ignored, 6.09 s; testkit: 225 passed. SDK library/doctest results above
apply to the exact pinned source. Dependency setup was separately compiled.
The independent architectural RED tests remain preserved in the integration
worktree, not removed from their pending requirements or claimed green here.
