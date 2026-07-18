# Active Live-Tail Freshness Design

**Date:** 2026-07-17

## Problem

After SyncService fails before establishing a room subscription, Koushi falls
back to legacy `/sync`. The legacy client reuses its persisted sync token. Its
first incremental response may therefore omit the active room even though the
room's locally rendered live edge is stale.

The current room-absent fallback inspects persisted historical gaps and repairs
the newest one. That is not a freshness proof. A persisted gap can predate the
missing live interval by days, as the production diagnostics demonstrated: the
repair inserted old events while the current live edge remained unchanged.

## Invariant

A live room timeline is current only after Koushi has a freshness proof for the
active sync epoch. A proof is one of:

1. an exact SyncService room-subscription checkpoint;
2. an exact legacy `/sync` room timeline update; or
3. a successful room-scoped live-tail snapshot from the homeserver.

A global committed response that omits a room advances synchronization, but it
does not prove that room's cached live edge is current.

## Freshness State

The timeline actor tracks a backend-neutral state for its current sync epoch:

- `Unproven`: no exact room update or live-tail snapshot proves freshness;
- `Refreshing`: one bounded room-tail request is in flight;
- `Fresh`: the current epoch has a proof;
- `Deferred`: work was preempted because another room became active;
- `Retryable`: the request failed without changing canonical cache state.

Exact room updates transition directly to `Fresh`. A room-absent legacy
checkpoint transitions to `Unproven`; it never authorizes selection of an
arbitrary persisted gap. A successful live-tail snapshot transitions to
`Fresh`, whether it adds events or proves that the existing tail is unchanged.

The proof is scoped to a sync epoch. It is invalidated when the backend or sync
run changes. Repeated room-absent responses in the same proven epoch do not
repeat the request.

## Room-Scoped Live-Tail Snapshot

The SDK exposes one token-free, room-scoped operation:

```text
refresh_live_tail(room, event_limit, operation_id) -> outcome
```

It requests `/rooms/{roomId}/messages` backwards with no `from` token. Matrix
defines that request as starting at the latest visible event. The default
foreground budget is 128 events.

The SDK owns all private pagination tokens and event identities. Core receives
only a coarse outcome:

- `Unchanged`;
- `Advanced { events }`;
- `Detached { events }`, meaning the previous cached edge was not present in
  the bounded tail and a persisted gap now separates old cache from new tail;
- `Stale`, when concurrent sync changed the cache before commit;
- `Failed`.

The response is decrypted through the normal room messages path. Before
mutation, the SDK revalidates the room cache revision captured when the request
started. It then applies the response using the same deduplication,
post-processing, persistence, and observable publication rules as a limited
sync timeline update.

Events returned by backwards `/messages` are reversed into chronological
order. Existing duplicates are removed before the authoritative latest tail is
appended. If the response has an `end` token, that token is persisted as a gap
immediately before the refreshed tail. Older persisted chunks remain available
for demand-driven pagination.

No room ID, event ID, message body, or pagination token crosses the SDK/Core
diagnostic boundary.

## Priority and Scheduling

TimelineManager owns a single live-tail refresh scheduler for the account so
independent timeline actors cannot compete for network priority.

Priority is:

1. the selected room;
2. other rooms waiting for delayed freshness recovery, one at a time;
3. historical pagination, which remains driven by viewport demand.

Selecting a room immediately queues or promotes its refresh. A successful
foreground refresh releases the single slot to delayed work. Delayed rooms are
ordered by the existing room activity/unread ordering where available; FIFO is
the deterministic fallback.

Historical gaps are not scanned or filled in the background. They are separate
from live-edge freshness. The initially rendered viewport is nevertheless
explicit user demand: if a gap row is visible when the room opens, Core repairs
that gap without waiting for a scroll event.

## Room Switch and Cancellation

When selection moves from room A to room B:

1. cancel A's request while it is waiting on network;
2. keep A's timeline actor and all already committed cache data;
3. return A to the delayed queue if its epoch remains unproven;
4. start B at foreground priority.

Cancellation is phase-aware. The network phase is abortable. Once the response
has entered the SDK cache-commit phase, the short commit is allowed to finish
atomically; cancellation must not leave linked-chunk persistence ahead of its
observable publication. A completed commit for an inactive room remains useful
cache state but does not replace or project the selected room's timeline.

