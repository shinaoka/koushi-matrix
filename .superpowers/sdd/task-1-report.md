# Task 1 report: SDK diffs projected into display index space

## Result

Implementation commits:

- `0598615` — `fix(timeline): project SDK diffs into display index space`
- `af7bb42` — `fix(timeline): fence projection recovery transactions`
- `c92b82d` — `test(timeline): cover media projection and linearize membership build`

Core now owns one stateful SDK-to-display projection transaction. It advances
the canonical timeline once, retains the exact pre-normalization canonical
slots represented by the bounded display, derives one authoritative normalized
display, runtime-validates the emitted display-space diffs, and publishes only
those diffs to `TimelineEvent::ItemsUpdated`.

The same state is used for bounded replay ownership, non-SDK item revisions,
relay recovery, and send-queue resync. Restore buffering stores projected
display diffs while canonical state advances immediately. The actor-generation
lease fences projection commit, `ItemsUpdated`, and replay-known reconciliation;
it is released before unrelated thread-attention and live-edge calculations.

## RED evidence

The production-shaped regression was added first and run with:

```text
cargo test -p koushi-core --lib sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo -- --nocapture
```

It failed before the production change because the canonical `Set` at index
9,039 was applied to a roughly 120-row display sequence and was ignored. The
transaction local echo therefore survived beside the confirmed event after the
following canonical `Remove(9039)` and `PushBack Event` batch.

Additional RED coverage was then added for duplicate ownership, out-of-window
operations, boundary insertion, live-edge and historical-edge growth,
clear/reset, invalid translation, restore buffering, and stale generations.
Before the remaining production integration, seven projection tests passed and
the restore/generation cases failed for the missing behavior.

## GREEN evidence

Fresh focused gates after the final self-review change:

- Production-shaped Core regression: 1 passed, 0 failed (16.34 s including
  recompilation); the ordinary-path fallback counter remained unchanged.
- `cargo test -p koushi-core --lib display_projection -- --nocapture`: 10 passed,
  0 failed.
- `npm --prefix apps/desktop test -- src/domain/timelineStore.test.ts
  --reporter=dot`: 69 passed, 0 failed.
- `cargo test -p koushi-core --lib timeline::tests -- --test-threads=4`:
  213 passed, 1 intentionally ignored, 0 failed.
- `cargo check -p koushi-core`: passed without warnings.
- `npm --prefix apps/desktop run typecheck`: passed.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.
- Impacted non-SDK and authoritative-recovery focused tests: 3 passed, 0
  failed.

The requested `cargo fmt --all -- --check` did not pass because clean,
out-of-scope files already present under `vendor/matrix-rust-sdk` differ from
the workspace rustfmt result. Stable rustfmt reported
`event_cache/redecryptor.rs` and the SDK event-cache integration test; nightly
rustfmt also reported pre-existing SDK event-cache source formatting. The
vendor repository remained clean and was not modified. The four Task 1 files
produce no rustfmt diff.

No long homeserver/integration lane was run, as requested.

## Projection and fallback behavior

All canonical diff variants are handled by `project_sdk_batch`. `Set` and
`Remove` capture the old canonical owner before mutation. Insertions use
displayed canonical neighbors and explicit prepend/append policy instead of
reusing a canonical index. Reset rebuilds the same bounded live-edge window as
fresh replay; restore temporarily disables that cap so historical prepends are
retained.

The release-path validator replays every output operation with the desktop's
render-identity normalization and checks both numeric bounds and final equality
with Core's authoritative display. Ambiguous canonical input or invalid output
emits `Reset { items: display_after }`. It increments the stable
`display_projection_reset_fallbacks` counter and records only fixed diagnostic
tokens and the count. The deliberate invalid-index test exercised this
fallback; ordinary production-shaped flows observed no fallback.

Projection work never scans the full canonical sequence for each diff. A
sparse implicit treap compresses canonical-only gaps and constructs its initial
membership in one Cartesian-tree pass. With W represented display slots, B SDK
operations, and D emitted display operations, projection overhead is expected
`O(W + B log(W+B) + D)` time and `O(W+B+D)` temporary space. The private diff
builder guarantees `D = O(W)`, and Room live-edge W is hard-capped at 120.
Reset scans its supplied replacement payload once. Existing canonical
`Vec<TimelineItem>` insertion/removal costs are deliberately outside this
projection-only bound and are stated as such in the source.

## Self-review

