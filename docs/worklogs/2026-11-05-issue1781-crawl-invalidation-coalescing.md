# #1781 — a late-decryption burst no longer multiplies by the joined-room count

## Decision

On a fresh 0.32.0 install the account recorded 3,070 `cause=reopen` crawl
enqueues in a 97-second window (peak 162 in one second) while the crawler had
processed and indexed nothing, together with late-decryption stream lag. The
amplification was in `crates/koushi-core/src/search.rs`: a lag report reopens
every available room because the affected rooms are unknown, and
`mark_room_needs_reread` removed the room's checkpoint (forgetting its durable
record), aborted the active page, enqueued from the beginning, emitted a new
`HistoryCrawlStarted` and pumped — per room, per report. One signal therefore
cost the joined-room count, repeatedly, before any useful indexing happened.

Read before editing: `REPOSITORY_RULES.md` (root-cause discipline, state-machine
discipline, test placement), the Search History Crawler section of
`docs/architecture/state-machine.md`, `docs/agents/verification.md`, and
`docs/architecture/test-suites.md` (Core private unit-test modules stay attached
to the library).

Canon amended in the same change: the Search History Crawler section documents
the coalescing rules and the new `Running --> Queued` follow-up edge. The
reducer-visible contract (`HistoryCrawlStarted/Progress/Completed/Failed/
Stopped`, `AppState.search_crawler`) is unchanged, so no reducer, protocol or
React change was needed; scheduling stays in Rust.

Shape (all in `crates/koushi-core/src/search.rs`, plus
`HistoryCrawlCheckpoint::starts_over` in `search_crawler.rs`):

- `note_reread_needed` records one room's durable `needs_reread` obligation and
  reports whether the record actually changed. `mark_room_needs_reread` (one
  room) and the lag sweep (all joined rooms, one record write) share it. A write
  that failed is retried by the next reopen (`search_progress_unsaved`), so a
  restart cannot trust the older settled record.
- `enqueue_reread` coalesces the obligation into existing work: a page in flight
  becomes one `pending_reread` entry (recorded while paused too); a queued
  checkpoint that starts over absorbs the signal; a queued continuation or
  catch-up is replaced in place by a fresh full scan. Only a room with no work
  at all is enqueued, once, and only while the crawler is not paused.
- A running scan keeps its position, so a burst cannot postpone progress by
  discarding each page. The end of that scan consults `pending_reread` instead
  of settling: the last page's progress is recorded, no `HistoryCrawlCompleted`
  is published, and exactly one fresh scan is queued. A stale completion
  therefore cannot clear a newer obligation. A failed page leaves the entry set
  for the next scan that starts after the change; a preempted page keeps its
  checkpoint and the entry.
- Pause, membership pruning, explicit stop and session shutdown clear the
  coalescing state; the durable `needs_reread` flag is what carries an
  obligation across a restart, where such a room is not seeded as settled.

## Verification

- RED before the fix (same test file, `search.rs` from `origin/main`): five of
the six new tests in `crates/koushi-core/src/search/crawl_invalidation.rs`
failed. The burst test's lag-only round reported 400 enqueues for 100 reports
over 4 rooms; the stale completion test settled the room and cleared the
obligation; the paused sweep published 10 enqueues; the queued continuation was
re-enqueued; the restart test found the stored record had lost its
`needs_reread` row. The sixth case is a product-level guard, not RED.
- GREEN after the fix: `cargo test -p koushi-core --lib search` — 108 passed,
  2 ignored, including the six new cases and the existing crawler scheduling,
  catch-up and reopen cases.
- Bound chosen from that measurement, not from a tuned constant: one entry per
distinct affected room per sweep, and zero for a repeat over unchanged pending
work. The fixture holds page progress with an interactive account-work permit
and settles the mailbox with a foreground empty-query command, so the
assertion does not depend on notification count or wall-clock time.
- `cargo test -p koushi-core -j4`, `cargo fmt --check`, `cargo clippy
  -p koushi-core --all-targets -- -D warnings`,
  `node scripts/check-diagnostic-test-isolation.mjs`,
  `node scripts/check-leaf-crate-boundaries.mjs`,
  `node scripts/check-rust-test-structure.mjs`, `node scripts/check-agents-docs.mjs`,
  `node scripts/user-help.mjs --check`.
- No user-guide change: no menu path, label, or documented outcome moved. The
  Settings crawler status row is unchanged.
- Independent review (gpt-6.1-sol, post-implementation, read-only): verdict
  `findings-fixed`. Fixed from it: a page that outlives a pause could settle the
  room (the pending entry is now recorded while paused); a running scan restarted
  from the newest event after every page, which could postpone progress forever
  under a continuous burst (it now keeps its position and the obligation is
  served at the end of the scan); a failed page dropped the obligation (it now
  stays set); the lag sweep scanned the queue once per room (the room-id index is
  consulted first); a failed record write could leave the older settled record on
  disk (it is retried by the next reopen); the lag sweep and the restart case
  were not covered by their own evidence (lag-only round, real store writer).
  Dismissed: the `biased` mailbox branch that prefers actor messages over a ready
  page result is pre-existing actor fairness, unchanged here, and not what the
  measured amplification was; changing it would need its own measurement across
  every search path.

## Known limits

- The renderer-side responsiveness measurement from the issue (input-to-visible
  selection latency and frame gaps under load) is not part of this change; the
  queue improvement is reported on its own. A synthetic fixture shows the
  enqueue bound, not a measured UI latency.
- A page that *fails* does not queue a retry itself: the obligation stays in
  memory and in the durable record, and the next room-list notification, restart
  or later signal serves it. Retrying a failing page in place was already out of
  scope before this change and is unchanged.
- The end-to-end case only proves that the room settles and the event is
  searchable. The mock SDK indexes synced events on its own, so that case does
  not prove which path indexed the event; the crawler's obligation is proven by
  the actor-level cases.
- Retrying a failed record write happens on the next reopen signal, so a broken
  store costs one write attempt per signal for as long as it stays broken. That
  is the failure path, already logged as `crawl_progress_save_failed`, and it is
  preferred over silently trusting the older settled record.
