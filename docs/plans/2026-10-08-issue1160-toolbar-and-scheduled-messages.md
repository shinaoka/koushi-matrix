# #1160: unified Home/Space toolbar and the scheduled-message right panel

Status: design revision 4, after three independent pre-implementation reviews
(GPT-6.1 Sol), which found no outstanding blocker. No implementation started.
Baseline `origin/main` `20c2cf61` (v0.20.0). Revisions 2 and 3 correct factual claims from review round 1 and fold
in every remaining round-2 item; the section "What the reviews changed" records
them.

## Scope

One deliverable, two surfaces sharing the same header:

1. Replace Home's three vertical navigation rows (Activity, Explore, Invites)
   with one horizontal icon toolbar below the Home/Space name, using the same
   header structure in Home and Space.
2. Add the scheduled-message list that the toolbar's clock button opens in the
   right panel, scoped to the account (Home) or to the Space that was active when
   the panel opened.

Non-goals: no change to scheduled-send creation/edit/dispatch/retry policy, no
new SDK call or network load, no space-scoped Activity/Explore/Invites, no
scheduled-send search or filtering, no new scheduling actor, polling, timer or
dependency, and not #1159's date/time confirmation UX. The per-room list in the
main room pane keeps its current behavior.

## Verified current state

- Home already renders Threads and Settings icon buttons in
  `workspace-header-actions`; only `SpaceMembersNavButton` is Space-conditional
  (`apps/desktop/src/components/Shell.tsx`). The Home-specific part is the three
  vertical `NavButton` rows (Activity with `Clock3`, Explore, Invites with the
  invites count) under `accountHomeActive`. Activity/Explore/Invites panes are
  `ActivityPane`/`ExplorePane`/`InvitesPane` in
  `apps/desktop/src/components/panes.tsx` behind `PrimaryView`.
- A scheduled-message list component already exists and is rendered for the
  selected room in the main room pane: `ScheduledMessagesList` in
  `apps/desktop/src/components/mediaLists.tsx:234`, with
  `capability`/`items`/`onCancel`/`onReschedule` props, an inline edit form, the
  `scheduled.title` catalog entry, and `items.length === 0 → null`
  (`panes.tsx:1213`).
- Scheduled sends are fully resident per account: `AppState.scheduled_sends:
  ScheduledSendStore` (`crates/koushi-state/src/state/timeline.rs:833`) with
  `items: BTreeMap<scheduled_id, ScheduledSendItem>`, where `ScheduledSendItem`
  (`.../timeline.rs:775`) carries `room_id`, `thread_root_event_id`, `body`,
  `send_at_ms`, `handle`, `is_dispatching`. Only the selected-room projection is
  exposed today (`items_for_room()` into `TimelinePaneState.scheduled_sends`,
  `reducer/directory.rs:207`).
- The sidebar's Space membership is a Rust projection with two lanes plus DMs
  (`crates/koushi-state/src/sidebar.rs`): joined non-DM rooms come from the
  selected `SpaceSummary.child_room_ids` resolved against `state.rooms`
  (`sidebar.rs:326-336`), the not-joined lane comes from `SpaceChildrenState`
  (`sidebar.rs:363-389`), and DMs are scoped by `RoomSummary.dm_space_ids`
  (`sidebar.rs:391-401`). `SpaceSummary.child_room_ids` is projected from the
  direct `m.space.child` edges (`crates/koushi-sdk/src/room_projection.rs:2326-2343,
  3235-3267`) — the immediate children only, because the SDK request sets
  `max_depth = 1` (`vendor/matrix-rust-sdk` `spaces/room_list.rs:281-282`), so
  there is no local recursive membership to consume and none is claimed. The
  sidebar can also fall back to every non-DM room when the active Space cannot
  be resolved (`sidebar.rs:326-336`), which is a hazard for a captured scope.
