# Issue #285 Display Diff Projection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure every SDK timeline batch emitted to the desktop is expressed in the bounded display index domain, so a confirmed text or media send cannot leave a stale `Sending` transaction row.

**Architecture:** Keep `navigation_items` as Core's full canonical sequence and keep the desktop TimelineStore bounded. Replace the current reuse of canonical indices with one Core-owned projection transaction that applies canonical diffs once, advances explicit bounded-display membership/mirror state, and emits display-space diffs whose application exactly equals the authoritative display result. Runtime validation falls back to a display `Reset` and records a private-data-free diagnostic when incremental translation is ambiguous.

**Tech Stack:** Rust, Matrix SDK `VectorDiff`, Koushi Core timeline actor, TypeScript TimelineStore contract, Rust unit tests, Vitest.

## 2026-07-20 Implementation Discovery Addendum

The original prohibition on Matrix SDK semantic changes remains in force for
timeline projection and send-queue behavior. Live E2EE gating exposed one
separate crypto defect: replaying the same remote SAS start could replace an
already-adopted responder continuation and produce a commitment/key mismatch.
The approved narrow exception is the vendored SDK's same-peer/device/flow
remote-start idempotency patch, with simultaneous-start and replay regressions,
the parent gitlink update, and an upstream-feedback ledger entry. Koushi also
enforces one adopted SAS continuation per flow and prevents its manual one-shot
sync path from overlapping an actor-owned restricted or continuous sync lane.
These hardening changes do not alter SDK timeline ordering or diff semantics.
The same live gate also exposed an existing replay-correlation gap: an
idempotent active-timeline Subscribe retained the original projection ACK ID
but discarded the new command cause. The approved root correction carries both
identities on `InitialItems`; projection acknowledgement remains bound to the
original actor/generation, while Subscribe success requires the new causal
request ID. Same-key matching alone is not acceptance evidence.
The post-correction live gate then exposed a QA timeout-liveness defect:
event-wait loops recreated a relative timeout after every unrelated event, so
continuous sync traffic could postpone a nominal 90-second failure forever.
The approved correction gives each logical waiter one monotonic absolute
deadline, exercises deadline starvation with a continuously ready unrelated
event stream, and audits the headless QA wait boundary for the same pattern.
The next full gate exposed a separate room-observer liveness gap: invite state
could commit outside the bounded `RoomListService` entries head without an
entries diff, leaving `AppState.invites` stale. The approved correction keeps
the existing single SyncService/RoomListService owner and adds the base client's
post-commit room-update broadcast only as an auxiliary wake. It filters and
coalesces that signal, reprojects invite payload/membership changes, performs a
single lag reconciliation, and proves that ordinary joined-room updates do not
trigger full room-list normalization.
The local Conduit gate then proved that advertising MSC4186 does not establish
the invited-room list behavior required by the product: Conduit's simplified
sliding-sync path omitted the requested invite-filtered list. Backend selection
therefore performs one authenticated, cursorless, zero-timeline invite-list
contract preflight before either continuous owner starts. Presence of the exact
requested list selects SyncService; omission, typed/malformed error, or the
single end-to-end two-second deadline selects LegacySync. The deadline encloses
automatic access-token refresh/retry, and the probe discards cursor/room data so
it cannot become a second owner. Family/version fingerprinting is forbidden.
The same gate finally exposed a cross-lane logout barrier: the `LoggedOut`
operation event could arrive before the reducer snapshot showed `SignedOut`, so
an immediate RestoreSession was projection-rejected while `LoggingOut` and then
silently discarded by the runtime. The approved correction makes projection
rejection an exactly-once correlated `OperationFailed`, requires headless QA to
observe both `LoggedOut` and authoritative `SignedOut` in either order, and
uses one absolute deadline for that waiter and its expected-failure follow-up.
If the post-logout restore is admitted and then returns `SessionNotFound`, its
failure terminal and the reducer's resulting `SignedOut` projection are the
same two-signal barrier; observing the failure alone is not permission to read
the dependent state.
The focused E2EE rerun then exposed the inverse observation race: the Ready
snapshot could commit immediately after the final broadcast and before the
timeout branch returned. Snapshot-plus-event waiters must therefore use events
only as wakes and perform one final authoritative snapshot read on timeout,
closure, or lag, without extending the original absolute deadline. The focused
waiter test holds exactly that Ready-without-a-following-broadcast ordering.
The final `all` lane exposed one further E2EE prerequisite race. A newly logged
in second device could send an own-user verification request before the
receiving device had learned that device's keys. `matrix-sdk-crypto` deliberately
discarded the otherwise valid request when `DeviceData` was missing, and the
event could not be reconstructed above the SDK. The approved root correction is
therefore a second narrow crypto exception: retain timestamp-valid unknown-device
to-device verification requests in a bounded, flow-deduplicated pending set;
mark the sender for the SDK's existing coalesced key-query lane; and, immediately
after a matching key-query response commits device changes, re-run the normal
timestamp, self-device, and device-data validation before each retry. Pending requests
expire under the existing verification-request age bound, duplicates do not
extend their lifetime, and the collection has a fixed capacity with strict FIFO
preservation: it rejects the newest overflow instead of evicting an existing
obligation. It must not start another sync owner, add a manual `SyncOnce`, blindly
resend competing flows, or retain still-invalid requests indefinitely. Headless
QA also establishes an exact receiver-device-known acknowledgement before issuing
`StartOwnUserSas`; this is a causal setup barrier, not a substitute for the SDK
no-loss contract. The vendored change requires focused unknown-device recovery,
deduplication, expiry/capacity, and still-missing-device tests plus an upstream
feedback ledger entry.
The next composed `all` runs exposed scenario ownership defects rather than
product transition failures. Their normal B participant was already
bootstrapped, logged in, and syncing, but the later E2EE stage opened another B
device while first hard-coding new-identity bootstrap and then recovering that
duplicate device. The first form waited for the wrong gate; the second caused a
fresh B3 own-user SAS request to reach two eligible base devices and legitimately
cancel one as accepted elsewhere. The approved root correction makes recipient
ownership explicit: composed `all` borrows its existing normal B connection and
account key without creating or cleaning it up, while focused E2EE, which has no
prior B participant, creates, bootstraps, owns, and cleans up exactly one B.
Scenario helpers must not duplicate an already-live role, infer a gate from
timing, stop a competing owner as a workaround, retry, sleep, or lengthen a
timeout.
The final quality review found two further ownership holes in the first crypto
recovery draft. It removed pending entries before fallible replay and swallowed
an initial key-query scheduling failure, while the Koushi adapter compensated
with a direct out-of-band key refresh. The approved correction keeps pending
FIFO slots until terminal validation or successful materialization, records
whether the coalesced query owner was scheduled so only a failed schedule is
retryable. Normal and recovered request materialization publish the same stable
handle through a typed incoming-request SDK lease stream. Koushi consumes that
stream without issuing a query or reconstructing identifiers. Tests must cover
schedule and store failures, multi-entry preservation, lease/capacity behavior,
and observer shutdown before live QA is rerun.
The same final review exposed cleanup ownership beginning too late: focused
E2EE participants were registered only after successful login, so a failure
between login submission and the next checkpoint could leave a runtime or
provisional server session behind and poison the following 300-second run. The
approved correction introduces a typed participant owner before any fallible
login step. It tracks runtime-only, login-submitted, and keyed-logged-in phases,
then attempts keyless or keyed logout confirmation as appropriate before
dropping the connection and shutting down the runtime. All owned participants
are cleaned even if an earlier cleanup reports an error; borrowed participants
remain the outer scenario's responsibility. The logout waiter uses events only
as wake signals and performs a final `SignedOut` snapshot observation after
timeout, lag, or closure without resetting its absolute deadline. Focused tests
must inject failures at each ownership phase and reproduce a final snapshot
commit without a following broadcast before live server evidence is accepted.
The concurrency review then found that sequential replay tests were not enough
to prove linearized bounded ownership. Two key responses could clone the
same pending entry, and raw redelivery could race a deferred replay after its
snapshot. Cache insertion also checked and inserted under separate locks, while
the then-proposed recovery-only stream could replace its subscriber after a
publisher cloned the old sender. That recovery-only/passive design is
superseded by the final unified typed stream below. The retained correction
returns existing-versus-inserted from one cache write transaction and
linearizes subscriber generation with the typed head claim. Deterministic
rendezvous tests must prove
replay-versus-replay, replay-versus-raw, same-flow concurrent insertion, and
replacement-versus-claim. A fallible pre-commit replay releases its claim
without changing FIFO order. A winning committed deferred insertion records a
pending publication obligation in the unified bounded owner; an unrelated
pre-existing cache entry carries no incoming provenance and is not published.
Soft-logout reauth must
also stop and join the old incoming observer before the replacement session
installs handlers, with a lifecycle-order test.
The final end-to-end review rejected coupling generic raw handler completion to
product delivery. Normal and recovered materialization now feed one typed
incoming-request lease stream. Pending entries, publications, subscriber
generation, and active head claim share one owner lock and one total bound of
32. Replay changes its existing slot into a publication under that lock; active
leases retain the head slot, commit pops it, and drop releases in place.
Generation check and claim are one linearization point. Capacity never evicts
an existing pending entry, publication, or active lease. At capacity a new
materialized request is explicitly cancelled with an outgoing protocol cancel,
because cursor advancement makes silent shedding unrecoverable; a newest unknown-
device request is not retained and does not schedule a query. A same-flow
collision never upgrades unrelated cached provenance. Applied key-query changes
are returned even if later replay/cache/reschedule work fails, and retry state
remains schedulable. Pending work uses explicit `NeedsQuery`, `QueryInFlight`,
`WaitingForExternalUpdate`, `ResponseClaimed`, and `ReplayClaimed` states. A
response-scoped RAII token claims entries before key-response processing and
returns only its own claimed entries to `NeedsQuery` on cancellation or error,
including after the durable commit. Normal still-missing completion enters
`WaitingForExternalUpdate`. A sender-wide retry reset is forbidden because it
can steal a newer concurrent response's claim for the same sender. Delivery `Debug`
implementations at the crypto and client wrapper layers are constant/redacted.
Every generated key query retains stable per-request metadata containing its
exact `request_id -> covered users` mapping, dirty-state sequence, and request.
Repeated, concurrent, or cancelled collection reuses existing requests and
creates only uncovered chunks; it never clears another live mapping. Final
dirty-snapshot revalidation and registry insertion share the dirty-query owner,
preventing a paused collector from registering stale metadata after cleanup.
Complete response-associated processing for the same request ID is serialized
by a stable per-entry async gate. Gate acquisition is awaited without a
registry or store guard, and no such guard crosses async identity processing or
verification recovery. Brief registry guards taken while the gate is held are
limited to pointer revalidation and successful consumption. A handler
revalidates the pointer-matched entry only after acquiring the gate.
Success consumes it; failure or cancellation preserves it for the next waiter,
while a waiter after successful consumption carries no metadata obligation.
Different request IDs remain concurrent. Recovery scopes the
response to exactly the union of returned `device_keys` users and that request's
covered users, so failure-only responses with empty returned keys still claim
the intended pending sender. Overlapping responses use a per-entry
committed-update generation: if one response commits while another owns replay,
that owner consumes the deferred generation before entering
`WaitingForExternalUpdate`. Request/user/sender/device/flow identifiers and raw
errors remain absent from creation, response, receive-span, and recovery
diagnostics.
Koushi removes its raw to-device handler and consumes only this typed stream.
It commits actionable leases after product-channel send and commits terminal
heads immediately. Generic SDK raw handlers remain independent compatibility
fanout and may repeat after partial cancellation, so transport is at-least-once
with stable sender/flow identity. `AccountActor` owns product idempotence using
the full peer/device `VerificationTarget` plus flow id: only an exact replay is
a no-op, while the same flow id from another peer/device or any other distinct
conflict is explicitly cancelled without raw error disclosure. Replayed SAS
continuations remain no-ops under their existing adoption rule. A pre-SAS
own-user verification owns the shared continuation and request-observer slots
without a typed incoming replay identity, so every incoming request is
cancelled as a conflict before either slot can be replaced. Do not extend
`ProcessedToDeviceEvent`, add a Koushi seen set, or rely on redelivery after an
overflow. Observer-to-actor delivery carries a dedicated authenticated-session
generation and rejects stale/sessionless work before adoption. Its actor-mailbox
send is stop-aware even when full, and stop uses bounded join followed by abort
and owned settlement.
The same review bounded timeline graceful shutdown: the complete manager-owned
enqueue worker set and its global terminal observer are boxed futures directly
polled under one absolute five-second deadline while terminal ingress stays
live. The observer remains polled while workers settle, and receives one final
non-blocking poll after worker quiescence or deadline cancellation so an already
queued exact terminal can be admitted. Remaining cooperative worker futures are
then synchronously dropped before observer stop, observation-loss handoff,
ingress drain, and acknowledgement. A per-worker deadline, detached task, or
`JoinHandle` abort/await lifecycle is not accepted.
Because these enqueue futures have no independent scheduler, an accepted route
also drives its specific worker through the reducer permit to a one-shot signal
at the start of payload-specific preflight. Draining one unrelated ready
completion does not satisfy that causal-start contract. Reply preflight may
suspend before the eventual SDK queue call, so the signal does not serialize
SDK enqueue order across workers; SDK transport owns subsequent FIFO retry
policy and timing.
The final branch review also found a send-lifecycle ordering race outside the
projection itself. A media enqueue worker bound its SDK transaction before the
manager loop published `MediaSendQueued`; an already-retained terminal could
therefore win the biased terminal branch and publish completion first. The
corrected manager-owned worker publishes the queue acknowledgement synchronously
before binding can admit any terminal. A deterministic pre-bind-terminal RED
must prove queued-before-terminal ordering; changing select priority is not an
accepted substitute.
The first behavior-probed Tuwunel Core run exposed cursor provenance.
A fresh device's verification-only filtered `/sync` persisted its `next_batch`
in the SDK store. Normal LegacySync then reused that restricted cursor and
permanently skipped existing room state excluded by the filter, leaving A2 with
zero rooms after SAS succeeded. SDK inspection then showed that `next_batch` is
persisted before fallible event handlers, so even an error return can leave the
shared cursor restricted. An initial memory-taint correction survived task
restart but failed the process/account lifetime boundary because the SDK token
was durable while the taint was not. The final design removes the compensation:
`SyncSettings::save_sync_token(false)` preserves all other sync processing while
preventing a verification-only response from writing the global cursor. The
restricted request also uses `NoToken`. SDK REDs must prove canonical-token
preservation across SQLite reopen, a fresh store remaining tokenless, and
to-device handler delivery. A Koushi mock-server RED must run the restricted
helper, destroy and reopen the session/store, then prove the next normal request
uses the original canonical `since`, never the restricted token, before
Tuwunel is rerun.

