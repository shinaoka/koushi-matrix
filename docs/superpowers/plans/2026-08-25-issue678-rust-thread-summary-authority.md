# Issue #678 — Rust-Owned Live Thread Summary Authority

## Status and review gate

This independently mergeable bug fix starts from `origin/main`
`e35043cc5fb2b51b83e676b20f2bbc5916f6a48b` with Matrix SDK gitlink
`56028a4ded016381d75bdd5ed978af380f0809a2`.

The user previously selected `reviewer-flash` as the mandatory cross-model
reviewer after `reviewer-flash-opencode-go` exhausted quota. Implementation must
not start until that reviewer records `Correct-to-merge` for this design and the
canon amendments in this change.

## Evidence and root cause

A live thread reply can reach the open Thread timeline while the canonical room
root keeps a complete but old `ThreadSummaryDto`. Restart repairs it because the
SDK event cache is rehydrated and the root is rebuilt.

The current ownership split is concrete:

- `thread_summary_from_sdk` copies the SDK bundled root summary directly into
  every canonical root `TimelineItem`;
- issue #570 Task B added `ThreadRootProjectionService` plus
  `resolve_thread_relation_aggregate`, exact activity/summary revisions, and
  correct edit/redaction/count handling, but that service currently owns only a
  root missing from the bounded canonical Room window;
- `handle_aggregate_refresh_finished` emits only the off-window
  `ThreadRootProjection`; replay-known ownership suppresses that terminal while
  the canonical root is present, so an authoritative aggregate cannot update the
  in-window item;
- `timelineDisplayProjection.ts` still fills null summary fields from visible
  replies, and deliberately preserves stale non-null values. This is a second,
  incomplete TypeScript authority.

The missing seam is therefore not another Matrix history algorithm. The existing
#570 service already owns the authoritative aggregate; its accepted result is
not applied to canonical root items.

## Canonical ownership

`TimelineManagerActor`'s existing session-scoped `ThreadRootProjectionService`
is the sole Koushi thread-summary reconciliation owner, keyed by account/session,
room ID, and root event ID. It remains bounded by the active Room timeline
window and existing replay/hydration retention. It stores no second thread
timeline and adds no persistence schema.

Inputs are:

- SDK root summary during initial Room hydration or replay;
- an eligible live reply observed by the exact Thread timeline;
- same-identity effective edits;
- redaction/removal/reset of a reply;
- the existing SDK event-cache/thread-list aggregate resolver;
- normal Rust display-label projection at event delivery.

The checked-out SDK remains protocol/event authority. Koushi reconciles only the
bounded presentation summary. On restart, the service is rebuilt from the SDK
event cache; no plaintext-derived first-party summary store is introduced.

## Reconciliation and routing

### Room and Thread observations

After an accepted timeline batch, derive only affected thread roots from the
pre/post Rust item windows:

- a thread reply contributes its `thread_root`;
- a canonical root with `thread_summary` contributes its own stable event ID;
- removal, redaction, reset, and clear retain the pre/post union long enough to
  refresh or clear the exact root;
- unrelated rows schedule no aggregate work.

A Thread actor sends one reliable, generation-fenced internal observation for
its exact room/root. Renderable activity reuses the existing matching-thread
reply eligibility: stable event identity, matching `thread_root`, no transaction
local echo, and attention-eligible content. Redacted `Set` rows and pre/post
removals are conveyed separately as invalidation observations instead of being
lost to the renderability filter.

The manager validates that the source Thread key and actor generation are still
current, then looks up the exact current `TimelineKind::Room` key in its
`timelines` map. It never awaits the Room actor's bounded mailbox. Each Room
actor handle instead owns a dedicated bounded latest-wins `watch` wake carrying
at most the existing 120 active-root slots. Manager publication is nonblocking;
the Room actor drains observations/accepted completions from the watch before
ordinary data work. Actor replacement drops the old watch and its values;
observations that cannot match a current Room actor are discarded because there
is no visible room-root surface, and initial Room subscription reconciles from
the SDK cache. A focused saturation test must prove the manager continues to
drain while the Room actor is waiting to schedule aggregate work—no manager ↔
actor mailbox ABBA cycle exists.

