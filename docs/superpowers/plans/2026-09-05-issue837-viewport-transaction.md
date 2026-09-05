# Unified renderer viewport stabilization (#837)

Status: implementation and deliverable reviews complete; local gates passed; CI and merge status are tracked in GitHub.

## Scope and ownership

Replace the overlapping renderer prepend, projection and measurement compensation
paths with one bounded viewport stabilization transaction. Rust continues to own
contents, display ordering, subscriptions, pagination and timeline generations.
Do not change pagination thresholds, SDK behavior, transport contracts or add
retry timers. Existing explicit jumps and live-edge intent must supersede stale
stabilization, not compete with it.

The prior #520 plan (`2026-08-14-active-prepend-anchor.md`) describes the former
localized fix; its prohibition on adding a transaction is superseded by #837's
explicit single-owner migration requirement. Both mechanisms must not survive.

## Required proof

- At most one active transaction for a timeline generation; replacement,
  generation/key changes, jumps and live-edge transitions invalidate old work.
- Every genuine user scroll advances a revision; input before a pending frame
  must cancel or rebase its capture before any write.
- Prepend, projection, measurements and virtual estimate/DOM correction share
  one lifecycle with bounded settlement and no duplicated final compensation.
- Continuous upward input through multiple pages preserves the user's motion;
  prepend and delayed row/media measurements preserve visible row and offset.
- Programmatic echoes are fenced by transaction/write generation.
- Lifecycle diagnostics expose closed reasons and local counts only, never
  room/event/user identities, key hashes, message contents or raw errors.
- Prove active prepend, input-before-frame, delayed measurement, virtual fallback
  then mounting, consecutive generations, and pending room/thread/root switches.
  Retain or strengthen the #278/#520 regression tests.

## Implementation design

Use a small renderer-only controller in
`apps/desktop/src/components/timeline/TimelineViewportTransaction.ts`, integrated
into `TimelineView.tsx`. It owns one nullable transaction and its current input
revision/write generation. Keep DOM access in the component. Reuse the existing
viewport scheduler, height model, anchor helpers and pure-prepend classifier;
add no package, timeout, retry loop or generic orchestration framework.

### State and transitions

A transaction carries a monotonically increasing local id, exact timeline key
and Rust generation, stable anchor row/offset, captured input revision, current
renderer projection/layout revision, phase (`waiting-prepend`,
`waiting-measurement`, `settling`), and whether its optional virtual estimate has
been written. The one owner also retains the pending settlement frame and scoped
write evidence. Terminal outcomes are settled or cancelled with closed reasons.
Do not mirror phase as separate pending/restore booleans.

- Begin before an admitted prepend changes the render store, before a structural
  projection commit through the existing snapshot boundary, or before a measured
  height-model commit. The projection boundary must also serve App-owned stores
  whose callback may observe already-updated store data. Reject stale event
  generations/batches before changing transaction state: reuse the existing
  `classifyTimelineItemsUpdatedApplication` store predicate (exact generation,
  initialized/resync status and batch-sequence acceptance). Do not invent a second
  semantic admission predicate or require an active transaction to accept data.
- The Rust generation is not a page/batch counter: ordinary `ItemsUpdated`
  batches retain it. Renderer projection/height versions are layout evidence,
  not semantic generations. A new Rust generation cancels the old transaction;
  a later page in the same generation may join only before any of that
  transaction's projection changes have committed to the DOM. After a projection
  commit, cancel the prior transaction and begin a replacement with the latest
  valid stable free-scroll anchor for the current input revision, not geometry
  captured from the intermediate shifted DOM. The old id cannot write afterward.
- Fold related prepend/projection/measurement commits into the active record,
  retaining its valid pre-layout anchor. Active pure-prepend rendering may still
  defer through the existing idle/max-defer presentation timers, but deferred
  phase and anchor belong to this record. Timer flushes publish geometry/release
  work; they never restore an independently saved anchor.