## Global Constraints

- GitHub issue [#285](https://github.com/shinaoka/koushi-matrix/issues/285), including its current body and review comment, is the approved design specification.
- Every `ItemsUpdated` batch is exclusively in the display index domain and transforms the previously emitted display sequence into Core's new authoritative display sequence.
- Preserve enough pre-normalization membership state for canonical/display offsets, duplicate render identities, `Transaction -> Event` identity changes, and bounded-window entry/exit.
- Apply SDK canonical diffs exactly once.
- Validate numeric display indices in release builds, not only debug assertions.
- If incremental projection is ambiguous or invalid, emit display `Reset { items: display_after }` or use the existing resync path; never silently drop the transition.
- Record a private-data-free `display_projection_reset_fallbacks` diagnostic
  and assert its counter delta is zero in the normal focused/headless path;
  absence from logs is not acceptance evidence.
- Projection work is independent of the full canonical length per diff. The
  implemented sparse membership rope has expected
  `O(W + B log(W + B) + D)` projection time and `O(W + B + D)` temporary
  space for represented membership `W`, SDK batch `B`, and emitted display
  diff `D`; it never scans the full canonical sequence per diff. Live-edge
  `W` is capped at 120. Historical/restore paths may have larger `W`, so a
  deterministic structural-work gate covers that uncapped case rather than
  inferring complexity from payload visits alone.
- Anchor-restore buffering stores projected display diffs, not raw canonical-index diffs.
- Preserve the actor-generation lease across display-mirror commit, `ItemsUpdated`, and replay-known reconciliation.
- Do not match body, timestamp, sender, formatted content, or media metadata to reconcile identity.
- Do not move the full canonical timeline into the desktop store, remove bounded replay, change Matrix SDK semantics, or redesign send-queue UX.
- Follow strict RED -> GREEN -> refactor order. Run long-duration QA only once after the coherent implementation and review are complete.
- Diagnostics, fixtures, and reports must contain synthetic/private-data-free values only.

---

### Task 1: Project canonical SDK batches into one validated bounded-display transaction

**Files:**
- Modify: `crates/koushi-core/src/timeline.rs`
- Modify: `crates/koushi-core/src/event.rs`
- Modify: `apps/desktop/src/domain/coreEvents.ts`
- Test: `crates/koushi-core/src/timeline.rs`
- Test: `apps/desktop/src/domain/timelineStore.test.ts`

**Interfaces:**
- Consumes: SDK-derived `Vec<TimelineDiff>`, pre-batch `navigation_items`, current bounded display membership/mirror, viewport/replay window policy, actor generation lease.
- Produces: updated canonical state, updated bounded display membership/mirror, and validated display-space `Vec<TimelineDiff>` for `TimelineEvent::ItemsUpdated`.
- Invariant: applying the emitted display diffs to `display_before` yields exactly `display_after` after every batch.

- [ ] **Step 1: Add the production-shaped failing Core regression**

Add a unit test named `sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo` beside the existing timeline display-mirror tests. Construct at least 9,040 synthetic canonical slots, select a roughly 120-row live-edge display, append a transaction, then apply canonical-index `Set`, followed by canonical-index `Remove` plus `PushBack Event`. Apply every emitted display batch to a separate desktop-model vector and assert after each batch:

```rust
assert_eq!(desktop_model, projection.display_items());
assert!(projection
    .display_items()
    .iter()
    .all(|item| !matches!(item.id, TimelineItemId::Transaction { .. })));
assert_eq!(
    projection
        .display_items()
        .iter()
        .filter(|item| timeline_item_event_id(item) == Some("$confirmed:test"))
        .count(),
    1
);
```

Use only synthetic IDs and bodies. The test must exercise indices around 9,039 while the display contains about 120 rows.

- [ ] **Step 2: Run the new test and capture the expected RED**

Run:

```bash
cargo test -p koushi-core --lib sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo -- --nocapture
```

Expected: FAIL because a canonical `Set`/`Remove` index cannot update the bounded display and the transaction survives beside the confirmed event. Record the command and failure reason in the task report before editing production code.

- [ ] **Step 3: Introduce one projection transaction and make the production-shaped test GREEN**

Replace the raw `apply_timeline_diffs_to_display_items` SDK path with a stateful projection boundary. The implementation may choose internal names, but it must expose semantics equivalent to:

```rust
struct DisplayProjectionBatch {
    display_after: Vec<TimelineItem>,
    display_diffs: Vec<TimelineDiff>,
    used_reset_fallback: bool,
}

fn project_sdk_batch(
    canonical_items: &mut Vec<TimelineItem>,
    display_state: &mut DisplayProjectionState,
    canonical_diffs: &[TimelineDiff],
    context: &DisplayProjectionContext,
) -> DisplayProjectionBatch;
```

The transaction must retain explicit canonical-slot/window membership information rather than attempting to reconstruct discarded duplicates from the normalized display vector. Read old canonical identities before `Set` and `Remove`. Advance canonical state exactly once. Emit only indices valid against the evolving display sequence.

Run the focused test again. Expected: PASS with one confirmed event and no transaction.

- [ ] **Step 4: Add RED coverage for every diff variant and exceptional fallback**

Add focused tests covering:

```text
duplicate canonical identity removal with another owner retained
out-of-window Set / Remove / Insert / Truncate
boundary-adjacent Insert
live-edge PushBack
backward-pagination PushFront
Clear and Reset
Transaction -> Event identity-changing Set
anchor-restore buffering and flush
stale actor-generation rejection
invalid or ambiguous incremental translation -> Reset fallback
```

For each test, apply emitted display diffs to a separate model and assert:

```rust
assert_eq!(apply_display_diffs(display_before, &display_diffs), display_after);
```

Run the new focused test filter before adding the remaining production logic. Expected: at least one new test FAILS for missing behavior rather than a compile/setup error.

- [ ] **Step 5: Complete all projection semantics, runtime validation, restore integration, and diagnostics**

Implement all variants under the single projection boundary. Runtime-validate each numeric index against the display state immediately preceding that operation. On ambiguity or invalid output, replace the incremental batch with:

```rust
vec![TimelineDiff::Reset {
    items: display_after.clone(),
}]
```

Record one private-data-free diagnostic for fallback use with a stable
token/counter named `display_projection_reset_fallbacks`; do not record IDs,
bodies, room identifiers, or raw errors. Ensure ordinary production-shaped
and focused headless flows assert the counter delta is zero and emit only the
safe `display_projection_reset_fallbacks=0` success token.

Change `restore_emit_buffer` to accumulate projected display diffs while canonical state advances immediately. The final flush must transform the desktop's pre-restore display into the authoritative post-restore display. Keep the existing actor-generation lease as the atomic publication/replay-known boundary.

The hot path may scan the represented display membership and current batch,
but must not scan all canonical items for every diff. Maintain sparse
slot/window membership bookkeeping with the expected
`O(W + B log(W + B) + D)` bound above. The logarithmic structural term is an
explicit implementation amendment to the earlier additive target: it avoids
`O(W * B)` indexed mutation work, remains independent of the canonical history
length, and must be guarded for uncapped historical/restore membership as well
as the 120-row live edge.

- [ ] **Step 6: Document the wire contract and keep frontend defenses non-authoritative**

Update the Rust `TimelineEvent::ItemsUpdated` and TypeScript `ItemsUpdated` comments to state:

```text
All numeric TimelineDiff indices are relative to the desktop display sequence
immediately before that operation, never to Core's full navigation sequence.
```

Retain frontend bounds checks as defenses. Do not add body/timestamp matching or a frontend-only stale-transaction cleanup. Add a TypeScript regression combining an earlier collapsed duplicate with a later transaction/event transition so the reducer contract remains pinned.

- [ ] **Step 7: Run focused GREEN gates and review the finished diff**

Run:

```bash
cargo test -p koushi-core --lib sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo
cargo test -p koushi-core --lib display_projection
npm --prefix apps/desktop test -- src/domain/timelineStore.test.ts --reporter=dot
cargo check -p koushi-core
npm --prefix apps/desktop run typecheck
cargo fmt --all -- --check
git diff --check
```

Expected: all commands exit 0; no warnings/errors introduced; fallback diagnostics remain zero on ordinary paths. Do not run a long homeserver lane yet.

- [ ] **Step 8: Self-review and commit**

Review the full diff against `REPOSITORY_RULES.md`, `docs/architecture/overview.md`, `docs/architecture/state-machine.md`, `docs/policies/engineering-rules.md`, `AGENTS.md`, issue #285, and this plan. Confirm no raw canonical index reaches `ItemsUpdated`, no duplicate projection implementation was added, and no private data enters diagnostics.

Commit:

```bash
git add crates/koushi-core/src/timeline.rs crates/koushi-core/src/event.rs \
  apps/desktop/src/domain/coreEvents.ts apps/desktop/src/domain/timelineStore.test.ts
git commit -m "fix(timeline): project SDK diffs into display index space"
```

Write the implementation report with RED evidence, GREEN commands/results, commit SHA, self-review findings, remaining concerns, and whether any fallback occurred.
