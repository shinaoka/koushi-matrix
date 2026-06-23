# Design: Crawler yield + startup delay (issue #123, Phase B)

Status: design spec. Tracks GitHub issue #123, Phase B — the fix for the felt
startup "loading", informed by Phase A measurements.
Date: 2026-06-23.

Phase A (observability) is complete and codex-clean. This spec is the first
behavior-changing slice for #123. It is subordinate to
`docs/architecture/overview.md`. The broader canonical-event-cache architecture
remains Phase C.

## 1. What Phase A proved

On a real-account cold restart (run 2 of the `startup_latency` lane), the
private-data-free tokens showed:

- `restore` ≈ 935 ms, `sync_to_ready` ≈ 5 ms, `room_list` ≈ 53 ms,
  `subscribe` ≈ 35 ms with `origin=cache` (initial timeline served from the
  on-disk cache, 0 ms) — **the timeline can paint immediately**.
- The startup auto-backfill pagination, however, reported
  `phase=paginate ms=1474` while the SDK paginate itself was
  `paginate ms=0 gate_ms=1473` — i.e. the call was a **cache hit (0 ms)** that
  **waited ~1.5 s on the `/messages` gate**, blocked behind a background
  search-crawler page (`crawler_page ms=1506`).

So the startup "loading" is **gate contention, not slow cache**: the
`MessagesBackpressure` gate is account-wide concurrency-1 and **non-preemptive**,
so a user-visible pagination waits a full crawler `/messages` round-trip. On the
maintainer's ~85k-event profile the crawler runs many such pages back-to-back,
scaling this to the multi-second freeze.

## 2. Goal

A user-driven `/messages` load (timeline pagination, and later room/context
open) must **never wait on the background history crawler** — `gate_ms` ≈ 0 —
**while preserving the deliberate account-wide concurrency-1 protection** that
keeps the client from tripping homeserver rate limits.

## 3. Decision (and why not the alternative)

**Keep concurrency-1 and add cancellation-based preemption + a crawler startup
delay.** Rejected alternative: two concurrent lanes (timeline + crawler in
parallel). Rationale:

- The gate exists for **rate-limit / homeserver protection** (one `/messages`
  at a time). Two lanes sacrifice that; the crawler's high page volume
  concurrent with user activity risks `429`s that slow *both*.
- Two lanes do **not guarantee** user priority — under server throttling the
  user load still competes. Cancellation keeps concurrency-1 **and** guarantees
  the user goes first.
- The cost of cancellation (one discarded in-flight crawler page) is negligible:
  history crawl is best-effort background work and simply re-fetches later.
- Smaller blast radius: add a cancellation token to the existing gate vs.
  re-architecting the concurrency model and its rate-limit reasoning.

Two-lane parallelism is explicitly deferred to Phase C, to be revisited only if
server-load measurement shows concurrency-1 is the bottleneck.

## 4. Design

Three cooperating pieces, all **actor/runtime level** — no reducer, no DTO, no
`AppState`, no state-machine change. Crawler timing is Rust-owned.

### 4.1 Crawler startup delay (`SearchActor`, `crates/koushi-core/src/search.rs`)

The crawler must not start history-crawl pages until a delay has elapsed after
it becomes active, clearing the startup window entirely.

- A core constant `CRAWLER_STARTUP_DELAY` (≈ 60 s; the maintainer confirmed a
  ~1-minute delay is fully acceptable). Not a user-facing setting.
- `SearchActor` records a crawl-start deadline = (crawler-activation instant) +
  `CRAWLER_STARTUP_DELAY`, where "activation" is when the crawler first has work
  (first `RoomsAvailable` / first enqueue). It does not start crawl pages before
  the deadline; the actor's `select!` loop wakes at the deadline to resume.
- Manual/QA probe crawls (explicit `StartHistoryCrawl` commands) are **not**
  delayed — only the automatic background crawl is gated by the deadline.
- The timer uses the executor abstraction already used for task spawning (no raw
  `tokio::time` in actor logic), consistent with the portability rules.
- The delay is a fixed interval. It does **not** reset on user activity (YAGNI);
  ongoing user activity after the delay is handled by preemption (§4.2).

### 4.2 Cancellation preemption (`MessagesBackpressure`, `crates/koushi-core/src/messages_backpressure.rs`)

The gate stays account-wide concurrency-1 and gains the ability to **cancel the
active crawler permit** when a user-visible timeline request needs the gate.

