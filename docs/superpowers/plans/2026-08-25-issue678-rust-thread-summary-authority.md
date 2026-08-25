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