- Increment input revision synchronously at wheel/touch/keyboard/scrollbar intent,
  before the browser scroll event, and on genuine scroll observations (including
  observations without a preceding intent). Clear old write evidence immediately.
  Invalidate each old scheduled correction on newer input. If a pure prepend
  remains uncommitted, rebase on its current old DOM before release. If layout
  already committed, cancellation alone would strand the prepend's displacement:
  retain the same stable row, update its offset by the negative of actual user
  `scrollTop` movement since capture/last observation, and advance its captured
  revision. Store the last observed physical scrollTop in the transaction and
  update it on its own estimated writes, so compensation is never counted as
  user movement. A callback created for an older revision still cannot write;
  only a newly scheduled, current-revision callback can settle the rebased capture.
  Account for physical movement before its delayed native scroll event when
  input intent is pending. Arm the existing idle/max-defer activity timers at
  the earliest input intent, not only the later scroll event; a nonmoving wheel
  must not strand deferred work. No new timer or retry loop is introduced.
- Coalesce settlement after the matching projection, virtual range and pending
  height-model commit have committed. Measurement-producing layout effects run
  before the single settlement path decides to write. Every scheduled callback
  validates active id, key, Rust generation, layout evidence and input revision.
  A replaced/cancelled frame may not clear its replacement's frame or blockers.
- Mounted anchor: compute the current DOM delta once, mark terminal before the
  final write to prevent re-entrant duplicate compensation, and refresh the
  settled free-scroll anchor plus viewport metrics/observation/backfill evaluator.
  Zero delta requires no write but must still release blockers.
- Unmounted virtual anchor: allow at most one estimated-offset write, update the
  virtual range to mount the anchor, then let the resulting committed layout and
  measurement feed the same transaction's one final DOM correction. Do not retry
  estimates. If the row is absent from authoritative projection or cannot mount
  at that bounded continuation, cancel with a closed reason and release blockers.
- Pending measurements belonging to the layout transaction must be folded before
  finalization. A genuinely later independent media/row resize starts a fresh
  transaction through this same owner with the most recent stable free-scroll
  anchor. It must preserve the anchor, not silently update only the height model.
  Duplicate unchanged measurements start no work, and terminal records are never
  resurrected. This distinction preserves delayed-media behavior without leaving
  a second compensation path active.
- Key changes, same-key generation changes, explicit event/bottom jumps, live-edge
  transitions and unmount cancel pending stabilization and release its resources.
  Existing jump/live-edge product behavior remains unchanged; these intents own
  their deliberate movement and cannot be overwritten by an old correction.
- Programmatic scroll evidence includes key/generation/input revision and write
  generation (with transaction identity for stabilization), plus expected DOM
  position. Publish evidence before a write and retain it through transaction
  completion until a matching scroll observation consumes it, or newer input,
  logical invalidation or a replacing write generation clears/replaces it. Do not
  clear evidence when marking the transaction terminal. A pending genuine input
  always wins over matching geometry. Otherwise an observation is programmatic
  only when the current scoped evidence matches its resulting DOM position; an
  unmatched observation is genuine and advances input revision. Coalesced writes
  classify their latest resulting position, not an unbounded queue of signatures.
  Jump/live-edge writes use the same write-generation classification boundary,
  not a second loose signature.

### Native-resize before-paint settlement

Additional deterministic RED checks show that merely scheduling after a native
ResizeObserver notification leaves an 80px displacement visible until the next
frame, even when the eventual anchor is correct. In that native observer only,
use React DOM's existing `flushSync` API to commit geometry updates and run the
same prepared-layout continuation before the observer returns. This adds no
new compensation owner, timer or dependency. Mark the active transaction's
layout prepared; flush its queued/current mounted measurements through the
existing flush function with an explicit `layout` reason. That reason preserves
input activity and its existing idle/max-defer timers. Related prepared-layout
measurements commit immediately instead of re-deferring; a deferred prepend may
release for this necessary layout settlement while genuine input remains active.
Unchanged measurements produce no model update; fences and the one-final-write
budget remain mandatory. Nonmoving input and later genuinely independent resizes
retain their existing lifecycle. Tests assert DOM geometry before the observer
returns for virtual and nonvirtual lists, not merely after waiting more frames.

