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
