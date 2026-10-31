# Issue #1677 — WIP recovery checkpoint

## Latest audit resume — 2026-11-01 — single main probe, cutoff04:45 JST

The maintainer lifts STOP with a concrete tier-B experiment and extends the
hard cutoff to **04:45 JST**. Compare ONE fresh standalone Synapse
`room_management` on detached unchanged`origin/main` (Koushi
`ce25e243304f491780ce9e8e415b65a726c4fcbc`, its unchanged SDK gitlink
`292b89c4500cff6bdbcaac4770cff36a14714076`), using a new mainprobe worktree
and its own target. One release QA build, jobs8/tests8/taskset idle8/kache.
Branch comparison is production08b89c19 at source1ae6e673 with SDK1c669ef10;
its exact pre-target failure is preserved below. Do not alter either source,
predicate, oracle, timeout, server or pin. No blind retry. Branch source and
build artifacts stay untouched during the probe; boundary checkpoint pushes
remain required.

If main shows the same `space_access membership create failed:
PolicyNotVerified`, record the standalone path as existing/CI-uncovered, then
run exactly one corrected-branch both-server `all --core` mandatory gate.
Only GREEN permits affected mandatory reruns, whole-diff exact-GPT post-review
in bounded complete slices, ONE PR with pin/fix rationale/Refs#1677, normal
CI/auto-merge and authorized cleanup. If main is GREEN, inspect only SDK
create-event/sync-state/state lookup differences and make a canon-compatible
minimal proven correction; no authorization relaxation. If comparison is
inconclusive, save and stop. Never claim the supplied membership-timeout
observation proves this separate PolicyNotVerified baseline comparison.

Probe started in the new detached sibling worktree`../koushi-mainprobe` with
its own target and exact main/SDK SHAs above. Bounded command-only worker:
`sa_2fadb36f-4bd3-4aaa-b95d-ae003153a3cd` (`mainprobe_once`), maximum36min,
including one build (1200s) and only on success one fresh scenario (900s).
Private receipts: `/tmp/koushi-1677-mainprobe-build.{log,exit,json}` and
`/tmp/koushi-1677-mainprobe-focused.{log,exit,json}`. Await only own exits,
artifact paths and closed tokens; no parent bulk-log inspection. No probe
result is claimed yet; no additional Cargo run while this worker builds.

While it runs, main prepared **not launched** final post-review slices:
`/tmp/koushi-1677-final-post-review/manifest.json`, five source packets and two
worklog packets. Their source bytes cover all4,613 reviewed root diff lines
exactly once, unchanged from1ae6e673 except checkpoint. Run bounded exact-GPT
reviews only after the required gates pass; complete every assigned slice and
split a tool-limited slice further rather than count an incomplete report.
No production edit, gate rerun, or speculative design was added by preparation.

### Maintainer-supplied pB budget evidence (not rerun locally)

Source: maintainer-shared pB experiment,2026-11-01; raw log path and SHA not supplied.
Knob: `ROOM_LIST_EVENT_TIMEOUT`, reported default90s; membership wait extended to300s.
Reported observation: timeout at300s,exit1; Space still listed, both selected children absent.
Scope: supports a local-leave projection gap rather than insufficient timeout; not PolicyNotVerified baseline proof.

Latest clarification makes the single main focused probe **mandatory**; the
omission suggestion is not a decision. No main probe has run yet. Build and
scenario execution are delegated as one bounded command-only task; parent
receives only own exits, artifact paths and closed success/failure tokens,
not bulk logs (context81%). Mainprobe uses detached unchanged main and its
own target; the unchanged-source SDK smoke binary is prepared in the same
single Cargo build invocation because the existing runner needs both binary
paths. No branch executable/source is reused as main evidence. No further
timeout experiment or external-rejoin investigation.

## Latest STOP — 2026-11-01 03:13 JST — post-fix focused QA RED, no PR

Latest instruction: stop at a required RED, do not open a PR. **Stopped.**
Whole-diff self-review and affected release binary preparation completed, but
one newly authorized fresh Synapse `room_management` invocation returned
**exit1** before the Space-leave oracle. No further test/gate attempt follows.

- Command: `npm --prefix apps/desktop run qa:headless-local --
  --server=synapse --scenario=room_management --core --timeout-ms=600000
  --cargo-profile=release`, existing prebuilt-only launcher.
- Actual03:11:48–03:12:49 JST, selected CPU affinity`0-5,7-8`, jobs8/tests8,
  worker environment. Source1ae6e673/production08b89c19; only checkpoint
  commits followed. Fresh disposable fixture, single post-fix focused attempt
  **1/1**; no aggregate invocation after this correction.
- SDK smoke **PASS**. Core closed failure:
  **`space_access membership create failed: RoomOperationFailed { kind:
  PolicyNotVerified }`**. `space_topic=ok` present in retained Core stdout;
  no `space_access=ok`, `space_add_existing=ok`, or `space_leave_children=ok`.
  The runner intentionally omits failing child output; failure/tokens were
  recovered from the retained private child logs, not inferred from omission.
  This is a pre-target failure, not proof for or against the corrected
  Space-leave oracle. No new attribution or policy correction is attempted.