Flash before-paint review: first attempt timed out (no verdict); constrained
subsection re-review returned **Correct-to-merge** before implementation. Final
review must check unchanged-measurement convergence and preservation of current
input revision during the synchronous geometry commit.

Browser verification caught the native ResizeObserver same-depth delivery error
when the synchronous commit changes the observed list itself. Pause that observer
before the owned DOM commit, and register/re-register observation on the next
native animation frame. This frame only resumes the DOM listener: it does not
write scrollTop, retry settlement or share the user-input cancellation epoch.
The observing effect owns its frame handle and a disposed flag, cancels on cleanup
and key/generation changes, and the initial re-observation samples any size change
that occurred while paused. Initial registration also uses that frame so an
effect recreated during flushSync cannot re-enter the current observer delivery.
No new timer or timeout is added. A browser error-event probe must report zero
ResizeObserver loop errors, and duplicate unchanged observations must cause no
extra model commits or compensation. Re-observation compares current DOM geometry
with the committed measured-height cache and the last stabilized anchor; the
unchanged path stays attached and does not enter flushSync. Flash resource-detail
review returned **Correct-to-merge** before implementation.

### Stable-anchor continuity between transactions

The strict browser gate reproduced accumulated 1.125px drift after four prepend
batches, and a deterministic rounded-scrollTop test reproduces it with genuine
input between resizes. The inactive free-scroll cache must retain the logical
anchor offset AND its physical scrollTop baseline across terminal transactions.
Move the existing `freeScrollAnchorRef` cache into the same viewport controller
(no second cache). Actual user movement adjusts both active and stable targets;
owned writes advance physical baselines without counting their delta as input.
Fresh DOM anchor selection carries the residual rounding error instead of resetting
the reference to the rounded position. Key/generation/jump/live-edge/unmount
invalidation clears this cache. No new timer or product state is added. Before
replacing a transaction while input is pending, account for already-applied
physical movement before selecting the retained anchor. Add direct tests for
post-terminal input with rounded writes and keep the <=1px browser oracle.

Flash stable-cache document review: **Correct-to-merge** before implementation;
no source-code approval is inferred from that document-only gate. Owned writes
advance both physical baselines; input reconciliation always uses those updated
baselines. The migration removes `freeScrollAnchorRef` from the component rather
than retaining a second cache.

### Migration and diagnostics

Remove `pendingAnchorRef`, `anchorRestorePendingRef`,
`deferredPrependPendingRef`, `pendingHeightModelCommitRef`,
`heightCompensationRecordedVersionRef`, `pendingProjectionLayoutRef` and its
independent compensation frame/effect, and the old generic backfill restoration
branch. Replace their pagination blockers with owner state. Geometry caches,
committed row snapshots, independent explicit-jump helpers and activity timers
may remain, but must not retain independent free-scroll correction authority.
Remove obsolete types and diagnostics branches once migrated. Existing scroll
counters may remain for compatibility, fed from the sole write path.

Use diagnostic source `timeline.viewport_transaction` with local transaction
ordinal, phase/terminal kind and closed reasons (input, key, generation,
replacement, jump, live-edge, missing-anchor, unmount). No key hash, anchor row
identity, room/event/user IDs or content is emitted. Test diagnostics privacy,
but assert actual scroll writes and anchor geometry for correctness.

### Test placement and RED-first evidence

