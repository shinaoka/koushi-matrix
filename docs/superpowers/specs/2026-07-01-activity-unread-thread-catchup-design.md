# Design: Activity unread and thread catch-up

Status: ready for implementation
Date: 2026-07-01

## Goal

Make missed work discoverable without forcing the user to hunt through the
main timeline. The first pass improves two connected surfaces:

- Activity / Unread should show how much unread work exists and should open
  directly at a known unread event whenever possible.
- Room threads should have a stable, discoverable entry point so new replies on
  old thread roots are visible through the room's thread list.

The main timeline remains canonical chronological room history. It must not
duplicate old thread roots at the bottom or mix thread replies into normal room
timeline order just to make catch-up easier.

## Current Shape

Activity already has the right UI and command boundary for event-level unread
navigation:

- `ActivityRow` supports `kind: "event"` and `kind: "roomUnread"`.
- `ActivityPane` opens event rows through `openActivityEvent(roomId, eventId)`.
- Tauri `open_activity_event` selects the room and stores a bottom-aligned
  scroll anchor without opening the focused-context panel.

The weak point is projection. `ActivityProjection::snapshot` currently creates
room-level `roomUnread` placeholders for unread rooms even when it has observed
timeline event rows that can be opened directly.

Threads already have a right-panel list:

- `ThreadsListView` renders `ThreadsListState`.
- `ThreadsListItem` carries root and latest-reply metadata.
- `ThreadListOrder::LatestReply` is already the default display order.

The weak point is discovery. The room header only shows the Threads button when
pane-level `thread_attention` is tracking counts, and that state is scoped to
the currently opened thread rather than to all unread or recent threads in the
room.

## Product Principles

- Prefer inbox/list surfaces for catch-up. Do not reorder the main room
  timeline for unread or thread discoverability.
- Prefer event-level rows over room-level placeholders. A room placeholder is a
  fallback for unresolved unread state, not the primary unread experience.
- Keep Matrix state and unread derivation Rust-owned. React may render counts
  and rows but must not infer unread totals by scanning visible DOM rows.
- Keep activity event navigation distinct from search/focused-context
  navigation. Opening Activity should bring the room timeline to the event, not
  open a context panel.
- Avoid leaking Matrix identifiers or message bodies in diagnostics or QA
  tokens. Activity rows may display previews because that is user-facing UI.

## Activity Architecture

Add Rust-owned stream metadata to `ActivityStream`.

```rust
pub struct ActivityStreamSummary {
    pub event_count: u32,
    pub room_count: u32,
    pub highlight_count: u32,
    pub unresolved_room_count: u32,
}
```

The summary is serialized with each `ActivityStream` and mirrored in TypeScript.
`unread.summary.event_count` drives the normal unread tab count. If the stream
has only unresolved room placeholders, the UI may display the unresolved room
count instead.

`ActivityProjection::snapshot` should build Unread in this order:

1. Observed timeline event rows whose event id is after the room's fully-read
   marker, excluding locally cleared event ids.
2. Latest-room-event rows when the room summary has a latest event that is
   unread and not already represented by an observed row.
3. One `roomUnread` placeholder only for an unread room that has no known
   unread event row.

Mark-read behavior stays compatible:

- Event rows mark up to that event id.
- Placeholder rows still mark the whole room because no target event is known.
- `Mark all read` clears event rows and placeholder rooms.

## Activity UI

The Unread tab label should include a count when Activity is open:

- `Unread (N)` when `unread.summary.event_count > 0`.
- `Unread (N rooms)` when there are no event rows but unresolved rooms exist.
- `Unread` when Activity is not open or all counts are zero.

Unread rows should primarily render as event rows with sender, timestamp,
context, preview, unread badge, and row-level mark-read action. Placeholder
rows remain visually simpler and clearly open the room rather than a known
event.

## Threads Architecture

The first pass should not invent account-wide thread notification state.
Instead:

- Always show the room-header Threads button when an active room exists.
- Keep the existing right-panel `ThreadsListView` as the room-scoped thread
  inbox.
- Keep `ThreadListOrder::LatestReply` as the default so old root events with
  new replies rise to the top.
- Continue to use `thread_summary` chips on timeline rows for local context,
  but do not rely on those chips as the only discovery mechanism.

Future account-wide or space-wide thread inbox work requires a separate design
because it crosses the current room-scoped `ThreadListService` boundary.

## Acceptance Criteria

- Activity / Unread shows a count when unread rows exist.
- Activity / Unread opens known event rows through `openActivityEvent` and lands
  on the target event in the room timeline.
- Activity / Unread only uses room placeholders for rooms whose unread event id
  cannot be resolved from the local projection.
- Muted and low-priority rooms remain excluded from Activity streams.
- Mark-read operations still clear event rows, placeholder rows, and room unread
  indicators consistently.
- The room header always offers the Threads button for an active room.
- Threads list remains latest-reply ordered by default.

## Out Of Scope

- Account-wide Threads inbox.
- Server-derived per-thread unread counts beyond the existing SDK thread list
  and current pane-level `thread_attention`.
- Duplicating old thread roots at the bottom of the main timeline.
- Replacing the Activity command boundary with search or focused-context
  navigation.
