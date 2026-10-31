# Issue #1677 — WIP recovery checkpoint

Date: 2026-10-31. Branch: `chore/1677-sdk-upstream-main`.

**WIP, not merge-ready.** The maintainer explicitly requested this checkpoint
commit and branch push before further investigation. This is a narrow exception
to the previous no-publication-before-green instruction, not permission to open
a PR, merge, weaken gates, or change API/design decisions. This file supersedes
the older verification-status paragraphs in the feedback packet; those still
need refreshing before final review/publication.

## Exact dependency/history state

- Koushi base: `eea1cbe1cdb8b557ad6150acc15adf75525a6986`.
- SDK gitlink: `b25e9df403aa76425703b3fa7806530eacaec2f0`.
- SDK merge: `daeb92176671a2f14c090ce048a9997936abb867`; parents:
  old fork `401be3df774170499be743de26bdd88c1cb58f56` and upstream
  `d477953d60c7aaddfd5d10c181f9d47c661290cd`.
- Reconciled 486 upstream-only commits; initially 40 conflicted paths.
  The final SDK correction removes unused search `once_cell` and its lock entry.
- SDK commits are preserved locally in the maintained fork worktree. **This
  Koushi WIP push does not publish the SDK fork branch.** Until that dependency
  is published in the approved final chain, this is not a remotely reproducible
  final deliverable. Do not reset the SDK worktree or replace the gitlink with
  an intermediate pin. Assistant-generated SDK `.codegraph/` is not source and
  must never be committed.

## Approved implementation already present

- Automatic upstream Client event-cache lifecycle adopted; cache admission
  precedes invariant-only diagnostics. Required reads propagate failures;
  optional enrichment does not erase existing projections. Existing checkpoint,
  late-decryption, encrypted-search and persistence protections retained.
- Session-bound SDK password/OAuth reset-authentication helpers and required
  Ruma, search, timeline, fixture and QA API migrations are included.