- Both executable SHA256/mtime identities were unchanged during QA; core
  SHA256`aef1db81b951…` is the newly built corrected binary, not the old one.
  Own exit/log/conditions and binary identities:
  `/tmp/koushi-1677-postfix-room-management.{exit,log,json}`. Private fixture:
  `.local-secrets/headless-local-qa/2026-10-31T18-11-48-584Z-synapse-room_management-EKbhTS/`.
  Raw server/child data remain ignored/private; no payloads or IDs copied here.
- **Step2 affected mandatory reruns NOT STARTED**, because ordered step1 QA
  was RED. Old successful gates and the two unit RED→GREENs remain scoped;
  no current integrated GREEN is claimed. No PR/CI/merge, new issue, pin,
  oracle, server, policy, paid infrastructure, production DB or cleanup change.
- The narrow exact-GPT corrective post-review hit its **24-tool-call limit**
  (persisted exit124) and returned an explicit **INCOMPLETE** report; the
  delayed completion clarified the limit, not a wall-clock timeout. It is
  **not a completed review** and is
  not publication evidence. No rerun after the required RED. Private report:
  `/tmp/koushi-1677-postfix-post-review.md`, persisted history alongside.
  The earlier completed integrated review still predates the correction.
- External-rejoin limitation remains exactly as previously accepted; not
  investigated. Preserve unmerged worktree and all targets. A future resume
  needs a bounded approved way to address/reach the pre-target authorization
  failure, not a blind retry, longer wait, weaker oracle or implicit SDK/policy
  change. Original04:26:24 budget was not exceeded.

## Latest resume — 2026-11-01 03:04 JST — self-review, post-fix QA, then affected gates

Resumed at clean/pushed1ae6e673 with context45%, original04:26:24 JST budget
unchanged. Latest maintainer instruction authorizes **ONE Koushi PR**, only
when every mandatory gate is GREEN, then CI confirmation/merge; it supersedes
older fork-PR-chain wording. No SDK fork PR operation/pin change/new issue.
Any required RED means no PR and stop/report; do not investigate the accepted
external-rejoin limitation. Paid resources, other publication, or destructive
production DB operations require stop/report.

Whole root diff self-reviewed: all4,613 lines excluding this checkpoint, plus
all957 checkpoint lines and connected local-leave dispatch/projection source.
No new source blocker in the audited scope. Private detailed record:
`/tmp/koushi-1677-postfix-self-review.md`. This does not claim a new review of
all upstream SDK commits or waive QA. Both original regressions already have
separate RED101→GREEN0 artifacts; no source change since08b89c19.

Only affected release core QA binary was prepared by a bounded build-only
worker while main reviewed; no concurrent Cargo run. Worker environment,
selected8 idle CPUs/taskset/jobs8/tests8, kache, unchanged source. Command:
`cargo build -j8 --release --locked -p koushi-qa --features qa-bin
--bin headless-core-qa`; own log/exit/conditions under
`/tmp/koushi-1677-postfix-qa-build*`. Build **exit0**,03:06:19–03:10:17 JST, CPU affinity`0-7`, jobs8/tests8.
Core QA executable changed from SHA256`58f190f986a4…` to`aef1db81b951…`;
SDK smoke executable hash/mtime unchanged. Exact before/after identities and
owned exit are retained in the build artifacts. Source is1ae6e673 (production
08b89c19); the subsequent root commit changed only this checkpoint.
Main runs one fresh Synapse room-management QA with the updated binary via
existing prebuilt-only launcher and before/after hash evidence. Only whole
scenario GREEN permits one final both-server aggregate; preserve existing
attempt history. Then affected mandatory checks and scoped post-review before
any single finished PR. All old source/gate results remain scoped, not new
post-fix evidence.

## Latest audit resume — 2026-11-01 — conservative correction authorized

The maintainer's tier-A audit explicitly lifts PARK: implement the existing
canon, without amendment or an absence-based membership inference. Accept known
Space IDs in `RoomLeftLocally`, remove the authoritative-list marker release,
and filter both rooms and Spaces through the unchanged pending set. Keep only
the existing successful `RoomJoinedLocally` release path, plus normal session
cleanup. No field/API/wire/dependency/oracle/pin/policy/new mechanism. The two
requested REDs remain the acceptance checks; their own prior exit101 logs are
preserved. Same tests GREEN, coherent self-review and affected local checks,
then the authorized fresh focused Synapse gate once, followed only on success
by one both-server aggregate and the conditional PR chain. Prior passing checks
are not repeated without an affecting change; required RED cannot be waived.
Original bounded-work deadline04:26:24 JST remains in force.

