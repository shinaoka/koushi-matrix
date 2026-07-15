# Room Entry Gap Repair Pacing Design

## Status

Approved design for the room-entry regression introduced by the automatic room
timeline gap repair work. This document narrows the scheduling and projection
contract in
`docs/superpowers/specs/2026-07-03-room-timeline-cache-repair-design.md`.
It does not replace the non-destructive continuity model from that document.

## Problem

Automatic gap repair currently inspects every persisted room-timeline gap as
soon as a Room timeline actor starts. A persisted gap whose boundaries are not
present in the active timeline window is projected at item index zero. Core then
selects that descriptor and asks the SDK to load up to 32 cached chunks or 64
network events per operation, repeating for up to 32 operations without waiting
for the desktop projection to render.

The SDK correctly publishes every loaded cached chunk as timeline diffs. The
combination therefore turns an off-screen cache repair into active timeline
materialization. Real-account diagnostics show a room growing from tens of
items to hundreds or thousands during entry, repeated single-gap repair
generations, erroneous underfilled-initial pagination while the virtual list is
settling, and multi-second main-thread stalls.

The same saturation can delay otherwise independent room-list and profile
projections. It can therefore make incomplete room titles or a partially
rendered sidebar appear correlated with room entry even though sync still owns
the full room and DM sets.

## Goals

- Preserve SDK-owned, non-destructive timeline gap inspection and repair.
- Complete room selection and the initial timeline projection before automatic
  repair work begins.
- Automatically repair only gaps whose position is known in the active
  timeline window.
- Admit at most one unrendered repair diff batch per Room timeline actor.
- Prevent automatic scrollback from firing against an unsettled virtualized
  projection.
- Keep manual repair useful while pacing off-screen cache hydration one chunk
  at a time.
- Preserve room-list and display-label responsiveness during repair.

## Non-Goals

- Changing the Room/People sidebar filter model or Space-scoped DM membership.
- Changing room display-label precedence.
- Guessing gap positions from timestamps, event IDs, or array adjacency.
- Deleting or resetting a room cache.
- Adding an app-owned raw `/messages` implementation.
- Repairing every persisted gap before a room can be entered.

## Gap Eligibility And Projection

Core continues to inspect all persisted gaps through the token-free SDK
adapter. Inspection and presentation are separate:

1. A gap is **projected** only when its newer boundary is present in the
   actor's canonical navigation items, or its older boundary is present and the
   insertion point immediately after that boundary is valid.
2. An unprojected gap has no timeline row. Core must not fall back to index zero
   or any other invented position.
3. Automatic repair chooses the projected gap intersecting the reported
   viewport when available, then the projected gap nearest the live edge.
4. When no gap is projected, continuity remains `Incomplete`, but automatic
   repair waits for ordinary pagination or another cache update to bring a
   boundary into the active window.

This keeps persisted continuity state authoritative without turning an
off-screen discontinuity into a visible gap at the top of the room.

## Room Entry Ordering

`InitialItems` remains the first Room timeline projection. The actor must accept
the existing generation-fenced projection acknowledgement before starting its
subscription inspection. Gap inspection, action-channel backpressure, cache
I/O, and network I/O therefore cannot delay the acknowledgement used to settle
room selection.

If the actor is replaced before acknowledgement, no inspection starts. If the
acknowledgement is stale, the replacement actor remains the only owner allowed
to inspect or repair.

## Repair Budgets

Automatic and manual repair use the same private scheduler but different
bounded cache-hydration policies:

- Automatic repair: at most 64 network events and zero off-screen cached
  chunks per operation.
- Manual repair: at most 64 network events and one cached chunk per operation.

An automatic operation can therefore repair a gap already represented in the
active SDK window, but it cannot pull unrelated persisted history into the
window. Manual repair may advance toward an off-screen descriptor, but only one
published cache chunk is admitted before the desktop must settle the resulting
projection.

The existing global maximum remains a safety ceiling, not permission to run a
tight repair loop.

## Render-Settled Repair Loop

Core adds an actor-private `AwaitingProjection` phase after a repair operation
publishes timeline diffs. The phase records the current actor generation,
timeline generation, room-local repair generation, and the minimum expected
timeline batch ID. `TimelineActor::next_batch_id` already names the next batch
that will be emitted, so the fence captures that value at repair start without
adding one; adding one would deadlock a repair that publishes exactly one diff
batch.

The desktop timeline store applies `ItemsUpdated` normally. After React commits
the corresponding virtual window and completes anchor or live-edge
restoration, it sends a typed render acknowledgement containing those four
fences. Core accepts only a matching repair generation and a matching or newer
batch acknowledgement owned by the same actor and timeline generation. Stale,
duplicate, cross-room, and cross-actor acknowledgements are ignored.