Add issue-specific component regression tests next to
`TimelineView.anchor-race.test.tsx` (a new `TimelineView.viewport-transaction.test.tsx`
is preferred over growing existing large test files), pure owner transition
checks alongside the new helper, and browser cases in a dedicated
`apps/desktop/e2e/timeline-viewport-transaction.spec.ts` reusing existing harness
helpers. Retain `apps/desktop/e2e/timeline-scrollback.spec.ts` #278/#520 cases and
`src/components/timeline/TimelineViewportScheduler.test.ts`.

First RED: queue a prepend while scrolling, deliver another wheel intent before
scroll/frame/idle settlement, then flush scheduler/timers and assert no stale
correction reclaims the user's position. Pair with no-input prepend plus delayed
measurements and a scrollTop setter trace proving one final correction. Add
virtual estimate/mount, consecutive same-generation pages and generation resets,
room/thread/root switch, jump/live-edge invalidation, nonvirtual resize and
unchanged duplicate-measurement tests. Explicitly prove that an estimated/final
programmatic write's own scroll echo does not advance the input revision or
cancel its current continuation, including an echo after terminal settlement,
while genuine input at the same geometry still invalidates it.
Browser assertions use actual first visible
stable row/offset with at most 1 CSS pixel rounding tolerance; preserve continuous
user input through more than one page, not only eventual after-idle geometry.

## Validation-blocker correction: Core QA pagination waiter

The unchanged Rust baseline's local `timeline` scenario failed on Tuwunel:
`paginate to EndReached: got Idle without first seeing Paginating`. SDK smoke
passed first. Source trace found `scenarios/search.rs::wait_for_paginate_end_reached`
consumes every backward pagination event for the key without request correlation,
and rejects the documented admission-rejected Idle while gap repair owns the actor
(`koushi-core/src/timeline/navigation.rs::handle_paginate`). This is a QA oracle
mismatch, not a renderer or SDK failure. Preserve the failing live-lane evidence.

Parent-owned, test-only correction (no product Rust/SDK changes):
- Match the exact current request id as well as key/direction; ignore initial,
  unsolicited and stale-request states.
- Model waiter phases as awaiting acceptance, paginating and awaiting gap release.
  Correlated Paginating establishes acceptance; correlated Idle after acceptance
  requests the next page. Correlated Idle without acceptance waits for the next
  key-matching `GapRepairReleased` and only then submits a replacement request.
  Do not retry on unrelated events, polling or sleeps. EndReached still requires
  the correlated accepted phase; matching failure stays failure.
- Use one absolute deadline for the whole waiter, including submissions and event
  consumption. Repeated unrelated events must not extend it.
- Add focused pure waiter-transition tests covering unrelated/stale request
  events, accepted pages, blocked admission/release, terminal/failure, and no
  duplicate submission. Rerun the exact failed both-server `timeline` lane and
  require existing success/cleanup tokens without changing its assertions.
- The same EndReached loop is duplicated in real-homeserver QA. Replace both
  loops with one QA-only shared module under `src/bin/common/pagination_waiter.rs`,
  imported by each existing wrapper with an explicit path. Existing wrappers
  retain their page sizes (headless 5, real 10) and duration inputs; no new crate
  or public production API. Shared tests run in both QA binary test targets.
  The existing real waiter also accepted EndReached without Paginating; shared
  behavior now requires correlated acceptance, as Core already promises. This
  strengthens the oracle rather than relaxing it. No real-account run is needed
  to prove this synthetic event-sequence correction.
- Files: both existing waiter modules plus the shared module and its sibling
  tests. Include the full QA diff in final Flash review. Correlated
  OperationFailed tests cover awaiting acceptance and awaiting gap release.
  Use monotonic `tokio::time::timeout_at` across reads and submissions.

Flash QA design round 1: Correct-to-merge; identified duplicate real QA loop.
Flash QA design round 2: **Correct-to-merge** for shared-module deduplication.
The recorded live-lane RED precedes all QA source changes. Parent implementation
may proceed; full QA diff and same-lane GREEN remain required.

### Additional QA observation boundary