- The scoped-right-panel precedent is the thread list: `ThreadsListScope
  { Room, Home, Space }` (`crates/koushi-state/src/state/thread.rs:23`), state
  `AppState.threads_list` (`crates/koushi-state/src/state/mod.rs:386`),
  transport slice `StateDeltaChangedSlices.threads_list`
  (`crates/koushi-protocol/src/state_update.rs:59,118`), panel mode `"threads"`
  in `apps/desktop/src/domain/rightPanel.ts` dispatched from
  `apps/desktop/src/components/rightPanel.tsx`. Its close points are
  CONDITIONAL: the reducer closes it on room selection/clearing
  (`reducer/mod.rs:2511-2580`) and on a successful directory join
  (`reducer/directory.rs:177-217`), while Home/Space selection reaches those
  helpers only when the restored room actually changes
  (`reducer/navigation.rs:299-378`).
- The IPC snapshot contract is versioned: `SNAPSHOT_SCHEMA_VERSION = 7` in
  `apps/desktop/src-tauri/src/dto.rs:723` and
  `apps/desktop/src/domain/types.ts:88`; `apps/desktop/src/App.tsx:1112` rejects
  a mismatch. Every app command carries a `RequestId` and returns
  `FrontendCommandAdmission` (`crates/koushi-protocol/src/command.rs:20-25`,
  `apps/desktop/src-tauri/src/commands/views.rs:209-237`).
- The minimum sidebar width is 260 px (`apps/desktop/src/App.tsx:398`).

## What the reviews changed

- Home is not missing header actions; only the vertical rows move, and Members
  stays Space-only.
- Reuse `ScheduledMessagesList`; do not write a second list component.
- The panel intentionally widens an existing privacy boundary, so it needs a
  canon amendment, Ready-only admission, and cleanup that covers the two
  session-retirement paths which bypass central cleanup.
- Core routing, command admission, `state_delta`, the five Tauri DTO touchpoints
  and a schema bump 7 → 8 belong in the contract checklist; the transport keeps
  its normal `RequestId` and admission receipt.
- The Space scope consumes the sidebar's existing membership for the Space that
  was active when the panel opened. There is no recursive traversal to build,
  because the local facts are immediate children of one Space only.
- Panel closure must be explicit per navigation transition and on panel
  replacement; "like Threads" is not a specification.
- The toolbar must be measured against real groups and badges, not six assumed
  squares: Home has six actions, Space has four, and Members is not a fixed
  32 px button.
- Reminders and evidence: the plan must record RED-before-fix evidence and the
  upstream UX comparison, and must not forbid synthetic bodies (only real-account
  data).

## Privacy and security boundary

