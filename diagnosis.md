# #1634 / #1636 — why the sync status stays `starting` while sync is working

Worktree: `investigate/1634-status-projection-stuck`, off `origin/main` = `7857aa7a`.
Artifacts: nightly run `41735586438`, jobs `126263835232` (tuwunel) and
`126263834576` (synapse). Line numbers are for `7857aa7a`.

## One sentence

`AppState.sync` is a **derived value that was never re-derived**: for an initial sync run the
only code that ever projects `SyncLifecycleStatus::Running` is the sync observer's
first-response → room-list-reconcile handoff, and that handoff never completed for the run
under test, so the app keeps the run's `Starting` record while the sync service keeps
committing responses — nothing about the sync is stale, only its projection.

## Evidence chain

### The projection is single-sourced, and it is downstream of the room list

- Producer: `send_sync_status` (`crates/koushi-core/src/sync.rs:221-241`). It is the **only**
  producer of `AppAction::SyncStatusChanged` in the workspace.
- Consumer: `handle_sync_status_changed` (`crates/koushi-state/src/reducer/sync.rs:8-44`).
- `AppAction::SyncStarted` / `SyncRecovered` / `SyncReconnecting` / `SyncStopped` /
  `SyncFailed` have **no producer** in product code (`crates/koushi-state/src/reducer/mod.rs:985-989`
  handles them; a workspace-wide grep finds only `koushi-core/src/state_delta/tests.rs:1146`).
  So the reducer's `handle_sync_started` — which would let `SyncEvent::Started` promote
  `Starting` → `Running` — is unreachable, and `AppState.sync` can only become `Running` from a
  `SyncStatusChanged { status: Running }`.

The four sites that project `Running`:

| site | line | precondition |
| --- | --- | --- |
| `SyncActorControl::Recovered` | `sync.rs:430-441` | actor lifecycle is `Reconnecting` (a reconnect inside a live run) |
| `handle_start` idempotent branch | `sync.rs:526-529` | actor lifecycle is already `Running` |
| observer `Signal::Reconciled` handoff | `sync.rs:1484-1502` | this run's first committed response was reconciled by the RoomActor and `connected == false` |
| observer encryption-handoff | `sync.rs:1576-1588` | same, for an owner-loss replacement (`replacement_recovery.is_some()`) |

**The site that should have fired here is `sync.rs:1484-1502`** (initial run, no replacement
recovery). `sync.rs:1576-1588` needs `State::Terminated` to have set `replacement_recovery`;
`Recovered` needs a `Reconnecting` lifecycle; `handle_start`'s branch needs `Running`. None of
the three matches a run whose last status record is `starting`.

Two facts make this the only candidate rather than a guess:

- `SyncActorControl::FirstResponseCommitted` (`sync.rs:403-412`) sets `self.lifecycle = Running`
  and **projects nothing** — the status comes from the observer block that sends that control
  (`sync.rs:1490-1502`), so a lost or rejected control would not by itself explain a missing
  status; the same block sends it.
- `do_stop` projects `Stopped` before it returns (`sync.rs:656`), and `fail` projects `Failed`
  (`sync.rs:473-480`), so a stopped or failed run cannot leave `Starting` behind.

### `Running` was projected for the other runs in the same process

Both aggregate artifacts print `sync_a=running` and `sync_b=running`
(`core-core-stdout.log:2` and `:16` in both the tuwunel and the synapse artifact), and the lane
reached `e2ee_unverified_peer_send_nonblocking=ok` — the token immediately before the failing
step — so participants A and B (named tokens) and A2 (whose `wait_for_sync_started_and_running`
at `scenarios/identity.rs:1706` passed, since the lane continued past it) each completed that wait
earlier in the *same process, same binary, same homeserver*. The B2 block is skipped in the
nightly lane because `--e2ee-recipient-second-device` is not passed
(`scenarios/identity.rs:2861`). Only the last-started run (B3) never completed the wait.

