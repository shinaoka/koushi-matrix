# Issue #725 Rust-owned pending-send projection

## Goal

Every accepted text/reply submission has one Rust-owned visible row from composer acceptance until canonical SDK convergence, even when `RoomSendQueueUpdate::NewLocalEvent` and the corresponding SDK timeline diff are absent. React remains an indexed projection cache only.

## Existing boundary

`TimelineManagerActor::route_submission_to_worker` already owns admission, the client transaction ID, accepted body, exact `TimelineKey`, manager-resident enqueue worker, and `SendCompletionCoordinator`. `TimelineActor` owns canonical SDK items and `DisplayProjectionState`; the TypeScript timeline store only applies Rust-authored diffs and already indexes exact render IDs. The manager and coordinator survive actor unsubscribe/replacement.

The fix extends these owners rather than adding a second optimistic-send service or SDK patch.

## Design

### 1. One bounded manager-owned record

Extend `CoordinatedPendingSend` with an optional `PendingSendProjection` for text/reply sends:

- exact `TimelineKey` and client transaction ID;
- existing `TimelineItem` payload with `TimelineItemId::Transaction(client_txn_id)`, accepted body, current Rust session user, Rust timestamp, reply/thread relation where available, and `send_state=Sending`;
- optional SDK transaction ID and `SendHandle` after enqueue binding;
- optional terminal event ID;
- closed phase: `Pending`, `FailedRecoverable`, `SentAwaitingRemote`, or `HydratedSent`;
- all known client/SDK/event render identities for exact correlation.

Use a hard `MAX_PENDING_SEND_PROJECTIONS=128`. If the cap is full, reject the new submission before acceptance with the existing queue-overflow path. Media-only registrations do not consume this display cap. Cancellation, unrecoverable failure, canonical convergence, and manager/session teardown remove records. Recoverable failure and successful sends awaiting a remote event remain bounded records; do not timeout-delete them. To prevent the cap from becoming a permanent dead end when a timeline diff is missing, every `SentAwaitingRemote` transition also starts an existing-SDK `load_or_fetch_event(event_id)` hydration in a manager-owned `FuturesUnordered` worker (never awaited inline by the actor). Success replaces the fallback with the authoritative fetched event, moves it out of the active cap into a separate 128-entry `HydratedSent` cache aligned with the 120-item live display window, and frees the active slot. A network/missing/decryption failure retains the visible fallback and retries only on an existing causal wake (actor subscribe/replacement, sync reconnect/live-tail refresh), never a timer. Hydrated entries leave on canonical SDK convergence/teardown; oldest-first cache eviction does not issue a remove to a live actor, and a replacement rehydrates retained event IDs. #725's reported remote echo normally converges before this ceiling; if sustained missing canonical diffs exceed the bounded cache, promote it to durable event fallback storage rather than increasing the cap. Reaching the active cap therefore applies backpressure rather than silently evicting an unconfirmed visible send, while authoritative hydration self-heals it.

### 2. Actor-owned combined display projection

Add pending items and a set of suppressed late transaction identities to `DisplayProjectionState`, separate from canonical SDK slots. Every display materialization:

1. projects canonical SDK slots;
2. filters only transaction identities explicitly suppressed by a correlated `SentAwaitingRemote` record;
3. appends manager-owned pending items;
4. normalizes exact render IDs with canonical items winning;
5. uses the existing `finalize_display_projection_diffs` to emit ordinary `TimelineDiff`s.

No new frontend event/diff type is needed. SDK diff indexes continue to apply only to canonical state. While pending records exist, combined output is still generated in Rust; TypeScript receives ordinary exact diffs.

`TimelineActorMessage::RefreshPendingSendProjection` carries the current bounded snapshot and an optional acknowledgement. Actor spawn/replacement receives the same snapshot before `InitialItems`, so unsubscribe/room switching cannot lose accepted rows.

### 3. Acceptance ordering

For a new submission:

1. begin and activate the existing completion registration with its pending item;
2. route `RefreshPendingSendProjection` to the current exact actor and await acknowledgement that its generation-fenced `ItemsUpdated` publication was admitted;
3. only then enqueue `ComposerSubmissionAcceptedAtRevision` / `ThreadSubmissionAcceptedAtRevision` and emit `SubmissionAccepted`;
4. release the existing enqueue permit.

If the actor update/ack fails, cancel registration, reject the submission, and do not clear the draft. Thus pending visibility causally precedes accepted composer clearing without delaying on HTTP/SDK send completion.

### 4. SDK binding and local-echo merge

