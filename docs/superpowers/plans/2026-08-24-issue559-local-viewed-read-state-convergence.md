# Issue #559 — Local Viewed Boundary and Bounded Read-State Convergence

## Status, dependency, and mandatory gate

This is one independently mergeable Wave C task. Implementation must not start
until issue #570 is fully merged and this exact document has a recorded
`reviewer-flash` `Correct-to-merge` verdict under the user-approved quota
substitution. The branch must then be
rebased onto that merged main before RED scaffolding or production changes.

The current design is preparatory only. It does not authorize implementation
while #570 or the mandatory design review is pending.

## Product contract

The product currently conflates two facts:

1. **local viewed boundary** — the newest canonical readable event Rust has
   proved the user viewed at the live edge on this device; and
2. **server-confirmed read boundary** — the newest receipt/fully-read fact
   acknowledged by the homeserver and used for cross-device unread,
   notification, and highlight semantics.

Task #559 separates them. Local viewing advances the visible **Read up to here**
boundary immediately. Server confirmation remains authoritative for room/DM
badges, unread/notification/highlight counts, cross-device receipts, and durable
outbox drainage. A pending or failed write is visible as a small Rust-owned sync
status and never rewinds the local boundary.

No React optimistic count clearing, retry ledger, or Matrix ordering semantics
are introduced.

## Current defects and reusable authority

The existing Rust `ReadStateEngine` already owns session/operation fences,
position evidence, persistent intent, retries, waiter settlement, and
privacy-safe Debug. It currently retains up to eight candidates per key and the
supervisor starts one future per key without a global write cap.

`wake_all_desired_reads` and `wake_desired_reads_for_room` invalidate a scheduled
retry before waking. Ordinary reconnect/subscription checkpoints can therefore
turn backed-off keys into immediate synchronized retries.

`perform_read_network_operation` erases every SDK error to `()`. Completion
records Debug-level `failed`/`timed_out`, so user diagnostics can still report
`Errors: 0` while the durable outbox grows.

`TimelineView` independently infers live-edge arrival from DOM facts, calls
`sendReadReceipt` and `setFullyRead`, and discards both rejected promises. The
same render pass also reports a typed `TimelineViewportObservation` to Rust.
Rust's actor already validates and stores this observation against the canonical
window and derives navigation snapshots. That existing viewport command is the
single source this design extends.

## Rust-owned local viewed boundary

### Admission from accepted viewport facts

Remove automatic live-edge `sendReadReceipt`/`setFullyRead` submission from
`TimelineView`. React continues to report only:

```rust
TimelineViewportObservation {
    first_visible_event_id,
    last_visible_event_id,
    visible_gap_ids,
    at_bottom,
}
```

After accepting the actor generation and observation, the timeline actor derives
one local viewed target only when all are true:

- the key is a Room or Thread timeline, not Focused context;
- `at_bottom` is true;
- `last_visible_event_id` is the actor's latest canonical readable event;
- that item is an eligible stable event under #570's shared eligibility helper;
- the actor's `TimelinePositionIndex` supplies exact generation/rank evidence;
- there is no visible unresolved gap at or after the target.

React's DOM observation is input evidence, not product state. Rust verifies the
target against its canonical items before advancing anything.

The actor stores:

```rust
struct LocalViewedBoundary {
    event_id: String,
    position: ReadPositionEvidence,
}
```

A same event is idempotent. Within one position generation, only an equal/newer
rank is accepted. A new actor generation may replace the old position only after
that actor proves its own current live edge; a stale actor cannot publish. Room,
Thread, account, and session keys remain isolated. The boundary never moves
backward because of server confirmation, retry failure, replay, pagination, or a
stale completion.

On acceptance the actor immediately emits a changed navigation snapshot with
sync state `pending` and sends a generation-fenced internal manager message. It
does not wait for network I/O.

### Internal read-intent creation

The manager admits background intent without allocating a Tauri command waiter:

- Room: `PublicUnthreaded` when `send_read_receipts` is enabled, plus
  `FullyReadAndPrivateUnthreaded`;
