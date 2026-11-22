# #2030 B2 — settle the confirmed read boundary into the sidebar

## Symptom

Automatically reading a room marked it read in the timeline at once, but the
sidebar badge stayed until Sliding Sync echoed the new receipt (measured 189 ms in
the report, reported as much longer in the field). The timeline's own snapshot was
correct; only the room list lagged, because it was fed exclusively by sync.

## What B1 left in place

`confirmed_boundary_accepts` (`crates/koushi-core/src/read_state.rs`) made one
confirmed boundary per account session and read scope, ordered only by the
timeline's position index. Nothing published that boundary anywhere outside the
timeline, so the sidebar had no way to use it.

## Design

Two halves, each where its evidence is:

- **Core** publishes `AppAction::ConfirmedReadBoundarySettled { room_id, event_id }`
  from `TimelineManagerActor::update_local_server_confirmation`, which both writers
  (the acknowledged write and an authoritative observation) already funnel through.
  Only an acknowledged write is publishable: our own write sent the private receipt
  the server counts unread from, whereas an observation reads `m.fully_read`
  account data, which no server derives counts from — settling on it could zero a
  badge the server still reports as unread, and the recorded activity would keep it
  zero. It publishes only when the current-generation position index names the
  boundary as the room window's newest readable row
  (`TimelinePositionIndex::newest_readable_event_id`), so no unread event can
  follow it in this scope; a state row or a thread reply may be newer without being
  activity of the main conversation, while a *hidden* row still counts as activity
  because hiding is a display decision and the server counts those events anyway.
  An unplaceable boundary and a boundary a later event follows publish nothing. The
  index is read again after the read waiters settle, because that await can block
  on action-channel capacity while a newer row is projected.
- **The reducer** publishes it as the room's read position
  (`reducer/live_signals.rs`) whatever room is in the foreground — into the
  existing `fully_read_event_id`, which the marker path owns, so it is only filled
  in when nothing published one yet and a stale settlement cannot move the read
  divider back — and `reducer/room.rs::settle_room_list_read_boundary` clears the
  room's main unread and attention only while the room list's own `latest_event`
  is that same event. The proof is therefore two-sided:
  Core proves nothing follows in the timeline window, the reducer proves the list
  has not moved either. Either half alone would be unsound — a list lagging behind
  a newer timeline event, or a timeline window that does not hold the list's newest
  event, keeps its counts.

Clearing follows the existing #1803 shape: `locally_read_activity` is recorded and
native attention is recomputed in the same reduce, so no later room-list update is
needed to finish the clear.

## What is deliberately preserved

The scopes that survive, and why, are specified once in
[room-list settlement](../../docs/architecture/state-machine.md#room-list-settlement);
what matters here is that each one is a different read scope or a different intent
rather than an omission.

## Evidence

- `crates/koushi-core/src/timeline/read_state/tests.rs`: the boundary is published
  when it is the window's newest readable row; an observation is never published
  however far it advances the scope; nothing is published when a later event
  follows it or when the scope is a thread; and a write that replaces a boundary
  the window cannot place is published.
- `crates/koushi-core/src/timeline/actor/tests.rs`: the position index names the
  newest readable row of the main conversation, not the newest item, with a state
  row, a hidden row, a redacted row and a thread reply all newer than the message.
- The acceptance test in the same file drives the manager's own settlement
  (`admit` → `wake` → `settle_read_operation`, the hop the actor's acknowledgement
  takes in production) and reduces every emitted action into a real `AppState`
  where the reader is looking at another room. The room-list projection's badge
  goes to zero with no room-list update afterwards, and the foreground room is
  untouched.
- `crates/koushi-state/src/reducer/tests/room_read_boundary_settlement.rs` (8
  tests): a
  settled room that is not the foreground room, with an unread foreground room and
  an unread unrelated room asserted unchanged; counts kept when a newer activity
  arrived; a manually marked room left untouched; a thread unread keeping the room
  attention counters; an old snapshot unable to revive a settled count; an unknown
  room only keeping the read position; a settlement with nothing left to clear
  still recording the covered activity, so a snapshot that keeps showing it unread
  stays stale; and a settlement never moving an existing, newer read position.

Reverting the publication (returning before `emit_action_reliable`) turns the two
positive Core tests and the acceptance test red; the absence tests
(`a_boundary_a_later_event_follows_is_not_published`,
`an_observed_boundary_never_settles_the_room_list`) assert that nothing is
published, so they pass either way and are controls, not evidence. Reverting the
count clear in `settle_room_list_read_boundary` turns three reducer tests red.
The index's own `newest_readable_event_id` is covered by
`position_index_names_the_newest_readable_row_of_the_main_conversation`, and the
"a settlement never moves an existing read position" test covers the
`fully_read_event_id` ownership rule.

## Limits

- The proof is conservative by construction, and every unprovable case is simply
  not settled: a list whose newest activity is a thread reply (its `latest_event`
  is not the boundary), a boundary the resident window no longer holds, or a
  readable row the window holds but the list does not. The settlement only removes
  a wait the echo would have resolved anyway.
- A `Focused` timeline never has a main-scope confirmation key, so it publishes
  nothing.
- The boundary fills `live_signals.rooms[room].fully_read_event_id` when it is
  empty, which is also what makes the existing stale-snapshot suppression apply; a
  later authoritative marker observation owns that field and always wins.
- The settlement is emitted best-effort on the manager's action channel; a send
  that fails loses this one settlement, and the sync echo remains the fallback.
