# Issue #786 People panel reopen design

Status: **Correct-to-merge design** — `reviewer-flash`; four non-blocking precision notes are incorporated below. `reviewer-flash` selected (fast/low-cost, strong independent correctness review; GPT reviewer is slower/costlier and strongest for broad architecture).

## Root cause

A repeated `RoomSettingsLoaded` emits correlated event progress but produces no state delta because the cached settings are identical. `snapshot_outcome_for_progress` rejects the baseline-generation snapshot, so Tauri waits until an unrelated generation bump or 60-second timeout. Both People callbacks also await that command before opening the panel, allowing the parked continuation to override a later Threads request.

## Change

1. In request-outcome settlement, allow the current baseline snapshot only when the expectation operation is exactly `RoomSettingsLoaded`, correlated progress has already arrived, and its authoritative room-settings snapshot matches. Gate on the expectation operation—not the generic progress shape—and do not admit `RoomSettingUpdated`, mutations, or other room operations.
2. Preserve and document the actor invariant that the reliable settings snapshot reduction completes before `RoomSettingsLoaded` is emitted; baseline admission is safe only because the event cannot lead its snapshot.
3. Extract one `openPeoplePanel` renderer function for both duplicate entry points. Set the room scope and open People first, then await/reconcile `loadRoomSettings`; never set panel mode after the load. Keep room/settings request and navigation fences for returned data. Call it through `runInBackground` so rejection is contained by the existing transport boundary. Render cached settings when present; on first load leave the panel open in its existing pending/empty presentation, and never close it on transport rejection.
3. Preserve separate room/Space navigation epochs and Rust ownership of settings, active room, and command settlement. Add no retry or timeout increase.

## Verify first

- Rust RED: correlated `RoomSettingsLoaded` progress plus an already-matching baseline snapshot settles immediately; same-room `RoomSettingUpdated` and unrelated room operations at baseline do not. A bounded no-event case must still settle `Err(TimedOut)`.
- Browser RED: open People, close, gate the repeat settings response, click People then Threads, release the response/state update, and assert Threads remains open. Also assert People can reopen immediately before the gated load settles.
- Replace any source-text assertions with behavior. Run focused/full request-outcome, Tauri, Vitest, Playwright, typecheck, lint, build, boundaries, format/diff, and full GitHub CI.

## Canon

Document event-progress-assisted idempotent read settlement and panel-first People navigation in the state-machine/navigation ownership canon.

## Gate record

- Design review: `reviewer-flash` **Correct-to-merge**; four Minor precision notes incorporated before implementation.
- Verify-first evidence: deterministic browser test gates both repeat settings loads; People opens immediately and remains superseded after Threads when late loads release. Core gating test proves only `RoomSettingsLoaded`, not `RoomSettingUpdated`/other operations, admits baseline progress.
- Implementation review: `reviewer-flash` **Correct-to-merge** with two Minor findings; fixed by testing correlation plus generation eligibility (including wrong request and mutation cases) and making Threads explicitly retire any in-flight People open. Final re-review: **Correct-to-merge**, no remaining findings.