- Thread: `ThreadRead` only when public receipts are enabled. Room-wide
  fully-read/private intent is driven only by the Room live edge, not by a
  thread actor whose position generation is incomparable;
- Focused: no automatic intent; returning to the canonical Room/Thread live edge
  is required.

When thread read receipts are disabled, the local thread boundary is
`notRequested` rather than falsely `synced`; no server write is expected.

The receipt privacy policy is not supplied by React. Runtime sends the initial
Rust-owned `settings.values.notifications.send_read_receipts` value and every
accepted settings change through a typed TimelineManager control update. Manager
fences it by session generation and reevaluates current local correlations; a
policy disable cancels only public/thread desired work and projects
`notRequested`, while room fully-read/private work remains.

Explicit user/API read commands retain their existing request waiters and
terminal responses. Automatic viewport intent is observed through the typed
navigation/read-sync projection, not a rejected promise.

The manager owns a bounded correlation map:

```rust
struct LocalReadCorrelation {
    timeline_key: TimelineKey,
    actor_generation: u64,
    local_target: ReadTarget,
    required_keys: BTreeMap<ReadStateKey, LocalReadRequirement>,
}
```

A requirement records the exact desired target and pending/active/failed/
confirmed state. `ReadCandidate` carries bounded correlation IDs separately from
explicit command waiters. Equal/newer confirmation settles every dominated
correlation; a newer desired target transfers dominated correlations. Room and
Thread never share an automatic fully-read requirement under the rule above.
The map is capped by the existing 128 read keys/active timeline ownership and is
removed on actor retirement, account/session replacement, or terminal sync.

The manager returns current-target settlement to the exact TimelineKey + actor
generation via the control lane. A replaced actor or older target cannot update
the projection.

## One desired target per read-state key

Replace `ReadKeyState.candidates: Vec<ReadCandidate>` with one optional desired
candidate:

```rust
struct ReadKeyState {
    desired: Option<ReadCandidate>,
    active: Option<ActiveReadOperation>,
}
```

`ReadCandidate` retains one target and all bounded waiters whose requested target
is equal to or dominated by it.

Admission rules:

- same event merges position evidence and waiters;
- an older/equal positioned target coalesces into the current desired target;
- a newer positioned target replaces desired, inherits all waiters, and cancels
  an older active operation through its fence;
- a later admission without comparable position replaces the desired target as
  the newest observation, while the actor-side local boundary still requires
  position proof;
- repeated same/latest viewport observations neither add candidates nor reset
  retry attempts;
- waiter count remains capped at 32 per key and key count at 128;
- operation-generation exhaustion remains fail-closed.

A failed/timed-out attempt settles only current explicit waiters, retains the
one desired target without waiters, and schedules retry. Success or an equal/
newer authoritative observation clears desired. A stale completion clears
nothing.

Persistence becomes an explicit encrypted **V2** one-ID envelope/path. V2
stores exactly one desired event ID per key and validates unique keys, bounds,
and nonempty IDs before restore.

V1 vector order is not an admission-order contract: current failure rotation
moves an attempted candidate to index zero before persistence. Migration must
not call `last()` the historical newest. Load V2 first. If only V1 exists, decode
it with the existing bounded validator and choose its existing next-wake
candidate (the last vector item) as a conservative one-time compatibility
policy, explicitly not a claim of newest chronology. Persist V2 atomically and
remove V1 only after the V2 generation-fenced write succeeds. A failed/rotated
A/B/C fixture pins the selected next-wake value and proves no multi-target replay.
Malformed V1 fails closed without deleting it.

All newly written/restored V2 state satisfies the issue's newest-only contract.
The one-time V1 migration preserves the old engine's next retry choice where
historical chronology is unknowable; it cannot fabricate ordering evidence.
No second live ledger or indefinite dual-write path is added.

## Bounded fair network dispatcher

Add a manager-owned dispatcher with:

```rust
const MAX_CONCURRENT_READ_WRITES: usize = 4;

struct ReadDispatchQueue {
    ready: VecDeque<ReadStateKey>,
    queued: HashSet<ReadStateKey>,
}
```

