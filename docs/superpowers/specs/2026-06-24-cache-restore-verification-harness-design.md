# Design: Offline cache-restore verification harness (issue #123, Phase C — verification first)

Status: design spec. Date: 2026-06-24. Tracks GitHub issue #123 Phase C.

This is the **verification harness ("体制") — built BEFORE any fix**, per the
binding rule: *correctness is guaranteed by reproducible headless verification,
never by human visual inspection; build the harness first, then fix (体制→修正).*
The same harness that reproduces the problem here will be the green proof of the
fix in the follow-up.

## Problem (codex-confirmed root cause)

On app relaunch, returning to a deep-history position is laggy (symptom A), and
the exact scroll position is slightly off (symptom B). Independent codex
investigation of `origin/main` confirmed:

- The persisted scroll anchor (`NavigationState.room_scroll_anchors[room]` =
  `{event_id, offset_px, updated_at_ms}`, persisted in `navigation/navigation.v1.enc`)
  IS auto-restored after subscribe — but **frontend-driven and pagination-based**,
  not a core-owned cache-only path.
- When the anchor event is not already in the live window, restore runs
  `handle_restore_timeline_anchor` → `paginate_backwards` loop. The SDK paginate
  is **disk-first then network**: contiguous cached chunks are served from the
  event-cache store (`EventsOrigin::Cache`), but a **gap / missing linked chunk
  is resolved with network `/messages`** (`EventsOrigin::Pagination`, which
  Koushi's observer labels `network`). **There is no direct "restore this event
  from disk cache only" path.** So the lag is pagination-bound — disk-bound or
  network-bound depending on cache continuity.
- Symptom B: `offset_px` is a rounded integer DOM distance; restart layout
  differs (fonts/media/virtualization) → drift.

So the open question the harness must answer empirically: **on a restart with
the network blocked, can the deep-history timeline/anchor be restored purely
from the on-disk cache, and how long does it take (and how many paginations)?**

## Goal of the harness

A reproducible, private-data-free **headless core-QA scenario** that, in a
**stress-like environment (deep history × many rooms)**:

1. builds and fully loads history into the on-disk SDK event cache,
2. restarts the runtime over the same data dir with **network blocked**, and
3. measures whether timeline + deep-anchor restore is served **only from cache**
   (no `/messages`), **how long it takes**, and **how many pagination batches**
   it costs.

On current `main` this characterizes/repro­duces the problem (network-required
and/or many-batch latency). After the fix it must show cache-only, low-latency,
few/zero-batch restore. It becomes a permanent regression gate.

## Harness design

A new `headless-core-qa` scenario `cache_restore` (sits beside `timeline_stress`,
`restore_cleanup`, `send_queue`), composing existing building blocks:

- **Fixture (stress):** create `CACHE_RESTORE_ROOMS` rooms (default e.g. 6) on a
  local Conduit/Tuwunel server, each with deep history `CACHE_RESTORE_DEPTH`
  messages (default enough to exceed the initial window and require several
  pagination batches, e.g. 120+). Reuse `timeline_stress` fixture helpers where
  possible. A helper account sends the history.
- **Connect 1 (populate + confirm fully loaded):** subscribe each room's
  timeline; paginate backward to `EndReached` so the FULL history is contiguous
  in the event-cache store. Record, per room, a **deep anchor** = an early event
  id (near the start). Assert each room reached `EndReached` (token
  `cache_restore_loaded=ok rooms=N`) — i.e. "all messages loaded" confirmed.
  Persist (data dir) and shut down cleanly (drop SDK handles in runtime context).
- **Connect 2 (restart + offline restore measure):** new runtime over the SAME
  data dir; `RestoreLastSession`; **block the network** with the existing
  `headless-core-qa` `QaTcpProxy.disable()` pattern (the `send_queue` offline
  injector) so no `/messages` can succeed. With `KOUSHI_STARTUP_TRACE=1`
  enabled, for each room: subscribe and drive the existing anchor-restore path
  toward the recorded deep anchor; capture from the Phase A origin observer and
  `startup_trace`:
  - whether the anchor was reached, and the **origin breakdown** (cache vs
    network vs sync) for the restore — assert **no `network` origin** when the
    cache is contiguous,
  - the **restore latency** (ms) per room and aggregate,
  - the **number of pagination batches** used to reach the anchor.
- **Tokens (private-data-free):** e.g. `cache_restore_loaded=ok`,
  `cache_restore_offline=ok|network_required`, `cache_restore_origin=cache_only|mixed`,
  `cache_restore_ms=<N>` (bucketed/aggregate), `cache_restore_batches=<N>`.
  No room/event/user ids, bodies, tokens, or raw SDK errors.

### What it proves

- **On current main:** with network blocked, restore either fails as
  `network_required` (cache gap dependency) or succeeds cache-only but with a
  measurable multi-batch latency — empirically characterizing whether the lag is
  network- or disk-pagination-bound. This is the reproduced/RED state.
- **After the fix (separate follow-up):** restore must be `offline=ok`,
  `origin=cache_only`, low `ms`, few/zero `batches` — the GREEN proof. The
  `EventsOrigin` observer emitting no `Pagination`/`network` is the regression
  gate codex recommended.

## Why a local-server harness (not the real account)

Reproducible, deterministic, no maintainer GO, no real credentials, fully
headless — satisfies "correctness without the human's eyes". The real-account
`startup_latency` lane stays as an optional confirmation; this local harness is
the gate.

## Out of scope (explicitly)

- The FIX (Rust-owned cache-aware subscribe-time restore + position robustness)
  — that is the NEXT phase, gated by this harness.
- The read-receipt avatar work (exclude-self + bottom-right) — separate, gets its
  own headless verification (koushi-state unit test + Playwright).

## Verification of the harness itself

The harness is "correct" when, run on current `main`, it executes end to end and
emits a clear characterization (offline restore status + origin + ms + batches)
— i.e. it reproduces/measures the problem deterministically. It must not depend
on timing sleeps; it asserts on `CoreEvent`/snapshot/tokens.

## Workflow

spec (this) → implementation plan → Sonnet implementation → codex review → run on
`main` to confirm it reproduces/characterizes the problem. THEN, separately, the
fix is designed and implemented, gated green by this harness. The controller
verifies every gate independently.
