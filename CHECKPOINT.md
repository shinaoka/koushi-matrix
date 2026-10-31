# Issue #1677 — WIP recovery checkpoint

## Final stop — 2026-11-01 01:00 JST, integrated source reviewed

**Stop for the maintainer's context-budget limit. No PR: required Synapse Core
aggregate is RED and dedicated migration QA remains unrun.** No gate was waived,
no new issue/scope/measurement was added, and no further aggregate retry ran.

- Tested/reviewed Koushi head: `c0054a718cb3f5f6170e0f7196a459fe5e67cae8`.
  SDK pin: `1c669ef107eba11b395efa275ed4f615a94c7c3b`. Both branch heads were
  pushed with ahead count zero. This final checkpoint-only commit follows them.
- SDK history now includes current-main receipt fix `292b89c45` as well as
  upstream `d477953d6`; the earlier intermediate dependency omission is resolved.
- **Combined-state final gates PASS**, each own exit status zero: feature-unified
  workspace `cargo test -j8 --profile ci --locked --workspace --exclude
  sidebar-composition --exclude key-management` (**4,615 passed, zero failed,
  16 ignored**); strict workspace all-targets clippy; strict QA-bin all-targets
  clippy; strict release workspace clippy with both stack lints; release
  `stack_budget` (**3 passed**); release QA `--features qa-bin --bins` build.
  Logs/exit files: private temporary `koushi-1677-combined-<gate>.log/.exit`
  (`workspace`, `clippy-dev`, `clippy-qa`, `clippy-release-stack`, `stack`,
  `qa-build`). Actual idle-picker CPUs **0-7**, jobs8/tests8, incremental off;
  kache used normally, no miss-diagnostic environment and no target deletion.
- **Integrated independent POST review COMPLETE**, exact model
  `openai/gpt-6.1-sol`, one review of this candidate. Reviewer read all 4,880
  branch-diff lines including new files, the entire receipt merge delta and
  connected source/canon. No new BLOCKER/IMPORTANT/MINOR code finding. Cache
  lifecycle/admission, separate missing identity/Unknown, generation/fence,
  ACK correlation, checkpoints, late decryption/search, auth continuation,
  receipt lock ordering, readiness/privacy and provenance passed source review.
  Historical six SDK-resolution packets were selectively traced, not all reread;
  their earlier completed review remains separately scoped evidence.
- Review delivery was truncated, **not** accepted on its visible prefix. The
  complete persisted history output was recovered through its final Summary,
  provenance, limits and remaining-risk sections, without a new review round.
  Full private report: `koushi-1677-integrated-post-review-complete.md` in
  temporary storage. It explicitly says source-acceptable, **not publication
  ready**, because Synapse QA is RED. No correction was requested or made.
- The preceding phase2 owner acknowledged exclusive handoff and no remaining
  owned gate PID. Its feedback edit was preserved and refreshed for the combined
  SDK pin; its frontend tests **2,362/2,362**, build, typecheck/lint/privacy,
  fixture/source guard tests and help/docs successes remain recorded. No
  affecting frontend edit followed. Prior leaf wasm/fmt/structural/privacy
  results below remain scoped evidence, not a claim of additional reruns here.
- The latest both-server aggregate **exit1** remains authoritative: Tuwunel Core
  aggregate and both SDK smoke lanes passed; Synapse Core stopped at structured
  mention receive (`undecryptable_seen=false sync=reconnecting session=ready`).
  Prior owner parsed 746 native-sync access statuses: 648 HTTP200 and 98 HTTP200!
  (disconnected responses), zero AssertionError/Traceback. Those observations do
  not establish a production cause. The earlier Space-leave failure is retained
  below. No blind rerun, timeout increase, assertion relaxation or server swap.
- Required dedicated `timeline_reconnect`, `user_verification`,
  `redact_edit_convergence`, `search_crawler_catchup` on both disposable servers
  are **not run on the combined state**. Do not replace this boundary with the
  aggregate/SDK-only successes. Native platform QA/hosted final CI are unclaimed.
- No #1677 SDK-update fork PR exists; the API listed only the separate existing
  receipt PR #34. No Koushi PR was opened. Preservation pushes are not final
  publication or permission to bypass required local QA.

