# #2073 — measure scroll-back over cached history after a relaunch

Independent review of the nightlane that measures "scroll up in a room whose
history this device already holds". The scenario itself stays; what changed is
what it claims, how it decides pass or fail, and how it waits.

## What the lane could not have proven

- **The only pass/fail criterion was an absolute page time.** 500 ms per page
  under `--profile=ci` (no optimisation, debug assertions) on a two-vCPU
  `ubuntu-latest` with a containerised homeserver measures the runner, so the
  gate flakes on a slow night and misses a regression on a fast one.
- **`scroll_back_cached_history=ok` was unconditional.** `network_pages` and
  `room_messages_requests` were computed and then used only in the failure text,
  so a walk that went entirely to `/messages` also passed — the one thing the
  lane exists to disprove.
- **A fixed 5 s settlement sleep.** `handle_paginate` returns without emitting
  `PaginationStateChanged` when a pagination is already in flight
  (`navigation.rs`), and the restore's own hydrate/backfill is exactly that. A
  page submitted inside that window waited out `EVENT_TIMEOUT = 30 s` and looked
  like a hang.
- **The distribution mixed two populations.** Core answers a request with no
  viewport observation by sliding its bounded display window
  (`bounded_window.rs`: 240 slots / 60 rows), so those pages never reach the SDK,
  and the SDK itself serves from the store only the chunk it reloaded last.

## Gate policy: report, do not gate on an absolute time

The page budget is printed, never enforced. Every run prints the per-page wall
time plus p50/p95/max, separately for SDK pages (the claim) and for all pages,
with store-served, window-only, network, gap-repair-skip and refused-attempt
counts. The lane fails on these alone:

- **runaway** — a page slower than `KOUSHI_QA_SCROLL_BACK_RUNAWAY_MS`
  (default 10 s), a page whose own terminal state never arrives before
  `EVENT_TIMEOUT`, or a walk that never reaches `EndReached`;
- **lost cache path** — no page was store-served (reached the SDK with zero
  forwarded `/messages`) while the walk did forward requests, in which case the
  token would only have said that pagination finished;
- **no progress** — the walk spent `max_pages` real pages, or as many
  gap-repair skips, without reaching the start.

Why this does not flake on a slow runner: nothing compares against a wall-clock
page time except a hang-shaped threshold an order of magnitude above the
measured baseline. Why it cannot miss a regression: the cache path is a hard
condition, and a slowdown moves the printed distribution the next night — the
numbers below are the reference for that comparison.

## Measured baseline (macOS, release-class host, `--cargo-profile=ci`)

Tuwunel v2.0.0, 300 seeded messages, restore, then the walk:

```
pages=3 sdk_pages=2 window_only_pages=1 store_served_pages=2 network_pages=0
room_messages_requests=0 gap_repair_skips=1 refused_attempts=0 walk_ms=929
sdk_page_wall p50=89ms p95=89ms max=89ms
sdk_page_sdk  p50=46ms p95=46ms max=46ms
all_page_wall p50=81ms p95=89ms max=89ms
```

Synapse (Docker), the same seed:

```
pages=4 sdk_pages=3 window_only_pages=1 store_served_pages=3 network_pages=0
room_messages_requests=0 gap_repair_skips=1 refused_attempts=0 walk_ms=968
sdk_page_wall p50=71ms p95=92ms max=92ms
sdk_page_sdk  p50=43ms p95=51ms max=51ms
all_page_wall p50=71ms p95=92ms max=92ms
```

Both hosts are much faster than the CI runner and the numbers are not a CI
budget; they are the distribution a regression would shift. The `walk_ms` above
the sum of the page times is a gap-repair skip waiting for the repair to end
before the next request.

Chunk placement is not controlled by the seed, so the page mix moves between
runs: the same 300-message seed produced 3 pages with no skip, 5 pages with two
skips (the contributor's tuwunel run), and the runs above. Which pages the store
serves varies with where the sequential sends left the chunk boundaries, and the
scenario says so rather than arranging it.

## What changed in the scenario

- `PageSample` records `reached_sdk`, the per-page forwarded request count, the
  SDK duration and the account work-gate wait, and classifies a page as
  store-served (`reached_sdk && room_messages_requests == 0`).
- `wait_for_timeline_idle` replaces the fixed sleep: it tracks paginations in
  flight against terminal states and `ItemsUpdated` for the key and returns after
  `TIMELINE_IDLE_QUIET = 750 ms` with nothing outstanding. It only sees
  paginations that start after it is called; one already running is covered by
  the retry, which the page deadline bounds.
- A page that produces no event at all is retried after the timeline is idle
  again (`PAGE_SILENCE_PROBE = 2 s`) for as long as its deadline lasts; a page
  that already reported `Paginating` extends its deadline instead of
  resubmitting. There is no attempt cap: a page queued on the account work gate
  is silent until it is admitted and every further attempt is dropped as
  `in_flight`, so counting attempts would fail a page that was progressing.
- A page Core answered with an instant `Idle` skip because a gap repair held the
  timeline does not count toward the walk budget, and the walk waits for the
  repair to end before its next request.
- Terminal states are accepted from any request id at or after this page's first
  submission (request ids are monotonic per connection), so a late state from
  the restore's own pagination cannot settle the page.
- Percentiles are computed over `reached_sdk` pages only; all-page wall times are
  printed beside them.
- The seed no longer waits for the last body after its last send: every send
  already completes with its own event id, and the extra wait raced the
  projection that carried that row — it timed out on both 300-message runs here
  before the fix and on none of the three after it.

## Related source contracts

- `crates/koushi-core/src/timeline/diagnostics.rs` maps the `gap_repair` outcome
  explicitly instead of letting it fall into `other`, so the scenario matches the
  skip on a named token rather than on an unmapped bucket.
- `scripts/check-rust-test-structure.mjs` pins
  `RENDERER_PAGE_EVENT_COUNT = 100` in the scenario to `TIMELINE_PAGE_EVENT_COUNT`
  in `apps/desktop/src-tauri/src/commands/timeline.rs`.
- `scripts/desktop-headless-local-qa.mjs` forwards the scenario's three
  `KOUSHI_QA_SCROLL_BACK_*` knobs and treats the scenario as core-only, so the
  lane it runs is the lane the docs describe. (`idle_sync_gap_inspection`'s
  documented `KOUSHI_QA_IDLE_INSPECTION_*` knobs are still not forwarded — the
  same gap, left alone here because it is a separate scenario.)

## Verification

- `cargo fmt --all -- --check`, and the QA binary builds with
  `--features qa-bin --profile ci`.
- `cargo test -p koushi-qa --profile ci --features qa-bin --bins` (155 tests),
  `cargo test -p koushi-core --profile ci` for the diagnostics token.
- `node scripts/check-rust-test-structure.mjs` plus its test file, and the page
  size drift was proven by setting the copy to 50 and reading the violation.
- The scenario ran green on both local homeservers, and the cache-path condition
  was proven load-bearing: forcing `store_served_pages = 0` with forwarded
  requests fails with "no page was served by the local store".