The failing step is `start_sync_for_qa(&mut participant_b3.conn, "blocked QA B3 sync")`
(`crates/koushi-qa/src/bin/headless_core_qa/scenarios/identity.rs:3010`), and its
`last_command_lifecycle=starting` says that Start was the **idempotent acknowledgement**
(`sync.rs:518-531`): the product had already started B3's sync from the trust-promotion path
(`crates/koushi-core/src/account/trust_gate.rs:1481` `spawn_sync_actor` +
`:1482-1486` `normal_sync_started`), which had already projected `Starting`.

### Generation 1 is the account runtime's first projection, not the 9th command

Three different counters are in play and none of them is the one the artifact looks like it is
comparing:

| name | scope | definition |
| --- | --- | --- |
| `generation` in `status_projected` | **one AccountActor** | `sync_generation.fetch_add(1)+1` in `send_sync_status` (`sync.rs:221-241`) |
| `run_generation` / `backend_generation` | one `start_sync_service` | `sync.rs:564`; fences `SyncActorControl` (`sync.rs:150-157`) and the room-list reconcile ack (`sync.rs:866-928`) |
| `AppState.sync_generation` | one `AppState` | the reducer's accepted high-water mark (`koushi-state/src/reducer/sync.rs:13-17`) |

`sync_generation` is owned by the AccountActor and documented as monotonic across SyncActor
replacement (`crates/koushi-core/src/account/actor.rs:1293-1295`, created at `:1640`), so a
restart inside one account runtime does *not* reset it. It is reset only with the AccountActor,
which is spawned together with the AppActor in `CoreRuntime::start`
(`crates/koushi-core/src/runtime.rs:919-930`) — i.e. a reset always comes with a fresh
`AppState.sync_generation = 0` (`crates/koushi-state/src/state/mod.rs:577`).

Consequences, per question:

- `last_status_generation=1` means the last projection in the whole process was the **first**
  projection of its account runtime: a `Starting`. B3's runtime had projected nothing before.
- `command_start=9` counts `core.sync stage=command` records (every `Start`, including
  idempotent acknowledgements); `status_projected=13` counts status records. They are different
  populations and their ratio is meaningless.
- A reset **cannot** make a later `running` stale in the shipped wiring: the only reset point is
  a new AccountActor, which comes with a new `AppState` whose fence starts at 0. The latent hole
  is that nothing *enforces* this coupling — if an AccountActor were ever replaced inside a live
  runtime (logout→login, account switch), its `1..N` projections would be silently dropped by
  `koushi-state/src/reducer/sync.rs:13` until they exceeded the previous runtime's high-water
  mark. Nothing in this run hits that path; it is a defect waiting for its first caller.

### The three `account_sync_actor_stop` records are not caused by stop effects, and the last trust recheck does not tear the sync down

- `runtime_diag_stop_command_effect=0` and `runtime_diag_stop_actor_projection=0` are accurate:
  no `AppEffect::StopSync` was dispatched anywhere in the run (`crates/koushi-core/src/runtime.rs:5716-5721`
  and `:6194-6198` are the only two dispatch sites).
- `account_sync_actor_stop` is recorded only in `stop_sync_actor`
  (`crates/koushi-core/src/account/runtime_children.rs:486-497`), which is reached from
  `stop_normal_runtime_children` (`:538-573`, also used by the trust **Gate** branch at
  `crates/koushi-core/src/account/trust_gate.rs:1452`) and from session replacement
  (`runtime_children.rs:215`). Those are teardowns, not stop commands — which is exactly why the
  stop-effect counters are zero.
- A recheck completion tears the sync actor down **only when it re-gates**: the decision comes
  from `trust_lifecycle_decision` (`trust_gate.rs:226-245`) — promoted + `Verified` →
  `AlreadyReady` (returns, nothing stops); promoted + not `Verified` → `Gate` → stop
  (`trust_gate.rs:1445-1452`) → sync actor stopped and a `Stopped` status projected.