Only `dispatch_ready_reads` calls `state.wake`/`spawn_network`. It starts FIFO
keys while total active network/actor-apply operations are below four. One key
can own at most one active operation. A cancellation request does **not** release
a slot or clear engine active state; the exact `Cancelled` completion does so,
then dispatches the next key. This prevents replacement/cancellation races from
briefly exceeding four. Queue insertion deduplicates keys; identifiers are never
logged.

Fairness is FIFO among ready keys. A key that fails leaves the ready queue and
enters backoff, so a persistently unhealthy room cannot monopolize the four
slots. When its exact retry becomes due it rejoins at the tail. Newly admitted
keys also join at the tail.

The cap covers public, private/fully-read, and thread writes together across all
rooms. Actor-apply completion remains part of the active slot so network success
cannot overrun reducer/control settlement.

No sleep is used as a correctness fence. Existing exponential timers remain the
only time-based mechanism and tests control them through channels/tokens.

## Backoff and wake invariants

Keep exponential per-key delay from one to sixty seconds. Carry an optional privacy-safe server `retry_after` duration in the typed
failure; rate limits use `max(exponential_delay, retry_after)` capped by the
existing documented maximum unless the server's required delay is longer.

A scheduled retry is authoritative:

- Checkpoint, reconnect, sync reconciliation, and repeated viewport observation
  never invalidate or bypass it.
- They enqueue a key only when it has desired intent, no active operation, and no
  scheduled retry.
- A strictly newer desired target replaces payload but preserves that key's
  retry attempt and due token.
- Success or an authoritative equal/newer server confirmation resets attempts
  and cancels the scheduled token.
- A stale/cancelled due token cannot enqueue.
- Restored keys enter reconciliation, then one scheduled retry; checkpoints do
  not fan them out early.

This removes the current `invalidate_retry`-then-wake behavior from both bulk and
room checkpoint paths. A deterministic test holds due tokens and injects any
number of checkpoints/reconnects; request count stays unchanged until the exact
due token is released.

## Typed failure classification and diagnostics

Replace `Result<(), ()>` with a closed internal outcome:

```rust
enum ReadStateFailureKind {
    Timeout,
    Transport,
    RateLimited,
    Authentication,
    Server,
    Sdk,
}

struct ReadNetworkFailure {
    kind: ReadStateFailureKind,
    retry_after: Option<Duration>,
}

enum ReadNetworkOutcome {
    Succeeded,
    Failed(ReadNetworkFailure),
}
```

The outcome retains the exact operation fence/target already carried by
`ReadWorkerCompletion`. `ReadKeyState` stores `last_failure` only for the exact
current desired target. Newer admission, equal/newer authoritative confirmation,
or success clears it. Starting an exact retry projects pending but does not lose
the typed failure from diagnostics; a stale/cancelled completion cannot replace
it. `schedule_retry` consumes `retry_after` without storing raw HTTP values.

Classify `matrix_sdk::Error`/`HttpError` structurally:

- timeout deadline => `Timeout`;
- request/connection failures => `Transport`;
- HTTP 429 / Matrix limit-exceeded => `RateLimited`;
- authentication-required, forbidden/unknown-token classes => `Authentication`;
- HTTP 5xx => `Server`;
- malformed/local/other closed failures => `Sdk`.

Reuse existing Matrix HTTP/error-kind helpers where accessible; do not parse
Display/Debug strings. #608 remains the sole session-invalidation owner for
`SessionChange::UnknownToken`; #559 only classifies a read write and does not
infer or mutate session trust.

Completion diagnostics use `DiagnosticLevel::Error` for current failed/timed-out
attempts and include only:

- read key kind token;
- failure kind token;
- candidate/key/waiter counts;
- attempt, queued, and active counts;
- retry source and delay bucket.

No room IDs, event IDs, user IDs, bodies, raw SDK errors, URLs, or credentials are
recorded. Success/retry/admission stay Debug/Info as appropriate. Aggregate
errors therefore make the diagnostics summary nonzero without exposing private
Matrix data.

Bound repeated detail events with the existing diagnostics ring/aggregation;
do not create an unbounded failure history.

## Navigation DTO and UI

