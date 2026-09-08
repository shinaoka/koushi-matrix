# Operation read facts: predicate inventory for #840

Status: proposed payload derivation, not implementation approval. Companion to
[scoped publication](2026-09-06-scoped-publication-contract.md). This records the
actual synchronous predicate boundary after the expired-deadline regression;
it is not a new pending-operation registry.

## Exact predicate inputs

Source: `crates/koushi-core/src/runtime/request_outcome.rs`.

| Existing predicate/path | Required current facts; excluded bulk data |
| --- | --- |
| `snapshot_outcome`, `progress_generation_is_eligible` | Published generation; preserve the same-generation RoomSettingsLoaded exception. |
| `snapshot_account_key`, `session_is_login_transport_terminal` | Optional current account key, signed-out flag and login-transport-terminal flag, including Provisional/RecheckingTrust with failure. No credentials or unrelated session details. |
| `auth_discovery_matches` | Ready/Failed discovery homeserver; no login-flow payload needed by the predicate. |
| RoomSelected, `room_target_matches` | Active room ID. |
| RoomCreated/Joined/DirectMessageStarted, SpaceCreated, RoomLeft/Forgotten | Exact current room/Space ID membership. This is not merely the visible room window. Reuse shared indexed membership from the Rooms owner; do not rebuild or clone it on unrelated updates. |
| `focused_context_matches` and closed checks | Closed versus Opening/Open and room/event IDs. Preserve Opening acceptance. No focused timeline rows. |
| MainTimelineAnchor and live fallback | Active room ID, anchor event ID and focused-closed status. No row contents. |
| `staged_upload_ids_match` | Current Main/Thread composer identities and ordered staged IDs. Order and exact length matter. No file bytes, paths or previews. |
| ComposerAccepted | Target identity and the ComposerDraftStore room/thread revision for that target (`composer_draft_revision:2428–2438`), not merely a pane draft revision. Only current Main/Thread targets can satisfy the guard. Retain private-write updates even without advancing public generation. No document contents. |
| SubmissionAccepted progress fallback | Current active submission identity, target and transaction-ID membership. Reuse the submission owner's current index; no terminal history. |
| `room_operation_snapshot_matches` | Current settings room ID; selected Space ID and Space generation; membership above; invite completion below. No settings/device/member arrays. |
| `invite_batch_snapshot_matches` | Current completed invite request sequence and room ID; result user IDs and Room/Space destination identities with multiplicity. `invite_batch_matches:1675–1708` checks each requested user's matching destination count (1 or 2) and total result length. It does not inspect success/error status or compare the Room destination's ID. Preserve that exact predicate; no workflow/error-message copy or accidental tightening. |
| InviteWorkflow | Exact default/closed predicate plus current query room ID and query string. `query.room_id == None` alone is NOT equivalent to default workflow equality. |
| SearchStarted/Closed | Closed versus TooShort/Searching/Results/Failed, current request sequence, query and scope. No search result rows. |
| `EventProgress::event_outcome` | Most terminal variants need only current generation alongside already-correlated event fields. Preserve these existing event-terminal branches; do not silently replace them with stronger snapshot gates. OIDC URL/state and SavedSessions remain event-carried. |

“Not a request registry” does not prohibit the current canonical request sequence
used by search/invite predicates. Removing those identities would weaken matching.
No extra history keyed by waiter or request is permitted.

## Coherence and ownership

One latest immutable read root groups references to these existing owners' narrow
read models with a publication generation and a private-write wake revision. Root
replacement is atomic; a predicate cannot mix independently loaded owner revisions.
Changes are supplied by actual mutation owners, not a whole-AppState comparison,
action-name classifier or per-wake reconstruction. Unchanged membership/operation
indices are shared. Canonical reducers and existing product stores remain sole
mutation authorities. These read models must also serve applicable scoped product
publication rather than establishing a competing operation-only database.

Waiters retain their event progress locally. Arm wake delivery, synchronously load
current facts, evaluate, then check the absolute deadline. Reload/evaluate once on
terminal timeout, lag and closure. The current read root remains readable after
closure. There is no need to move the entire final AppState merely for settlement;
explicit full-state diagnostics require a separately documented lifetime boundary.

## Explicit return-payload migration

Replace embedded `VersionedAppStateSnapshot` in the 19 snapshot-bearing outcome
variants with committed generation and the relevant matched operation facts.
Retain existing event-carried IDs/revision/transaction/failure fields. Do not
introduce optional full-state output or a permanent compatibility mode.

Tauri's command settlement already serializes only generation. Most direct
consumers therefore need no wire change. Actual exceptions must be traced through
wrappers: `select_room_and_wait`, MediaStagingService's returned snapshots, room
operation helpers, auth helpers and submission responses. A direct pattern search
is not proof those transitive consumers use only generation.

Existing complete-state equality assertions must remain at explicit state
inspection boundaries, alongside new exact result-field assertions. Preserve the
expired-deadline success, same-generation private updates, account/target/request
fences, terminal-event semantics and shutdown/lag cases. The old full-state return
API intentionally changes; do not claim source compatibility.

## Remaining approval prerequisites

- Trace the transitive returned-snapshot consumers and specify exact result types.
- Identify mutation-owner update seams for each shared read index; no global scan.
- Review the resulting concrete payload/migration and amend canon before coding.
- Run the existing behavior suite against the migration and demonstrate bounded
  publication with unrelated profile/room growth. This inventory is not that proof.
