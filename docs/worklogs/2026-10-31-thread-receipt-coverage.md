# Thread receipt coverage after sign-in (#1696)

## Scope and ownership

Keep unread authority in the Matrix SDK. Koushi continues to mirror its
per-thread counts; neither Core nor React clears counters speculatively.
The private SDK correction separates event ownership (root in main, replies
in their thread) from receipt coverage (unthreaded public/private receipts
cover earlier events across scopes). There are no changes to public API shape,
runtime ownership, persisted schema or networking model.

A completed independent pre-review accepted this bounded design. Earlier
incomplete reviews were not treated as approval. The implementation reuses
existing store locks, validated linked-chunk metadata and the existing generic
pending-receipt/newest-boundary selector. Event timestamps and numeric chunk
identifiers are not used as chronological evidence. Unknown targets do not
imply coverage.

## Verify-first evidence

Before production changes, the fresh-session headless scenario failed on both
disposable local servers. Five replies were loaded. The unthreaded-only room
showed thread/chip counts of 5 on Tuwunel and 6 on Synapse. The extra root was
being counted as a thread event. Synapse's same-target clash case also
regressed after a duplicate threaded read. No external account or homeserver
was used.

Three new independently written SDK integration tests failed before the fix:
covering receipts preserving a later unread reply, a stored covering receipt
competing with a newer threaded boundary, and root double-counting. The tests
were not copied from Element Web or any other project.

After the fix:

- All three focused SDK regressions passed, including a final extension for
  unknown targets. The receipt matrix covers public/private receipts and
  targets in the same thread, main timeline and another thread. A further
  RED/GREEN extension catches an unknown outside target arriving later in a
  main-only sync without another receipt. Existing pending targets route an
  empty refresh to the affected thread; no fake receipt is emitted. The
  counter change is observed through the existing `ThreadInfo` subscription.
- The initial SDK event-cache integration group passed: 90 tests; after the
  pagination-only correction below, the expanded group passed all 91 tests.
- The SDK receipt-cache unit group passed: 24 tests. The final expanded
  event-cache library unit group passed: 112 tests (including those 24).
- The original Synapse headless scenario passed after correcting a QA observer
  race: the sequential subscription helper discarded thread history updates
  while waiting for the room's initial items. One collector now handles both
  streams and correlates both subscriptions and the read confirmation. Every
  phase observed all five replies, chip/thread/badge zero, including after
  the read and after restarting the runtime.
- The same Tuwunel scenario remains red for the separate main-counter failure
  below. Its thread and chip counts are zero, including after restart.

Rust builds used invocation-scoped `KACHE_EXPLAIN_MISS=1`,
`CARGO_BUILD_JOBS=8`, `RUST_TEST_THREADS=8` and eight dynamically selected idle
CPUs with `pick_idle_cores.py 8` plus `taskset`. No shared target directory was
deleted. `kache doctor --verify` passed.

The focused SDK commands were:

```bash
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml \
  -p matrix-sdk --features testing --test integration \
  event_cache::threads::unthreaded_receipts --locked
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml \
  -p matrix-sdk --features testing --test integration event_cache:: --locked
cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml \
  -p matrix-sdk --features testing --lib \
  event_cache::caches::read_receipts --locked
```

These used a separate local target directory. The SDK workspace's existing
manifest/lock mismatch first rejected `--locked`; resolving it locally updated
unrelated, unused search dependencies. The SDK tests ran with that local
resolution, after which the original tracked SDK lock was restored byte for
byte. No unrelated dependency update is part of this fix. Koushi checks use
its own unchanged root lockfile with `--locked`.

Related Koushi checks passed with the unchanged root lockfile:

- `cargo test -p koushi-core --lib read_state --locked`: 79 tests.
- `cargo test -p koushi-sdk --lib room_projection --locked`: 51 tests.
- `cargo test -p koushi-state --test thread_receipt_scope --test thread_badge_state --test threads_list_unread_remainder --test sidebar_unread_badge_consistency --test attention_surface --locked`: 45 tests.
- `cargo test -p koushi-qa --features qa-bin --bin headless-core-qa --locked`:
  135 tests after the post-review missing-room regression below.