Extend `TimelineNavigationSnapshot` with:

```rust
pub local_viewed_event_id: Option<String>,
pub server_confirmed_read_event_id: Option<String>,
pub read_state_sync: TimelineReadStateSync,

pub enum TimelineReadStateSync {
    Synced,
    Pending,
    Failed { kind: ReadStateFailureKind },
    NotRequested,
}
```

Keep existing `read_marker_event_id` as the server-confirmed compatibility field
for unread derivation during this task; it must equal
`server_confirmed_read_event_id`. `read_marker_display_event_id` becomes the
Rust-derived local viewed display anchor when that event is present in the
canonical display projection, otherwise it uses the existing server/own-message
fallback. React does not compare event order.

Status aggregation for one local target:

- `pending` while any required current-key operation is queued, backed off, or
  active;
- `failed(kind)` after a current-target failed/timed-out attempt during backoff;
  deterministic precedence is Authentication > RateLimited > Timeout >
  Transport > Server > Sdk when the two required writes differ;
- `synced` only when every required key is confirmed at the local target or a
  newer authoritative target;
- `notRequested` only when policy disables every server write for that timeline
  (currently a Thread with read receipts disabled);
- stale target/key settlements are ignored.

Cross-device unread counts and `first_unread_event_id` continue to derive from
the server-confirmed marker. Only the visible read divider uses the local viewed
anchor.

Render a compact accessible status adjacent to the divider:

- pending: “Syncing read state”;
- failed: “Read state not synced” plus a localized coarse reason available to
  assistive technology;
- synced/not-requested: no extra label.

Use `role="status"`/polite announcement only on state transition, not every
snapshot. No retry button is required because retry is automatic and bounded.
The UI must not hide unread badges or claim cross-device confirmation while
pending/failed.

Carry the types through Core event generation, Tauri forwarding, TypeScript
`coreEvents`, generated examples, app harness, Browser Fake fixtures, and
Playwright. No new IPC command is needed.

## Restore and lifecycle

The durable outbox is the source for pending local boundaries across restart.
After actor startup/reconciliation:

- one restored desired ID per matching key is offered to the current actor;
- the actor adopts it as `local_viewed_event_id` only when its canonical position
  index can prove the event; otherwise status stays pending without rendering an
  incorrect divider;
- when multiple required keys restore the same room target, they collapse to one
  local boundary/status aggregate;
- a server-confirmed equal/newer target drains intent and produces `synced`;
- account switch, logout, session-generation replacement, and actor retirement
  fence old local/status completions and release dispatcher slots;
- unknown-token teardown from #608 cancels work; it never displays a later
  success from the retired session.

No local viewed boundary moves backwards during replay. If the pending target is
outside the bounded loaded window, it remains non-rendered/pending until
reconciliation supplies proof or server confirmation drains it.

## Browser Fake boundary

Browser Fake stops optimistically mutating own receipts and
`fully_read_event_id` in `sendReadReceipt`/`setFullyRead`. These methods may
record the typed command but product state changes only when a supplied
Rust-shaped snapshot/event is installed.

The #559 browser test supplies local pending, failed, recovery, and synced
navigation snapshots. Fake code does not schedule retries, classify failures,
advance local boundaries, clear unread counts, or drain an outbox.

## Verify-first RED matrix

Add behavioral tests before production wiring and preserve the commands/logs.
Type scaffolding may land first only so these tests compile.

### Pure read engine

- A then positioned B leaves exactly B and carries A/B waiters;
- repeated B is one desired target and does not reset attempts;
- older A after B cannot replace B;
- unordered later admission deterministically replaces prior desired;
- failure retains B without waiters; success/authoritative >=B drains it;
- stale completion cannot drain or alter B;
- V2 persisted C restores only C;
- rotated V1 A/B/C migrates the documented next-wake candidate, writes one V2
  ID, and removes V1 only after successful atomic save;
- malformed V1 is retained and fails closed;
- 128 keys/32 waiters/correlation references remain bounded and Debug leaks no
  IDs.

### Dispatcher and retry