**Next single action:** use the preserved failed Synapse attempt and existing
harness to choose a focused reproducible delivery/reconnection check before
any production fix or another aggregate. Finish the dedicated both-server
migration QA; only after required gates are green create the finished fork-to-
Koushi PR chain in English with `Refs #1677`. Keep existing source review unless
an actual scope/design change requires reopening it. Park only a concrete new
Tier-C decision with choices and a recommendation; no such new design decision
was made here. Do not repeat successful gates without an affecting change.


## Resume audit — 2026-11-01, receipt reconciliation

This resume supersedes the old session-only publication/QA restrictions below.
The maintainer requested preservation pushes, final integrated gates and review,
and the fork-to-Koushi PR chain only after green. No required gate is waived.

- Koushi `4ea3aff0` was pushed first; tracking ahead count verified zero.
- That merge explicitly retained an **intermediate** SDK pin. Current Koushi
  main's `292b89c4500cff6bdbcaac4770cff36a14714076` receipt correction was not
  its ancestor. Do not discard that fix or treat the intermediate as final.
- SDK `1c669ef107eba11b395efa275ed4f615a94c7c3b` now merges that correction into
  `b25e9df403aa76425703b3fa7806530eacaec2f0`, preserving both histories.
  Its branch `chore/1677-sdk-upstream-main` is pushed. The only textual conflict
  was the thread-root filter; preserve main's root exclusion. Automatic cache
  Option and test-user API adaptations retain all receipt assertions. No new
  API, schema, owner or concurrency design was introduced. The complete SDK
  thread-cache integration group passed **25/25**, none failed/ignored.
- Before that receipt merge, fresh final-state checks passed: feature-unified
  workspace **4,616 tests, 16 ignored, zero failed**; fmt; strict dev all-targets,
  QA all-targets and release with stack lints; 512 KiB stack tests; leaf wasm;
  QA binary tests; structural tests/guards; privacy scanner. These checks are
  not yet evidence for the new combined SDK revision; rerun affected gates.
- The preceding session's phase2 artifacts also record workspace/lint/stack,
  desktop typecheck/lint/vitest successes; **both-server Core aggregate exit1**.
  Tuwunel aggregate and both SDK smoke lanes passed; Synapse Core failed at the
  structured-mention receiver with `undecryptable_seen=false sync=reconnecting
  session=ready`, without the old server `AssertionError`. This newer failure
  does not erase the earlier Space-leave failure or establish a production cause.
  Do not rerun the same aggregate blindly.
- A separate still-live agent was discovered running phase2 gates and editing
  the feedback document in this same worktree after the old session ended.
  Its existing edits were preserved; exclusive handoff was requested before
  further shared-target builds or edits. No unrelated process was terminated.
- Whole first-party diff (all 4,227 lines including new tests), root lock diff,
  and the current-main receipt import were self-reviewed. Completed earlier SDK
  review remains scoped evidence, not the required integrated post-review.
- Next: consume the other agent's handoff/review results, refresh feedback to
  the new pin, run affected combined-state gates, complete the exact-model
  integrated post-review and dedicated both-server migration scenarios, then
  diagnose the recorded Synapse failure with a focused reproducer. No PR until
  every required gate/review is green; new Tier-C decisions are parked alone.


## Latest handoff — scoped resume, 2026-10-31 19:44 JST

This section supersedes the older stop/resume instructions below. The maintainer
resumed **todo #7 only**, not SDK pin PR/todo #3 or the cross-repository publication
chain. No new PR, issue, branch or publication is authorized; only this existing
branch's checkpoint commit/push is permitted. Do not run the known failing
Synapse lane again, and do not run aggregate QA in this scoped work.

- Re-read the confirmed 19:31 handoff first. `space_leave_children memberships:
  timed out` remains a known unresolved fact, with the exact private-log paths,
  CPU affinity and membership booleans preserved below. It is not admission
  evidence and has not been re-investigated or re-run here.
- **Removed** the temporary closed-enum Reconnecting stderr diagnostic from
  `crates/koushi-core/src/sync.rs`; no temporary diagnostic is left there.
  This restores its preceding control handling without changing transitions,
  actor ownership or any public interface.
- Canon consulted: `docs/architecture/state-machine.md:584–588`, beginning
  **“A genuine missing cross-signing identity may enter mandatory bootstrap.”**
  Existing identity without proof remains gated; no skip/reset/verify-later exit
  is introduced.
