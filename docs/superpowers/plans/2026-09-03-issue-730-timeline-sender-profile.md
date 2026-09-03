# Issue #730 — open room sender Profile from timeline name

## Scope and acceptance

- In the main room timeline, a visible non-continuation sender name with stable `item.sender` is an accessible button labelled with existing `people.openProfile` (`Open profile for {name}`). Mouse, Enter, and Space open that exact Matrix user ID's room-scoped Profile.
- Duplicate display labels and local aliases never determine identity; callback payload is `(roomId, item.sender)`. The current user's own sender name opens the same room-scoped self Profile rather than being specially disabled.
- Items without stable sender ID and continuation metadata remain plain, non-focusable text whether the current density displays or visually collapses that metadata.
- The existing Profile view/actions are reused after exact room settings/members load. People is never rendered as an intermediate view; Back opens that room's People list and Close closes the panel.
- Rapid sender clicks and room navigation are latest-wins. Stale completion cannot open a profile for an old room/user.
- Message context menus, reply/thread/link activation, typography/presence/timestamp layout, virtualization identity, thread/focused/search/file surfaces, and avatar behavior are unchanged.

## Ownership and route

Rust owns timeline sender identity (`TimelineItem.sender`), room membership/settings, profile/alias labels, permissions, moderation availability, and command settlement. React may only forward the stable sender ID and own ephemeral panel-navigation demand.

Add one optional typed callback through:

`App.openRoomUserProfile -> TimelinePane -> TimelineView -> TimelineItemRow -> MessageMeta`

Only `TimelinePane` (the main room conversation surface, including its anchored jump-to-date state) supplies it. Thread/focused/search/files surfaces do not. `TimelinePane` stabilizes the callback with `useStableEvent`; `TimelineItemRow` binds current `roomId` plus `item.sender`; `MessageMeta` receives the already-bound intent and never infers identity from its display label.

## Profile opening and races

`openRoomUserProfile(roomId, userId)`:

1. reads `getAppStoreSnapshot()` (not render-lagging `snapshotRef`) and rejects immediately unless both authoritative active room and timeline room equal `roomId`;
2. captures `roomNavigationIntentEpochRef`, increments the existing `roomSettingsRequestRef`, and defines a fence over epoch/request plus active+timeline room read from `getAppStoreSnapshot()`;
3. clears only the renderer load marker and settles `loadRoomSettings(roomId)`;
4. requires the fence, returned active/timeline room, and `exactRoomSettingsForRoom`;
5. sets room People scope plus selected stable user, then opens `profile` through `setRightPanelModeClosingFocusedContext("profile", fence)`.

No `people` mode is written. Reuse this helper from `openDmUserInfo` after its authoritative `selectRoom`, eliminating duplicate profile-load navigation. Existing Back already clears selected user and opens People while retaining scope; existing Close clears the panel.

## Verify first

1. `MessageMeta`/row component tests: stable sender renders a button with exact accessible label; click emits stable ID once and stops propagation; Enter/Space use native button activation; missing sender/callback and continuation rows remain plain/non-focusable; display/presence/time structure remains.
2. `TimelineView.interactions.test.tsx`: a duplicate-label timeline emits exact `(roomId, sender user ID)` and does not fire reply/thread/context/link actions.
3. Browser/App integration with controlled `load_room_settings`: click a sender, prove no People heading appears while load is pending, release exact member/settings snapshot, prove Profile identity/actions, Back→room People, Close→closed. A second duplicate-label sender opens its distinct ID.
4. Browser race: gate two loads, click user A then user B and release out of order; only B opens. Navigate rooms before release; stale completion opens nothing.

Run the checks before production wiring and record RED, then unchanged GREEN.

## Styling and accessibility

Use a native button retaining `.sender` typography plus a narrowly scoped reset class and existing focus-visible token/style. Do not add positive `tabIndex`, role emulation, document handlers, or avatar activation. Stop click propagation at the sender control boundary.

## Canon and gates

Update `docs/agents/state-ownership.md` with the stable-ID/panel-demand rule. No Rust DTO/Tauri/i18n/generated mirror changes are needed because the DTO already carries sender ID and the accessible label already exists.

Run focused component/App/browser tests, full Vitest and relevant Playwright, typecheck, lint, build, format/diff, SDK/submodule, secret/boundary/docs checks, required Rust package tests, independent implementation review, GitHub CI, merge, Issue close, and main CI.

## Review record

- Design review Round 1: one Important and two Minor findings. The plan now uses the synchronously updated app store so DM delegation after `selectRoom` cannot read a render-lagging snapshot, stabilizes the callback in `TimelinePane`, and explicitly includes anchored main-room timelines.
- Design re-review: `reviewer-flash` **Correct-to-merge**. Its self-profile clarity note was incorporated; redundant returned/current snapshot checks remain intentional defense-in-depth.
- Verify-first evidence: the deterministic browser test timed out before wiring because no sender Profile button existed; unchanged test then passed. Focused MessageMeta/TimelineView tests passed 33/33; full Vitest passed 100 files/1218 tests; full room/space browser spec passed 25/25. Core passed 907/8 ignored, SDK 143, desktop 116, state 40; typecheck, lint, build, format/diff, SDK-submodule, and secret checks passed.
- Implementation review: `reviewer-flash` **Correct-to-merge** with two Minor test-hardening findings. Both were fixed: rows are proven visible before absence assertions, and the focus ring is checked after establishing keyboard modality plus `:focus-visible`. Final re-review: **Correct-to-merge**, no remaining findings.