Reviewed the full diff against `REPOSITORY_RULES.md`,
`docs/architecture/overview.md`, `docs/architecture/state-machine.md`,
`docs/policies/engineering-rules.md`, `AGENTS.md`, issue #285, its approval
comment, and the Task 1 brief.

Confirmed:

- Rust remains the owner of Matrix/index-domain semantics; no frontend repair
  heuristic or body/timestamp/sender matching was added.
- No raw canonical SDK diff is sent to `ItemsUpdated` or restore buffering.
- Numeric indices are documented in Rust and TypeScript as display-relative.
- Explicit duplicate canonical owners survive normalization until their last
  owner leaves membership.
- Generation rejection occurs before either canonical or display state commits.
- Replay-known lifecycle publication shares the same generation lease as the
  display update.
- Diagnostics contain no room, event, sender, transaction, body, URL, path, or
  raw-error data.
- The former raw display reducer and model helpers are test-only; no second
  production projection implementation remains.
- Repeated synthetic fixtures were consolidated into one helper, and
  `cargo check` introduced no `cfg(test)` or dead-code warnings.

## Remaining concerns

- Workspace-wide rustfmt is blocked by the pre-existing clean vendor baseline
  described above.
- The intentionally deferred long homeserver lane still needs to be run once
  after parent review.

## PAUSE CHECKPOINT — 2026-07-20

The overflow-recovery atomicity slice is complete. A stale actor is rejected
before relay teardown and is fenced again after the SDK subscription await.
The recovery generation, batch-id reset, projected-gap reset, authoritative
canonical/display window, media-source cache, `ResyncRequired`, `InitialItems`,
and replay-known ownership now commit under one actor-generation lease.

Fresh checkpoint evidence:

- `cargo test -p koushi-core --lib authoritative_resync_projects_event_only_and_emits_ordered_recovery_events -- --nocapture`: 1 passed.
- `cargo test -p koushi-core --lib stale_generation_recovery_does_not_commit_candidate_or_publish -- --nocapture`: 1 passed.
- `cargo test -p koushi-core --lib relay_overflow_recovery_subscribes_once_emits_snapshot_then_next_live_update -- --nocapture`: 1 passed.
- `cargo check -p koushi-core`: passed without warnings.
- `cargo fmt -p koushi-core -- --check`: passed after applying package formatting.
- `git diff --check`: passed.

Also completed before this pause: the restore terminal now publishes its
coalesced projected `ItemsUpdated`, replay-known lifecycle, changed navigation,
and `AnchorRestoreFinished` under one lease. A causal projection whose display
transition normalizes to a no-op still emits an empty `ItemsUpdated` render
fence. The exact projection-overhead contract is documented as expected
`O(W + B log(W+B) + D)` time and `O(W+B+D)` temporary space, with `W <= 120`
at the Room live edge and `D = O(W)` for the private builder; canonical Vec
mutation costs are explicitly outside that bound. The operational test counter
now measures only visible-payload visits and no longer claims strict additive
tree work.

Remaining after resume:

1. Re-run the full focused display/restore/timeline unit set after the latest
   formatting and inspect any failures.
2. Run the pending TypeScript TimelineStore regression and desktop typecheck.
3. Consolidate the still-large implementation diff and remove any remaining
   superseded helpers/tests without weakening the projection/lease contracts.
4. Run the requested focused gates, package format/diff checks, and independent
   review; update this report with final GREEN evidence and commit SHA.
5. Leave the long homeserver lane to the root agent's single post-review run;
   do not create/push a PR from this worker.

## FINAL RESUME CHECKPOINT — 2026-07-20

The pending media reducer regression is GREEN and committed. The membership
constructor was consolidated from repeated treap merges to a single linear
Cartesian-tree build, removing an avoidable `W log W` setup term while keeping
all dynamic SDK index operations in the same sparse projection boundary.

Fresh final evidence:

- `cargo test -p koushi-core --lib sdk_canonical_indices_project_to_bounded_display_and_converge_local_echo -- --nocapture`: 1 passed.
- `cargo test -p koushi-core --lib display_projection -- --nocapture`: 10 passed.
- `cargo test -p koushi-core --lib timeline::tests -- --test-threads=4`: 213 passed, 1 intentionally ignored, 0 failed.
- `npm --prefix apps/desktop test -- src/domain/timelineStore.test.ts --reporter=dot`: 69 passed.
- `cargo check -p koushi-core`: passed without warnings.
- `npm --prefix apps/desktop run typecheck`: passed.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