**Context-safety handoff:** maintainer reported87.1% and requires a hard90%
edit stop. Both exact regressions are now GREEN, saved one at a time. End this
session at this verified boundary rather than spend the remaining context on
integration. Resume in this same worktree/branch; no new design is needed.
Production correction is `08b89c19`: known-Space admission, deleted list-driven
release, and Space filtering only. No absence-driven release or new mechanism.
The audit scope was first pushed as54838a69, applied/unverified production was
immediately pushed as08b89c19, and the first GREEN was pushed as5ed75617.
No new QA fixture/rebuild/aggregate/PR has been started.

First RED→GREEN boundary: production source `08b89c19`,
`local_leave::space_local_leave_removes_space_immediately` **exit0,1 passed,
zero failed** (previously its own exit101 assertion failure). Worker env,
kache, CPUs`0-7`, jobs8/tests8. Own evidence:
`/tmp/koushi-1677-spaceleave-green-immediate.{log,exit,env}`.
This result was committed/pushed as5ed75617 before the second regression.

Second RED→GREEN boundary: same production `08b89c19`,
`local_leave::space_local_leave_survives_stale_authoritative_room_list`
**exit0,1 passed,zero failed**, previous own exit101 assertion failure.
Worker env, kache, CPUs`0-7`, jobs8/tests8. Own evidence:
`/tmp/koushi-1677-spaceleave-green-stale.{log,exit,env}`. Exactly the original
requested two tests remain tracked; no assertion/oracle was changed.

**Next session, in order:** read this latest section and governing canon;
self-review the coherent corrective delta (`git diff be61605b..HEAD`, including
new tests already added in23e167cb), run affected state-suite/workspace/lint/
stack checks with8 idle CPUs/jobs8/tests8, and obtain the narrow exact-GPT
post-review of this new corrective diff (the old integrated review predates it).
Prior successful unaffected frontend/SDK checks are recorded below, not new
post-correction results. Refresh only the affected release QA binary so it
contains08b89c19; old prepared binaries do not prove this fix. Then run exactly
one fresh Synapse `room_management --core --timeout-ms=600000
--cargo-profile=release` through the existing prebuilt-only launcher, recording
before/after executable hashes. Only if the entire scenario including Space
leave is GREEN, run one `--server=both --scenario=all` aggregate. Required RED
means save/report, no blind retry or assertion/deadline/server/pin/policy edit.
Only all mandatory gates GREEN permits the English base-main PR chain,
`Refs #1677`, CI/auto-merge and authorized unshared cleanup. No new issue or
protected-item operation. Preserve existing prepared binaries and targets.
Original04:26:24 JST deadline still applies unless the maintainer changes it.

**Known residual limitation (explicitly accepted out of scope):** conservative
projection suppression is not released by list presence or absence, nor by an
external rejoin on another device. In this session it remains until an admitted
successful local join clears it; account/session cleanup still retires its
state. This correction adds no external-membership ordering mechanism and does
not claim that external-rejoin visibility is fixed. Record the limitation in
handoff/PR reporting without creating an issue.

## Latest canon-gap stop — 2026-11-01 — requested PARK, two REDs saved

**Stopped before any production edit, per the maintainer's explicit canon-gap
condition.** The two requested deterministic no-server tests are committed and
pushed as `23e167cb`; both fail by assertion with their own exit101. This is a
RED reproduction checkpoint, not a verified fix or PR-ready state.

- The proposed simple retain inversion would release a local-leave marker when
  an authoritative snapshot omits its ID. That inference is not justified:
  `room/list_observer.rs:73-84` admits authority with an unknown room count and
  explicitly allows later responses to reveal more rooms. Its joined/invited
  projection at `:1523-1529` also omits several distinct memberships. Absence is
  therefore **not proof of an ordered membership-left observation**.
- `state-machine.md:1699-1721` instead requires left followed by joined/invited
  before external evidence clears leave state; local admitted join may clear
  immediately. No release-by-omission contract was found. A canon amendment or
  a demonstrated existing ordered-evidence path is needed before choosing the
  release rule. The guard/filter defects remain real but do not justify a
  partial fix that silently changes rejoin semantics.
- Exact-GPT pre-review identified this PARK finding (run
  `sa_4a43ada3-3943-4e4b-a0d6-743be82d718f`, exit0). Persisted final report:
  `/tmp/koushi-1677-spaceleave-pre-review-complete.md`; raw history alongside.
  The persisted text reaches Summary, permission verdict, limits and delete
  list; capture metadata flags truncation, so no broad complete-review or
  publication approval is claimed. Main independently verified the cited
  source/canon before parking; this is not an unsupported review allegation.
- Two additional exploratory boundary tests were run privately: absence-release
  RED and admitted-local-join GREEN. The unapproved absence/rejoin assumption
  was **not** retained in tracked tests; both temporary additions were removed,
  preserving exactly the two requested regressions. Private patch and raw
  boundary log remain under `/tmp/koushi-1677-spaceleave-*` for provenance.
- No production/SDK/pin/canon/policy/oracle change, QA release rebuild, new
  server fixture, post-fix scenario or aggregate was performed. All old evidence
  remains scoped as before; required aggregate RED remains blocking. No PR,
  hosted CI, auto-merge, cleanup, or protected-item operation was performed.