The corrected local lane subsequently exposed `send flow msg1` with
`local_echo=false local_echo_send_state=Sent send_completed=true event_id=true`.
`SendFlowWaiter` reads only single-item ItemsUpdated diffs, ignoring Reset and
InitialItems even though those are authoritative timeline publications. Extend
its existing local-echo observer to inspect rows from both publication forms
through one item-slice method; still require an observed SDK Transaction identity
plus the exact client request/key/transaction SendCompleted. Never count a
remote Event-only Sent row as local-echo proof. Cover Reset followed by Set in one
batch, InitialItems, unrelated requests and Event-only non-proof with focused
synthetic tests. Error text must not echo transaction ids or expected bodies.
This is QA-only; no producer, SDK, send settlement or assertion relaxation.
Flash additional-observer design: **Correct-to-merge**. Before changing the
observer, focused synthetic tests reproduced three failures (Reset, InitialItems,
identifier-echoing error); the remote-only negative control passed.

## Verification plan

First establish RED through a deterministic headless production-path regression;
then implement and run the exact same test GREEN. Use scheduler-controlled
component tests for interleavings and real headless-browser geometry for visible
anchor preservation. No fixed sleeps or diagnostic-only correctness oracle.

Run focused checks before expanding to frontend Vitest, typecheck, lint, build,
secret scan, boundary guards and the complete Playwright DOM tier. Check npm
lockfile advisories before build. Run required local Rust/core/homeserver gates
and inspect CI on the final PR SHA before merge. Keep each investigative command
bounded; split long checks and inspect logs/descendants on timeout. Record exact
commands, outcomes and review verdicts below. No coverage threshold is currently
configured by the frontend CI workflow; behavioral acceptance is mandatory.

## Execution and review record

- User selected DeepSeek V4 Flash for read-only design and full-diff review.
- Luna implements only after an accepted design verdict; parent owns canon,
  shared-file integration, independent verification, PR and merge.
- Work is isolated from the existing dirty user branch in a dedicated worktree.
- Baseline: `npm --prefix apps/desktop ci --no-audit --no-fund` passed.
- Baseline: `npm --prefix apps/desktop run typecheck` passed.
- Baseline: `npm --prefix apps/desktop audit --package-lock-only --audit-level=high`
  passed with zero vulnerabilities.
- Consulted canon: repository rules; architecture overview (timeline viewport,
  generation and UI ownership); timeline relay state-machine contract; i18n;
  engineering rules; agent verification/environment notes; prior #520 plan.
- Baseline lint passed, including IME inventory, agent-doc and frontend owner guards.
- SDK exact gitlink initialized; `node scripts/check-sdk-submodule.mjs` passed.
- Local Rust baseline: `cargo test -p koushi-state --lib` (40 passed) and
  `cargo test -p koushi-state --test session_state` (82 passed, including current
  authentication/verification contracts). The policy's historical `koushi-auth`
  package command fails because that package is absent from the current workspace;
  do not claim it passed or add a dummy crate to satisfy a stale command.
- Core cold dependency compilation exceeded two bounded 60-second invocations;
  logs showed compilation, not test failure. The complete Core library gate then
  passed under an explicit 600-second hard deadline: 919 passed, 8 ignored.
- `cargo test -p koushi-state` passed: 778 tests across all integration/library
  targets plus doctest execution (no doctests present), no failures/ignored tests.
- Tauri/command snapshot, domain/leaf dependency boundary and tracked secret-scan
  gates passed before integration.
- CI comparison baseline: successful main run `33900982736` on base
  `0b373ef7403cec3e5d3825f1c81a7cb94dc78628`; Rust job 15m22s, browser 7m13s,
  frontend 1m17s, Tuwunel invitations 7m13s, Synapse invitations 7m51s.
- Baseline `npm --prefix apps/desktop test -- src/components/timeline`: 16 files,
  201 tests passed.
- Canon amendment review by GPT-5.6 Sol round 1: Not Approved (same-key generation
  invalidation and renderer projection ownership needed explicit wording).