Only after a matching acknowledgement may Core re-inspect and schedule the next
repair operation. If an SDK outcome produces no timeline diff, Core may
re-inspect immediately because no desktop work is pending. Logout, unsubscribe,
account replacement, and actor replacement cancel the phase with the rest of
the actor-owned scheduler.

The acknowledgement is a projection-flow fact, not a product success signal.
It does not enter `AppState`, does not settle a user intent, and contains no
Matrix identifier beyond the already typed private command key.

## Underfilled Backfill Guard

The underfilled-initial check must not use a transient DOM `scrollHeight` while
a timeline projection or virtual range is unsettled. It skips automatic
pagination while any of the following is true:

- projection layout or anchor restoration is pending;
- the virtual range has not been committed for the current item revision;
- the height model already predicts overflow even if the current DOM window is
  temporarily empty.

After settlement, the existing pagination-state and in-flight guards still
apply. This enforces the repository rule that automatic pagination cannot begin
before the previous diff has rendered and anchor restoration has completed.

## Manual Repair

The Room info action continues to use the shared scheduler. A repeated click
coalesces with active inspection, repair, or render settlement. A manual request
may select the nearest persisted gap when no gap is projected, but each cached
chunk and network batch is paced by a render acknowledgement. Existing events
remain visible on failure, cancellation, or timeout.

## Diagnostics And Privacy

Diagnostics remain coarse and private-data-free. New outcomes may report:

- `offscreen` when automatic repair has no projectable gap;
- `awaiting_render` after a diff-producing repair operation;
- `render_acknowledged` when the matching projection fence arrives;
- `stale_render_ack` for rejected acknowledgements.

Diagnostics contain counts, generations, batch IDs, trigger kind, and coarse
outcomes only. They must not contain room, event, or user IDs; pagination
tokens; message bodies; display names; or raw SDK errors.

## Room List And Display Labels

Gap repair remains owned by the selected Room timeline actor and must not gate
RoomActor snapshots, profile updates, or room-list state deltas. Scale coverage
will prove that room selection settles and room-list/display-label updates are
still delivered while a repair is inspecting, awaiting network I/O, or awaiting
render acknowledgement.

This change does not alter the existing sidebar contract: the Rooms filter
excludes DMs, and an active Space scopes the DM section by counterpart
membership. If room labels remain incomplete after the repair pressure is
removed, that behavior is a separate display-label defect and will be fixed at
its Rust projection source.

## Verification Contract

Implementation is test-first and headless-first.

### Core

- A Room actor does not inspect gaps before its initial projection is
  acknowledged.
- An unprojected persisted gap produces no gap row and no automatic repair.
- A projected gap selects the expected automatic budget.
- Manual repair hydrates at most one cached chunk per operation.
- A diff-producing repair cannot start another operation before a matching
  render acknowledgement.
- Stale actor, generation, and batch acknowledgements do not advance repair.
- A no-diff outcome can re-inspect without waiting for an impossible
  acknowledgement.
- Cancellation and actor replacement discard pending acknowledgements.

### Desktop

- A large `ItemsUpdated` batch crossing the virtualization threshold does not
  request underfilled-initial pagination before the virtual range settles.
- The height model prevents a false underfilled request for thousands of
  canonical items while the DOM window is transiently empty.
- The render acknowledgement is sent only after the relevant commit and anchor
  restoration and carries the current actor generation, timeline generation,
  repair generation, and batch ID.

### Scale

- A real-account-shaped fixture of roughly 110 rooms, five Spaces, and 57 DMs
  enters a room with a repairable gap while room-list and profile updates are in
  flight.
- Room selection reaches its correlated terminal outcome.
- No room-list or display-label update is lost or blocked by gap work.
- Automatic repair never materializes an unprojected cached chunk.
- A zero-cache-budget automatic `Deferred(0)` stops as a retryable unsupported
  anchor instead of immediately repeating the same inspection/repair pair.
- No normal repair path calls `EventCacheStore::remove_room`.

## Acceptance Criteria

- Entering a room is independent of gap inspection and repair latency.
- An off-screen persisted gap does not add a synthetic row at the top of the
  active timeline.
- Core never has more than one unacknowledged repair projection batch.
- Automatic repair does not load off-screen cached chunks.
- Manual repair advances off-screen cache hydration one rendered chunk at a
  time.
- Automatic backfill does not fire against an unsettled virtual projection.
- Existing events, drafts, E2EE state, settings, and unrelated rooms remain
  unchanged on every failure path.