- **Human options:** (A) authorize a short canon design/approval for what ordered
  evidence releases projection suppression, then implement only that agreed
  correction and resume the gated sequence; or (B) authorize a bounded read-only
  search for an already-existing canon-backed ordered-membership path, keeping
  production and gates parked. Neither option is silently selected here.

## Active correction handoff — 2026-11-01 02:26 JST

The maintainer now authorizes **only the small existing-canon app correction
for Space local-leave suppression**, with deterministic no-server RED before
production edits, then the same tests GREEN. This supersedes the diagnosis-
only stop below; it does not waive any gate or authorize SDK/policy changes.
Starting head `bc364a0d`, SDK `1c669ef10`, both clean/pushed; context audit81%.
**First action: checkpoint/push this state before implementation.**

- Budget **02:26:24–04:26:24 JST**, no silent extension. At each boundary save
  checkpoint and push before proceeding; if context cannot support safe work,
  leave exact artifacts/next command and stop rather than lose state.
- **RED boundary verified 02:33 JST, production still unchanged.** Added
  `tests/navigation_state/local_leave.rs` to the existing state test suite.
  `local_leave::space_local_leave_removes_space_immediately` and
  `local_leave::space_local_leave_survives_stale_authoritative_room_list`
  each ran separately with **exit101, one assertion failure, zero compile
  errors**. Logs and own exits:
  `/tmp/koushi-1677-spaceleave-red-immediate.{log,exit}` and
  `/tmp/koushi-1677-spaceleave-red-stale.{log,exit}`. The second fixture
  isolates already-settled leave markers before an old authoritative payload,
  including the selected child; no server or altered production branch.
  Commands: `cargo test -j8 --profile ci --locked -p koushi-state
  --test navigation_state <exact-test-name> -- --exact --test-threads=8`.
  CPU affinity`8-11,13,15-17`, jobs8/tests8, kache; doctor verified8,886 valid
  entries, zero corruption/missing/checksum failures. No target deletion.
- The bounded exact-GPT pre-review returned **PARK for absence-based marker
  release**; main verified the source/canon gap described above. Proposal:
  `/tmp/koushi-1677-spaceleave-correction-plan.md`. No production edit made.
  This proposal is not an approved design or permission to continue.
- Only after both RED, change the existing local-leave early return and
  local-left retention/filter interaction. No new mechanism/public API/wire
  type/dependency/oracle. Consult state-machine/engineering canon; a missing
  design contract is parked, not silently written or guessed.
- Same units GREEN, then refresh only affected QA release binaries so the
  subsequent QA actually contains the fix (old binaries cannot prove it).
  Run fresh Synapse `room_management` **once** using existing prebuilt-only
  launcher, worker environment,8 idle CPUs/jobs8/tests8 and before/after hashes.
  If and only if the whole scenario including Space leave passes, run one
  final `--server=both --scenario=all` gate. This is the newly authorized
  post-correction gate, not permission to repeat the old unchanged RED until
  green. No retries/third post-correction aggregate.
- Only if required checks are green: one English Koushi PR into `main`,
  `Refs #1677`, explaining the existing fork pin and the canon correction;
  auto-merge/CI confirmation and merged unshared-target/worktree cleanup.
  No admin/force merge, force-push, new issue, protected issue/PR/pin change,
  or shared-target deletion. Preserve SDK fork PR34 and other worktrees.
- Current evidence remains: four dedicated migration scenarios PASS on both
  servers (8/8), all recorded static/workspace checks and integrated source
  review PASS; old aggregate RED is not erased. #5 actual remote memberships
  left while local Space/child0 remained; exact callback cause unproven.
  The two requested RED units are now verified and saved above. No production
  correction or post-correction GREEN is claimed.


## Latest remaining-gate resume — 2026-11-01 02:16 JST — dedicated8/8 PASS, overall RED / STOP

The maintainer resumed #2 remaining gates and authorized #4 publication **only
if all required gates are GREEN**; #5 stays diagnosis-only. Earlier no-third-
Synapse-aggregate/protected-item/shared-target constraints remain preserved.
No app/SDK/policy/oracle change, rebuild or additional aggregate was made.

- Source head `27e488ee`, SDK `1c669ef10`; same prepared executables, verified
  unchanged hash/mtime before and after each command. Idle-selected8CPUs,
  jobs8/tests8, existing private prebuilt-only PATH launcher and fresh fixtures.
- `npm --prefix apps/desktop run qa:headless-local -- --server=both
  --scenario=timeline_reconnect --core --timeout-ms=600000 --cargo-profile=release`
  **exit0 / PASS both**, CPUs`0-7`. Both checkpoint and gap-repaired tokens plus
  final timeline-reconnect token present on each server.
- Same command with `--scenario=user_verification`: **exit0 / PASS both**,
  CPUs`0-1,3-8`. Both servers reached incoming-request, SAS match, done and
  identity-verified tokens; source request/observer delivery requirement retained.
