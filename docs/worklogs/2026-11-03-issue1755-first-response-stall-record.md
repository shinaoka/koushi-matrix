# #1755 — a bounded first-response stall record

## Decision

#1736 measured that holding the first Simplified Sliding Sync response makes
Koushi retry silently for hundreds of seconds: the four forced faults collapse
into two kinds of record, so an export cannot say where a run stopped. The
SyncActor now reports that stall as bounded diagnostics; nothing about the wait
itself changes.

Read before editing: `REPOSITORY_RULES.md` (canon-first, verification,
"Output must be private-data-free"), the engineering rules diagnostics format,
and the sync sections of `docs/architecture/state-machine.md`. No canon change:
the first-response gate, retry policy and projected sync status are untouched.

Shape, matching the existing `room_list_reconcile_wait` diagnostic in
`crates/koushi-core/src/sync.rs`:

- one watchdog per run generation, racing the observer's own wait;
- `core.sync first_response_stalled` at `Warn` after 30 s, repeated with a
  doubling window capped at 5 min;
- `core.sync first_response_stalled_cleared` at `Debug` when the response
  finally commits, only if a stall was reported;
- fields: `phase`, `elapsed_ms` since the generation started, `generation`.

`phase` names the last step the actor can observe: `sync_started` (observer
started), `sync_running` (`SyncService::State::Running` published) and
`encryption_sync_requested` (the encryption sync stream for a new generation
started; its readiness `Pending` is set at the head of that stream, before the
cross-process lock is acquired, so it is **not** proof that a request was sent). A
request count (`attempt`) is **not** reported: the SDK exposes no per-request
observable for the `room-list` connection, and the existing states only advance
after a committed response. The phase vocabulary is the closest available
stand-in for the "asked nothing yet" versus "asked and got no answer" split.

Known limit: the observer only consumes a committed response while no
replacement reconciliation is pending (`sync.rs:1334` and the matching
`can_consume_commit` in `sync/observer.rs:47`), so a commit that lands during a
replacement recovery is not seen by the watchdog and can still produce a stall
record before its clear.

## Verification

- `cargo test -p koushi-core --lib`: **1,884 passed**, 11 ignored. Includes new
  watchdog tests for the phase vocabulary, the doubling window, one record per
  episode and the silent path.
- `cargo test -p koushi-core-testkit --test runtime -- runtime_room_list_sync::`:
  **8 passed**, including a contract that holds the first room-list response,
  asserts exactly one `first_response_stalled` with
  `phase=encryption_sync_requested`, `elapsed_ms >= 30000` and three fields,
  then the single `_cleared` after release, plus a healthy-start negative case.
- `cargo clippy -p koushi-core -p koushi-core-testkit --all-targets -- -D
  warnings` passed; `cargo clippy --release -p koushi-core --locked -- -D
  warnings -D clippy::large_stack_frames -D clippy::large_futures` passed;
  `cargo fmt --check` passed; `node scripts/check-agents-docs.mjs` passed.

Not run: the whole workspace and QA-binary clippy gates, the remaining testkit
binaries, and any headless or GUI QA lane. The change is diagnostic-only and
touches no QA scenario.

Formatting note: `cargo fmt --all` rewrites `vendor/matrix-rust-sdk` path
dependencies as `docs/agents/verification.md` warns; use `cargo fmt --check`.
