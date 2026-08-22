# Issue #650 Browser Fake Room-removal Cleanup

## Scope

Fix only the observable ownership defect in `BrowserFakeApi.removeRoomFromFakeSnapshot`. No decomposition, helper abstraction, public API/DTO/class-field change, or unrelated navigation redesign.

Baseline `3f8b255d`: `browserFakeApi.ts` 5,867 lines / 197,440 bytes / SHA-256 `275f1effba0ad9ba16c1183b5bb6c66629e8ad598e435780e112e26aef931e60`.

## Ownership contract

For an ordinary removed room, delete/filter only state keyed by or containing that room:

- delete `room_preferences.rooms[roomId]`, `link_preview_settings.room_overrides[roomId]`, `room_notification_settings[roomId]`, `room_interactions[roomId]`, `search_crawler.rooms[roomId]`, and `live_signals.rooms[roomId]`;
- clear `search_crawler.last_active` only when it names the room;
- retain mention targets, search results, activity rows, Threads items, and Files items whose `room_id !== roomId`;
- close room-scoped thread pane/top-level thread snapshot, Threads/Files/focused-context views naming the room; retain Home/account/other-Space views after filtering, including removing the room from a retained Files Space scope's `child_room_ids`;
- reset a pending activity mark-read to `{ kind: "idle" }` only when its room target names the room;
- if active, reuse `clearActiveRoomSelection()` for the canonical complete timeline/composer/staged-upload/thread reset, clear `navigation.main_timeline_anchor`, and close `currentRoom` search; retain the fake's existing explicit `active_room_id: null` behavior rather than adding Rust auto-selection semantics; if inactive, preserve the active room/timeline;
- preserve every other room, Space, global setting/profile/presence, Home projection, and opaque monotonic counter.

Existing composer draft/revision/prepared-byte cleanup and room-list/sidebar refresh stay in the same owner.

For a removed Space, retain ordinary child rooms, remove only that Space from `parent_space_ids` and `dm_space_ids`, clear its Space navigation memory, reset `space_members` with `emptyBrowserFakeSpaceMembersState()` when selected, and close removed-Space Threads/Files scopes. Do not treat a Space as an ordinary room or remove its children.

Deliberate exclusions: composer leases are renderer-owned and fail closed/release independently; `room_management` and `invite_workflow` match Rust retention on room-list removal; `currentSpace` search is not rebound by this path; native attention is a static zero-candidate fake fixture; `room_scroll_anchors` is never populated or mutated by `BrowserFakeApi` (the app harness owns its separate test-only setter), so this fake removal path has no room-anchor entry to prune; and no new active-room auto-selection is introduced.

Rust authoritative comparison: `handle_room_list_updated_with_crawler` retains room interactions and composer/scheduled/upload/media stores to joined room IDs and completely resets active timeline/thread/Threads state (`crates/koushi-state/src/reducer/room.rs:68-94,146-173`). Browser-only projections require equivalent key-based retention because they mirror the same ownership but have no actor event stream.

## Verify first

Use only public fake APIs to dirty Alpha and Planning projections: URL preview/notification preferences, pin interaction, mention candidates, crawler, read receipt/fully-read/typing live signals, search, activity and pending room mark-read, Threads, Files, focused context, composer and upload staging. No private snapshot mutation.

1. RED for both `leaveRoom` and `forgetRoom`: active Alpha disappears from `rooms` but its listed maps/views remain and timeline composer/upload metadata is stale.
2. RED inactive Planning removal: all Planning keys/items are removed while active Alpha timeline and Alpha-owned keys/items remain exact.
3. RED Space removal: child rooms remain, removed Space associations, selected `space_members`, and Space-scoped secondary views disappear, unrelated Space/Home data remains.
4. Pin active ordinary-room removal to the fake's existing postcondition `active_room_id: null`; this task does not add Rust's subsequent preferred-room selection.
5. Compare the complete post-removal projection, not only the room list. Re-run the same tests at least three times.

## Implementation evidence

- Immutable-production public RED: 4 cleanup cases failed and the unrelated-scope preservation case passed.
- Final focused GREEN: browser fake139/139 x3; browser fake139 + client25; typecheck/lint/source/diff checks green.
- Deterministic verifier: only `removeRoomFromFakeSnapshot` changed; 205 method signatures, 15 class fields, and exports exact; all required cleanup predicates present; tests use no private snapshot access.
- Post-implementation full-diff review: `reviewer-flash` `Correct-to-merge`; no blocking findings. The non-empty `dm_space_ids` coverage note is fixture-limited and the exact filter is statically verified without mutating immutable module fixtures. Full matrix pending.

## Implementation

Keep the fix inline in `removeRoomFromFakeSnapshot`. Reuse `{ kind: "closed" }`, `{ kind: "idle" }`, existing filtering idioms, `clearActiveRoomSelection()`, `retainNavigationRoomMemory(true)`, `refreshSidebar()`, and `refreshRoomListProjection()`. Do not introduce a generic cleanup framework or duplicate full DTO defaults.

## Gates

- `reviewer-flash-opencode-go` design verdict: `Correct-to-merge`; no blocking findings. The focused re-review verified the `room_scroll_anchors` exclusion, every listed state shape, Rust reducer parity, and verify-first scope.
- Focused RED/GREEN and deterministic source/API/resource checks.
- `reviewer-flash` full-diff `Correct-to-merge`.
- Full local matrix, CI 7/7, latest-main confirmation, merge, #650/#551 evidence, cleanup.
