# Timeline Unread Replay State Machine Design

## Goal

Fix issue #127 at the ownership boundary: opening a room must not depend on a
mounted `TimelineView` catching `InitialItems`, and Activity Unread must not
disagree with Home/sidebar unread counts when unread rooms have no observed
timeline row yet.

## Current Evidence

- `select_room` reaches Rust navigation and emits the normal timeline
  subscription effect for DMs. No DM-only subscribe drop was found.
- The runtime can replay `InitialItems` to an existing timeline actor, but the
  React timeline store is currently owned by `TimelineView`. A view that mounts
  after `InitialItems` was emitted depends on a fire-and-forget
  `ensureSubscribed` replay.
- `TimelineView` also owns the CoreEvent listener and filters events by the
  current key. Events for another key are counted as dropped, not retained for a
  future mounted view.
- `ActivityProjection` is seeded from `ActivityRowsObserved` emitted by room
  live timelines. RoomSummary unread counts are authoritative for Home/sidebar,
  but an unread room with no observed row is absent from Activity Unread.
- The latest `01c02c4` clears room unread counts after Activity mark-read, which
  aligns one symptom but does not make Activity Unread authoritative from
  `RoomSummary`.

## Approach Options

### Option A: Keep View-Local Store And Add More Replays

Ask the timeline actor for replay on every view mount, key change, resync, and
empty state. This is small but still depends on view lifecycle and cannot retain
events for keys that are not currently mounted. It also leaves Activity Unread
row membership coupled to observed timeline rows.

Rejected: it treats delivery timing as the problem instead of the owner split.

### Option B: App-Level Timeline Store Only

Move the `TimelineStoreState` and CoreEvent listener to `App.tsx`, then pass the
store into every `TimelineView`. This makes `InitialItems` durable across view
mounts and fixes the blank timeline race. It does not fully address Activity
Unread, because unread membership still depends on observed timeline rows.

Partial but insufficient for #127.

### Option C: App-Level Timeline Store Plus Typed Rust-Owned Activity Fallback

Move timeline CoreEvent application to one App-level listener/store and update
`ActivityProjection::snapshot` so `RoomSummary` unread/highlight state creates a
private-data-minimized room-level placeholder row when no unread observed row
exists for that room. `ActivityRow` gains a typed source discriminator so Rust
and React never infer placeholder semantics from an encoded event id. Activity
mark-read then clears event-backed rows through the existing event-id path and
room-level placeholders through a room-id path that never writes a synthetic
fully-read marker. Activity row open uses the same existing focused-context
navigation only when a real event id is present; room placeholder rows are not
clickable to an event until Rust resolves/backfills a first unread event in a
later slice.

Recommended for this issue: it fixes both roots while keeping Matrix semantics
Rust-owned and limiting the GUI change to projection-store ownership.

## Selected Design

Implement Option C.

### Rust/Core Ownership

`ActivityProjection::snapshot(state)` remains owned by `AppActor`, but its
Unread stream uses `RoomSummary` as the authoritative membership source:

- Low-priority rooms remain excluded.
- Observed event rows remain preferred. They keep event ids, sender labels,
  previews, timestamps, and highlight facts projected by Rust timeline actors.
- For each room where `room.unread_count > 0 || room.highlight_count > 0 ||
  room.marked_unread`, if no observed unread row survives marker/cleared-event
  filtering, core adds a room-level placeholder row.
- `ActivityRow` is typed, not string-sniffed:
  - event-backed row: `kind = Event`, `event_id = Some(Matrix event id)`.
  - room placeholder row: `kind = RoomUnread`, `event_id = None`.
- Placeholder rows are private-data-minimized:
  - `room_id`: room id, because existing Activity commands already carry it.
  - `room_label`: `RoomSummary.display_label`.
  - `sender_label`: `None`.
  - `preview`: `None`.
  - `timestamp_ms`: `RoomSummary.last_activity_ms`.
  - `unread`: true when unread or marked unread is present.
  - `highlight`: true when highlight count is present.
- Recent remains observed-row based for this slice. It should not invent message
  previews or event references.
- Row-level mark-read remains event-backed only because it requires an
  `up_to_event_id`.
- Mark-all-read handles both row kinds in Rust:
  - event rows produce the existing cleared event ids and fully-read marker
    updates using only real event ids.
  - room placeholder rows produce room-id clears through
    `RoomMarkedAsReadSucceeded` only; they never produce
    `FullyReadMarkerUpdated` and never enter `ActivityEvent::MarkedRead`
    `cleared_event_ids`.
  - the runtime explicitly enumerates placeholder room ids from the current
    Activity snapshot. It must not derive them from `cleared_event_ids`, because
    eventless placeholders have no event id to clear.
  - after the reducer clears the local room unread source, placeholder rows do
    not reappear in the next local Activity projection.

Placeholder mark-all is a local room-unread clear, not a Matrix fully-read
receipt. Without a resolved event id there is no durable SDK receipt to send, so
a later sync may reassert unread state until the deferred first-unread
resolution/backfill slice exists. This still satisfies #127 because Activity,
Home, and sidebar all read the same Rust-owned `RoomSummary` source and stay
consistent.

