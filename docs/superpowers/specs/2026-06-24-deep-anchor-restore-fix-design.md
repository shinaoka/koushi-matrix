# Design: Deep-history anchor restore — fix (issue #123, Phase C)

Status: design spec. Date: 2026-06-24. Tracks GitHub issue #123 Phase C.
Gated by the offline cache-restore harness
([2026-06-24-cache-restore-verification-harness-design.md](2026-06-24-cache-restore-verification-harness-design.md)),
which reproduces the problem RED before this fix and turns GREEN as proof.

## Problem (codex-confirmed, file:line)

On app restart, returning to a deep-history scroll position is laggy (symptom A)
and the restored position is slightly off (symptom B). Root cause, confirmed
independently by codex against the local source + upstream matrix-rust-sdk:

1. The live UI `Timeline::paginate_backwards` returns after the FIRST non-empty
   stored chunk (`matrix-sdk-ui/src/timeline/pagination.rs:30-50,91-106`); the
   requested `event_count` bounds only the NETWORK path, not disk. Each
   app-level backward pagination advances by ~one stored chunk.
2. The app's restore hard-caps at `LIVE_ROOM_ANCHOR_RESTORE_MAX_BATCHES = 6`,
   `EVENT_COUNT = 100` (`apps/desktop/src/components/TimelineView.tsx:406-407`)
   → koushi `handle_restore_timeline_anchor` continuation
   (`crates/koushi-core/src/timeline.rs:1963-2056`) walks at most ~6 chunks
   (~tens of events) backward, then emits `AnchorRestoreFinished{BudgetExhausted}`.
3. CONSEQUENCE: a deep anchor (hundreds of events back) is NEVER reached within
   the 6-batch cap → deep position not restored (symptom B). Raising the budget
   reaches it but via O(number-of-chunks) one-chunk-per-call pagination, each
   round-trip incurring core/SDK/React diff work (symptom A lag). With contiguous
   on-disk history the SDK serves these from the store via `load_previous_chunk`
   (`matrix-sdk/src/event_cache/caches/room/pagination.rs:165-184`) — no network.
4. `TimelineFocus::Event` is NOT a cache-only O(1) alternative: its initial
   context comes from a network `/context` request and it is a separate Timeline
   instance from the live one (`matrix-sdk/src/event_cache/caches/event_focused/mod.rs:125-186`).
   Rejected.
5. There is NO public SDK "load N events of context around an event from disk
   only" API; the store trait exposes single-event `find_event` and one-step
   `load_previous_chunk` (`matrix-sdk-base/src/event_cache/store/traits.rs:72-126`).

## Fix — two stages

### Stage 1 — koushi-core correctness + React churn (no SDK change)

In `crates/koushi-core/src/timeline.rs` restore continuation:

- Replace the frontend-supplied hard `max_batches=6` ceiling with a **Rust-owned,
  cancellable restore budget**: keep walking backward until `Found`,
  `EndReached`, a paginate failure, or a generous safety bound (max chunks AND an
  elapsed-time cap). A superseding restore request cancels the in-flight walk.
  This makes deep anchors actually reachable (fixes symptom B "not restored").
- **Coalesce per-chunk emissions during restore**: while a restore walk is
  in-flight, buffer/suppress the per-chunk `TimelineEvent::ItemsUpdated` (and
  intermediate `PaginationStateChanged`) so React is not re-rendered ~N times
  mid-restore; flush ONE hydrated item update when the anchor is reached (or the
  budget ends). Must not drop diffs — flush the final consistent item set.
- Frontend: stop passing a tiny `max_batches`; the core budget is authoritative.
  React receives one settled update, not N intermediate growth diffs.

Stage 1 alone: deep anchor reaches `Found` offline (correctness gate GREEN), with
materially less React churn — but still O(chunks) Rust paginations (perf gate may
remain RED until Stage 2).

### Stage 2 — vendored matrix-rust-sdk cache-only bulk backfill (conditional on Stage-1 measurement)

Only if Stage-1 wall-time at stress depth is still too slow. Expose a **cache-only
bulk backward load** built on the store-level `load_previous_chunk`
(`matrix-sdk-base/.../store/traits.rs:89-104`): "load backwards until event /
count / start-of-cache, from disk only, never resolving a gap via network." koushi
restore calls it → O(depth) chunk walk collapses to ~1 call for contiguous cache.
Justify under `REPOSITORY_RULES.md` (vendored changes allowed when easy to
upstream/revert) and record in the upstream feedback ledger. Deeper variant
(`load_previous_chunks` batch on the store trait) is more invasive (memory/sqlite
backends) — avoid unless measurement demands it.

### Position robustness — symptom B drift (frontend)

`offset_px` is a rounded integer DOM distance (`TimelineView.tsx:251-284`);
cross-session reflow shifts it. Persist a richer anchor: subpixel offset,
row-height / intra-row ratio, neighbor event ids, viewport height. Restore after
font/layout readiness, then run ONE exact DOM correction after virtualization/
row measurement settles, suppressing anchor capture until the correction completes.

## Verification gate (codex-corrected)

Driven by the offline cache-restore harness (`headless-core-qa` `cache_restore`),
stress fixture (many rooms × deep history, fully loaded before restart, network
disabled after restart), restoring the deepest fixture anchor:

- **PRIMARY (correctness)** — `AnchorRestoreFinished{Found}` AND zero `network`
  origin (offline cache-served, via the #123 `EventsOrigin` tokens at
  `timeline.rs:1469-1499` / `startup_trace.rs:82-89`). RED on main
  (`BudgetExhausted` for a deep anchor); GREEN after Stage 1. GREEN must come from
  the Rust-owned budget reaching the anchor, NOT from hardcoding a larger batch
  count in the test (do not "bake in Found within max_batches=6").
- **SECONDARY (perf)** — pagination cycle count + wall-time ms. Diagnostic on main
  and after Stage 1; becomes a hard regression gate (cycles ≤ small N) after
  Stage 2.
- **Position (symptom B)** — pixel-tolerance "viewport lands on anchor" is a DOM
  assertion: covered by a Playwright/TimelineView test, not the core harness.

Private-data-free throughout (no room/event/user ids, bodies, raw SDK errors,
paths, secrets).

## Sequencing

1. Finalize the harness to the faithful two-gate form above; confirm RED on main.
2. Stage 1 (koushi-core) → correctness gate GREEN; measure cycles/ms.
3. Decide Stage 2 from the measurement; if needed, implement + re-measure → perf
   gate GREEN.
4. Position robustness + its Playwright gate.
5. (separate) read-receipt avatars exclude-self + bottom-right.
6. (last) AGENTS.md: codify verify-first/no-human-eyes; fix stale "no CI" note.

## Out of scope

- The read-receipt avatar work (separate track, own headless verification).
- Real-account confirmation (optional; the local harness is the gate).