- `cargo clippy -p koushi-qa --features qa-bin --all-targets --locked -- -D warnings`.
- `cargo fmt --check`, SDK and root `git diff --check`, and the agent-doc,
  headless-QA-lane, Rust-test-structure, SDK-submodule and user-help checks.

Broader Linux-local checks subsequently ran as recorded below. This is not a
claim that all cross-platform, browser or hosted-CI gates are green. No PR,
push or SDK gitlink update has occurred while Tuwunel acceptance is parked.

The application-level commands were:

```bash
node scripts/desktop-headless-local-qa.mjs --run --core \
  --server=synapse --scenario=thread_receipt_fresh_session --timeout-ms=480000
node scripts/desktop-headless-local-qa.mjs --run --core \
  --server=tuwunel --scenario=thread_receipt_fresh_session --timeout-ms=480000
```

## Expanded local gates

All Cargo builds below use dynamically selected idle CPUs, eight build jobs,
`RUST_TEST_THREADS=8` and invocation-scoped `KACHE_EXPLAIN_MISS=1`.

| Check | Observed result |
| --- | --- |
| `cargo test --profile ci --workspace --exclude sidebar-composition --exclude key-management --locked` | 4,605 passed, 16 existing ignored |
| CI-profile workspace/all-targets clippy with `-D warnings` | Passed |
| CI-profile QA-bin clippy and QA-bin tests | Passed; 135 tests |
| Release workspace clippy with `-D warnings -D clippy::large_stack_frames -D clippy::large_futures` | Passed |
| `cargo test --release -p koushi-core-testkit --test stack_budget --locked` | 3 passed |
| CI-profile wasm check of state/search/protocol | Passed |
| `cargo deny check`; `cargo machete` | Passed |
| Desktop typecheck, lint, boundary checks, build, tracked-file secret scan | Passed |
| Desktop Vitest, `--maxWorkers=2` | 2,362 passed in 192 files |
| Browser-headless, configured one worker | Chromium 611 passed; local WebKit unexecuted (missing `libavif.so.16`); CI owns WebKit verification; local full gate failed |
| Root format, diff checks, diagnostic isolation, protocol/QA boundaries, retained reachability, agent docs, QA lanes, Rust test structure, SDK pin and help checks | Passed |

Browser startup first failed before any test because the host's inotify watcher
limit was reached. An invocation-only `CHOKIDAR_USEPOLLING=1` retry ran all
Chromium tests successfully, then the first WebKit launch failed before its
assertions: the installed runtime cannot find `libavif.so.16`. The remaining
94 tests did not run. Both failed attempts are retained; no shared limits,
services or packages were changed, and the local browser gate is **not green**.

Local WebKit was not executed because `libavif.so.16` is missing; CI is
responsible. The user explicitly accepted leaving it unexecuted locally and
prohibited shared-host library/package changes. The existing `browser-headless`
job in `.github/workflows/ci.yml` runs
`npx playwright install --with-deps chromium webkit`, then `npx playwright test`.
That CI result is pending, not assumed green; no extra local run is required.

An additional standalone `cargo fmt --manifest-path vendor/matrix-rust-sdk/Cargo.toml -p matrix-sdk -- --check`
reported existing formatting deviations in four untouched SDK files and an
unchanged import in `thread/mod.rs`. Each offending region was compared with
SDK HEAD and confirmed unchanged. No newly added/changed region was flagged;
this is not a claim that whole-package SDK formatting passes. Unrelated
baseline formatting has not been folded into this fix.

The final call-placement correction described below was followed by fresh SDK
91/112-test runs, CI-profile workspace and QA clippy, QA 135 tests, release
strict clippy and the release stack-budget tests. Synapse's original scenario
again passed with five replies and zero counts at fresh/read/restart, and
`thread` plus `read_state_convergence` passed on both local servers:

```bash
node scripts/desktop-headless-local-qa.mjs --run --core --server=synapse \
  --scenario=thread_receipt_fresh_session,thread,read_state_convergence --timeout-ms=480000
node scripts/desktop-headless-local-qa.mjs --run --core --server=tuwunel \
  --scenario=thread,read_state_convergence --timeout-ms=480000
```

The known Tuwunel fresh-session acceptance failure was not rerun merely to
reconfirm its parked boundary. Native macOS/Windows and hosted CI were not run.

## Completed post-review and history-only correction

