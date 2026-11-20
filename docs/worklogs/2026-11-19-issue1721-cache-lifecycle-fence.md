# #1721: the cache lifecycle contract waits on the shared fence, labelled by stage

## What the issue asked for, and what was already true

#1721 reports the load-sensitive timeout in
`crates/koushi-core-testkit/tests/contracts/event_cache_lifecycle.rs`: the
restored-timeline wait failed under CI load with the original **fixed 30 s
deadline** (`let deadline = started + Duration::from_secs(30)`).

That premise is stale. `a8445304` (#1756/#1757) already replaced both fixed
deadlines with a progress-based watchdog, and the wait now fails on silence, not
on elapsed time. A local load repetition on current `main` — 16 CPU burners plus
ten concurrent copies of the exact test on macOS (18 cores) — passed 10/10, so
there is no residual fixed-deadline failure to reproduce.

What remained is the reason the issue kept re-opening: this contract carried its
**own copy** of the watchdog — `PROGRESS_WINDOW` / `HANG_BACKSTOP` / `struct
Watchdog` / `wait_for_state` — written before the shared fence existed
(`koushi_core_testkit::StateWaitWatchdog` + `support::wait_for_state_event`,
#1759). The copy had drifted:

- its failure said `first_sync silence after 30s` and nothing about the state the
  wait was stuck on, while the shared fence reports the cause, the observed
  events, and a private-data-free state summary;
- it had no branch for a closed snapshot stream;
- a change to the fence contract would not reach this contract test at all.

## What changed

`crates/koushi-core-testkit/tests/contracts/event_cache_lifecycle.rs` drops its
private watchdog and uses the shared fence at all five convergence points:
`first_sync`, `restore`, `new_update` and `logout` through one twelve-line
`wait_for_stage` helper, and `cached_delivery` through the same watchdog in its own
event loop. `HANG_BACKSTOP` comes from the testkit
instead of a local copy, and `lane_probe`'s liveness bound is the shared silence
window rather than a number of its own, so a loaded host that admits late is no
longer reported as `lane_wedged`. That probe runs only on the failure path, where
it may now take up to `PROGRESS_WINDOW` instead of a private 10 s; a passing run
is unaffected.

`crates/koushi-core-testkit/tests/support/mod.rs` keeps one fence loop and gives
it an optional stage label: `wait_for_state_event_at(stage, connection, watchdog,
predicate)`, which `wait_for_state_event` and `wait_for_state_event_within` call
with `None`. The unlabelled diagnostics the fence contract tests assert are
therefore unchanged.

The file also gains the usual `#[path = "../support/mod.rs"] mod support;`, so the
`contracts` binary now registers this file's three tests plus the two
`oauth_fixture` self-checks, the same duplication every other including file has
(`docs/architecture/test-suites.md`).

`docs/policies/engineering-rules.md` states the carve-out the code already
practised: product and QA waits keep one monotonic absolute deadline, while
integration-test convergence waits use the shared silence-window-plus-backstop
fence, and a multi-stage contract labels each wait.

No product code is touched, and no assertion is relaxed: the restored timeline
must still carry the persisted first event without server replay, and the
`initial_items` observations are unchanged.

## Verification

```bash
cargo fmt --check                                              # clean (cargo fmt --all rewrites vendor/)
cargo test -p koushi-core-testkit --test contracts --profile ci event_cache_lifecycle   # 5 passed (3 here + 2 oauth_fixture self-checks)
cargo clippy --profile ci -p koushi-core-testkit --all-targets --locked -- -D warnings  # clean
node scripts/check-agents-docs.mjs                             # ok
```

Load repetition: `cargo test … --profile ci --no-run` to get the binary, then 20
concurrent copies of `--exact automatic_cache_delivers_first_sync_persisted_restore_and_new_updates
--nocapture` with 16 CPU burners on an 18-core macOS host: **20/20 passed**
(baseline on unmodified `main`: 10/10). Wall time per attempt stayed under a
second; the wait itself settles in single-digit milliseconds, so the window is
never approached.

RED: `state_wait_watchdog::a_labelled_wait_names_the_stage_it_stalled_at` fails
while the label is removed from the fence message and passes with it.

Captured stage diagnostic (the shipped window shortened to 1 ms for the sample,
the room lane and stage label are the real ones):

```text
state predicate was not satisfied at stage `first_sync`: silence for 2.05ms after
1 state events in 4.10ms (window 1ms, backstop 20ms); last observed generation 1,
session authenticating, sync stopped, room selected false, rooms 0, invites 0
```

Before this change the same stall read `first_sync silence after 4ms (window 1ms,
backstop 20ms)` — no stage name outside the watchdog's own label, no state.

## Canon consulted

- `REPOSITORY_RULES.md`: canon-first for a QA gate contract, and a worklog for a
  QA gate change.
- `docs/policies/engineering-rules.md` "Async and Runtime" 1 (the deadline rule
  this change writes the fence exception into) and "Logging and Diagnostics"
  rule 3 (the observed-state summary renders counts, flags and kinds only).
- `docs/architecture/overview.md` "SDK Event Cache Lifecycle" for what the
  contract is allowed to assert about the restored cache.
