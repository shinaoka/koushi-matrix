# #1735 — recover a stalled first sync from the startup gate

## Contract and decision

Consulted `REPOSITORY_RULES.md` (Canon-First, state machines, review and audit),
`docs/agents/environment.md`, `docs/agents/verification.md`, engineering rules
(async ownership, test placement, localization), and the Secure Backup/sync
sections of `docs/architecture/state-machine.md`. Amended that canon before
production edits; updated the matching security/recovery help page.

The AccountActor sends explicit startup recovery only for a live promoted
session with unproven connectivity. The SyncActor owns the decision:

- `Starting`: cancel and join the old observer, stop the SDK service through
  the existing teardown, then start a replacement run.
- `Running`: re-project without replacing the service, retaining #860 recovery
  of a missed connectivity projection.
- `Reconnecting`: retain the SDK's existing offline/replacement recovery. The
  report did not demonstrate a stalled reconnect; broadening that policy is
  outside this fix.

The joined-observer barrier orders its status messages before replacement
`Starting`; monotonic `sync_generation` rejects older projections, and
`run_generation` rejects old controls. No new sync owner or public transport
command is introduced. Retry with no promoted session returns
`SessionRequired` and arms no work.

Rust distinguishes a defer deadline before any accepted sync response
(`firstSync`) from an actual inspection timeout (`inspectionDeadline`). The
historical connectivity fact resets with full session teardown, not on each
connectivity loss, trust-gate re-entry, or Retry. Both deadlines retain the retryable timeout kind.
React renders the typed stage through a message supplied in English, Japanese,
Simplified Chinese, Traditional Chinese and Korean (pseudo-locales use the
shared generated catalog).

## RED, before the production fix

```text
cargo test -j8 -p koushi-core --lib retry_replaces_stalled_first_sync_request
retry_replaces_stalled_first_sync_request ... FAILED
Retry must issue a new sliding-sync request, not wait on the stalled run
test result: FAILED. 0 passed; 1 failed
```

A real SDK room-list request is held unanswered by a disposable loopback
homeserver. The bounded gate deadline fails before Retry; the regression
asserts that Retry issues another HTTP sync request. The original code did not.
The owned-server version was also run RED, before production edits.

## GREEN evidence

All Cargo commands used eight jobs and `RUST_TEST_THREADS=8`, through kache.
The store verification passed before work. No shared cache/service was changed.

- New focused actor suite: **7 passed**. It proves replacement HTTP requests,
  repeated Retry, inert late HTTP responses from two replaced runs, active-run
  response to `Running` to a real backup inspection, no revival after logout,
  account replacement isolation, no-session rejection, and no false first-sync
  label after prior connectivity or trust-gate re-entry. The no-backup fixture settles as
  `SetupRequired`; existing typed Ready-inspection settlement tests cover gate
  opening. This is not a configured-backup end-to-end stall fixture.
- Secure Backup owner suite: **49 passed** before the added trust-gate re-entry
  case; all **50** passed in the final workspace gate, including #860 bounded retryable
  failure and #1361 promotion/deadline cases. Existing sync generation/control
  tests also passed in the workspace gate.
- Canonical `cargo test --profile ci --workspace --exclude sidebar-composition
  --exclude key-management --locked`: **4,640 passed**, **16 existing ignored**,
  63 successful targets including doctests. The final run includes explicit
  server cancel-and-await shutdown and the trust-gate re-entry regression.
- Frontend: **192 files / 2,367 tests passed**, including corrected first-sync
  renderer copy, unchanged real-inspection timeout copy, and catalog completeness.
- Browser gate spec: **16 passed** (headless Chromium).
- Workspace all-target Clippy; QA-bin all-target Clippy; release workspace
  Clippy with `large_stack_frames` and `large_futures` denied: passed.
- Release `stack_budget`: **3 passed**, including the small-worker attachment
  path (the other two are the suite's existing shared fixture checks).
- `qa:headless-local -- --server=both`: passed against disposable Tuwunel and
  Synapse. Additional Core `login_sync,gate_restore` scenarios passed on both:
  `safety=ok`, `login_sync=ok`, `gate_restore_ready=ok`,
  `gate_verified_restore=ok`.
- Frontend build, typecheck, lint, secret scan, SDK guard, Rust test structure,
  command/snapshot contract, help navigation/generated links, formatting and
  diff whitespace: passed.

## Review

A completed independent design review accepted the Starting-only recovery,
joined-observer/generation fence, typed copy distinction and test boundary.
The earlier tool-budget-exhausted attempt was incomplete and was not counted.
The completed independent finished-diff review found one Important issue:
the historical fact was cleared on trust demotion. The reviewer accepted the
remaining retry/ownership/fencing, deadline coalescing, DTO/localization and
regression scope. Its truncated report was recovered through a bounded
restatement and the missing provenance/limitations tail, not a new review.
The main agent verified and corrected the finding as described below; the
reviewer did not independently rerun builds or re-review that correction.
Self-review traced command routing, SDK response/reconciliation, reducer
connectivity, deferred deadline, typed wire stage, and catalog rendering. It
caught the historical-fact reset on trust demotion: moved the reset to full
session teardown and pinned it with the seventh test. The first version of
that test harness omitted the reducer demotion acknowledgement and timed out;
no owned processes survived. Added the acknowledgement and a bounded
re-promotion wait; the final focused/workspace gates passed. Clippy and Core
QA were also rerun successfully after the production correction.

No dependencies or upstream code were imported. No work on #1736 or #1737.
Native GUI/macOS/Windows checks and the hosted CI matrix were not run locally;
headless evidence is the correctness basis. The cause of the original sync
stall remains outside this recovery fix.