The existing room-switch cancellation for user pagination and link previews is
extended with live-tail refresh cancellation. The timeline actor itself is not
dropped.

## More Than 128 Missing Events and Older Gaps

If the cached live edge occurs within the latest 128 events, the snapshot
deduplicates the overlap and publishes only the authoritative refreshed tail.

If more than 128 events are missing, the latest 128 are still displayed
immediately. The SDK persists the response's `end` token as a gap:

```text
older persisted cache -- gap(token) -- latest 128 events -- live edge
```

When the user scrolls upward, standard live timeline back-pagination loads
disk chunks first and resolves the encountered gap from the network in bounded
batches. Further historical gaps are repaired the same way. This matches the
Element X model: the application requests ordinary backward pagination and the
Rust SDK decides whether the next data comes from memory, disk, or a network
gap.

The final live-tail projection is inspected in two causally separate phases.
The `LiveTailSnapshot` phase publishes the authoritative gap topology but never
consumes a historical continuation token. After that snapshot settles, Core
re-evaluates the current viewport. If a projected gap intersects the initial
viewport, it queues exactly one ordinary `Automatic` historical-gap repair in
the historical causal domain. This second phase does not require the viewport's
first and last event IDs to change. A room switch cancels the queued or network
phase under the existing historical-pagination rules.

## Existing Gap Repair Policy

Remove `RepairPersistedLiveEdge` behavior that maps a room-absent checkpoint to
the last persisted gap ordinal. Room-absent now requests freshness proof.

Keep exact-response gap repair for an actual limited room update and keep
viewport-driven repair for a gap projected near the user. Neither path may be
used as a substitute for proving the live edge.

Publishing a new gap projection must not mark its viewport candidate as already
handled before a repair-capable inspection has been admitted. An observe-only
live-tail snapshot may seed the projection, but it must either queue the
separate viewport repair itself or leave the candidate eligible for the next
viewport observation.

## Diagnostics

Add privacy-safe structured events for:

- freshness state transition and sync epoch;
- scheduler priority, queue depth, and preemption;
- refresh started, unchanged, advanced, detached, stale, failed, or cancelled;
- requested limit and returned event count;
- cache commit started/completed;
- whether a persisted historical gap remains.

Diagnostics contain counts, booleans, coarse states, generations, and durations
only. They exclude room IDs, event IDs, tokens, message content, and raw SDK
errors.

## Regression Coverage

1. **Production-shape stale tail:** seed multiple unrelated historical gaps,
   omit the active room from the first legacy response, and return recent
   events only from tokenless `/messages`. Assert that recent events appear and
   no historical gap token is selected as the live-edge source.
2. **Unchanged tail:** return only cached events and assert one `Fresh` proof
   with no duplicate projection or retry loop.
3. **More than 128 missing:** return a disjoint latest tail plus an `end` token;
   assert that the latest events render immediately and exactly one navigable
   gap precedes them.
4. **Historical continuation:** scroll into that gap and assert standard
   back-pagination requests and renders the next bounded page.
5. **Initial visible historical gap:** open a room whose current live edge is
   already fresh but whose initial viewport contains a persisted gap. Assert
   the live-tail snapshot first publishes the gap without consuming it, then
   exactly one separate automatic historical repair starts without user
   scrolling. Assert the gap row disappears after the bounded repair.
6. **Preemption:** block room A's network response, select room B, and assert A
   is cancelled before B starts. Revisit A and assert it resumes without losing
   committed cache data.
7. **Commit boundary:** cancel after A's response enters cache commit and assert
   persistence and observable publication both complete exactly once.
8. **Epoch fencing:** a late outcome from an old sync epoch cannot prove or
   project the replacement epoch.
9. **Privacy:** diagnostic field tests reject identifiers, tokens, bodies, and
   raw errors.

## Rejected Alternatives

- **Full tokenless legacy `/sync`:** correct but fetches every room and cannot
  prioritize the selected conversation.
- **Filtered tokenless `/sync`:** room-scoped but risks replacing the client's
  global sync token with a token produced under a different filter.
- **Newest persisted-gap repair:** bounded but repairs history rather than
  proving the current live edge, which is the observed failure.
- **Application-owned `/context` merge:** can find events after an anchor, but
  unnecessarily exposes merge semantics outside the SDK and fails when no
  reliable anchor is cached.