- The admission implementation and regression tests were already in the saved
  WIP. They were inspected rather than duplicated or redesigned: the existing
  `request_user_identity` result carries the separate identity classification,
  while the SDK subscriber's `Unknown` verdict remains unchanged. Only successful
  genuine absence, current generation and the existing restricted-sync fence
  permit the ordered bootstrap phase-entry batch. Existing identity, partial
  query failure and stale results cannot offer bootstrap. No second discovery
  query, retained guard, registry or admission relaxation was added.
- The existing compiled behavioral **RED** was re-read: the genuine-missing
  actor regression exhausted its30-second mandatory-bootstrap deadline before
  the fix. This is an assertion/deadline failure, not a compilation failure.
  It remains preserved as `koushi-1677-admission-red.log` in private temporary
  storage; no raw log content is copied into the repository.
- Fresh focused **GREEN** on this scoped tree: Core missing-identity actor
  tests **2/2**, SDK authoritative identity/trust recheck tests **2/2**, state
  missing-identity bootstrap reducer tests **2/2**, authentication audit
  integration tests **3/3**. **9 passed, zero failed/ignored.** The oracles cover
  unchanged SDK Unknown, exact query counts/no extra discovery, fence and stale
  generation handling, existing-identity/error rejection, and protection of
  bootstrap/key-confirmation/Ready states. No production behavior was rewritten
  merely to manufacture another RED.
- Commands were the existing `cargo test -j8 --profile ci --locked` focused
  targets: Core `--lib account::trust_gate::missing_identity_tests`; SDK
  `--lib current_device_trust_recheck_tests`; state
  `--test session_missing_identity_bootstrap`; testkit
  `--test issue_1105_authentication_audit`. Tests ran serially across packages,
  each with `--test-threads=8`, invocation-scoped `KACHE_EXPLAIN_MISS=1` and CPU
  affinity **`27-29,31-35`**, jobs8. These are mock/local focused Rust tests,
  not the prohibited Synapse or aggregate lane.
- Submodule guard and plain `cargo fmt --check` passed. Cache doctor verified
  **4,556 valid entries, zero corrupted/missing/checksum failures**, no repair.
- This is a scoped progress checkpoint, **not** a finished deliverable or new
  validation of the full workspace. Integrated post-review and broader gates
  remain unclaimed. SDK pin and todo #3 remain untouched; no PR/issue was opened.


## Latest handoff — maintainer audit, 2026-10-31 19:31 JST

**Stop here. The nonzero-exit reason is identified; do not rerun this lane.**
The maintainer requested committing/pushing the five current WIP files and
ending this session. SDK pin PR/todo #3, new investigations, PRs and issues
are deferred to the next session. No aggregate GREEN is claimed.

- One direct Core execution, without `scripts/desktop-headless-local-qa.mjs`,
  finished at 19:31:33 JST with exit status **1**.
- Confirmed failure: `Headless core QA failed: space_leave_children memberships: timed out`.
- Same prior Core CPU affinity: **`18-20,22-23,25-26,54`**; **jobs=8**.
  Existing QA environment variables were restored without printing credentials.
  The stopped disposable Synapse fixture was recreated with the same connection
  endpoint/account inputs. Prior failed Core stores were preserved before fresh
  stores were prepared at the same environment paths.
- Reproduction command, with that disposable QA environment already supplied:

  ```bash
  CARGO_BUILD_JOBS=8 KACHE_EXPLAIN_MISS=1 \
    taskset -c 18-20,22-23,25-26,54 \
    cargo run --quiet --release -p koushi-qa --features qa-bin --bin headless-core-qa \
    > /tmp/koushi-1677-direct-core-stdout.raw.log \
    2> /tmp/koushi-1677-direct-core-stderr.raw.log
  ```

- Raw stdout: `/tmp/koushi-1677-direct-core-stdout.raw.log`.
  Raw stderr: `/tmp/koushi-1677-direct-core-stderr.raw.log`.
  Both are private temporary files with mode0600. Do not copy their raw contents
  into shared logs, PRs or issues. The file paths above are recorded explicitly
  at the maintainer's request; no account values or home-directory paths appear.
- Closed membership observations:
  `space_listed=true`, `space_locally_left=false`,
  `left_child0_joined=false`, `left_child1_joined=false`,
  `left_child0_locally_left=true`, `left_child1_locally_left=true`.
  In particular, the child observations were **joined=false but locally_left=true**.
  `kept_child_joined=true` and `outside_joined=true` were also observed.
  These identify the failed QA condition; no unproven production root-cause
  attribution is added. The maintainer declared exit-reason identification done.