Final implementation audit found no second production SDK projection path and
no raw canonical-index `ItemsUpdated` emission. Text and media confirmation,
duplicate ownership, every diff variant, stale actor rejection, validated Reset
fallback, real restore terminal grouping, recovery grouping, and display no-op
causal fences remain covered. No body, sender, event, room, transaction, URL,
path, or raw error is recorded by the fallback diagnostic.

The only gates deliberately left to the root agent are independent frontier
review and the single long homeserver lane. No PR was created or pushed here.

## POST-REVIEW HARDENING — 2026-07-20

The root review found five real boundary weaknesses; all were addressed before
handoff:

- SDK `Append`, `PopBack`, and `PopFront` now convert as one ordered batch with
  an evolving canonical length. `Append` emits ordered `PushBack` operations,
  `PopBack` removes only `len - 1`, and a mixed batch preserves its prefix.
- Awaitable reducer/search delivery reserves channel capacity first, then
  reacquires the actor-generation lease and publishes synchronously. A
  replacement during the await discards the prepared continuation. Diff and
  authoritative-recovery continuations likewise reacquire ownership before
  each synchronous mutation/publication stage and stop after a stale await.
- Actor-originated `Set` changes resolve the exact retained
  `DisplayProjectionSlot.canonical_index`. Duplicate render identities cannot
  redirect a revision to the wrong owner; an exact owner outside the bounded
  display produces no display diff. SDK and non-SDK projection now share the
  same final diff builder, validator, and Reset fallback.
- The production actor/manager restore test now drives two distinct SDK diff
  batches through active buffering and a real `finish_anchor_restore`
  terminal. It requires a non-empty multi-batch buffer, exactly one convergent
  `ItemsUpdated`, and ordered `ItemsUpdated` → `NavigationUpdated` →
  `AnchorRestoreFinished` publication.
- The brittle source-parsing gateway test was deleted. The obsolete
  flush-only test helper was also removed; restore tests exercise the real
  terminal boundary.

Fresh post-review evidence:

- Ordered SDK variant fixture: 1 passed, 0 failed.
- Duplicate-owner and out-of-window non-SDK projection: 2 passed, 0 failed.
- Replacement-during-capacity-await generation fence: 1 passed, 0 failed.
- Production actor restore settlement: 1 passed, 0 failed (0.60 s).
- Full `timeline::tests`: 214 passed, 1 intentionally ignored, 0 failed.
- TimelineStore: 69 passed, 0 failed.
- Desktop typecheck: passed.
- `cargo check -p koushi-core` and the `--no-default-features` variant: passed
  without warnings.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

The long homeserver lane remains deliberately delegated to the root agent.

## POST-LANE RECONNECT DIAGNOSIS — 2026-07-20

The first targeted `timeline_reconnect` failure contained one QA observer
defect: the non-legacy waiter discarded the `InitialItems` returned by the
reopened subscription and only watched future `ItemsUpdated`. That was fixed in
`b8d564c`; a synthetic network-free regression now composes both sources.

After that harness correction the lane still reports exactly one missing body.
The production-path trace ranks the remaining hypotheses as follows:

1. **SDK initial-subscription window boundary (strongest evidence).** The QA
   creates 21 offline messages. The vendored SDK's `TimelineSubscriber`
   deliberately exposes at most 20 initial items via
   `MAXIMUM_NUMBER_OF_INITIAL_ITEMS`, and SyncService room subscriptions also
   request a timeline limit of 20. Koushi calls `timeline.subscribe()` directly
   on actor spawn and only auto-paginates when the initial Room timeline is
   empty. Therefore the oldest one of 21 messages can remain outside the public
   subscriber window while event-cache continuity still reports no gap.
2. **Koushi display projection (weaker evidence).** The issue-285 projection
   receives only the SDK subscriber's exposed canonical sequence. Its live-edge
   cap is 120, so it cannot itself reduce a 21-message batch to 20. The focused
   projection suite covers ordered append/pop, duplicate ownership, local-echo
   convergence, reset, and display-space validation. A projection defect is
   still possible, but it does not explain the exact upstream 20/21 boundary as
   directly.
3. **Identity deduplication (weakest evidence).** Distinct confirmed events use
   distinct event identities; transaction-to-event convergence only collapses
   the local echo with its own confirmed event. There is no evidence that two
   of the 21 remote sends share an identity.