The Room actor records a present canonical root as a Ready root item without a
network hydration fetch. A genuinely off-window root keeps the existing one-fetch
hydration path. Both paths schedule the same existing aggregate resolver and use
the same checked activity and summary revisions. Duplicate/replayed observations
are idempotent; a later scheduled revision makes an older completion inert.

### Aggregate floor and repair

The SDK event-cache aggregate is authoritative when it is at least as current as
accepted live activity. A stale bundled or aggregate candidate may not regress a
newer accepted renderable live reply:

- a newer accepted live timestamp advances latest identity/sender/label/body/
  timestamp;
- the same identity may repair edited preview/label without changing count;
- a newer SDK aggregate may repair activity missing from the loaded window;
- duplicate/replayed activity does not increment count;
- when renderable live activity is newer than a lagging aggregate, count is at
  least the prior accepted count plus one for a genuinely new latest identity
  and at least the SDK count;
- a matching redaction or removal observation retires that live floor before
  comparison and yields to the exact current SDK event-cache aggregate, allowing
  latest B/count2 to roll back to A/count1 or empty/count0; the invalidated event
  identity is held only in the bounded in-memory root record and is never logged;
- Task A/B's relation aggregate remains authoritative for edit validity,
  redaction fallback, and exact settled count. No local relation ledger is added.

The existing Task B revision and disappearance tests remain the authority for
count 2→1→0, latest edit, latest redaction, older edit, clear/reset, delayed
completion, and serial exhaustion.

### Canonical application

An exact accepted aggregate completion publishes an
`ApplyThreadSummaryProjection` wake into the current Room actor's dedicated
latest-wins watch. The wake contains only root identity and activity/summary
revisions; it never consumes or waits for ordinary actor-mailbox capacity. The
actor rechecks the shared service at those exact revisions, overlays the
aggregate on the matching canonical root, and emits one normal
generation/batch-fenced `TimelineDiff::Set` through
`emit_non_sdk_item_sets_and_reconcile_replay_known`.

Before any later SDK `Set` is published, Core overlays an already accepted
service aggregate onto that root. Thus an older bundled SDK summary cannot
regress a newer Rust-owned projection while a refresh is pending. A missing root
continues through the existing `ThreadRootProjection` event path.

No best-effort `try_send` may discard an observation, completion, or canonical
patch. The dedicated watch is an explicit latest-wins projection wake whose
bounded current value remains owned until the actor observes or replaces it; it
is not a lossy notification. Actor replacement, unsubscribe, account change,
and generation mismatch make late work inert through existing owner/generation/
revision fences.

## TypeScript boundary

Delete `inferLatestReplyFromVisibleItems`, `InferredThreadReply`, its comparator,
and `threadRootItemWithInferredSummary`. `asThreadRoot` uses only the
Rust-supplied `thread_summary` for identity, preview, timestamp, and placement.
Visible replies remain rows used for presentation ordering/suppression only; they
never repair summary semantics.

No new TypeScript map, retry, timer, expected-latest value, or reconciliation
state is added. Browser Fake and app harness continue to consume supplied
Rust-shaped timeline events.

## Diagnostics

Add one `core.thread_summary` diagnostic at each accepted reconciliation. It is
private-data-free and contains only:

- process-local bounded room/root ordinals, never Matrix IDs;
- closed source token: `sdk_summary`, `live_reply`, `edit`, `redaction`, or
  `rehydration`;
- previous/candidate latest identity relation: `missing`, `same`, or `different`;
- decision: `advance`, `retain`, `repair`, `remove`, or `no_op`;
- reply counts before/after;
- `dto_changed` boolean.