- Same command with `--scenario=redact_edit_convergence`: **exit0 / PASS both**,
  CPUs`0-7`. Both servers emitted `redact_edit_convergence=ok` and
  `thread_summary_convergence=ok`.
- Same command with `--scenario=search_crawler_catchup`: **exit0 / PASS both**,
  CPUs`0-7`. Both emitted `crawl_catchup_live=ok`, `crawl_catchup_restart=ok`,
  `search_crawler_catchup=ok`, including the required real startup-delay path.
- Actual JST execution windows: timeline **02:00:40–02:01:52**;
  verification **02:02:38–02:07:08**; redact/edit **02:08:30–02:09:40**;
  crawler **02:10:22–02:15:33**. Each gate has its own actual exit0; all four
  binary hash/mtime checks were unchanged, two fresh fixtures per command.
  No successful check was rerun and no negative result was discarded.
- Private logs/own exits/conditions/fixture paths/binary identities are saved
  under `/tmp/koushi-1677-remaining-<slug>.{log,exit,json}`, with slugs
  `timeline-reconnect`, `user-verification`, `redact-edit-convergence`, and
  `search-crawler-catchup`. Full synthetic server/child evidence is retained
  in the ignored per-server fixtures listed in each JSON; no raw payloads are
  copied to public records. The first two results were checkpoint-pushed as
  `f641554d`; the last two affect no source or dependency revision.
- **#2:** all four dedicated migration scenarios now pass on both servers,
  **8/8 cells**. Previous wording that these dedicated scenarios are pending
  is historical. Static/workspace/integrated source-review PASS remain valid
  in their recorded scope; these successes do not replace the aggregate.
- **#4: NOT STARTED / blocked by required RED.** The known Synapse Space-leave
  aggregate remains **exit1**, ordinal2/2, signature
  `space_leave_children memberships: timed out`. No correction has turned
  that oracle GREEN. No third aggregate or gate waiver was authorized/run.
  Thus not all required gates are green: no fork-to-Koushi PR, hosted CI,
  auto-merge/admin merge, branch deletion or post-merge target/worktree cleanup.
- **#5:** retain the read-only diagnosis below: remote selected memberships
  left, local Space/child0 still projected; concrete existing app local-leave
  handling gap, exact SDK-versus-app callback cause unconfirmed. No further
  investigation or speculative product/SDK/harness/policy change in this resume.

**Stop reason:** required aggregate remains RED despite the now-finished
missing dedicated evidence. Preserve this distinction; never mark #2/#4 done
or report final GREEN solely from8/8. Save/push this final checkpoint, retain
unmerged worktree/shared targets, and report the failed gate rather than open
an incomplete PR. No protected item, new issue, force-push or paid resource
was touched. Remaining technical work is a proven correction for the diagnosed
Space-leave failure and its relevant checks, not another blind aggregate run.


## Latest bounded read-only diagnosis — 2026-11-01 01:51 JST — RED, STOP

Maintainer scope: read-only diagnosis, no rebuild, then **one** same-fixture
predecessor-condition attempt; no third Synapse aggregate. Started **01:37:46
JST**, hard limit **02:17:46 JST**; diagnosis stopped **01:51:08 JST**, before
that limit, because the single allowed execution was consumed and further
causal attribution would need new evidence, not another blind attempt.
Source head `23de0f83`, SDK pin unchanged `1c669ef10`. Only checkpoint edited.

### 1. What the focused authorization failure means

- Opened the existing failed focused fixture's Synapse SQLite DB with `mode=ro`
  and `PRAGMA query_only=ON`; queried only synthetic role-labelled state.
  Attachment Space: v10, invite rule, A/B joined, A creator/power100, state
  threshold50. Allow Space: v10, invite rule, A/B joined, B creator/power100,
  A default power0. Child: invite rule, A joined. No explicit allow policies
  on these Spaces, as expected; requested restricted room does not exist.
- All **8** recorded server `POST createRoom` accesses returned **HTTP200**.
  Existing client failure is `RoomOperationFailed { kind: PolicyNotVerified }`.
  Its production source is `crates/koushi-core/src/room/operations.rs:411–426`,
  **before** the SDK create request: the single selected allow target failed
  `matrix_is_joined_verified_space`. This is not server power-level refusal
  or E2EE account verification. The canon does not require A to administer B's
  allow Space (`state-machine.md:4503–4516,4580–4589`).
- That helper requires an SDK room in Joined state and a positive unredacted
  **Sync** create-event proof of Space type from the state store
  (`crates/koushi-sdk/src/room_projection.rs:2191–2242`). The prior settings read
  does not establish that every such store predicate was positive at the
  subsequent check. Exact negative sub-branch (room lookup/state, missing or
  stripped event, deserialization) was **not recorded** by the old binary;
  do not invent it or relabel Unknown as permission. No gate relaxation.
- Safe state report: `/tmp/koushi-1677-policy-fixture-state.json`; read-only
  extractor `/tmp/koushi-1677-policy-fixture-state.py`. No IDs/credentials or
  raw event payloads were printed. Initial extractor key-name error was fixed
  before the successful query; no DB write was performed.