- Before the direct run, the own readiness diff was checked: its probe executes
  before SDK/Core startup, and its failure throws before the SDK success line.
  New JavaScript assertions exist in the test file only. The actual Core child,
  not a post-success readiness assertion, returned1 in the direct execution.
- The three-migration readiness correction has deterministic behavioral RED
  followed by **20 fixture tests PASS**, including real read-only SQLite probe
  cases, pending/error/deadline paths and unchanged Tuwunel/disabled-feature
  fixtures. A fresh corrected Synapse aggregate removed the original server
  assertion but still failed; a later run reached the Space leave case above.
  Thus the correction is **not** claimed to resolve Core aggregate.
- The first detached readiness pre-review was incomplete at its tool limit.
  A smaller completed exact-GPT permission/root-cause pre-review, using a live
  server-only same-request500-before/200-after migration proof, allowed enforcing
  the existing fixture prerequisite with canon first. Integrated post-review
  and final gates remain pending; neither review nor focused PASS replaces them.
- `crates/koushi-core/src/sync.rs` contains a **temporary closed-enum diagnostic**
  at the existing Reconnecting control, included solely in this requested WIP
  preservation. It changes no API/transition and must be removed or explicitly
  justified before the finished diff/review/PR; it is not a production fix.
- The earlier historical estimate of58 native HTTP500 responses was corrected
  to **56** by parsing the actual HTTP status field, not matching digits in
  timings or pagination tokens. Preserve the failed attempt and this correction.
- No further Core/Synapse `all` run is authorized in this session. No PR or issue
  was opened. Required validation and the SDK-fork→Koushi finished PR chain stay
  incomplete/parked. Existing admission/ACK corrections are not reverted.

The sections below preserve the preceding investigation history; where status
paragraphs disagree, this latest handoff takes precedence.

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

## Investigation after the pushed WIP (not yet a correction)

- The requested checkpoint was committed and pushed as
  `e91cc1264412d58eee6a0d6e4457c76f743b3c88`. This addendum records subsequent
  read-only investigation; it does not authorize another incomplete-state push.
- The pinned image selects its internal fallback precisely when
  `have_finished_sliding_sync_background_jobs()` is false. That predicate checks
  `sliding_sync_prefill_joined_rooms_to_recalculate_table_bg_update`,
  `sliding_sync_joined_rooms_bg_update`, and
  `sliding_sync_membership_snapshots_bg_update`.
- Its membership snapshot filters forgotten rooms. Its newly-left delta can
  still contain ordinary leave events with a non-null event ID; the absent-
  snapshot-room branch incorrectly assumes such a room must represent state
  reset and asserts the event ID is null.
- SDK `Room::leave` automatically forgets a room when declining an invitation.
  This is present at both old and current pins; do not remove it as an alleged
  new upstream regression. The aggregate executes invitation decline first.
- Read-only queries of the owned failed disposable database found one current
  leave membership with `forgotten=1`; current joined/invited memberships had
  `forgotten=0`. No identifiers were printed. Safe server-log sequence:
  successful forget +10.933 s, first assertion +11.007 s, 56 assertions through
  +40.104 s, 56 native-sync HTTP 500 responses; the three required migration
  jobs finish around +40.699 s (all background updates +62.931 s). These are
  timestamps from one diagnostic attempt, not a performance benchmark.
- A narrow proposal to make existing `waitForHomeserver` enforce the server's
  own three-job readiness predicate is under exact-GPT pre-review. No product,
  SDK, QA implementation, API, server-version or architecture change has been
  made for it. Review must reject mere failure masking or any new policy need.
- The attempted narrowed run initially timed out before Core while the SDK QA
  release binary was absent. Separate SDK and Core release prebuilds passed;
  that setup timeout is not behavioral RED. Owned descendants/containers were
  checked after timeout; none remained. Cache verification: 2,604 valid entries,
  zero corrupted/missing/checksum failures.
- With prepared binaries, `invites_dm,room_people_projection` passed, but the
  runner executes comma-separated scenarios in **separate fresh fixtures**.
  This did not reproduce their shared-state aggregate sequence and neither
  fixes nor invalidates the recorded aggregate failures. Do not cite it as
  aggregate GREEN or an exact reduced reproducer.
- The B3 source investigator completed its report. Mailbox delivery and
  event-before-projection ordering remain candidate discriminators only; no
  deterministic evidence establishing either cause was returned, and no patch
  is authorized merely by those hypotheses.
