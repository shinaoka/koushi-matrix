# Design: Deep-anchor restore Stage 2 — cache-only bulk backward load (issue #123 Phase C)

Status: design spec. Date: 2026-06-24. Follows Stage 1
([2026-06-24-deep-anchor-restore-fix-design.md](2026-06-24-deep-anchor-restore-fix-design.md)),
which is merged + harness-GREEN (deep anchor reached offline, but O(number-of-chunks)
restore cycles). Stage 2 makes the restore ~O(1) for contiguous on-disk history.

## Problem (Stage 1 residual)
Stage 1 reaches a deep anchor, but the live `Timeline::paginate_backwards` returns
after ONE stored chunk per call (`live_paginate_backwards`, matrix-sdk-ui
pagination.rs:107), so koushi's restore loops O(number-of-chunks) actor cycles
(~12 for depth 200 in the harness; ~seconds at months-deep). Each cycle carries
actor/relay/reschedule/emit overhead (~17ms), which is the real cost — the SQLite
chunk read itself is sub-ms. Chunk capacity is fixed (`DEFAULT_CHUNK_CAPACITY = 128`,
matrix-sdk-base store/traits.rs:34) but actual stored chunks are often smaller
(sync fragmentation), so chunk COUNT scales with depth.

## SDK facts (investigation-confirmed, file:line)
- The live Timeline subscribes to `RoomEventCache` updates; ANY
  `RoomEventCacheUpdate::UpdateTimelineEvents` broadcast (incl. from an external
  pagination caller) is ingested (matrix-sdk-ui tasks.rs:256). Disk chunk loads
  broadcast unconditionally via `conclude_backwards_pagination_from_disk`
  (matrix-sdk room/pagination.rs:313).
- BUT a `Skip` adaptor keyed on `skip_count` (matrix-sdk-ui subscriber.rs:74-98)
  HIDES prepended items while `skip_count > 0` (typical session: `max(0, N-20)`).
  Only `live_lazy_paginate_backwards` (controller/mod.rs:425-438) decrements
  skip_count, and it is reached only through `Timeline::paginate_backwards`.
  ⇒ Loading events into the cache alone does NOT reveal them; skip_count must be
  decremented by the loaded count.
- `run_backwards_until` is NOT cache-only: it resolves a Gap via NETWORK
  (`paginate_backwards_with_network`, matrix-sdk caches/pagination.rs:253). No
  existing public disk-only bulk path.

## Approach (Option C — minimal vendored patch, two new methods)
A vendored matrix-rust-sdk patch is unavoidable but non-invasive: add new methods,
touch no existing call-path (`run_backwards_once`/`run_backwards_until`/
`live_paginate_backwards` unchanged).

1. **matrix-sdk** `crates/matrix-sdk/src/event_cache/caches/room/pagination.rs` —
   add `pub async fn run_backwards_cache_only(&self, n: u16) -> Result<CacheOnlyOutcome>`
   on `RoomPagination`. Loop the existing one-chunk disk loader
   `load_more_events_backwards` (room/pagination.rs:161); on
   `LoadMoreEventsBackwardsOutcome::Events` call
   `conclude_backwards_pagination_from_disk` (fires the broadcast the live Timeline
   ingests) and accumulate the loaded event count; STOP — without networking — on
   `StartOfTimeline` (reached_start=true) or `Gap` (return a "hit_gap" marker, NO
   `paginate_backwards_with_network`). Return total events loaded + reached_start +
   hit_gap. This preserves the offline guarantee (never networks).
2. **matrix-sdk-ui** `crates/matrix-sdk-ui/src/timeline/pagination.rs` — add
   `pub async fn live_restore_from_cache(&self, n: u16) -> Result<RestoreFromCacheOutcome>`
   on the live `Timeline`. It (a) calls `run_backwards_cache_only(n)` on the room
   event-cache pagination, then (b) decrements the Timeline `skip_count` by the
   number of events actually loaded (reuse the `live_lazy_paginate_backwards` skip
   machinery, controller/mod.rs:425) so the loaded items are REVEALED. Returns
   reached_start / hit_gap / loaded_count.

## koushi wiring (koushi-core)
`handle_restore_timeline_anchor` / continuation (crates/koushi-core/src/timeline.rs):
replace the per-chunk `paginate_once(Backward, event_count)` loop with a single
`self.timeline.live_restore_from_cache(N)` call (N sized to the Rust budget). If it
returns `hit_gap` before the anchor (cache not contiguous to the anchor — needs
network history not on disk), fall back to the existing bounded paginate loop (which
may network) so non-contiguous caches still degrade gracefully. The coalescing +
deterministic-settle from Stage 1 remain: emit ONE ItemsUpdated when the restore
terminates. The diff-buffer/settle terminals (Found/EndReached/BudgetExhausted/
Failed/Superseded) are unchanged.

## Verification (cycle-count regression gate — RED→GREEN)
The `cache_restore` harness (Task A) currently gates on Found (Stage 1 GREEN) with
cycles as a diagnostic (~12/room). Stage 2 re-adds a **cycle-count gate**:
`CACHE_RESTORE_MAX_CYCLES` (e.g. 3). RED on Stage-1-only (~12 > 3), GREEN after
Stage 2 (the single bulk load ⇒ ~0-1 backward-paginate cycles). The PRIMARY Found +
zero-network-origin assertions stay. This is the headless proof of the speedup; no
human eyes.

## Repo-rule compliance
- Vendored-SDK change: justified (no public cache-only bulk path exists; needed for
  the offline-restore product requirement) and minimal/revertible (new methods only).
  Record in the upstream feedback ledger per REPOSITORY_RULES; the methods are
  upstream-friendly (general cache-only restore primitive).
- Private-data-free throughout: no event/user/room ids, bodies, raw SDK errors in
  logs/QA tokens.

## Out of scope
- Position-drift (symptom B "微妙にずれる") offset_px subpixel fix — separate, frontend.
- Non-contiguous-cache deep restore that genuinely needs network history — falls back
  to the bounded paginate loop; not the Stage 2 fast path.

## Workflow
spec (this) → codex design review → harness cycle-gate added (RED on Stage 1) →
Sonnet implements vendored patch + koushi wiring (isolated worktree) → harness GREEN
→ codex diff review.