- 20 failing room/thread keys create at most four concurrent network writes;
- FIFO fairness starts every key before a failed key retries;
- held retry due + 100 checkpoints/reconnects produces zero extra requests;
- exact due token enqueues once; stale/cancelled tokens enqueue zero;
- newer desired target during backoff preserves attempt/due token;
- typed rate-limit retry-after delays the exact key without blocking fair peers;
- cancellation retains its active slot until exact Cancelled completion;
- recovery drains newest targets for all keys and leaves outbox empty;
- actor/session cancellation releases every slot and fences late success.

### Local/server projection

- accepted live-edge viewport immediately moves local divider B while server
  remains A and status is pending;
- failed receipt keeps local B, server A, unread badges/counts unchanged, and
  status failed with coarse kind;
- success advances server B, drains intent, status synced, local remains B;
- newer local C followed by stale B success cannot rewind or sync C;
- non-bottom, stale actor, Focused, gap-obscured, ineligible, and non-latest
  observations do not advance local state;
- public-receipt-disabled Room mode requires only fully-read/private
  confirmation; disabled Thread mode is local/not-requested;
- initial and changed Rust settings policy reaches the current manager and stale
  session updates are rejected;
- concurrent Room and Thread boundaries remain isolated and never share an
  automatic fully-read requirement;
- correlation settlement requires exact TimelineKey + actor generation + target;
- restore of newest B retries only B and never displays older A.

### Diagnostics and UI

- timeout/transport/rate-limit/auth/server/sdk classify structurally and carry
  bounded retry-after without raw error text;
- failure records Error-level closed fields and diagnostics error count > 0;
- formatted diagnostic output contains none of seeded private IDs/bodies/errors;
- TimelineView sends no automatic receipt/fully-read commands from viewport;
- pending/failed/synced Rust snapshots render the correct divider/status;
- failed/pending never clear Rust-owned unread/badge state;
- Browser Fake command calls do not repair product state locally.

### Headless stalled/recovery scenario

Add one registered `read_state_convergence` scenario accepted on tuwunel and
synapse. Use a deterministic read-endpoint fault seam in the QA transport—not a
sleep, proxy race, or wall-clock threshold—to fail/hold receipt writes while
Sliding Sync continues committing checkpoints.

Prove:

1. healthy sync + stalled receipt endpoint;
2. local boundary advances to newest viewed event;
3. server boundary/counts stay unchanged;
4. diagnostics reports a closed failure and nonzero errors;
5. opening more rooms never exceeds four writes and persists one target/key;
6. checkpoint bursts do not bypass held backoff tokens;
7. while the fault/backoff is still held and V2 outbox is nonempty, restart and
   prove exactly one newest submission per key;
8. release the fault, drain fairly, server boundaries converge, outbox becomes
   empty, and no local boundary moves backward.

Extend the existing `QaTcpProxy` request classifier/gate to the Matrix receipt
and read-marker endpoints. The proxy holds/releases exact requests through
channels and keeps Sliding Sync traffic flowing; no wall-clock race or external
proxy is introduced.

Emit only fixed token `read_state_convergence=ok` and privacy-safe counts.
Register it in Rust/JS scenario contracts, QA token contract, and
`docs/agents/qa-lanes.md`. Run `--server=tuwunel`, `--server=synapse`, then
`--server=both`.

## Expected files and non-goals

Expected surface:

- `crates/koushi-core/src/read_state.rs`;
- `crates/koushi-core/src/store/read_state.rs` and V1→V2 migration tests;
- `crates/koushi-core/src/account/runtime_children.rs` for startup/restore policy;
- `crates/koushi-core/src/timeline/{read_state,actor,manager,navigation,diagnostics}.rs`;
- Core runtime settings-policy forwarding, timeline event/command forwarding,
  and focused runtime/headless tests;
- `apps/desktop/src/components/TimelineView.tsx` and focused tests;
- `apps/desktop/src/backend/browserFakeApi.ts` and boundary tests;
- Tauri/core-event generated contracts, TypeScript mirrors, harness/goldens;
- headless scenario registry/orchestrator/fault seam and JS runner/token tests;
- architecture/state-machine/state-ownership/QA docs and this plan/index. The
  normative state machine must replace its bounded unordered-candidate contract
  with V2 one-desired-target/correlation/dispatcher rules before implementation.