The broad post-review and its discovery-limited retries did not complete and
are not approvals. Two bounded, enforced-read-only gpt-6.1-sol reviews then
completed against the actual diff and line-numbered immediate callers.

- SDK finding, verified against the pagination and dispatch source: room-only
  history pagination persisted an outside receipt target and recomputed only
  room receipts, leaving the thread stale. The approved corrective change now
  calls the existing per-room `Caches` owner after releasing the room
  state/store write guard and before pagination returns. Final self-review
  retained the original room-diff publication ordering: normal pagination
  publishes under its room guard, then releases it and refreshes; targeted
  repair keeps its original release/publication sequence, then refreshes.
  No awaited refresh is inserted between releasing the room guard and
  publishing its diffs. That owner refreshes only existing thread caches with
  pending receipts, through
  the existing private empty-update path and `ThreadInfo` observable. Normal
  backwards pagination and targeted gap repair both use the refresh. No cache
  owner is created by the lookup, and no public API, schema, ownership design,
  callback framework, queue or synthetic receipt is added.
  `unthreaded_pagination.rs` independently reproduces this: only room history
  reveals a main receipt target between older replies and a later unseen
  reply. Before the fix it failed with unread 3 instead of 1; afterwards all
  four combinations of public/private receipts and normal/targeted pagination
  pass. The test also requires the updated `ThreadInfo` boundary at reply 2
  and no invented receipt/thread event. The affected SDK integration group
  passes 91 tests and the event-cache library group passes 112 tests.
- QA finding: default zero counters could pass when the room summary was
  absent from every sampled snapshot. The real evidence validator now
  requires an observed room summary. A focused regression first failed, then
  passed with the guard; all 135 QA binary tests and QA clippy passed. The
  original Synapse scenario was rerun after this correction and remained
  green, with all five replies loaded and every phase's counts zero.

These completed reports cover their bounded SDK and Koushi QA/canon scopes;
they do not establish merge readiness, prove authorship history, rerun tests,
or approve the parked server failure. The user accepted the QA correction and
approved the minimal SDK history refresh. PRs, pushes and the SDK gitlink
update remain on hold pending the user's Tuwunel acceptance decision; other
regressions and local gates continue independently.

## Local commit snapshot and publication hold

The SDK fix is saved on local branch
`feat/1696-unthreaded-thread-receipts` at
`c681d5e9bf6e5cce00aec65c66fc4d6653afffec`. The Koushi RED scenario, QA
corrections and documentation are collected on
`feat/1696-thread-unread-after-sign-in`.

The parent repository's tracked SDK gitlink remains
`401be3df774170499be743de26bdd88c1cb58f56`. Its local SDK checkout now contains
the topic commit, so the unstaged submodule HEAD difference is intentional;
it is not an authorized gitlink update. The earlier SDK-pin guard PASS
preceded the local SDK commit. This snapshot is not a self-contained consumer
integration until an authorized gitlink update is made.

SDK and Koushi PR body drafts are prepared locally, without opening a PR.
Pushes, PR creation and the gitlink update remain on hold until the user
decides the Tuwunel boundary. Assertions are not relaxed and no acceptance
waiver is inferred. No additional measurements, reviews or issues are needed
for this local commit snapshot.

## Parked: Tuwunel's initial receipt omission

For the clash room, the writer sets both an unthreaded and a root-specific
receipt on the last reply. Token-only raw-response diagnostics prove that:

- A full `/sync` returns both receipts.
- A fresh Simplified Sliding Sync response returns only the threaded receipt.
- The unthreaded-only control room receives its unthreaded receipt normally.

Consequently the SDK has no room-wide boundary with which to mark the root
and the later main-timeline message as read. The clash room has main badge 2
but thread and chip counts 0, at fresh open, after threaded read and after
restart. This is not repaired by inventing a receipt, assuming a threaded
receipt covers main, suppressing counters or switching production to legacy
sync. The original zero-count assertion has not been relaxed.

The failed attempts and the pre-fix evidence are retained in local QA
artifacts, with only counts/booleans in the scenario output. Raw payloads and
private identifiers are not copied here.

The Tuwunel case needs a server correction or an explicitly agreed acceptance
boundary before the both-server deliverable can be called complete. Do not
merge or describe the issue as fixed while that required check remains red.