- Existing canon (`docs/architecture/state-machine.md`, genuine-missing
  cross-signing identity exception, and #1049) is implemented using only the
  result of the existing #375 authoritative `request_user_identity` call.
  SDK trust and existing account-kind classification travel separately.
  Confirmed missing identity plus `Unknown` reaches mandatory Bootstrap only
  after the existing restricted-sync fence. No extra discovery query is added.
- Existing identity plus `Unknown` remains non-admitted. SDK `Unknown` is never
  rewritten to `Unverified`; Ready verification remains the SDK three-state
  mapping. All successful non-Verified inspections re-enter the existing gate.
- Existing stale ACK bug corrected: an older transition ACK cannot clear a
  newer pending Verified promotion. Same-transition wrong-flags handling and
  explicit-demand replay remain intact. The actor-level regression failed
  before the correlation correction and passes afterward.
- Exact-model independent pre-review completed for admission and its narrow
  ACK correction. Integrated implementation post-review is still outstanding.
  No new API/design decision is authorized by this checkpoint.

## Confirmed current diagnosis: Synapse native Sliding Sync

The current reproducible aggregate failure is **not** the earlier fresh-account
admission stall or offline restore ACK bug.

1. Disposable Synapse is pinned to `matrixdotorg/synapse:v1.157.0` by
   `scripts/lib/local-homeserver-qa.mjs`, with pin tests in the same directory.
2. Core aggregate reaches the room-people scenario, including leave, invitation
   and rejoin. Joined/invited candidate scopes and membership refresh pass.
   Sending the structured mention completes, but the receiver does not obtain
   the expected projected body.
3. A diagnostic run reproduced the failure with closed state tokens:
   `undecryptable_seen=false sync=reconnecting session=ready`.
4. The owned disposable homeserver log contains repeated `AssertionError`
   stacks through the native Sliding Sync request handler:

   ```text
   synapse/http/server.py:335                  _async_render_wrapper
   synapse/http/server.py:576                  _async_render
   synapse/rest/client/sync.py:862             on_POST
   synapse/handlers/sliding_sync/__init__.py:191 wait_for_sync_for_user
   synapse/handlers/sliding_sync/__init__.py:281 current_sync_for_user
   synapse/handlers/sliding_sync/room_lists.py:258 compute_interested_rooms
   synapse/handlers/sliding_sync/room_lists.py:678 _compute_interested_rooms_fallback
   synapse/handlers/sliding_sync/room_lists.py:1210 get_room_membership_for_user_at_to_token
       assert newly_left_room_for_user.event_id is None
   AssertionError
   ```

   The actual function is `get_room_membership_for_user_at_to_token`; the
   asserted variable is `newly_left_room_for_user`. The server's internal
   `_compute_interested_rooms_fallback` name concerns its room-list calculation;
   it is **not** permission to use the retired client legacy `/sync` fallback.
5. `room_people_projection` alone passes on Synapse. Full Synapse aggregate
   failed at the same structured-mention step in two runs; the second included
   the state/undecryptable diagnostic. This is not classified as infrastructure
   merely because the stack is in Synapse, nor as an SDK bug without further
   evidence. The exact preceding membership/token/table-readiness trigger is
   still to be isolated.

Do not publish raw child/server logs, message bodies, synthetic account/room
identifiers, credentials, or machine-local paths. This checkpoint deliberately
records only source locations, closed tokens, counts and public commit IDs.

## Rejected hypotheses and limitations

- A new admission policy was unnecessary: the genuine-missing exception was
  already canon. No Unknown-to-Unverified shim or admission relaxation is valid.
- The original eleven workspace failures are not still unresolved: seven
  fixtures needed an actual existing identity and an own-user device response
  entry, one runtime fixture needed real identity priming, and three genuine-
  missing authentication cases pass with the approved production correction.
  Their coordination, backoff, child-lifecycle and account/journal assertions
  were retained, not weakened.
- The stale ACK is a confirmed separate defect, not a speculative explanation:
  compiled actor RED then GREEN, followed by successful offline send-queue
  restore on both servers. It does not explain the remaining Synapse assertion.
- A suspected missing `Append` branch in the QA body waiter was rejected against
  the actual protocol: `TimelineDiff` has no `Append` variant; its item-bearing
  variants were already covered. No production patch was made for that guess.
- The latest Synapse timeout did not observe an undecryptable projected item.
  This does not prove that encryption is globally irrelevant; do not infer a
  payload-format defect or weaken the delivery assertion from that observation.
- An earlier Tuwunel aggregate timed out waiting for B3 Started/Running during
  multi-device E2EE. Focused `e2ee_trust` passed; after adding safe sync-wait
  diagnostics, a Tuwunel aggregate passed. Preserve the earlier failed attempt:
  there is no established causal fix for that intermittent timeout. A bounded
  read-only source investigation was dispatched; consume its completed report
  before treating it as evidence, and verify any finding against source.
- SDK-only both-server smoke and focused scenario passes do not substitute for
  full Core aggregate or required dedicated scenarios.

## Verification boundaries

- Eight Koushi-used SDK crates: initial 3,349 lib/integration tests (4 ignored)
  and 284 doctests (4 ignored), strict clippy/rustdoc/scoped fmt passed.
  SDK review's disabled-cache hydration correction subsequently passed 791 SDK
  unit and 488 integration tests; final dependency cleanup passed 50 search
  tests and 2 doctests. Unused FFI/labs/IndexedDB builds are not claimed.
- Admission checkpoint full feature-unified workspace passed **4,615 tests,
  16 ignored, zero failures**, with only the two standard spike exclusions.
  This precedes the later ACK correction and QA diagnostics; repeat affected
  final gates before declaring the finished tree verified.
- Admission checkpoint strict dev all-targets, QA-bin and release/stack clippy,
  three 512 KiB stack tests, QA-binary 133 tests and leaf wasm checks passed.
- After ACK correction, the focused trust-gate suite passes. Offline
  `send_queue` passes on **both** Tuwunel and Synapse.
- After diagnostic changes, QA-binary 133 tests pass; focused Synapse
  `room_people_projection` passes. Core Tuwunel aggregate passed before the
  latest body-wait diagnostic wiring; final relevant rerun is still required.
- Core Synapse aggregate remains **RED** as described above. Required dedicated
  `user_verification`, `redact_edit_convergence`, `search_crawler_catchup`,
  `timeline_reconnect` and required baseline completion are still pending.
- Earlier unchanged frontend checks and dependency audits passed. No claim of
  current integrated post-review, cross-platform native QA, or final CI success.

QA diagnostics now use existing closed-token summaries for sync-start waits;
body waits reuse the existing observer, use one absolute deadline, omit the
expected body from timeout text and report whether UTD was observed. These are
observability/oracle corrections, not evidence that the underlying sync fault
has been fixed.

## Next steps and remaining todos

### Todo #7 — in progress after this checkpoint

1. Inspect the pinned Synapse assertion and its preceding membership-delta
   construction, plus why the server selects its internal room-list fallback.
   Inspect actual source/installed version, not guessed upstream behavior.
2. Reduce the aggregate-only leave/invite/rejoin/token sequence to the existing
   local headless harness. Correlate safe server exception/status counts with
   Core reconnecting state; preserve both failed attempts. No blind retry,
   timeout inflation, real homeserver, legacy backend, or policy workaround.
3. Verify any pending B3 investigation findings independently. If resolving the
   server problem needs a server-version/policy or ownership-design decision
   outside the approved migration, park that part and report ranked options;
   do not silently patch shared services or add a concurrency mechanism.
4. Apply only an approved, source-supported minimal correction with behavioral
   RED/GREEN; otherwise report the concrete blocker. Finish Core aggregates
   and required dedicated scenarios on both disposable servers.

### Todo #3 — validation and final publication remain incomplete

- Rerun the affected feature-unified workspace and strict dev/QA/release/stack,
  stack behavior, wasm, formatting, structural and privacy gates on the final
  state, retaining failed/not-run boundaries.
- Refresh coherent whole-diff self-review, including every new test, and the
  upstream feedback packet's now-stale status section.
- Obtain complete integrated independent implementation post-review using
  `openai/gpt-6.1-sol`; the completed SDK-scoped review and pre-reviews are not
  substitutes. Resolve verified findings with relevant checks.
- Only after required gates/review are green, publish the finished SDK-fork then
  Koushi PR chain in English with `Refs #1677`, preserve history/required CI and
  approvals, and report exact pins/evidence/URLs. No PR is authorized now.
- Cold-room retirement and child teardown remain parked.

Resource discipline remains serialized Cargo/shared target, idle-selected
8 CPUs, jobs8, tests8, invocation-scoped `KACHE_EXPLAIN_MISS=1`. Do not delete
shared build targets or alter unrelated worktrees/services.