- The artifact's trust path ends `…normal_sync_started > trust_recheck_finished_verified`, i.e.
  the last admission record is a recheck that finished **verified** *after* promotion, which maps
  to `AlreadyReady`. So the final recheck did **not** stop B3's sync actor, and the projection did
  not have to survive a teardown. This also means the earlier `trust_projection_reduced_gated`
  records in the same path belong to an earlier gate/promote cycle (of A2/B2), not to the last
  run.
- No timestamps exist in the captured stderr, so the correlation above is by record order and by
  the state machine, not by clock. `record()` never prints; only `record_and_stderr`
  (`crates/koushi-diagnostics/src/lib.rs:528-530`) does, which is why `core.sync`,
  `core.account` and `core.runtime` records are absent from `core-core-stderr.log` altogether
  (0 occurrences of each) while `core.timeline*` and `core.verification_admission` are present.

### What the QA wait actually requires

`wait_for_sync_started_and_running` (`crates/koushi-qa/src/bin/headless_core_qa/event_wait.rs:940-1004`):

- one 30 s deadline for the whole wait (`:946`, `EVENT_TIMEOUT = 30s`,
  `registry.rs:33`);
- event: `CoreEvent::Sync(SyncEvent::Started { request_id })` with **the caller's** request id
  (`:965-968`);
- state: `conn.snapshot().sync == SyncState::Running` (`:985`), re-checked after every event.

`start_sync_for_qa` (`:1065-1075`) submits `SyncCommand::Start` and then waits with that helper.
So the harness waits for a product-owned projection, not for an SDK signal. It is not reading the
wrong object — but the object it reads has only one producer, and that producer is downstream of
the room-list round trip. The harness therefore inherits the coupling; there is no other DTO that
means "this sync run is proven" (`SyncEvent::Running` is emitted from the same observer block,
`sync.rs:1495`).

**A harness defect that this round still hits:** the diagnostic string is built from
`koushi_diagnostics::snapshot()` (`crates/koushi-qa/src/bin/headless_core_qa/diagnostics.rs:421`,
filter at `:448`), which is the **process-global** ring (`crates/koushi-diagnostics/src/lib.rs:441`,
`DEFAULT_DIAGNOSTIC_CAPACITY = 10_000`). The aggregate lane runs four `CoreRuntime`s in one
process, and nothing in the summary is scoped to the stuck run: `sync_diag_committed_response`,
`last_committed_sequence`, `rooms_from_response`, `reconcile_wait`, `last_reconcile_outcome` and
`status_projected` may each describe a different participant. `sync=starting` and `session=ready`
are per-connection; every `sync_diag_*` field is per-process. That is why two rounds of triage
could not attribute this evidence, and it is the first thing to fix.

## Verdicts

### Q1 — where `running` is projected, and did it ever

**Observed.** The ring's last `core.sync status_projected` record is
`generation=1 lifecycle=starting`; `task_ended=0`, `last_observer_exit_reason=unknown`, no
`Failed` and no `Stopped` record after it, and `AppState.sync == starting`. `sync_a=running` /
`sync_b=running` in stdout prove the other runs in the same process reached `Running` through
`sync.rs:1484-1502`.

**Inferred.** For the stuck run, `Running` was never *requested*: a gen-2 record would be the
newest status record in the ring, and it is not there (the ring keeps the newest records
regardless of eviction). Site `sync.rs:1484-1502` is the one that should have fired; the other
three sites are excluded by precondition, above.

### Q2 — why generation 1

**Observed.** `generation` is `send_sync_status`'s per-AccountActor counter; `command_start=9`
counts commands; `status_projected=13` counts status records. A gen-1 record is by construction
the first projection of an account runtime.