This makes Activity Unread and Home/sidebar badges agree at the source of
truth: unread room membership comes from `RoomSummary`, while richer per-event
rows are layered on when observed.

### React Timeline Store Ownership

Create an App-level timeline projection store around the existing
`TimelineStoreState` reducer:

- `App.tsx` owns one `TimelineStoreState`.
- One App-level CoreEvent listener applies all timeline reducer events to that
  store.
- `TimelineView` receives the store from the App-level context or explicit test
  props instead of owning the reducer state.
- `TimelineView` still keeps a lightweight CoreEvent listener for view-local
  side effects that are not product state: message-source dialogs, navigation
  snapshots, anchor completion, diagnostics, and avatar/account presentation
  events. When an App-level store is present, this listener must not apply
  timeline reducer mutations.
- `TimelineView` still owns presentation-only state: scroll anchors, DOM
  measurements, context menus, avatar download request throttling, and media
  preview request bookkeeping.
- `TimelineView` may still call `ensureSubscribed(key)` after mount/key change,
  but this becomes a freshness/replay request rather than the only way to catch
  `InitialItems`.
- Key mismatches are no longer dropped by the reducer owner. Events for any key
  are applied by the App-level listener and become visible when a matching view
  mounts; view-local listeners may still ignore mismatched keys for their local
  side effects.

The store remains in memory only. It stores visible timeline DTOs already sent
to the WebView, is cleared on `ResyncMarker` using the existing
`applyGlobalResync` path, and is bounded:

- Retain active selected room, open focused context, open thread, and at most
  eight recently seen keys.
- Evict older inactive keys by LRU after applying a timeline event.
- Clear all keys on logout/account switch/session clear through the existing
  snapshot reset path.

This avoids promoting decrypted visible timeline DTOs into an unbounded
secondary store.

### Activity GUI Contract

The GUI renders Rust-shaped Activity rows without deriving unread membership:

- Event-backed rows keep the current open-row behavior.
- Placeholder rows with `kind = RoomUnread` are shown as unread room rows with
  no preview and no event-specific open action.
- The row-level mark-read button is hidden for placeholders.
- Mark-all-read remains available and dispatches the existing typed command;
  Rust clears event rows and placeholder rooms separately.

This preserves the current privacy and Matrix ownership boundary instead of
making React guess a first unread event.

## Tests

Follow TDD and add RED checks before production code:

1. `crates/koushi-core/tests/runtime_activity.rs`
   - Unread RoomSummary with no observed row appears in Activity Unread as a
     typed placeholder with `event_id = None`.
   - Observed unread rows still win over placeholders for the same room.
   - Mark-all-read on placeholder-only rooms clears room unread counts, does not
     write a `FullyReadMarkerUpdated` synthetic id, and does not emit synthetic
     ids in `ActivityEvent::MarkedRead.cleared_event_ids`.

2. `crates/koushi-state/tests/activity_state.rs`
   - `ActivityRow` serialization/debug redacts private values and carries a
     typed `kind`.
   - Mark-read reducers ignore non-matching or eventless row clears safely.

3. `apps/desktop/src/domain/timelineStore.test.ts`
   - Already proves a store can apply `InitialItems` and diffs by key.
   - Add a test for any new bounded-retention helper.

4. `apps/desktop/src/components/TimelineView.test.tsx`
   - A pre-populated App-level store renders rows when `TimelineView` mounts
     after `InitialItems`.
   - The view-local listener still handles presentation events such as
     `MessageSourceLoaded` without invoking the App-level store setter.

5. `apps/desktop/src/components/panes` or browser-headless test
   - Activity placeholder row renders from `kind = RoomUnread`, without event
     preview, event-open action, or row-level mark-read action.

6. Canon/DTO sync
   - Update `docs/architecture/state-machine.md` Account Activity notes for
     RoomSummary-authoritative Unread membership and typed room-unread rows.
   - Update Tauri DTO serialization tests, TypeScript `ActivityRow`,
     `coreEvents.generated.json`, browser fake snapshots, and app/IPC harness
     snapshots in the same change.

7. Focused verification
   - `cargo test -p koushi-core --test runtime_activity`
   - `cargo test -p koushi-state --test activity_state`
   - `npm --prefix apps/desktop run test -- --run src/domain/timelineStore.test.ts src/components/TimelineView.test.tsx src/backend/browserFakeApi.test.ts`
   - `npm --prefix apps/desktop run typecheck`

## Out Of Scope

- Persisting timeline DTOs outside memory.
- Resolving placeholder unread rows to first unread event through backfill.
- Replacing `select_room` with a new public command name. Existing commands may
  be internally treated as the open-room lifecycle, but the IPC surface should
  stay stable unless a later contract change requires it.
- Manual/native GUI inspection.

## Self-Review

- No placeholder sections remain.
- The design keeps Matrix semantics in Rust and uses React only for projection
  storage and presentation.
- The selected scope is narrow enough for one implementation pass on top of the
  existing branch.
- Claude design-review findings were folded in: typed activity row kind,
  Rust-side mark-all handling for eventless placeholders, canon/DTO sync, and
  bounded App-level timeline retention.
- The main known deferral is explicit: placeholder rows are not event-openable
  until Rust resolves a first unread event.