This boundary exists independently of issue 285: the merge-base comparison
shows no change to `sync.rs`, `koushi-sdk`, the SDK subscriber's 20-item cap, or
the SyncService room-subscription limit. Issue 285 changes how an already
exposed canonical diff is mapped to the bounded display; it does not enlarge
the SDK subscription window.

The QA timeout diagnostic now reports only the missing expected-array indices
and count. It never records body text, room/event/user/transaction identifiers,
URLs, paths, or raw errors. The exact next experiment is one rerun of only the
targeted `timeline_reconnect` lane with this diagnostic. `missing_indices=[0]`
would confirm the 20/21 oldest-edge boundary; any interior or newest index would
falsify that specific explanation and redirect the trace to projection/dedupe.
No long lane was run during this diagnostic slice.

## FINAL RE-REVIEW HARDENING — 2026-07-20

The final ownership review closed two replacement races that were not covered
by the first hardening pass:

- Composer success, failure, and cancellation terminals are now handed back to
  the manager as submission-ledger work. They are deliberately independent of
  the replaced timeline actor generation, and the submission is tombstoned
  only after the reducer terminal action has been delivered reliably.
- Thread-root hydration now prepares only immutable activity inputs, reserves
  publication capacity, then applies service deltas and publishes reducer
  actions, projection events, and a generation-tagged manager fetch batch under
  one current-generation lease. This avoids both stale mutation and whole-state
  candidate swaps that could lose a concurrent completion for another root.
  Manager fetch completion repeats the same post-await generation validation
  before mutating shared state. Multiple root fetches use one manager-mailbox
  message, so a projection containing more roots than mailbox capacity cannot
  deadlock on unpublished permits.

Fresh focused race coverage verifies terminal delivery across actor
replacement for success/failure/cancel, stale hydration preparation while
waiting for capacity, a fetch count larger than mailbox capacity, and manager
rejection of stale-generation starts and completions.

Fresh verification evidence:

- Full `timeline::tests`: 217 passed, 1 intentionally ignored, 0 failed.
- TimelineStore: 69 passed, 0 failed.
- Desktop typecheck: passed.
- `cargo check -p koushi-core` and the `--no-default-features` variant: passed.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

The long homeserver lane remains delegated to the root agent; this worker did
not push or create a PR.

## TARGETED RECONNECT QA HARNESS FIX — 2026-07-20

The targeted `timeline_reconnect` rerun reached timeline recovery but reported
one missing expected row. This was a QA observer bug already present on
`origin/main`, not a product projection failure: `subscribe_timeline_for_qa`
consumes and returns `InitialItems`; the legacy waiter seeded its model from
those returned items, while the non-legacy waiter discarded them and observed
only future `InitialItems`/`ItemsUpdated` events.

The non-legacy waiter now seeds expected-body observation from its returned
`reopened_items` before consuming future diffs. It still requires every truly
absent expected body: the network-free regression composes one body from the
initial snapshot with one from a future diff and verifies that a third absent
body remains missing.

Fresh verification evidence:

- Focused initial-plus-future observation regression: 1 passed, 0 failed.
- Full headless Core QA binary tests: 64 passed, 0 failed.
- Headless Core QA binary check with `qa-bin`: passed.
- `cargo check -p koushi-core`: passed.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

No homeserver scenario was rerun by this worker; the root agent owns the single
targeted long rerun after review.

## FINAL PERMIT-ORDER HARDENING — 2026-07-20

A final bounded-channel review found one lock-free deadlock cycle: hydration
reserved reducer capacity and then awaited manager-mailbox capacity, while the
manager could be processing an earlier message that itself needed reducer
capacity. Hydration now reserves the manager batch first and reducer capacity
second. The reducer reservation is the final await; actor-generation lease,
service mutation, reducer publication, and manager StartFetch publication are
then one synchronous commit with no permit, lease, or mutex crossing another
await.

The deterministic capacity-one regression saturates both channels, manually
first-polls the earlier manager sender, hydration reservation, and manager
reducer send to prove their FIFO enrollment before releasing either capacity,
and then polls the selected owner directly. It uses no scheduler yields, sleeps,
or timeout-based ordering. Reverting only the production permit order makes it
fail immediately because hydration owns the first freed reducer slot; the fixed
order passed 10 consecutive focused runs while also checking publication order,
actor generation, fetch payload, and pending service state.

Fresh verification evidence:

- Focused saturation regression: 10/10 consecutive runs passed.
- Full `timeline::tests`: 218 passed, 1 intentionally ignored, 0 failed.
- TimelineStore: 69 passed, 0 failed.
- Desktop typecheck: passed.
- `cargo check -p koushi-core` and the `--no-default-features` variant: passed.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

The long homeserver lane remains delegated to the root agent; this worker did
not push or create a PR.

## INTEGRATED QA EVIDENCE — 2026-07-20

The exact implementation and diagnostic HEAD exercised by the integrated QA
was `bdb3d80e157a91e85a26f5b40056b7c645b7bff2`.

The official `qa:headless-core` attempt passed the SDK Conduit prelude. It then
stopped in the generic Core `login_sync` stage: `normal_sync_started` was not
observed before timeout because the existing new-identity gate expectation did
not match the account state. This branch does not modify account or sync code,
so this is an unrelated verification-gate failure. The official long lane must
not be reported as GREEN.

The targeted Conduit probed `timeline_reconnect` run after the `b8d564c`
initial-snapshot seeding fix completed normal sync, timeline setup, disconnect,
reconnect, and reopened-timeline observation. Its private-safe diagnostic
reported exactly `missing_indices=[0]`. A repeated diagnostic produced the
same oldest-index result. During those runs continuity remained
`gap_count=0`, and no `display_projection_reset_fallback` diagnostic was
observed.

That repeated result confirms the pre-existing 20/21 boundary described above:
the vendored SDK public timeline subscription exposes at most 20 initial items,
the QA creates 21 offline messages, and Koushi does not automatically paginate
a non-empty initial timeline merely because the SDK subscriber retained an
older hidden prefix. The event cache can therefore be structurally gap-free
while the oldest expected row remains outside the public subscriber window.
This behavior is present at the merge base and is an SDK/QA/product-pagination
boundary, not an issue-285 display-projection regression. The issue-285
projection operates downstream on the already exposed canonical sequence, has
a 120-row live-edge display bound, emitted no Reset fallback in the failing
run, and cannot account for the exact repeated oldest item at the SDK's 20-item
boundary.

All non-homeserver verification remained GREEN at this HEAD: full Core timeline
tests (218 passed, 1 intentionally ignored), TimelineStore (69 passed), desktop
typecheck, Core checks with default and no-default features, the full headless
QA binary test suite (64 passed), the QA binary check, package rustfmt, and
`git diff --check`. The independent implementation review was approved. No
production or test code was changed for this evidence update, and no scenario
was rerun here.

## SEND-QUEUE FRESH-IDENTITY QA HARNESS FIX — 2026-07-20

The official long-run failure before `normal_sync_started` was isolated to the
generic QA login harness on a fresh account. That path completed the new-
identity gate before `wait_for_logged_in` only for `E2eeTrust`, `GateRestore`,
and `GateNegative`; `SendQueue` could therefore remain restricted before the
scenario reached its actual send-queue assertions.

The harness bootstrap predicate now includes only `SendQueue` in addition to
those three existing scenarios. `GateNoProof` remains outside the generic path
and retains its dedicated no-proof semantics. Ordinary `LoginSync` and the
specialized `TimelineReconnect` path remain unchanged. No product account,
identity, trust-gate, or sync behavior was modified.

A network-free contract regression records that `SendQueue`, `E2eeTrust`,
`GateRestore`, and `GateNegative` bootstrap before waiting for `LoggedIn`, while
`GateNoProof`, ordinary `LoginSync`, and the specialized `TimelineReconnect`
path do not use that generic bootstrap. RED failed because the predicate did
not exist; GREEN passed after routing the production harness condition through
the same predicate.

Fresh short-gate evidence:

- Focused SendQueue bootstrap contract: 1 passed, 0 failed.
- Full headless Core QA binary tests: 65 passed, 0 failed.
- Headless Core QA binary check with `qa-bin`: passed without warnings.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

No long scenario was run by this worker. The separate
`timeline_reconnect missing_indices=[0]` result remains the pre-existing SDK
20-item initial-window versus QA 21-offline-item pagination boundary described
above; this SendQueue-only harness change does not address or alter it.

### Secondary participant follow-up

The next targeted Conduit SendQueue log proved that the primary A bootstrap
succeeded and exposed a second harness-only omission. The generic room-flow
participant B submits a separate `LoginPassword`; on a fresh account it stayed
at `trust=unknown` / restricted catch-up because that direct login had no
SendQueue identity bootstrap before its own `wait_for_logged_in`.