Message bodies, labels, room/root/event/user IDs, timestamps, raw SDK errors, and
local paths are forbidden. Ordinal maps are cleared with the session service and
share its root/room bounds.

## Verify-first RED

Before production wiring, add only the minimum internal message/type scaffolding
needed for runnable checks, then capture behavioral RED:

1. Core manager/actor regression: seed a canonical root with a complete non-null
   summary for reply A, admit newer live reply B, complete the existing aggregate,
   and assert that the Room actor emits an `ItemsUpdated(Set)` whose root summary
   is B. Current code emits no canonical Set.
2. Stale SDK regression: after B is accepted, apply an SDK root Set carrying A
   and assert the emitted item remains B.
3. TypeScript render-only regression: a root with null latest fields plus a
   visible reply remains exactly the Rust-supplied null summary. Current code
   fills the fields locally.
4. Restart equivalence: initial/replayed Room items plus the same event-cache
   aggregate produce the same summary as the live pre-restart projection.

The checks must fail by assertion against current behavior, not by compile error.
The same checks must pass unchanged after wiring.

## Implementation evidence

### Behavioral RED before production wiring (2026-08-25)

The approved design was recorded before these checks. Only tests changed; no
production path was wired.

- `cargo test -p koushi-core --lib canonical_root_with_live_reply_schedules_authoritative_summary_refresh`: **RED** (exit 101), 0 passed / 1 failed / 1,083 filtered. A canonical root (`missing_activities=[]`) emitted no `StartAggregateRefresh`.
- `cargo test -p koushi-core --lib newer_live_activity_floors_a_lagging_sdk_aggregate_without_double_counting`: **RED** (exit 101), 0 passed / 1 failed / 1,083 filtered. The lagging A/count1 aggregate replaced accepted live B instead of projecting B/count2.
- `npm --prefix apps/desktop run test -- --run src/domain/timelineStore.test.ts`: **RED** (exit 1), 75 passed / 1 failed. TypeScript filled all null Rust summary fields from the visible reply.

Logs: `/tmp/issue678-red-canonical.log`, `/tmp/issue678-red-floor.log`, and
`/tmp/issue678-red-ts.log`. The unchanged assertions are the GREEN gate.

### Integrated GREEN before exact review (2026-08-25)

- The three unchanged RED checks are GREEN. Focused Core thread-summary service,
  canonical application, affected-root filtering, revision, redaction rollback,
  diagnostic, manager-mailbox/watch, and replay suites pass; the complete Core
  lib is **1,093 passed / 8 ignored**.
- `ThreadRootProjectionService` now seeds a canonical bundled summary before
  publication, retains a newer live floor across stale SDK/event-cache inputs,
  requires an independently matching event-cache aggregate before a non-explicit
  rollback, and treats bounded-window disappearance as non-authoritative. The
  real QA lane exposed and fixed both count1-on-live-B and transient-removal
  regressions rather than weakening its exact count assertion.
- TypeScript no longer contains any visible-reply summary inference. Focused
  timeline/store/token tests are GREEN; full Vitest is **1,494/1,494**,
  typecheck/lint/build/secret and boundary checks pass, and Playwright is
  **262/262** without an App unhandled-error signature.
- Rust CI-shaped gates are GREEN: workspace all-targets **2,524 passed / 12 ignored**,
  Tauri **174 passed / 1 ignored**, state/search wasm check, QA binary
  **133/133**, rustfmt, SDK submodule, diagnostic-isolation, agents/docs,
  cargo-deny, cargo-machete, and diff checks. One default-parallel all-targets
  rerun exposed the two unrelated SDK global-mock probe tests racing each other;
  both tests passed individually and the complete all-targets matrix passed with
  one test thread. The ordinary CI-shaped workspace command was already GREEN,
  and current-head CI remains the authoritative default-parallel gate.