- Canon amendment round 2: Approved after specifying renderer-side phases and
  key/generation/transaction-write/input-revision validation before every
  stabilization write. No implementation preceded this approval.
- Flash design round 1 timed out during source exploration: no verdict.
- Flash design round 2 required explicit echo-evidence lifetime, join/replacement
  boundary and stale-batch predicate. Updated the plan for all three findings;
  no implementation started.
- Flash design round 3: **Correct-to-merge for the design gate** after reading
  the amended plan and exact worktree canon. No remaining findings. Implementation
  authorization follows this verdict; full-diff review remains mandatory.

## Integrated verification and discoveries

- Parent took over the partial Luna implementation after two 900-second work
  deadlines. Incomplete checkpoints were never treated as GREEN.
- Flash approved the input-offset rebase amendment before coding it. The stable
  cache, before-paint commit and observer lifetime details have separate recorded
  document approvals above; these are not substitutes for the full-diff gate.
- Re-enacted RED against original base `0b373ef7403cec3e5d3825f1c81a7cb94dc78628`
  in a temporary detached worktree: the new 601-row test lost 10px of user motion
  and recorded three compensation writes. The integrated test preserves the
  exact offset with at most one estimate plus one final correction. Temporary
  baseline worktree removed; its shared dependency symlink was unlinked first.
- Additional deterministic RED→GREEN checks cover offscreen anchor rejection,
  programmatic-echo consumption/invalidation, identical-row generation changes,
  stale-generation prepend rejection, native resize before observer return,
  unchanged observation convergence, and rounded scrollTop with/without input.
- App browser oracle discovery: the old approximate index placement tracked a
  row ~1551px above a 422px viewport. The helper now positions its measured row
  and asserts viewport intersection before selecting the anchor. Its tolerance
  was not relaxed; all four App anchor cases passed afterward.
- Latest complete Vitest run: 106 files / 1246 tests passed with default test
  deadlines and four local workers. Typecheck, lint, lockfile audit (zero
  vulnerabilities), and production build passed. Earlier local diagnostic runs
  used longer per-test deadlines only while investigating host load; no checked-in
  deadline or assertion was loosened.
- Both-server local SDK + Core `timeline` lane passed after the QA observer fixes:
  Tuwunel and Synapse each reported `timeline=ok`, `timeline_nav=ok` and
  `restore_cleanup=ok`. The same lane had been RED before the QA changes.
- SDK library: 143 tests passed; Tauri library/DTO/IPC: 125 tests passed.
- Release interleaving RED→GREEN: idle release followed by new input before React
  commit must publish the held prepend. Deferral now follows the transaction's
  waiting-prepend phase, and the committed render cache is generation-scoped.
- Pagination chrome is included in the pre-commit layout signature. Its spinner
  had introduced an untracked 32px inset before prepend; chrome-only and combined
  chrome/projection changes now use the same before-paint continuation.
- Headless QA binary: 96 tests passed, including the new shared pagination and
  snapshot-send observer tests. The shared real-homeserver waiter focused tests
  passed without accessing a real account.
- Workspace formatter initially failed on three untouched baseline files. Only
  formatter-generated wrapping/trailing-comma changes were made there; original
  user worktree changes were preserved. `cargo fmt --all -- --check` then passed.
- The retired `koushi-auth` gate is a canon-maintenance proposal: replace its
  package reference with the current authentication/session targets. This change
  does not add a dummy package or claim the nonexistent command passed.

### Acceptance evidence map

