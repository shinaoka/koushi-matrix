# #1160: unified Home/Space toolbar and the scheduled-message right panel

Status: design for review. No implementation started. Baseline `origin/main`
`20c2cf61` (v0.20.0). Plan index: add this file to `docs/agents/plans.md` when
implementation starts, if that index still lists active plans.

## Scope

One deliverable, two surfaces that share the same header:

1. Replace Home's three vertical navigation rows (Activity, Explore, Invites)
   with one horizontal icon toolbar directly below the Home/Space name, using the
   same header structure in Home and Space.
2. Add the scheduled-message list that the toolbar's clock button opens in the
   right panel, scoped to the account (Home) or to one Space.

Non-goals: no change to scheduled-send creation/edit/dispatch/retry, no new SDK
call, no space-scoped Activity/Explore/Invites, no scheduled-send search or
filtering, no change to RoomInfo's existing per-room scheduled list, and not
#1159's date/time confirmation UX.

## Verified current state

- Home renders Activity/Explore/Invites as vertical `NavButton` rows under
  `accountHomeActive` in `apps/desktop/src/components/Shell.tsx`; their panes are
  `ActivityPane`, `ExplorePane`, `InvitesPane` in
  `apps/desktop/src/components/panes.tsx`, reached through `PrimaryView`.
- The Space header already has `workspace-header-actions no-wrap` containing
  `SpaceMembersNavButton`, Threads and Settings icon buttons. Home has no header
  actions, so the two headers are where the unification happens.
- The scoped-right-panel-list precedent is the thread list: scope
  `ThreadsListScope { Room, Home, Space }`
  (`crates/koushi-state/src/state/thread.rs:23`), state field
  `AppState.threads_list` (`crates/koushi-state/src/state/mod.rs:386`), snapshot
  slice `StateUpdate.threads_list`
  (`crates/koushi-protocol/src/state_update.rs:118`), commands
  `openThreadsList`/`paginateThreadsList`
  (`crates/koushi-protocol/src/command/app.rs`), reducer handlers in
  `crates/koushi-state/src/reducer/{mod,thread}.rs`, panel mode `"threads"` in
  `apps/desktop/src/domain/rightPanel.ts` dispatched from
  `apps/desktop/src/components/rightPanel.tsx`.
- Scheduled sends are already fully resident per account:
  `AppState.scheduled_sends: ScheduledSendStore`
  (`crates/koushi-state/src/state/timeline.rs:833`) holds
  `items: BTreeMap<scheduled_id, ScheduledSendItem>` with
  `ScheduledSendItem { scheduled_id, room_id, thread_root_event_id, body,
  send_at_ms, handle, is_dispatching }` (`.../timeline.rs:775`). Today only
  `items_for_room()` is projected, for RoomInfo
  (`crates/koushi-state/src/reducer/directory.rs:207`).
- Room membership in a Space already comes from Rust:
  `crates/koushi-state/src/state/space_children.rs` and
  `crates/koushi-state/src/sidebar.rs` (`space_rooms`, nested children, dedupe).

No SDK read, no crawler and no new actor is needed: everything the list shows is
already in `AppState`.

## Public contract changes

Rust owns the scope semantics; React renders the projected list.

- `ScheduledSendsScope { Home, Space { space_id } }`, new in `koushi-state`
  (`state/timeline.rs` or `state/thread.rs`-adjacent). Deliberately narrower than
  `ThreadsListScope`: RoomInfo already shows a room's own items, so no `Room`
  variant is needed, and because the list is derived synchronously in the reducer
  there is no wire round trip and therefore no `scope_key()`/`from_scope_key()`.
- `ScheduledSendsListState { Closed, Open { scope, items: Vec<ScheduledSendItem> } }`
  as a new `AppState` field `scheduled_sends_list`, plus the matching optional
  `StateUpdate` slice, following the `threads_list` shape exactly.
- Actions/commands: `AppAction::OpenScheduledSendsList { scope }` and
  `AppAction::CloseScheduledSendsList`, with the corresponding app command
  variant in `crates/koushi-protocol/src/command/app.rs` and the Tauri command
  registration. Reuse `ScheduledSendItem` as the entry type: React already has
  the room summaries it needs for destination labels, so no new entry DTO and no
  room-label copy are required.
- Ordering: `send_at_ms`, then `scheduled_id` (same tiebreak as
  `ScheduledSendStore::items_for_room`).

Derivation rules (Rust, single owner):

