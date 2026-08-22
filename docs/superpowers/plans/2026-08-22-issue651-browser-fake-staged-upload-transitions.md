# Issue #651 Browser Fake Staged-upload Transition Cleanup

## Scope

Fix only outgoing main-composer staged-upload ownership in `BrowserFakeApi.selectRoom` and the existing `clearActiveRoomSelection` transition-to-none owner. No cap, helper, registry, decomposition, public API/DTO/class-field change, or draft behavior change.

Baseline `7b9210b4`: `browserFakeApi.ts` 5,985 lines / 202,403 bytes / SHA-256 `543435b3b4c00ae6481953eb2514fa5a542f07a31602599dc085e0ceb16f783b`.

## Contract

- Before changing main room A to distinct room B, clear A's `preparedUploadBytes` with existing `clearPreparedUploadBytes({ kind: "main", room_id: A })` and set the active timeline's `staged_uploads` to `[]`.
- Before clearing active room A to no room, perform the same outgoing-target byte cleanup; `clearActiveRoomSelection` already constructs empty staged metadata.
- Same-room reselection does not clear metadata or bytes.
- Invalid room selection remains a no-op because outgoing cleanup occurs only after the selected-room validity guard.
- Room removal continues using its existing target cleanup; duplicate fail-safe clearing through active-selection reset must not alter output.
- Thread-target cleanup, composer drafts/revisions/leases, submission IDs, scheduled sends, media gallery/downloads, batch limits, opaque IDs, and other-room state remain unchanged. Their pre-existing navigation projection behavior is outside this staged-upload-only fix.
- No aggregate byte cap. The fake intentionally discards outgoing staging because it has no Rust-equivalent per-room staging store; Rust restores per-room staging on re-selection. Adding a fake registry would be more complex than this bounded-fixture owner warrants, so this fidelity limitation is recorded rather than hidden.

## Verify first

Public APIs only; no private map/snapshot mutation.

1. RED cross-room: stage ready metadata/bytes in Alpha, select Planning, assert Planning has no Alpha staged metadata and Alpha's captured prepared variant preview is empty.
2. GREEN preservation: same-room Alpha reselection retains the exact staged metadata and prepared preview bytes.
3. RED transition-to-none: stage Alpha, select a Space with no remembered room, assert active timeline is empty and Alpha preview bytes are gone.
4. Room-removal interaction: stage Alpha, leave/forget Alpha, assert both metadata and preview bytes are gone.
5. Run exact focused tests at least three times.

The test obtains a real `variant_id` from the public staged-upload projection and queries it through public `preparedUploadPreview`.

## Implementation

Use one local `outgoingRoomId` in each existing transition owner and the existing clear method. Do not generalize target transitions or change `stageUploadBytes`.

## Gates

- `reviewer-flash` design verdict: `Correct-to-implement`; no blockers. The accepted A→B→A fake-vs-Rust staging divergence and unrelated scheduled/media projection asymmetry are explicitly recorded above.
- Public immutable-baseline RED/GREEN plus deterministic method/signature/field/export checks.
- `reviewer-flash` full-diff `Correct-to-merge`.
- Full local matrix, CI 7/7, latest-main confirmation, merge, #651/#551 evidence, cleanup.