- The unchanged event-driven `redact_edit_convergence` lane passed separately on
  tuwunel and synapse and finally through `--server=both`. It proves old A →
  live B/count2 on the already-open Room and Thread surfaces, same-ID edit/count2,
  B redaction → A/count1, real runtime shutdown/restore equality, and emits both
  `redact_edit_convergence=ok` and `thread_summary_convergence=ok` with
  private-data validation.
- Linux cannot execute the macOS-only cargo lane; the cross-platform Tauri crate
  check is GREEN locally and current-head macOS CI remains a mandatory merge
  gate. Exact full-diff review, final post-review reruns, PR CI, and merge remain
  pending.

## Required focused semantics

Add or retain tests for:

- non-null old A → live B advances canonical root immediately;
- older SDK/root replay cannot regress B;
- newer authoritative hydration repairs missing activity;
- same-ID edit changes preview/label with unchanged count;
- older-reply edit does not replace latest;
- latest redaction selects the prior renderable reply or all-null details/count 0;
- duplicate/replayed diff is idempotent;
- profile-label patch changes label without identity/count change;
- canonical ↔ off-window transition uses the same aggregate and emits no split
  authority;
- manager/Room mailbox saturation cannot deadlock observation or completion;
- actor replacement, missing current Room actor, and stale activity/summary
  revisions emit nothing and initial Room hydration recovers current cache truth;
- restart/replay equals the accepted live projection;
- diagnostics expose only closed tokens, ordinals, counts, and booleans.

## Headless QA

Extend the existing event-driven `redact_edit_convergence` scenario rather than
adding another room/session setup:

1. create a thread with older reply A and open both Room and Thread timelines;
2. send remote reply B and observe B in the Thread timeline;
3. observe the canonical Room root `ThreadSummaryDto` advance to B without room
   reopen;
4. edit B and verify same identity/count with changed projected preview;
5. deliver B's redacted row through the Thread observation path, retire its live
   floor, and verify the canonical root selects A;
6. perform the scenario's real runtime shutdown/restore, reopen the Room
   timeline, and verify the same A summary;
7. emit the additional fixed token `thread_summary_convergence=ok`.

Register that token in the existing scenario contract and docs. Run unchanged on
tuwunel, synapse, and `--server=both`; no sleeps, identifiers, bodies, or raw
errors may enter output.

## Expected files and limits

Expected production/test surface:

- `crates/koushi-core/src/threads_list.rs`;
- `crates/koushi-core/src/timeline/{actor,manager,relay,thread_projection,diagnostics}.rs`;
- existing focused Core thread/manager tests;
- existing `redact_edit_convergence` QA scenario and token/docs contracts;
- `apps/desktop/src/domain/timelineDisplayProjection.ts` and focused tests;
- architecture/state-machine/state-ownership docs and this plan/index.

Do not change vendor code/gitlink, public Tauri commands, AppState/reducer DTOs,
persistence schema, Browser Fake behavior, search, read-state, notification
attention, or room-latest algorithms. Add no dependency, generic projection
framework, compatibility shim, timer/retry, frontend store, TODO, or sleep.

## Validation and merge gate

Run and record:

- focused behavioral RED and unchanged GREEN;
- complete Core lib/all-targets and QA-binary tests;
- existing Task B aggregate/edit/redaction/replay suites;
- TypeScript focused projection/store/component tests, full Vitest, typecheck,
  lint, build, and Playwright;
- Tauri tests and generated wire/golden checks (expected unchanged);
- workspace all-targets, wasm checks, rustfmt;
- SDK submodule, docs/agents, boundary/security/dependency/secret checks;
- `redact_edit_convergence` on tuwunel, synapse, and both with both fixed tokens;
- generated output, exact artifact, status, and `git diff --check` inspection.

Then generate one exact full-diff artifact, obtain mandatory `reviewer-flash`
`Correct-to-merge`, fix and re-review every finding, push, open a PR closing
#678, wait for current-head CI 7/7, merge, verify ancestry and issue closure,
remove disposable artifacts, and leave the worktree clean.