### 2. One existing same-fixture predecessor route, target reached, RED

The native registry offers no custom same-participant prefix selection. CSV
runner scenarios create fresh fixtures, so that would not reproduce shared
state. The unchanged `All` path is the existing selection that runs
`InvitesDm → RoomSpace → Directory → RoomManagement` on the same live
participants (`registry.rs:1187–1196`). Used it **once**, without changing the
harness, server version, SDK, oracle, timeout or stage order.

- Command: `npm --prefix apps/desktop run qa:headless-local --
  --server=synapse --scenario=all --core --timeout-ms=600000
  --cargo-profile=release`, via the same private prebuilt-only PATH exec launcher.
- Time **01:43:36–01:46:18 JST**. Idle-selected CPUs **`0-3,5-8`**,
  **jobs8/tests8**. Both prepared binary hashes and mtimes unchanged.
  Aggregate ordinal **2/2**, single predecessor-condition attempt **1/1**.
  Counter recorded in `/tmp/koushi-1677-bounded-resume.json`; no third run.
- Actual **exit1 / RED**: `space_leave_children memberships: timed out`.
  `PolicyNotVerified` absent; `space_access=ok` and `space_add_existing=ok` passed.
  Thus the target is reachable on the same binaries with this prefix; this
  single observation is not proof that ordering rather than timing caused it.
- Exact closed oracle row:
  `space_listed=true space_locally_left=false left_child0_joined=true
  left_child1_joined=false left_child0_locally_left=false
  left_child1_locally_left=true kept_child_joined=true outside_joined=true`.
  Compared with the prior aggregate, Space remains listed in both, but child0
  is now also listed; preserve that difference rather than claiming equal rows.
- Read-only server DB confirms current memberships and complete membership
  histories: Space, selected child0 and child1 each **join→leave**, while kept
  child and outside remain **join**. All five have **forgotten=false**. Hence
  the selected leaves succeeded remotely; this target failure is not a
  decline→forget operation or the historical migration assertion. Server log
  has zero AssertionError/Traceback. No claim about the SDK's live room state
  at the failed client projection is possible from server state alone.
- Private runner/exit: `/tmp/koushi-1677-prefix-condition-synapse.log/.exit`;
  conditions/tokens/binary identities: `/tmp/koushi-1677-prefix-condition-resume.json`;
  server-state report/extractor: `/tmp/koushi-1677-space-leave-server-state.json`
  and `/tmp/koushi-1677-space-leave-server-state.py`.
- Retained private fixture:
  `.local-secrets/headless-local-qa/2026-10-31T16-43-36-918Z-synapse-all-SecswX/`;
  `core-core-stdout.log`, `core-core-stderr.log`, `homeserver.log` stay there.

### 3. Narrow owner findings, limitations and next choices

- SDK projection separates Spaces from ordinary rooms (`room_projection.rs:
  2614–2634`). App local-leave reducer admits only IDs in `state.rooms`, not
  `state.spaces` (`crates/koushi-state/src/reducer/room.rs:272–275`); its ordinary
  room filter also does not filter Spaces. RoomActor does dispatch local leave
  after the manager ACK before emitting RoomLeft (`room/operations.rs:1215–1241`).
  This is a concrete existing app-side gap for immediate Space suppression.
- The same reducer clears an ordinary room's local-left marker when that ID
  appears in an authoritative list (`room.rs:326–335`). This can explain a
  reappearing child only if such a list arrived; the exact live callback order
  was **not captured**, so do not assert that causal sequence as observed.
- These owner files and the oracle are **unchanged versus main**. Main nightly
  41135492942's success remains valid, but cannot prove environment-only
  causality at8CPUs. A new app-code regression is not demonstrated either.
  A latent app gap exposed by new SDK/timing is plausible, not a tested fix.
  No app/SDK/policy/harness edit or speculative upstream feedback was made.
- **Recommended next choice (~1–2h, needs permission to compile/test):** drive
  deterministic reducer/actor REDs for Space local leave and stale list/rejoin
  ordering against the existing residency canon, then fix only proven app gaps.
  A Space-only patch cannot be claimed complete while child0 reappearance is
  unexplained. Do not weaken the gate or reopen unrelated SDK work.
- **Alternative (~1–2h):** obtain bounded identifier-free SDK room-state and
  list-authority receipt-order evidence, plus a valid matching main8CPU baseline
  if available, before selecting the owner. No server/pin/policy change is
  justified yet; if evidence requires one, park only that unit (escalation2/4).

No remaining gate, both-server aggregate, PR, auto-merge, hosted CI, new issue,
protected issue/PR/pin or cleanup deletion was performed. No third aggregate is
authorized. Keep the unmerged worktree and targets; push this checkpoint and
stop. Prior static/source-review PASS never converts this RED to GREEN.


## Latest one-level isolation — 2026-11-01 01:31 JST — BLOCKED, STOP

