# Thread Inbox Design

## Goal

Make the room-level Threads entry point useful as a thread inbox. Pressing the Threads button should open a right-panel view that shows the room's threads that need attention, not an empty or ambiguous list. Opening a thread from that panel should show the selected thread timeline and move to the relevant unread or selected message.

## Current Problems

- The current Threads panel can show "No threads" even when the visible room has thread summaries and replies.
- The Threads sidebar badge is driven by thread attention counts, but the panel it opens does not clearly represent those counts.
- Opening an individual thread uses a thread timeline, but viewport reporting is room-keyed, so thread-local behavior such as reply quote detail fetches can fail to update the right panel.
- The same right panel is used for a thread inbox and an individual thread timeline, but those modes are not clearly separated.

## User Model

Threads means "things in this room's threads that I should review." It is not a generic room timeline and not just a raw SDK thread list. The panel should behave like an inbox:

- Each row represents one thread root.
- Rows show root context, latest reply preview, reply count, and unread/highlight state when available.
- Selecting a row opens the individual thread timeline in the right panel.
- If the selected row corresponds to unread thread activity, the thread opens near the unread/latest relevant event.

## Architecture

Keep the existing `ThreadListService` as the source for thread roots and latest reply summaries. Do not subscribe every thread timeline up front. The thread inbox should be a Rust-owned projection, delivered through the existing `ThreadsListState` path, with enough metadata for React to render rows and route selection.

The individual thread timeline remains a separate `TimelineKind::Thread` subscription. Timeline viewport observations must address the actual timeline key, not only the room timeline. This lets thread-local behaviors, including reply quote detail fetches, run against the thread actor.

## Data Flow

1. Clicking the Threads button opens the thread inbox for the active room.
2. Rust subscribes or refreshes the room's `ThreadListService`.
3. Rust projects thread rows and applies existing thread list ordering settings.
4. React renders rows in the right panel.
5. Clicking a row opens the corresponding `TimelineKind::Thread`.
6. The thread timeline reports viewport observations with its thread key.
7. Rust fetches missing visible reply details for that thread timeline and emits normal timeline diffs.

## Error Handling

- If the SDK thread list is unavailable or fails, the panel should show a failure state, not "No threads."
- "No threads" should only appear after a successful load with an empty projected list.
- If a thread row can no longer be opened, the panel should keep the inbox open and surface a recoverable failure state.

## Testing

- Unit test that a visible thread timeline reports viewport observations with `TimelineKind::Thread`.
- Unit test that thread reply quote detail fetching is triggered for visible missing quotes in a thread timeline.
- UI test that the Threads button opens a non-empty thread inbox when thread summaries exist.
- Regression test that "No threads" is not shown while the thread list is still loading or failed.

## Non-goals

- Do not flatten all thread replies into one global chronological event stream.
- Do not subscribe every thread timeline just to build the inbox.
- Do not change room timeline unread semantics in this step.