**Inferred.** No reset can discard a later `running` in the current wiring, because the reset is
tied to the AppState's own lifetime. The `1..N` restart hazard exists only if an AccountActor is
ever replaced inside a live `AppState`; it is not on this path.

### Q3 — the three `account_sync_actor_stop` events

**Observed.** No `AppEffect::StopSync` anywhere (`stop_command_effect=0`,
`stop_actor_projection=0`); `account_sync_actor_stop` is recorded only by `stop_sync_actor`, which
is called from teardown paths. The final admission record is a **verified** recheck after
`normal_sync_started`.

**Inferred.** The stops are teardowns (gate re-projections and participant cleanup), not stop
commands, and the last recheck completed *without* tearing anything down — `AlreadyReady` by
`trust_lifecycle_decision`. The projection did not have to survive a recheck teardown.

### Q4 — is the QA wait reading the right thing

**Observed.** It waits for `SyncEvent::Started{our request_id}` and `AppState.sync == Running`
within a single 30 s deadline.

**Verdict.** Not a harness oracle bug in what it asserts — but the harness is asserting a product
projection whose only producer sits behind the room-list reconcile, so a room-list stall is
reported as a sync failure. The genuine harness bug is the process-global scoping of every
`sync_diag_*` field, which makes the failure string unattributable when several runtimes share a
process.

### Q5 — is synapse (#1636) the same shape

Field-by-field, tuwunel vs synapse: `started=true`, `sync=starting`, `session=ready`,
`service_build_failed=0`, `task_ended=0`, `command_start=9`, `command_stop=3`,
`command_restart=0`, `last_command_lifecycle=starting`, `last_state=running`,
`last_lifecycle=starting`, `last_observer_exit_reason=unknown`, `last_status_generation=1`,
`reconcile_outcome=received`, `first_response_stalled=0`, `last_stall_phase=none`,
`stop_command_effect=0`, `stop_actor_projection=0`, `account_sync_actor_stop=3`,
`session_invalidated=0` — identical. Different only in magnitude: `committed_response` 116 vs 134,
`status_projected` 13 vs 15, `reconcile_wait` 5 vs 6, `last_reconcile_elapsed_ms` 752 vs 618,
`last_committed_sequence` 899 vs 11, `last_rooms_from_response` 31 vs 0.

**Verdict: one defect wearing one symptom, not two defects.** Same last-projection shape, same
missing handoff, same trust tail. One caveat for whoever fixes it: the synapse value
`rooms_from_response=0` means the last committed response skipped the
`RefreshCommittedProjection` send (`sync.rs:1405-1412`) that tuwunel's `31` would have taken, so
the two artifacts cannot be pointing at that send as the shared blocking await. What they *do*
share is the structural cause: the sync lifecycle's only `Running` producer is behind the room-list
path.

## The smallest correct fix, and its blast radius

**Do not derive the sync lifecycle from the room-list round trip.** Project
`SyncLifecycleStatus::Running` from this run's own sync evidence — `SyncService::State::Running`
plus the first committed response that carries a `pos` (`sync.rs:1376-1395`) — and leave room-list
authority where it already lives, in `RoomListReadiness` and the `committed_response_owned` /
`range_fully_loaded` fence (`crates/koushi-core/src/room/observer`-side:
`crates/koushi-core/src/room/list_observer.rs:164-215`). Concretely, the `else` branch of
`if !connected || reconnecting` at `sync.rs:1431-1444` is where the lifecycle can be closed; the
existing reconcile keeps owning the room list, and `Signal::Reconciled` keeps owning `connected`
for reconnect bookkeeping only.

Blast radius:

- `crates/koushi-core-testkit/tests/runtime/runtime_room_list_sync.rs:378-389` pins that a
  committed *partial* `all_rooms` range must not project `Running` before the RoomActor reconciles
  the complete range. That assertion has to be re-read, not deleted: what it protects is that the
  room list must not be *authoritative* on a partial range, and since #1737/#1745 that is enforced
  independently by `committed_response_owned` (`room/list_observer.rs:164-215`) and by the
  separate `RoomListReadiness` projection. Canon agrees that the two contracts are separate
  (`docs/architecture/state-machine.md`, "Room-list bootstrap readiness": `SyncState::Running` is
  not evidence that the room list is authoritative).
- User-visible: `SyncState::Running` (and the `SyncConnectivityChanged { proven }` effect, plus
  the #1009 session-status wake-up and the Secure Backup admission replay) would fire a few
  hundred milliseconds before the room list becomes authoritative. Anything that treats
  `Running` as "the list is authoritative" is already a canon violation.
- It makes a room-list defect *visible as a room-list defect*: the QA's next wait
  (`wait_for_room_in_room_list`) would fail instead of the sync-status wait, which is the
  accurate report.

**What would break if the projection merely moved "a little earlier" without re-reading the pin:**
the testkit pin fails, and the ambiguity it encodes comes back — a `Running` that can be reached
with a partial or unreconciled range. The distinction that matters is *which* evidence closes the
sync lifecycle (this run's committed response) versus *which* evidence makes the room list
authoritative (the reconcile). Move the former; leave the latter.

**Before that lands, the enabling diagnostic (QA-only, no product behaviour change):** scope the
failure string to the stuck run — put `backend_generation` into `committed_response` and
`status_projected` and filter the summary by it, or snapshot/reset the ring per participant. Then
the next occurrence names the await instead of leaving it to inference. That is the same
sequencing the owner set on 2026-11-03, and this artifact shows why it is still the right first
step.

## What is ruled out, and by what

| hypothesis | ruled out by |
| --- | --- |
| The RoomActor silently dropped the reconcile ack (pre-#1768) | `last_reconcile_outcome=received`; a stale ack now answers `StaleGeneration` → `Failed` → `internal_observer_failure_at` → `task_ended` record **and** a `Failed` projection. Neither exists. |
| The reconcile timed out | `last_reconcile_outcome=received`, not `timeout`/`ack_closed`/`invalid_ack`; `last_reconcile_elapsed_ms=752` is far below `ROOM_OBSERVATION_ACK_TIMEOUT = 10s` (`sync.rs:56`, `:894-900`). |
| The first-response gate never opened (no committed response at all) | `first_response_stalled=0`: the watchdog (`sync.rs:1186-1247`) records a stall when its 30 s window expires, and that window starts at `run_started_at` (`sync.rs:568`), which is *earlier* than the QA's own 30 s deadline, so a run with no first response would have produced a stall record before the QA gave up. |
| Stop/restart churn reset the state | `do_stop` projects `Stopped` before returning (`sync.rs:656`); no `Stopped` record follows the `Starting`, `command_restart=0`, and no stop effects exist at all. |
| The reducer's generation fence dropped the `Running` projection | the counter is per account runtime (`account/actor.rs:1293-1295`), the run's `Starting` at generation 1 *was* accepted (`sync=starting`, not `stopped`), and a gen-2 `Running` would have passed `koushi-state/src/reducer/sync.rs:13`. |
| `is_session_ready` coerced the status to `Stopped` | the observed state is `starting`, and the session is `ready`. |
| The QA wait itself is wrong (waits for an event the product never sends) | `sync_a=running` and `sync_b=running` are printed by the same lane in the same process; the wait is satisfiable and was satisfied three times before the failing step. |
| The last trust recheck tore the sync actor down | `trust_lifecycle_decision` returns `AlreadyReady` for promoted + verified (`trust_gate.rs:226-245`); only a non-verified recheck re-gates and stops (`:1445-1452`). |
| A `SyncActorControl` was dropped by the `run_generation` fence | the observer and actor share one `run_generation` for the run (`sync.rs:564`, `:1304`), and the handoff sends `FirstResponseCommitted`, which the actor accepts from `Starting` (`sync.rs:403-412`). |

## Reproduction: 3 local aggregate attempts, none reached the failing step

Environment: this macOS host, disposable local `tuwunel` (`~/.cargo/bin/tuwunel`) started by
`scripts/desktop-headless-local-qa.mjs`, `CARGO_BUILD_JOBS=3`, `CARGO_INCREMENTAL=0`,
`--server=tuwunel --core --scenario=all --cargo-profile=ci`. Nothing was pointed at matrix.org or
at a real account. The build of `koushi-qa --bin headless-core-qa --features qa-bin` took 2m56s
first time and 20s incrementally (kache warm).

The B3 step only exists in the **aggregate** lane: `verify_multi_user_multi_device_room_key_delivery_for_qa`
is called from `run_e2ee_trust_stage(..., Some((conn_b, key_b)))`, and that call site is inside
`if scenario == QaScenario::All` (`crates/koushi-qa/src/bin/headless_core_qa/orchestrator.rs:1831-1837`).
A focused `--scenario=e2ee_trust` run never reaches it, which is why the earlier round's five
focused passes proved nothing about this failure.

| attempt | outcome | stage |
| --- | --- | --- |
| 1 | failed, unrelated | `timed out waiting for new identity gate; phase=authenticating` |
| 2 | failed, unrelated | `timed out waiting for new identity gate; phase=rechecking_trust` |
| 3 | failed, unrelated | `thread_chip_dot failed: chip dot after threaded read` |

Three different pre-existing flakes stopped the lane before the B3 step, on a host that is also
compiling for other workers; I did not extrapolate from them.

What the attempts *did* establish, with temporary instrumentation (since reverted; the tree is
clean apart from this file) that printed every `core.sync` ring record and the reducer's
accept/reject decision:

- the happy path is `command start lifecycle=stopped` → `status_projected generation=1
  lifecycle=starting` → reducer accepts → `committed_response sequence=1` →
  `temp_reconciled result=reconciled connected=false` → `status_projected generation=2
  lifecycle=running` → reducer applies it (and, when the service went offline,
  `generation=3 reconnecting` → `generation=4 running` through `Recovered`, i.e. the
  `connected == true` handoff);
- every reconcile observed locally returned `reconciled` at `requested_sequence=1` — never
  `superseded` — so a superseded-reconcile livelock is not what the local runs do, and the
  artifact's `reconcile_wait` counts stay in single digits for the same reason;
- `RefreshCommittedProjection` (`sync.rs:1405-1412`) and the timeline forward
  (`sync.rs:1413-1424`) are plain awaited sends into mailboxes of 1024
  (`crates/koushi-core/src/lib.rs:18`); `action_tx` is 16384 (`runtime.rs:173`);
  `control_tx` is 4 (`sync.rs:331`). A block on any of them needs a wedged consumer, not a busy
  one.

## Residual uncertainty, stated plainly

The artifact localizes the stall to the observer's window between
`committed_response` (`sync.rs:1376-1404`) and `status_projected(Running)`
(`sync.rs:1496-1502`). Inside that window there are exactly four awaits: the
`RefreshCommittedProjection` send, the timeline-response forward, the reconcile round trip, and
the handoff's `control_tx.send`. The artifact excludes the reconcile round trip as the *last*
thing that happened (`received`), and the synapse `rooms_from_response=0` excludes the refresh
send as a *shared* cause, but it cannot name the single await — because every `sync_diag_*` field
is process-global while the run under test is one of four in the process. I did not close that
gap locally (3 attempts, all stopped earlier), so I am not claiming which await it was.

That residual does not change the fix: three of the four candidates are on the room-list path, and
the fourth is the control send inside the handoff that only exists because the lifecycle is closed
from there. Bounding the gate instead would convert a permanent stall into a recurring delay for
those three, which is why the owner's earlier refusal to do that still holds.