The maintainer authorized exactly one fresh Synapse `room_management` attempt
with existing release binaries, followed by a remaining Synapse aggregate only
if the targeted Space-leave failure did not reproduce. **The targeted oracle
was not reached**, so neither reproduction nor non-reproduction is established;
no aggregate follow-up, gate bypass or PR was justified or run.

- Source head: `5e5b27cf`; SDK unchanged at
  `1c669ef107eba11b395efa275ed4f615a94c7c3b`. No rebuild and no shared-target
  modification. Existing private prebuilt-only PATH launcher reused; both
  prepared executable hashes and mtimes stayed unchanged.
- Exact command: `npm --prefix apps/desktop run qa:headless-local --
  --server=synapse --scenario=room_management --core --timeout-ms=600000
  --cargo-profile=release`. `--scenario` follows the actual harness; fresh
  disposable fixture, not a replay. **One** scenario invocation, no retry.
- Time: **2026-11-01 01:27:41–01:28:43 JST**. Idle-selected CPUs
  **`0-1,5,7-9,12-13`**, **jobs8/tests8**, incremental off; existing runner
  deadlines unchanged, safety ceiling1500s.
- Actual **exit1 / RED before Space leave**. Closed failure label:
  **`space_access membership create failed`**, app failure type
  **`RoomOperationFailed`**, kind **`PolicyNotVerified`**. The last success
  tokens include `space_topic_basic=ok` and `space_topic=ok`. No Space-leave
  membership boolean line or `space_leave_children=ok` was emitted.
- This ordering is explicit in `scenarios/rooms.rs:965–967`: space-access,
  add-existing, then Space-leave, each fallible. The rejected restricted-room
  Create is awaited at `scenarios/rooms/space_access.rs:226`; the correlated
  `OperationFailed` path returns its label and typed failure through
  `event_wait.rs:120–124`. This is not a timeout at the target membership oracle.
  Do not weaken the authorization gate or classify this as a shared-state
  non-reproduction. No authentication/product correction was attempted.
- Server log: zero `AssertionError`, zero `Traceback`. That is not evidence of
  a GREEN scenario, and says nothing conclusive about the new policy rejection.
- Runner log: `/tmp/koushi-1677-room-management-synapse.log`;
  exit: `/tmp/koushi-1677-room-management-synapse.exit`;
  conditions/unchanged binary identities:
  `/tmp/koushi-1677-room-management-resume.json`.
- Private retained fixture:
  `.local-secrets/headless-local-qa/2026-10-31T16-27-41-911Z-synapse-room_management-UfowHp/`.
  Child logs: `core-core-stderr.log`, `core-core-stdout.log`; server:
  `homeserver.log`. Raw fixture/session data remain private and ignored.

### Comparison with main and the preceding aggregate

- Read-only download of main nightly **41135492942**, head
  `eea1cbe1cdb8b557ad6150acc15adf75525a6986`, confirms the reported main success
  token. Overall run failure is not absence of that stage's success. Main log
  has **no individual membership boolean line**: the helper emits it only on
  failure. Do not fabricate main's `locally_left` values from an `ok` token.
  Private downloaded run/log: `/tmp/koushi-1677-main-nightly-41135492942.json`
  and `/tmp/koushi-1677-main-nightly-41135492942.private.log`.
- The Space-leave helper is **identical** between that main head and this branch
  (empty `/tmp/koushi-1677-space-leave-main-vs-branch.diff`). Its settled oracle
  requires Space absent, both selected children absent from joined-room
  projection, and kept/outside rooms present; it does **not** require any
  `locally_left` boolean to be false.
- The **previous aggregate**, not this early-failing focused attempt, recorded:
  `space_listed=true space_locally_left=false left_child0_joined=false
  left_child1_joined=false left_child0_locally_left=true
  left_child1_locally_left=true kept_child_joined=true outside_joined=true`.
  Thus both children and both retained rooms satisfy that oracle. The observed
  mismatch is **Space remains listed**. `joined` in this diagnostic reads
  `AppState.rooms`, **not a server membership query**. No new evidence supports
  an SDK decline/forget-versus-app attribution for the Space itself.

**Stop / remaining decision:** recommended next is bounded read-only diagnosis
of the observed `PolicyNotVerified` rejection so an authorized test can reach
the Space-leave oracle, with no gate weakening, rebuild, SDK/policy/server
change or additional attempt until authorized. Alternatively keep the phase
parked with the existing evidence. A small canon correction cannot be claimed
RED→GREEN under the no-rebuild boundary; no such correction is demonstrated.
There is no evidence yet requiring an SDK/policy change or upstream feedback;
if established, park only that unit under escalation2/4. No new issue or
protected issue/PR/pin was touched. The remaining Synapse aggregate, both-server
follow-up, PR/auto-merge/CI and post-merge cleanup were **not run**. This
checkpoint is the only tracked edit; commit/push and stop, retaining the
unmerged worktree and targets. Previous integrated source review/static PASS
remain scoped evidence, not a waiver of these RED checks.