Do not change Matrix unread/count semantics, search, Activity ordering, #570
eligibility, receipt privacy setting meaning, session trust, or persistence
outside the existing read-state outbox. Do not add dependencies, frontend retry
state, unbounded queues/history, correctness sleeps, random jitter, compatibility
shims, TODOs, raw-error strings, or a second read ledger.

## Complete gates

Run focused RED/GREEN plus Core read-state/timeline/navigation/runtime/headless
suites; state and generated-event contracts; Tauri tests; frontend focused/full
Vitest, typecheck, lint, build, Playwright; Rust workspace/all-targets; wasm;
SDK submodule guard; QA binary tests; both-server scenario; rustfmt; docs/agent
docs; boundary/security/dependency/secret scans; generated-output and artifact
inspection; `git diff --check`.

Generate one exact full-diff artifact, obtain mandatory user-approved
`reviewer-flash` `Correct-to-merge`, fix and re-review every finding,
push, open a PR closing #559, wait for current-head CI 7/7, merge, verify ancestry
and issue closure, then remove only disposable artifacts and the clean worktree.

## Implementation evidence

### RED before production wiring

Added only deterministic pure-engine behavioral checks to
`crates/koushi-core/src/read_state.rs` before changing production code. The
focused command was:

```text
RTK_DISABLED=1 cargo test -p koushi-core --lib read_state::tests:: -- --nocapture
```

Actual result: exit `101`; 60 tests ran, 57 passed, and 3 failed:

- `read_state::tests::failed_latest_target_is_retained_without_replaying_an_older_target`
- `read_state::tests::persistence_snapshot_writes_only_the_newest_unordered_target`
- `read_state::tests::unordered_latest_admission_replaces_the_older_desired_target`

All three failed because the current engine retained two unordered targets
(`left: 2`, expected `right: 1`). This is the recorded RED proving the
multi-target/unordered retention defect before production wiring.

### GREEN and complete validation

The unchanged engine check is GREEN. Focused final matrices are GREEN: combined
read-state filters 73/73, timeline read-state 44/44, V1/V2 store 10/10,
frontend read-state/Fake/viewport 199/199, and QA binary 132/132. The matrices
pin one desired target, 20-key four-slot FIFO fairness, cancellation slot
retention, exact retry-after/due tokens, stale B→new C fencing, bounded actor
correlations, restored public/private pending→confirmed projection, truthful
capacity failure, and crash-safe V1→V2 migration/cleanup.

Complete final gates are GREEN: Core lib 1,074/1,074 (8 ignored), Rust workspace
2,508/2,508 (12 ignored), Tauri 174/174 (1 ignored), wasm state/search, Vitest
1,494/1,494, typecheck, lint/IME/agent docs, production build, secret scan,
Tauri/domain boundaries, SDK submodule, rustfmt, and diff checks. Browser-headless
is 262/262 with no App unhandled-error signature; its assertions require only
typed viewport observation and prove React emits no automatic receipt or
fully-read command. The event-driven `read_state_convergence` lane is GREEN on
tuwunel, synapse, and `--server=both`: a remote event advances the local
boundary, the proxy holds/fails only read endpoints while sync continues, 100
viewport checkpoints do not bypass backoff, a real runtime shutdown/restore
loads the V2 outbox, and the bounded newest retry converges before the fixed
`read_state_convergence=ok` token.

## Review record

- Advisory panel attempt: incomplete; `reviewer-flash` returned an invalid panel
  contract, leaving only one valid review and no synthesis. It cleared no gate.
- Advisory `reviewer-gpt` Round 1: not ready. It found unknowable legacy vector
  chronology, no exact per-actor correlation over the shared room-wide key, an
  insufficient failure carrier, impossible restart-after-drain QA ordering, and
  missing persistence/startup/normative files.
- This revision adds V2 one-ID persistence with explicit conservative V1
  migration, removes Thread/Room shared automatic fully-read intent, adds exact
  bounded correlations and Rust settings-policy forwarding, carries typed
  retry-after/current-target failures, restarts before drainage through the
  extended QA proxy, and expands scope/canon/tests.