- The crawler permit carries a cancellation signal (e.g. a `Notify`/token held
  in the gate's shared state for the currently-active crawler permit).
- When `acquire_timeline()` is called and a crawler currently holds the permit,
  the gate **triggers that cancellation signal** (in addition to the existing
  `waiting_timeline` counter that already prevents *new* crawler acquisitions).
- The crawler's page (`run_history_crawl_page`, `search_crawler.rs`) runs its
  `/messages` call inside a `select!` against the permit's cancellation signal.
  On cancellation it **drops the in-flight `room.messages()` future** (aborting
  the request), releases the permit, and returns a new
  `HistoryCrawlPageResult::Preempted` outcome.
- `acquire_timeline()` then acquires immediately once the crawler releases.
- The existing behaviors are preserved: timeline priority for new acquisitions
  (`waiting_timeline`), and "a cancelled/dropped timeline waiter must not starve
  the crawler".

### 4.3 Re-queue on preemption (`SearchActor`)

- On `HistoryCrawlPageResult::Preempted`, `SearchActor` re-queues the active
  checkpoint (front of `crawl_queue`, preserving its `crawl_settings_generation`)
  so no history is lost — it is retried after the user load completes / the
  crawler resumes. A stale-generation preempted result is dropped like any other
  stale page result.

### 4.4 Diagnostic (reuse Phase A)

- Add one env-gated, private-data-free token via the existing `startup_trace`
  module, e.g. `koushi.startup phase=crawler_preempted`, emitted when a crawler
  page is preempted. This makes preemption observable in the `startup_latency`
  lane (`KOUSHI_STARTUP_TRACE=1`) and proves the fix. No new identifiers.

## 5. Ownership / boundaries

- Pure actor/runtime change in `koushi-core` (`search.rs`, `search_crawler.rs`,
  `messages_backpressure.rs`). No `koushi-state` reducer change, no `AppState`
  field, no Tauri DTO, no TypeScript, no React, no CoreEvent. The Tauri/GUI
  contract is unchanged.
- The crawler-vs-timeline priority and rate-limit policy stay Rust-owned in the
  gate and `SearchActor`.

## 6. Verification

Phase A's instrumentation is the verification harness for Phase B:

- **Real-account proof (re-run the Phase A lane):**
  `npm --prefix apps/desktop run qa:real-homeserver -- --scenario=startup_latency`.
  After the fix, run 2's `startup_lat phase=paginate` and the sub-phase
  `koushi.startup phase=paginate gate_ms=…` must show **gate_ms ≈ 0** (was
  ~1473), i.e. the user pagination no longer waits on the crawler. Requires
  maintainer GO (the persistent QA device is already kept).
- **Unit tests (`MessagesBackpressure`):** a timeline waiter cancels the active
  crawler permit and acquires immediately; a preempted crawler does not starve;
  the existing priority/no-starve tests still pass.
- **Unit tests (`SearchActor`):** a `Preempted` result re-queues the checkpoint
  (same generation, front of queue); the startup delay suppresses automatic
  crawl-page starts before the deadline and resumes after; manual probe crawls
  bypass the delay.
- **Gates:** `cargo build -p koushi-core` warning-free; `cargo test -p koushi-core`;
  `npm --prefix apps/desktop run qa:secret-scan`.

## 7. Risks / open questions

- **SDK cancellation safety:** dropping the `room.messages()` future must abort
  cleanly with no partial event-cache write. Expected (the response — and any
  cache write — happens after the await completes), but the implementer must
  confirm against the vendored SDK before relying on it. If unsafe, fall back to
  the cooperative variant (small crawler pages + don't-start-new, bounded short
  wait) for the in-flight case.
- **Crawler starvation under continuous user activity:** by design the crawler
  yields whenever the user is loading; it makes progress when the user is idle.
  Acceptable — history crawl is best-effort and the maintainer accepts crawler
  delay.
- **Delay tuning:** `CRAWLER_STARTUP_DELAY` is a fixed core constant; if 60 s
  proves wrong it is a one-line change.

## 8. Out of scope

- Auto-backfill suppression (render cached items without paginating when the
  cache fills the viewport): once contention is gone the startup auto-backfill is
  a 0 ms cache hit, so this is unnecessary for #123's felt pain. Revisit only if
  the spinner still flashes after this fix.
- The canonical-event-cache architecture and the intent-based load API (Phase C).
- Any UI/DTO/reducer change.

## 9. Workflow

design (this spec, Opus) → implementation delegated to a Sonnet agent against
the approved plan → codex diff review (AGENTS.md recipe) → re-run the Phase A
`startup_latency` lane (with maintainer GO) to confirm `gate_ms` ≈ 0. The
controller verifies every gate independently.