Today the full queue is deliberately kept out of the WebView
(`docs/architecture/state-machine.md`: the full queue "is excluded from the full
webview snapshot because it can contain future message bodies for non-visible
rooms"; `TimelinePaneState.scheduled_sends` is the selected-room projection only;
`docs/agents/state-ownership.md`: "React may render only
`snapshot.state.timeline.scheduled_sends` for the selected room"). This panel
widens that on purpose.

Required, all part of this change:

- Amend both canon passages to permit exactly this explicitly opened scoped
  projection and to say its contents are body-bearing for non-visible rooms.
- Admit the open only in a Ready session (`crates/koushi-core/src/command_policy.rs`
  classifies the command like the other Ready-requiring ones).
- Clear the copied list in central session cleanup
  (`crates/koushi-state/src/reducer/mod.rs:2128-2145`, reached from
  `reducer/session.rs`) AND at the two transitions that reach a non-Ready
  session without it: `reducer/sync.rs` (`sync_failed_auth` → `Locked`) and
  `reducer/sliding_sync.rs` (unsupported revalidation → `CapabilityBlocked`).
  The smallest correct shape is a reducer-level invariant that closes the
  projection whenever the session is no longer Ready, rather than three
  independent call sites — but do not refactor unrelated teardown.
- Keep the bodies in the panel DTO only while it is open; no React cache; no
  logs, screenshots or fixtures with real-account bodies. Synthetic bodies are
  required for the golden and rendering tests and are allowed.

## Rust-owned contract

- `ScheduledSendsScope { Home, Space { space_id } }`, new in `koushi-state`.
  Narrower than `ThreadsListScope`: no `Room` variant, and no
  `scope_key()`/`from_scope_key()` because the projection is derived
  synchronously in the reducer with no asynchronous completion to correlate.
  `space_id` is the Space that was active when the panel opened.
- `ScheduledSendsListState { Closed, Open { scope, capability, items: Vec<ScheduledSendItem> } }`
  as a new `AppState` field `scheduled_sends_list` plus a matching
  `StateDeltaChangedSlices` entry. `capability` is derived from
  `state.scheduled_sends.capability` and refreshed on a capability-only change,
  because the renderer has no independent account-level capability today and the
  room pane's capability comes from a selected room's timeline state that is not
  refreshed when no room is selected. Reuse `ScheduledSendItem` as the entry
  type: the renderer already has room summaries for the destination label.
- One app command carrying the normal `RequestId`, turned into an open/close
  reducer action, routed from `crates/koushi-core/src/runtime.rs` the way the
  Threads commands are. No actor, no list correlation id, no pagination,
  Loading/Failed states or subscription.
- Ordering: `send_at_ms`, then `scheduled_id`, matching
  `ScheduledSendStore::items_for_room`.
- Derivation, single owner in Rust:
  - `Home`: every item of this account's store.
  - `Space { space_id }`: the items whose `room_id` is in the room set the
    sidebar computes for that Space — its joined rooms from
    `SpaceSummary.child_room_ids` resolved against `state.rooms`, plus the DMs the
    DM-space relation assigns to it, i.e. the sidebar's `space_rooms ∪
    global_dms` for that Space, including low-priority conversations and
    independent of collapsed sections or a filter box. Consume that projection;
    do not recompute membership differently, do not traverse nested Spaces, and
    do not infer membership from React state. Filtering the backing map's values
    already yields each reservation once; no separate deduplication pass.
    A room that is only a child of a child Space is NOT part of the parent's
    sidebar scope, and the panel must not claim it. If #1160 requires descendant
    rooms regardless of what the sidebar shows, that is a separate scope decision
    for the maintainer, not something this projection silently adds.
  - Recompute while open when `state.scheduled_sends` changes, so
    create/reschedule/cancel/dispatch are reflected without reopening. Add the
    change detection the new slice needs; note `state_delta.rs` currently does
    not change-detect the backing `scheduled_sends` store. Emit the scoped list
    only — never the backing queue.
- Scope lifecycle, per transition (the panel's displayed scope comes from its own
  DTO, never from `active_space_id`):
  - open: Ready session only; records the active Space, or Home;
  - room selection/clearing, Home/Space selection, and a successful directory
    join explicitly CLOSE the panel, including the Home-with-no-previous-room
    and Space-change-without-room-change cases where the Threads helpers are
    skipped;
  - account-tab selection must not clear another account's reservations: each
    account owns its runtime and only the selected tab is visible; a backgrounded
    tab keeps its state;
  - panel replacement (opening Home Settings, Threads, Room Info, a thread or
    another panel, through both the direct mode setters and the shared transition
    helper) closes it, and backgrounding a tab closes ONLY the projection — never
    the queue — through the existing before-account-switch path while the previous
    bound API is still selected (`App.tsx` `drainBeforeAccountSwitch`,
    `onRegisterBeforeAccountSwitch`), not by relying on a React remount. Returning
    to that tab starts with the panel closed and clicking the clock derives a
    fresh list from the preserved queue. If the runtime still reports `Open` while
    a newly attached renderer starts closed, close the Rust projection before
    treating the owner as attached rather than leaving two meanings of "open".
  - scope guard on every refresh of an `Open` Space projection: the captured
    Space must still exist and still be the active scope; otherwise close the
    projection BEFORE deriving membership. Without this, an automatic Space
    removal (`reducer/room.rs:281-299` clears `active_space_id`) or an automatic
    room clear (`room.rs:301-350`) would silently broaden a captured Space list
    to the sidebar's all-non-DM fallback.
  - session retirement and the two non-Ready transitions above close it.
- Invalid or unknown `space_id`: reject the open (or close an already-open
  panel) rather than listing every room; the Threads reducer already checks its
  scope against navigation (`reducer/thread.rs`) and the same guard applies.
- A `thread_root_event_id` marks a thread reply; the panel must not claim a
  relation it was not given.

## Transport and mirrors

Keep the ordinary command path: `RequestId` + `FrontendCommandAdmission` in the
public command (`crates/koushi-protocol/src/command/app.rs`), with no request id
inside `ScheduledSendsListState` or the reducer actions. Mirror list:

- Core: `crates/koushi-core/src/runtime.rs` (command → action routing),
  `command_policy.rs` (Ready-session admission), `state_delta.rs` (new slice,
  change detection).
- Protocol: `crates/koushi-protocol/src/{state_update.rs,command/app.rs}`.
- Tauri: `apps/desktop/src-tauri/src/dto.rs` (declaration, emptiness, delta
  conversion, full snapshot, snapshot conversion) and command registration.
- Snapshot schema: bump `SNAPSHOT_SCHEMA_VERSION` 7 → 8 in `dto.rs` and
  `apps/desktop/src/domain/types.ts`.
- Frontend: `apps/desktop/src/backend/{desktopApi.ts,client.ts}`,
  `apps/desktop/src/App.tsx` (open/close orchestration and the replacement
  hooks), `apps/desktop/src/domain/rightPanel.ts`,
  `apps/desktop/src/components/rightPanel.tsx`,
  `apps/desktop/src/components/{Shell.tsx,mediaLists.tsx}`. Apply the new slice
  through the existing `Partial<AppUiState>` handling
  (`domain/coreEvents.ts`, `domain/appStore.ts`) — no new frontend consumer.
- Fixtures/tests: `src/test/{desktopApiFixture.ts,tauriIpcMock.ts,appHarnessMain.tsx}`,
  the forwarded state-delta contract fixture
  (`apps/desktop/src-tauri/src/core_event_forwarder/tests.rs`), and
  `apps/desktop/src-tauri/tests/golden/frontend_app_state.json` with a POPULATED
  open list (not an empty default) plus a closed state and an omitted slice.
- Docs: `docs/architecture/state-machine.md`,
  `docs/agents/state-ownership.md`, the user guide, and `docs/agents/plans.md` if
  it lists active plans.

## UI

- Reuse `ScheduledMessagesList` unchanged for the selected-room pane. The panel
  renders the same component and supplies the extras outside it: an
  always-visible empty state (the component returns `null` for zero items, and
  that room-pane behavior must not change), and the destination room label plus
  the thread-reply flag as a small optional rendering addition that the room
  pane does not use. The panel keeps the component's cancel/reschedule controls
  and reuses the existing account-bound commands; if a read-only variant is
  chosen instead, do not render no-op controls. The capability shown for Home
  must come from the account's projected capability, not from a selected room's
  timeline state, because the backing store can be installed without refreshing
  the timeline when no room is selected.
- Toolbar: keep `workspace-header-actions`; add Scheduled messages (clock) beside
  Threads and Settings in the right group in both Home and Space; move
  Activity/Explore/Invites from the vertical rows into the left context group of
  the same row (Home only); Members is the Space's left context group and stays
  Space-only. Activity stops using `Clock3`; use an activity/feed icon so the
  clock means scheduled messages only.
- Layout is measured, not assumed: Home holds six actions, Space holds four, and
  Members has automatic width with counts and padding
  (`styles.css` `workspace-*` rules). Reduce toolbar-local spacing/button size so
  the real groups fit `MIN_SIDEBAR_WIDTH = 260` in one row with the name
  truncated, then prove it in the browser tier: one row, no sibling overlap, the
  left context group at the logical start and the Threads–Scheduled–Info group at
  the logical end with that intra-group order, the invite badge contained, and
  nonzero invite and member counts including the child-only suffix and a
  multi-digit count. No overflow menu unless that measurement fails.
- Home Info/settings keeps opening the existing `spaceInfo` all-rooms summary;
  do not invent account settings for that button.
- New product text goes into the message catalog for English and Japanese.

## Acceptance criteria

Rust (`koushi-state`, `koushi-protocol`, `koushi-core`):

1. Home scope returns all account items across rooms and DMs, ordered by
   `send_at_ms` then `scheduled_id`.
2. A Space scope returns exactly the items whose room the sidebar's membership
   for that Space contains (joined `child_room_ids` plus the assigned DMs),
   excludes rooms outside it, and lists each reservation once. A synthetic
   parent-Space → child-Space → room fixture must produce the same room set the
   real sidebar shows for the parent, including the not-shown case, so the test
   cannot pass from a pre-flattened fixture.
3. The open list reflects create/reschedule/cancel/dispatch without reopen, and a
   membership-only change while open (DM reassignment, a removed child edge, a
   duplicate path, an automatic Space removal, and an automatic room clear)
   updates or closes it per the scope guard.
4. Queue load and a mutation in a room that is not the selected room reach the
   panel through emitted Core state deltas, not only through reducer field
   assertions, including a capability-only change.
5. Opening outside a Ready session is rejected; invalid/unknown `space_id` is
   rejected or closes the panel; a duplicate close is a no-op. Sign-out/session
   retirement, `sync_failed_auth` and unsupported revalidation must close it AND
   the closed projection must reach the frontend through the delta → Tauri DTO →
   renderer path so rendered bodies are removed, not merely asserted in the
   reducer.
6. Selecting another Space/Home/a room, room clearing, a same-room selection, and
   a successful directory join close the panel per the stated lifecycle,
   including the no-previous-room and no-room-change cases where the Threads
   helpers are skipped; closure happens before the relevant early returns. Panel
   replacement closes it through the real `App.tsx` handlers (direct mode setters
   and the shared transition helper), and backgrounding then returning leaves the
   list closed with the queue preserved.
7. An entry with `thread_root_event_id` is marked as a thread reply; one without
   it is not.

Mirrors and transport:

8. `StateDeltaChangedSlices` entry, the five Tauri `dto.rs` touchpoints, the
   schema bump 7 → 8, the forwarded state-delta contract fixture, and the
   `frontend_app_state` golden are updated. The two contracts are asserted
   separately, because a full snapshot carries mandatory UI fields while a
   delta carries optional changed slices: a FULL snapshot test covers a populated
   `Open` list and an explicit `Closed`; a FORWARDED DELTA test covers an `Open`
   replacement, an explicit `Closed` replacement and the omission of the slice
   when it did not change. The normal (non-`UPDATE_*`) golden run passes.
9. Two accounts with distinct reservations and overlapping room ids: switching
   the account tab never shows the other account's items and never clears the
   other account's reservations; a delayed update from the previous tab cannot
   appear in the new one; switching back still shows them. Asserted through the
   account-bound command/renderer boundary, not the legacy
   `SwitchAccountRequested` reducer path alone.

Frontend (vitest, DOM/aria):

10. Home and Space toolbars render the documented order, labels, tooltips,
    accessible names, invite badge, keyboard activation and focus/selection
    state.
11. Activity and Scheduled messages are distinguishably named, and the scheduled
    action opens the scheduled panel.
12. The panel renders date/time, preview, room label, thread flag, ordering, empty
    state and close, driven only by the Rust-projected list, including a
    populated open list from a real state slice.

Browser-headless (Playwright): minimum-width Home and Space toolbars hold their
real action groups in one row with no sibling overlap, the invite badge
contained, the name truncated and keyboard focus reaching every action; Japanese
and long/pseudo-locale labels do not clip or overlap.

Linux GUI lane (`--server=tuwunel`): light/dark appearance confirmation only.

Evidence discipline: each behavior fix records the RED check first with its
command, exit status and captured failure, then the same check GREEN
(`docs/agents/verification.md`), and the plan records the upstream
client comparison (Element Web/X) that `REPOSITORY_RULES.md` requires for a new
shell interaction.

## Upstream comparison (recorded before implementation)

`REPOSITORY_RULES.md` requires inspecting the equivalent Element Web and Element X
flow before designing new user-visible Matrix functionality.

- **Scheduled messages.** Element Web exposes MSC4140 delayed events only as
  client methods (`_unstable_sendDelayedEvent`, `_unstable_sendScheduledDelayedEvent`,
  `_unstable_cancelScheduledDelayedEvent`, `_unstable_restartScheduledDelayedEvent`,
  surfaced through `apps/web/src/stores/widgets/ElementWidgetDriver.ts`). There is
  no scheduled-message list surface and no client API that enumerates an
  account's delayed events. Element X Android and iOS have no scheduled-message
  feature at all (their `scheduled` hits are WorkManager, `BGTaskScheduler` and
  test clocks). **Intentional divergence:** Koushi owns a persisted local
  reservation store, so it can enumerate and render them; consequently the list
  can only show reservations this install created, which is what the existing
  per-room list already does. This panel changes where that list is presented,
  not where the data comes from, and it adds no Matrix API.
- **Threads entry point.** Element Web renders the threads activity centre from
  the `SpacePanel` (the rail area beside user settings,
  `components/views/spaces/SpacePanel.tsx`), not from the room-list header.
  Koushi already keeps Threads in the room-list header, and this change keeps that
  divergence and adds the scheduled-messages button beside it. **Intentional
  divergence**, recorded here: the header is Koushi's single place for
  account- or Space-scoped list surfaces.
- **Header layout.** Element Web's room-list header carries the room-list filter
  and quick settings rather than a row of account-global actions; Activity,
  Explore and Invites are not a header toolbar upstream. Koushi already models
  them as Home-only entries in the sidebar, and this change keeps them Home-only
  while moving them from vertical rows into the header row. **Intentional
  divergence**, consistent with the existing desktop shell.

## Verification plan

The list below is ITERATION verification while implementing. Before the PR the
applicable repository-local landing gate is the feature-unified workspace suite,
the full lint gate, the frontend tests and
`qa:headless-local -- --server=both` (`docs/policies/engineering-rules.md`); a new
headless QA scenario name must not be invented for this projection, so the
landing gate uses the existing scenarios.

Focused `cargo test -p koushi-state`, `-p koushi-protocol`, `-p koushi-core`
(routing/admission), `cargo fmt --check`, clippy `-D warnings` for changed
crates; golden regenerate + normal run; `npm run typecheck`, focused
`npx vitest run`, the toolbar geometry Playwright spec from `apps/desktop`; the
Linux GUI lane for visual confirmation. Every gate's own exit status is read
(`cmd > /tmp/x.log 2>&1; echo EXIT=$?`), a filtered run that matches zero tests is
not a pass, and no new headless QA scenario name is invented.

## Removals (Delete list)

- The three vertical Home `NavButton` rows and their now-unused wiring once the
  toolbar carries the same actions — do not keep both.
- No generic scoped-list framework, no scope-key encoding, no pagination or
  Loading/Failed states, no subscription or request correlation.
- No second scheduled-list component, no new frontend store or map, no React
  timer, no capability copy outside the projected value.
- No recursive Space traversal, graph service, membership cache or extra network
  load; consume the sidebar's membership.
- No multi-state golden framework: keep the existing populated snapshot golden,
  and assert delta omission in focused serialization tests.
- No read-only variant of the panel: keep the existing account-bound
  cancel/reschedule controls.
- No sort/order abstraction; keep the existing comparator.
- No overflow menu before the minimum-width measurement.
- No new actor, polling, dependency or search/filter scaffolding.

## Open decisions for the reviewer

1. Session-cleanup shape: one reducer-level "close when not Ready" invariant
   (recommended) versus adding the close to the two bypass transitions
   individually.
2. SCOPE DECISION for the maintainer: rooms that exist only below a child Space
   are not in the parent Space's sidebar scope today (the SDK asks for immediate
   children only). The panel matches the sidebar. If #1160 means those rooms must
   appear under the parent regardless, say so before implementation, because it
   needs either a recursive hierarchy load or a different membership source —
   not a silent change in this projection.