- `Home`: every item of this account's store, across rooms and DMs.
- `Space { space_id }`: only items whose `room_id` belongs to that Space under the
  existing sidebar/space-children membership semantics, including nested rooms,
  with a room reachable by several membership paths contributing each item once.
  Never infer membership from the currently active Space or from React state.
- The scope is pinned when the panel opens, matching the thread list. Switching
  Space or Account while the panel is open must not silently re-scope it;
  account switching closes it so another account's reservations are never shown.
- While open, the list is recomputed from `state.scheduled_sends` whenever a send
  is created, rescheduled, cancelled or dispatched, so the panel stays live.
- A `thread_root_event_id` marks the entry as a thread reply; the panel must not
  claim a relation it was not given.

## UI

- Home toolbar, left group: Activity, Explore, Invites (with the invite count
  badge, as today). Home toolbar, right group: Threads, Scheduled messages,
  Info/settings. Space toolbar, left group: Members. Space toolbar, right group:
  Threads, Scheduled messages, Info/settings. Shared actions keep the same order
  and right alignment in both. Activity, Explore and Invites stay Home-only.
- Activity stops using `Clock3`; use an activity/feed-style icon so the clock
  means scheduled messages only, including at creation. Keep icon-only buttons
  with localized tooltips, accessible names, keyboard activation, visible focus
  and selected state.
- Narrow widths: the row stays a single nowrap icon-only row and the workspace
  name truncates. Add a bounded overflow menu only if the documented minimum
  sidebar width cannot hold six icon buttons; do not wrap into several rows.
- The panel lists send date/time, body preview, destination room and whether the
  entry is a thread reply, sorted by send time, with an empty state and the
  existing panel close behavior. It is a right-panel mode, not a new main pane.

## Acceptance criteria (testable)

Rust (`crates/koushi-state`, plus `koushi-protocol` mirror tests):

1. Home scope returns all account items across rooms and DMs, ordered by
   `send_at_ms` then `scheduled_id`.
2. A Space scope includes an item in a nested child room and lists it exactly
   once when that room has several membership paths.
3. A Space scope excludes items whose room is outside the Space.
4. The open list reflects create/reschedule/cancel/dispatch without reopening.
5. Closing/account-switch/logout leaves no entry from another account and closes
   the list.
6. An entry with `thread_root_event_id` is marked as a thread reply; an entry
   without it is not.
7. Snapshot/DTO mirror and `frontend_app_state` golden are regenerated and the
   normal (non-`UPDATE_*`) run passes.

Frontend (vitest, DOM/aria assertions):

8. Home and Space toolbars render the documented order, labels, tooltips,
   accessible names, invite badge, keyboard activation and focus visibility.
9. Activity and Scheduled messages are distinguishable by accessible name, and
   the scheduled action opens the scheduled panel.
10. The panel renders date/time, preview, room label, thread flag, ordering,
    empty state and close, driven only by the Rust-projected list.

Not provable headlessly: light/dark contrast and true narrow-width layout. Those
are confirmed in the Linux GUI lane (`--server=tuwunel`) as visual evidence only,
never as correctness evidence.

## Contract surfaces to mirror

`crates/koushi-protocol/src/{state_update.rs,command/app.rs}` → Tauri
`apps/desktop/src-tauri/src/dto.rs` and command registration → `apps/desktop/src/domain/types.ts`
and the core-event/state-update handling → test fixtures
(`src/test/desktopApiFixture.ts`, `tauriIpcMock.ts`, `appHarnessMain.tsx`) →
`apps/desktop/src-tauri/tests/golden/frontend_app_state.json`. New product text
goes into the message catalog for English and Japanese; user-editable text rules
are not affected.

## Verification plan

Focused `cargo test -p koushi-state` (and `-p koushi-protocol` if the mirror
changes), `cargo fmt --check`, clippy `-D warnings` for changed crates;
`npm run typecheck`, focused `npx vitest run`, golden regenerate + normal run;
existing headless QA scenario only if one already covers the right panel (do not
register a new scenario name without its runner entry); Linux GUI lane for visual
confirmation. Read every gate's own exit status.

## Removals

Delete the three vertical Home `NavButton` rows once the toolbar carries the same
actions, and remove whatever Home-only wiring becomes unused. Do not keep both
entries.

## Open decisions for the reviewer

1. New `ScheduledSendsScope` (recommended) vs reusing `ThreadsListScope` for the
   scope input.
2. Pin the panel scope at open (recommended, matches Threads) vs follow the active
   Space.
3. Whether the panel needs an overflow menu at narrow widths at all, or the
   single nowrap icon row is sufficient.