## Acceptance map

| #678 requirement | Evidence owner |
| --- | --- |
| live reply updates canonical root | Core RED/GREEN + both-server QA |
| one Rust summary authority | shared `ThreadRootProjectionService` + TS inference deletion |
| canonical/off-window parity | shared aggregate and transition tests |
| edit/redaction/count semantics | existing Task A/B resolver + new canonical application tests |
| no regression from older SDK input | revision fence + pre-publication aggregate overlay test |
| replay/restart equality | focused replay test + real QA restore |
| render-only TypeScript | deleted inference helpers + supplied-summary test |
| diagnostics | closed-token/ordinal privacy test |

## Design review record

- Mandatory `reviewer-flash` Round 1 on design commit `39c7433`: `Not
  correct-to-merge`. It found a manager↔Room bounded-mailbox ABBA risk, an
  undefined redaction rollback through the live-activity floor, and missing
  exact eligibility/current-Room routing statements.
- The design moved manager→Room delivery to an actor-owned bounded latest-wins
  watch, made redacted/removal observations retire the matching live floor,
  reused matching-thread eligibility, and specified exact source/current-Room
  generation validation and replacement recovery.
- Mandatory `reviewer-flash` Round 2 on amended design commit `f0d5879`:
  **Correct-to-merge**. It traced the watch against the biased actor loop, the
  service bounds/revision fences, redaction rollback, TypeScript deletion,
  diagnostics, RED matrix, QA token, and every #678 acceptance row; no finding
  of any severity remained. Implementation is authorized under this design.
- Mandatory exact implementation Round 1 on artifact SHA-256 `7b330fda…`:
  `Not correct-to-merge`. It found that Room diffs overlaid the retained service
  value before recording a newer raw SDK root summary, which could hide the raw
  affected-root evidence and skip the validating aggregate refresh; it also
  found that a confirmed provisional bundled-summary rollback was mislabeled as
  `redaction` diagnostics.
- The accepted-batch lease now derives affected roots from the raw pre-overlay
  window, records the provisional bundled summary, overlays only afterward, and
  validates any newer identity through the exact event-cache aggregate. A
  focused opposite-direction regression proves A remains displayed only until
  the exact aggregate validates B, after which B/count2 is emitted. Confirmed
  bundled rollback diagnostics remain `sdk_summary`; only explicit invalidation
  or authoritative disappearance is `redaction`. Core/full workspace and the
  unchanged both-server QA matrix are GREEN after the fixes.
- Mandatory exact implementation Round 2 on artifact SHA-256 `2e2c35b3…`:
  `Not correct-to-merge`. It found a leading-aggregate case where a later live
  observation could count the already-included reply again, and an unfiltered
  no-reply-row loop that refreshed every tracked canonical root on unrelated
  batches while labeling the work as redaction.
- `update_live_activity_floor` now detects when the exact accepted aggregate
  already contains the observed event and keeps its count unchanged. The
  no-reply-row loop obeys the same affected-root filter and uses
  `CanonicalBatch` for a still-present root; only an inactive noncanonical root
  uses `Removal`. Focused regressions prove no +1 recount and no unrelated
  worker churn. Raw provisional SDK candidates remain separately validated, and
  full Core/workspace plus the unchanged both-server QA matrix are GREEN.
- Mandatory exact implementation Round 3 on artifact SHA-256 `b5c013e8…`:
  **Correct-to-merge**. The reviewer traced the full artifact, both prior-round
  fixes, count arithmetic, raw/effective diff parity, revision/watch/lease
  ownership, cleanup, diagnostics, TypeScript deletion, and QA restore proof;
  no finding requiring change remained. Two nonblocking observations were
  recorded: Matrix deletion is represented by redaction in the supported
  contract, and summary-only Sets do not need a position-index refresh because
  item ordering is unchanged. Final submitted-state identity audit remains.