Strict TDD added a separate network-free secondary-participant policy contract
before production changes. Its RED result was the expected Rust compiler error
`cannot find function should_bootstrap_secondary_identity_before_logged_in`.
The contract requires true only for `SendQueue` and explicitly keeps
`LoginSync`, `TimelineReconnect`, `GateNoProof`, `E2eeTrust`, `GateRestore`, and
`GateNegative` out of this generic B policy so their unchanged or dedicated gate
semantics cannot be broadened accidentally.

The smallest harness fix adds that SendQueue-only predicate and calls the
existing `complete_new_identity_gate_for_qa` immediately after participant B's
`LoginPassword` submission succeeds and before `wait_for_logged_in`. No product
account, sync, trust, or gate implementation changed.

Fresh secondary-participant GREEN evidence:

- Focused B bootstrap policy contract: 1 passed, 0 failed.
- Full headless Core QA binary tests: 66 passed, 0 failed.
- Headless Core QA binary check with `qa-bin`: passed without warnings.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

No long Conduit lane was run here. The root agent owns one targeted SendQueue
rerun. The separate timeline-reconnect SDK 20/QA 21 pagination boundary remains
unchanged.

### Idempotent active-timeline replay follow-up

The same single targeted Conduit run then reached `room_space=ok` for both
participants. Core emitted a same-room replay (`replay_initial_emitted` with a
count of 10), but QA timed out at `subscribe timeline A` because the generic
waiter required the newly submitted Subscribe request id exactly.

This is a second, distinct pre-existing QA contract mismatch. The room can
already be auto-subscribed when generic phase 5 submits its explicit Subscribe.
The actor's idempotent fast path intentionally ignores that new request id and
re-emits `InitialItems` with the original unacknowledged
`projection_request_id` (or no id after acknowledgement). Retaining that
identity is correct: the frontend projection ACK must acknowledge the same
delivery identity until the actor accepts it; substituting the newest
idempotent Subscribe id would weaken lost-delivery replay and ACK correlation.
No product timeline protocol or projection identity was changed.

Strict TDD first added a network-free matcher contract. RED failed with the
expected missing `InitialItemsWaitPolicy`, `InitialItemsWaitMatch`, and
`match_initial_items_wait_event` symbols. GREEN proves:

- the normal policy still accepts only the exact request id and key;
- only the explicit active-key replay policy accepts the same key with an old
  request id or no request id;
- a wrong timeline key is always ignored; and
- `OperationFailed` is still accepted only for the newly submitted exact
  request id, while an old-request failure is ignored.

The generic `subscribe timeline A` call alone now uses a dedicated
`wait_for_initial_items_or_active_replay` waiter because that room may already
be active. All other `wait_for_initial_items` calls, including the corresponding
B subscription, remain exact; the log provided no evidence that B entered the
idempotent path.

Fresh short-gate evidence:

- Focused idempotent replay matcher contract: 1 passed, 0 failed.
- Full headless Core QA binary tests: 67 passed, 0 failed.
- Headless Core QA binary check with `qa-bin`: passed without warnings.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

No long Conduit lane was rerun here. This harness-only change is independent of
both issue-285 display projection and the separate timeline-reconnect SDK
20/QA 21 pagination boundary.

### Secondary active-timeline replay follow-up

The final targeted Conduit run confirmed the A-side active replay waiter and
then advanced through both messages' local echoes and send-completed terminals.
It failed only at the generic `subscribe timeline B` exact waiter. Core recorded
a B-side active replay with three initial items, proving B was also already
auto-subscribed and hit the same established projection-identity boundary.
This is not a new product failure.

Strict TDD added a deterministic network-free source-policy contract first.
RED failed exactly because the generic B subscription block did not contain
`wait_for_initial_items_or_active_replay` and still contained the exact
`wait_for_initial_items` call. GREEN changes only that B call to the existing
same-key active replay waiter. Every other initial-items call site remains
exact.

Fresh short-gate evidence:

- Focused generic-B replay-waiter policy: 1 passed, 0 failed.
- Full headless Core QA binary tests: 68 passed, 0 failed.
- Headless Core QA binary check with `qa-bin`: passed without warnings.
- `cargo fmt -p koushi-core -- --check`: passed.
- `git diff --check`: passed.

No long Conduit lane was rerun here. Product replay request/ACK identity remains
unchanged, as do issue-285 projection behavior and the separate SDK 20/QA 21
timeline-reconnect pagination boundary.