- Advisory `reviewer-gpt` Round 2: `Correct-to-seek-mandatory-review`; every
  Round 1 blocker was verified fixed. This is non-authorizing.
- Mandatory user-approved substitute `reviewer-flash`: `Correct-to-merge` at
  `2035054de0355c59ff3080bd0b0557937656ca26`; after #570 merged, the exact
  rebased design at `3f33b6b145c1f39a219c05d3609caf13c458d15c` was revalidated
  `Correct-to-merge` before implementation. The review confirmed the V2
  migration order, exact correlations, four-slot fairness, typed failures,
  restart-before-drain QA, and complete ownership/gate scope. Its two
  non-blocking observations are pinned by existing requirements: the compatibility
  read-marker field must equal server-confirmed state, and correlations are
  capped by 128 active read keys and removed with actor/session retirement.
- Implementation advisory Round 1 found missing dispatcher/correlation and V1
  migration matrices, restored-outbox projection, obsolete normative docs, and
  six boundedness/diagnostic/QA gaps. All were fixed. Round 2 verified those
  fixes and found one stale fully-read B completion could still reach the actor
  after desired C replaced it; actor apply and server confirmation now require
  the still-current desired target, with a deterministic regression test.
- Mandatory exact Round 1 reviewed the full 7,686-line artifact and found two
  Important and three Minor gaps. Persisted receipt privacy now seeds every
  AccountActor before session/manager spawn; Room and Thread viewport IPC carry
  an optional typed thread root; stale completion immediately refills its FIFO
  slot; unprovable restored state projects pending without a divider; and actor
  retirement now retires/cancels its keys and persistence. Focused and complete
  matrices above are GREEN after all five fixes.
- Mandatory exact Round 2 verified those five fixes and found one further
  Important privacy gap: disabling public/thread receipts removed memory state
  but did not persist the reduced outbox. Policy changes now publish immediately,
  and the policy test proves disable → persisted empty snapshot → synthetic
  restart emits zero public/thread writes.
- Mandatory exact Round 3 found the crash-window/V1 variant of the same privacy
  issue: restore scheduled legacy public/thread keys before policy reconciliation.
  Restore now filters those keys before building reconciliation state, persists
  the filtered snapshot, and the sole enqueue path also refuses them while
  privacy is off; fully-read/private Room work remains. Pure snapshot and stale
  nonempty-restart tests pin both layers.
- Mandatory exact Round 4 verified the restore filter and found one Minor: an
  explicit receipt command could enter the engine while privacy was off but the
  enqueue defense would leave its waiter pending forever. Admission now returns
  a typed `Forbidden` terminal before waiter allocation; focused and Core-full
  tests are GREEN.
- Mandatory exact Round 5 found the flip-off counterpart: already-admitted
  explicit waiters could be orphaned when the background key was removed.
  Privacy disable now retires every public/thread desired key independently of
  local correlations, cancels its active operation, removes queue/retry state,
  emits one `Forbidden` terminal per waiter, and persists the result. The
  mid-flight toggle test proves cancellation, one terminal, empty waiter/outbox,
  and no network request. Final exact re-review remains pending.

## Acceptance mapping

| #559 requirement | Evidence |
| --- | --- |
| local boundary advances without ACK | actor viewport + pending projection tests |
| server confirmation stays separate | navigation DTO/count tests |
| visible pending/failure | Rust sync enum + accessible UI tests |
| newest-only coalescing | pure engine + persistence restore tests |
| bounded multi-room writes | four-slot dispatcher/fairness tests |
| checkpoint/reconnect respects backoff | held due-token matrix |
| healthy sync + stalled receipts | deterministic both-server QA |
| failure→recovery/no backward move | projection/dispatcher/QA matrix |
| newest-only restart retry | V2 normalized outbox restore; explicit conservative V1 migration |
| privacy-safe diagnostics | Error-level closed classifier + secret scans |

Implementation and complete local validation are finished; merge remains
blocked only on the mandatory exact full-diff verdict and current-head CI.
