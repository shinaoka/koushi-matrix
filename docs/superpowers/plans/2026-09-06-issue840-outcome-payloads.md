# #840: narrow settled outcome payloads

Status: implemented; parent audit, local gates and final review passed; CI pending.

Pre-implementation review: Sol high, read-only — **Correct-to-implement**, no
material findings. Reviewed this complete plan, the 19 outcome variants, waiter
ordering, current canon and Tauri settlement converter. Canon amended before
implementation; full scoped-publication design remains separately unapproved.

## Independently mergeable boundary

Remove the complete AppState payload from the 19 snapshot-bearing RequestOutcome
variants. Replace `snapshot: VersionedAppStateSnapshot` with `generation: u64`.
Retain each existing request ID, room/Space ID, composer revision, submission ID,
transaction ID, timeline key and failure kind. OIDC and SavedSessions variants
already carry their distinct event results and remain unchanged. No generic
settlement wrapper, optional snapshot flag or compatibility branch is added.

Methods returning only a settled snapshot (`select_room_and_wait`, staged-upload
mutation helpers, Tauri internal navigation/auth/room helpers) instead return the
published generation as `u64`, with explicit rustdoc. Prepared-upload send results
replace their snapshot field with generation. Tauri `command_settlement` accepts
that scalar; existing serialized settlement/submission DTOs are unchanged.

The waiter still evaluates its existing watched state synchronously, with exactly
the same predicate/event/deadline/lag/closure ordering. This step does NOT implement
the proposed read-facts publisher or remove the full-state watch. Its purpose is
to make removal possible without pretending narrow facts reproduce full AppState.
No command, admission, security or reducer semantics change.

## Verified consumer boundary

Production direct consumers in Tauri use returned snapshots for published
generation, including the submission tuple and prepared-send settlement. Core
MediaStagingService and select-room wrappers forward snapshots. Staging tests use
snapshot state for preparation, captions and target-isolation assertions; request
outcome tests also compare full snapshots. These assertions are not disposable.

Migrate full-state behavioral assertions to explicit current-state inspection,
and independently assert that the returned generation equals that inspected
snapshot's generation. Keep every existing state assertion and all correlation,
wrong-target, private-write, same-generation, timeout and shutdown cases. Preserve
the new real expired-deadline initial-success case. Do not claim old Rust return
API compatibility; the goal requires deliberate removal of that payload.

## Material implementation-boundary finding (correction approved)

Compiler-guided transitive migration found a production state consumer missed by
the initial direct-pattern inventory: `MediaStagingService::clear` uses the returned
snapshot's state for preparation-registry reconciliation (`media_staging.rs:634`).
That registry checks session account identity and all current UploadStagingStore
keys (`media_preparation.rs:572–612`), not merely the cleared target. Do not remove
that reconciliation or replace it with target-only clearing.

Proposed smallest correction: after the unchanged successful wait, inspect
`connection.snapshot()` explicitly and apply the same reconciliation policy,
returning the settlement generation separately. This moves the reconciliation read
later and may observe newer authoritative state; it does not guarantee a snapshot
from the settlement generation. Existing reconciliation already takes an async
registry lock after its captured state and is separately driven by the runtime's
watch lifecycle. Sol high read-only affected-boundary review: **Approve**, no
findings; correction then applied. Preserve account and other-target cleanup and
test the clear/target-isolation cases. No new callback, optional snapshot mode or
retained result-history API is proposed.

## Canon amendment before implementation

In architecture/overview.md's Core request outcomes section, retain the current
watch-based predicate authority/order; replace the stale Phase-A-only/Tauri-not-
migrated paragraph with the explicit narrowed result contract above and the
statement that full state is available only by separate inspection, not as the
settlement payload. In the staged-upload section replace “serialize the settled
snapshot” with “serialize the settled published generation”. This does not amend
the state machine or prematurely claim scoped publication is implemented.

## Proof and gates

Before implementation strengthen the real RoomSelected outcome test with an
exhaustive `RequestOutcome::RoomSelected { generation }` pattern (no `..`) and a
`u64` type assertion. This compile-RED proves the intended result shape cannot
retain an extra snapshot field or a renamed snapshot wrapper. Preserve full state
through explicit inspection and all existing behavioral assertions. Report this
as API-removal evidence, not a heap benchmark; Debug output length is not an
allocation metric. The separate 1,500-profile publication regression stays RED
until the actual publisher migration, and is not claimed by this step.

Run Core, testkit, Tauri, Core doctests, formatting, SDK/domain/Tauri/protocol/test-
structure/docs gates. Audit the entire diff then one read-only Flash final review.
Create a clean task worktree from main; exclude the independent SDK changes and
architectural RED tests from this PR, leaving both explicitly outstanding.

## Recorded evidence

- API RED: exhaustive RoomSelected pattern failed with E0026/E0027 before the
  enum migration (`/tmp/issue840-outcome-payload-red-resume.log`). SDK compilation
  setup was separate; the earlier setup timeout was not reported as a test RED.
- Core: 944 passed / 8 existing ignored; testkit: 225 passed; Tauri: 127 passed;
  Core doctests: 3 passed (including changed return/field examples); QA binaries:
  96 + 17 passed with `--features qa-bin --bins`.
- SDK/domain/Tauri/protocol boundaries, Rust test structure, agent docs, formatting
  and diff checks passed. No frontend source or wire type changed.
- Parent full-diff audit preserved all old state assertions using separate
  inspection plus generation equality. The real expired-deadline assertion passed
  both before and after migration. No heap/performance acceptance claim is made.
- Logs: `/tmp/issue840-outcome-{core,testkit,tauri,doc,qa}.log`.

Final read-only cross-model review: DeepSeek V4 Flash medium — **Correct-to-merge
subject to CI**, no Critical/Important findings. Complete reviewed patch SHA-256:
`a5c71fee6385e2b4291a090726b8ab21ace30767f2f1356bccbeb0d8e1fe4ce6`.
Two minor suggestions addressed: assert clear's returned generation in the existing
controlled test (PASS, 0.01 s), and rename two scalar test variables. No semantic
change or additional review round required.

This is not completion of #840. Routine watch clones, publication, scoped models,
resource demand and #839/#846 acceptance remain in the integration ledger.