| Requirement | Evidence |
| --- | --- |
| Single transaction, replacement and write budget | `TimelineViewportTransaction.test.ts`; bounded actual writes in `TimelineView.viewport-transaction.test.tsx` |
| User input before layout / rebase | mixed prepend + delayed-height component tests, four-batch native-wheel browser cases |
| Delayed sizes and before-paint restoration | native ResizeObserver-return tests for both list modes and combined virtual prepend/input; real-browser resize cases |
| Virtual estimate then mounted correction | existing anchor-race and mixed large-prepend #520 tests; new actual-write-budget tests |
| Consecutive pages / generations / stale input | four-batch browser cases; controller replacement tests; identical-structure generation and stale-batch component tests |
| Room/thread/root switch and jumps | retained key/reset/follow-up tests in `TimelineView.scrollback.test.tsx`, projection/jump tests in `TimelineView.threads.test.tsx`, exact key-kind fence in the controller |
| Programmatic classification and rounding | own-echo, synchronous write, unrelated-write, post-terminal baseline and rounded-input tests |
| Privacy and resource cleanup | identifier-free lifecycle tests; zero ResizeObserver loop browser probe; key/unmount cancellation and observer reattachment cleanup |
| Old owners removed | no pending prepend/projection/height compensation refs or restore helpers; stable cache moved inside controller; one stabilization write path |

### Deliverable review record

- DeepSeek V4 Flash (read-only, high) reviewed the full patch with SHA-256
  `452dc0f8498ad9403837ddb5dea74ed0290ce41d96d594440f956551ba3eb820`:
  **Correct-to-merge**, conditional on final broad gates. No blocking findings.
  Diagnostic/comment suggestions were non-blocking: jump variants intentionally
  share an invalidation category while retaining detailed write reasons;
  pending-row gauges describe current, not future work; unavailable anchors are
  deliberately discarded rather than restored unsafely.
- Subsequent browser RED exposed a test synchronization race: a cumulative
  measurement flush could refer to initial virtualization rather than the row
  resized by the test. Flash's bounded diagnosis recommended observing the
  actual grown row. The test now samples anchor geometry inside that native
  ResizeObserver callback, before paint, with the unchanged 2px tolerance.
- Flash reviewed this test-only delta: **Correct-to-merge**, conditional on the
  full browser tier. It cannot falsely pass by waiting for geometry convergence.
  A later full run passed the geometry assertion but found separately published
  diagnostics still at zero; only those secondary counters are now awaited
  after the already-captured before-paint geometry assertion.
- Local native rendering also stalled without any observer notifications. Use
  the existing documented software-rendering opt-in from `playwright.config.ts`:
  `KOUSHI_PLAYWRIGHT_EXTRA_ARGS='--disable-gpu --enable-unsafe-swiftshader'`.
  Browser configuration, pixel tolerances and checked-in deadlines are unchanged.

### Final integration verification

- Integrated upstream `55b99dfa` in merge `5061e72e`. Renderer/controller/QA
  implementation bytes were unchanged by that merge; new upstream Rust formatting
  required one additional braces-only change in the navigation adapter.
- Final narrow-delta Flash review returned **Correct-to-merge** in its bounded
  timeout checkpoint. Scope: post-geometry diagnostic publication wait and the
  inert formatting change, not a new whole-implementation approval.
- Complete browser tier: **289 passed**, 6.7 minutes, using the documented local
  software-rendering option. No retries and no relaxed geometry assertions.
- Complete Vitest: **1246 passed / 106 files**.
- Fresh Rust tests: Core **943 passed / 8 ignored**; SDK **143**; desktop/Tauri
  **127**; State package **786**; headless QA **96**; real-homeserver QA unit tests
  **17**. Compilation was completed separately after a bounded QA compile timeout;
  no timeout was counted as a passing test.
- Typecheck, lint, production build, rustfmt, secret scan, domain/Tauri boundary
  guards, Rust test-structure guard, SDK submodule guard and dependency audit
  passed after integration.
- The original worktree's four modified files remain untouched.

- Final post-integration local SDK + Core timeline lane: **Tuwunel and Synapse
  passed**, including `timeline=ok`, `timeline_nav=ok`, and `restore_cleanup=ok`.

GitHub PR, required CI and merge state remain authoritative for issue completion;
local verification alone is not a merge or issue-closure claim.