Production enqueue success retains the returned SDK `SendHandle` and SDK transaction ID in the same coordinated record. These are new mechanics, not pre-existing helpers: change `SendEnqueueSuccess` to carry `Option<SendHandle>`; change `SendCompletionRegistration::bind`/the coordinator binding path to store it and return the changed `TimelineKey`; change unit `SendEnqueueWorkerCompletion` into `{ changed_key: Option<TimelineKey> }`; and make the manager's currently no-op worker-completion handler async so it refreshes that exact current actor. The actor atomically replaces the client-ID row with the SDK-transaction-ID row through existing display diff finalization.

Before each canonical SDK batch is materialized, the actor asks the shared coordinator to reconcile exact incoming render identities:

- a matching SDK transaction while `Pending`/`FailedRecoverable` removes the synthetic overlay; canonical SDK content wins in the same emitted batch (`sdk_local_echo_merged`);
- a matching SDK transaction after `SentAwaitingRemote` is suppressed as a delayed stale local echo and cannot resurrect/duplicate the row;
- a matching terminal event ID removes `SentAwaitingRemote`; canonical remote content wins in the same batch (`remote_echo_converged`).

No body/timestamp/sender comparison is permitted.

### 5. Terminal, retry, and cancellation

- `Sent`: if no canonical event is present, change the retained row atomically to `TimelineItemId::Event(event_id)` and `send_state=Sent`; record `sdk_local_echo_missing` and start the manager-owned hydration worker. If the canonical event already exists, the actor refresh immediately converges it.
- Recoverable failure: retain one SDK-transaction row with `NotSent(Recoverable)`.
- Unrecoverable failure/cancellation: apply existing terminal policy and remove the pending display record atomically.
- Retry/cancel first use actor-local SDK handle state; when a local echo was omitted, they fall back to the exact manager-retained `SendHandle`. Add explicit coordinator methods `retry_handle_and_mark_sending(render_id)` and `cancel_handle_and_remove(render_id)`; the actor invokes them synchronously by exact identity, performs `unwedge`/`abort`, and refreshes its combined projection in the same command path. Retry resets phase to `Pending` and clears `failure_reported` before a later success; cancel is admitted only for `Pending`/`FailedRecoverable`, never `SentAwaitingRemote`/`HydratedSent` (server-delivered messages require redaction). Thus retry has a concrete reverse route to the manager-owned phase rather than an assumed notification. Late terminals remain protected by existing tombstones.

Terminal handoff includes only the changed `TimelineKey` needed for the manager to refresh the current actor after existing reliable reducer/completion delivery. No identifier enters diagnostics.

## Verify-first matrix

Add RED tests before production changes:

1. Synthetic enqueue accepted with no `NewLocalEvent`: actor publishes `Sending` before accepted composer settlement.
2. Normal SDK local echo: client pending becomes one SDK transaction row, never two.
3. `SentEvent` before local echo: one event-ID `Sent` row remains until remote echo.
4. Remote canonical event after terminal: atomically becomes one canonical event row.
5. Remote/event and delayed local-echo permutations remain one row and never resurrect the transaction.
6. Recoverable failure → retry and cancel use retained handle when actor never observed `NewLocalEvent`.
7. Unsubscribe/replacement replays the bounded pending projection; terminal settlement still correlates.
8. Cap 128 rejects the 129th display projection without clearing its draft; teardown clears all.
9. Diagnostics contain only `pending_projection_inserted`, `sdk_local_echo_merged`, `sdk_local_echo_missing`, and `remote_echo_converged` plus counts/correlation numbers.
10. Browser-headless scenario observes continuous exact-ID visibility from accepted draft clearing through event-ID convergence, with the local-echo event deliberately omitted.

Reuse existing `SyntheticSendEnqueueRequest`, completion-coordinator permutations, actor generation gates, `DisplayProjectionState`, and timeline-store exact-ID assertions. Do not add sleeps, React optimistic state, SDK extensions, body dedupe, or timeout cleanup.

## Canon and gates

Update `docs/architecture/state-machine.md`, `docs/agents/state-ownership.md`, protocol/TS mirrors only if their wire shape changes, and the existing send-queue canon. Run focused outbound-send/display tests first, then full Core/testkit/state/protocol/desktop/Vitest/Playwright, format, secret, boundary and submodule gates. Obtain independent implementation review, required PR CI, merge, Issue closure, post-merge main CI, then remove generated artifacts.

## Review record

- Design review Round 1: architecture accepted, with two Important clarifications. The plan now enumerates the new worker-completion/handle/retry mechanics explicitly and adds causal exact-event hydration so a missing remote diff cannot permanently exhaust the bounded active projection cap.
- Final design re-review: `reviewer-flash` **Correct-to-merge**. Its final clarity/minor notes were incorporated: hydration moves authoritative results to a separately bounded live-window cache, uses manager-owned futures, retry resets `failure_reported`, and cancel cannot hide server-sent rows.
- Implementation review: pending.