## Latest bounded audit — 2026-11-01 01:13 JST — RED, STOP

This section supersedes the old pending-readiness proposal and stop/resume
instructions below. Scope was the maintainer's bounded QA-only audit: no new
product/SDK/public API/server-version/policy change, no rebuild, maximum two
aggregate invocations, and the second only after a GREEN first invocation.

### Readiness permission and the one unsettled claim

- Read the requested **Confirmed current diagnosis** and the subsequent
  investigation/history. That diagnosis records the original pre-readiness
  Synapse v1.157.0 assertion: 56 native-sync HTTP500 responses while the three
  Sliding Sync migrations were pending. It is not the latest post-readiness
  failure. The original observed count is **56**, not the draft design's58.
- The completed exact `openai/gpt-6.1-sol` narrow pre-review is preserved in
  `/tmp/koushi-1677-synapse-readiness-permission-review.md`. Its judgment is:
  **“Permission PASS: enforcing the prerequisite fixes fixture readiness, not
  server behavior/client expectations.”** It required canon first, the existing
  helper/container, a read-only predicate, one deadline and fail-closed errors.
- The approved implementation, matching QA canon, deterministic RED/GREEN tests
  and integrated post-review are already in this branch. `waitForHomeserver`
  requires the versions response plus the same three pending-job names that
  the pinned dispatcher's own safety predicate checks; no fixed grace wait,
  increased deadline, DB write, scenario relaxation or backend selection.
  **No second implementation or production correction was made here.**
- This permission judgment does not prove every aggregate failure is caused by
  those migrations. Post-readiness failures remain separate unresolved facts.

### Exactly one fresh aggregate, prepared binaries only

- Source head before execution: `8a9a65fa`; SDK pin remains
  `1c669ef107eba11b395efa275ed4f615a94c7c3b`. No protected issue/PR/pin touched.
- Command: `npm --prefix apps/desktop run qa:headless-local --
  --server=synapse --core --timeout-ms=600000 --cargo-profile=release`.
- Execution: **2026-11-01 01:09:56–01:12:38 JST**, idle-selected CPU affinity
  **`0-6,8`**, **jobs8 / tests8**, incremental off. Outer safety deadline1500s;
  existing runner/startup/scenario deadlines unchanged.
- The existing runner normally invokes `cargo run`. A private temporary PATH
  executable accepted only its two exact release SDK/Core launch argv sequences
  and directly exec'd the existing prepared executables. All other invocations
  were refused (checked with exit64). It never delegates to Cargo/rustc. This
  changed no tracked source, QA oracle, fixture lifecycle or success token.
  Both binary hashes and mtimes were unchanged across execution: **no rebuild**.
- Attempt counter: **1 of maximum2**; second attempt **not run**, because the
  first is RED. No alternate scenario, extra reproduction or retry followed.
- Actual aggregate **exit1 / RED**. Core failure signature, confirmed exactly:
  **`Headless core QA failed: space_leave_children memberships: timed out`**.
  This matches the already-recorded 19:31 failure, rather than establishing a
  new cause. Last successes include `space_access=ok` and
  `space_add_existing=ok`. No success claim is made for Space leave/aggregate.
- This attempt's server log has **zero `AssertionError` and zero `Traceback`**.
  Therefore the observed remaining failure cannot be reported as another
  reproduction of the original migration assertion. The membership wait's
  underlying product/SDK/server cause is **unresolved**, not classified as
  infrastructure and not fixed by readiness. No assertions were weakened.
- Private runner log: `/tmp/koushi-1677-bounded-synapse-aggregate.log`;
  exit: `/tmp/koushi-1677-bounded-synapse-aggregate.exit`;
  conditions/binary identities/counter: `/tmp/koushi-1677-bounded-resume.json`.
- Retained disposable fixture:
  `.local-secrets/headless-local-qa/2026-10-31T16-09-57-252Z-synapse-all-ISBTNk/`.
  Exact child failure: `core-core-stderr.log`; progress: `core-core-stdout.log`;
  server: `homeserver.log` inside that directory. Raw logs and fixture
  credentials remain private/ignored; do not copy them to PRs/issues.

### Stop boundary / next action needs a new instruction

**No PR, no both-server follow-up, no CI/auto-merge, no new issue and no further
attempt.** Existing workspace/release/static gates and the completed integrated
source review remain scoped PASS; this does not waive the required RED QA.
The runner returned through its existing server cleanup; retained evidence and
this unmerged worktree are kept, no shared or local target directory deleted.
Only this checkpoint is committed/pushed. Remaining work is resolving the
existing Space-leave membership failure and completing required green QA before
a finished PR. Do not start it autonomously under this bounded instruction.


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

**Historical pre-readiness diagnosis, not the latest attempt.** This section
records the original server assertion, independently of the fresh-account
admission stall and offline restore ACK bug. The latest bounded post-readiness
attempt is recorded at the top: Space-leave membership timeout, no server
AssertionError/Traceback. Do not treat the pending proposal language farther
below as current implementation/review status.

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
