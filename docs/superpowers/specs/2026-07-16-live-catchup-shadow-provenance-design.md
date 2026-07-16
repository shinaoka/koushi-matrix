# Live Catch-Up Shadow Provenance Design

## Status

Approved design for the first implementation slice of issue #271. This slice
adds a production-faithful reproduction and private internal observations. It
does not change repair selection, pagination direction, budgets, render ACKs,
or user-visible state.

## Problem

PR #270 made the newest unprojected persisted gap selectable, but production
still loses events sent while Koushi is offline or the room is not subscribed.
The current room-open flow calls `RoomListService::subscribe_to_rooms` and then
immediately builds a `TimelineActor`. The subscription call updates the next
sliding-sync request but does not wait for a response containing that room.
The actor can therefore run `LiveEdge` repair against stale persisted topology
before current server-derived room coverage is known.

The existing `timeline_reconnect` headless scenario covers a different case:
the timeline remains subscribed while the client is offline. It does not cover
an unsubscribed room opened after reconnect.

The targeted SDK gap API is explicitly backward pagination from a persisted
gap token. Backward direction is correct when a limited sync has supplied the
current live chunk and a `prev_batch` gap immediately before it. The defect is
therefore not yet proven to require forward pagination. First the product must
distinguish a current sync-created gap from arbitrary older persisted gaps.

## Goals

- Reproduce the unopened-room reconnect race with a real sliding-sync request,
  Matrix SDK event cache, and real `TimelineActor` scheduling.
- Associate a room-subscription generation with the first successful response
  that actually contains that room.
- Record the raw timeline facts and gap created by that response inside the SDK
  boundary.
- Compare the current `LiveEdge` decision with the observed sync-derived gap in
  shadow mode.
- Emit private-data-free evidence that distinguishes repair-before-checkpoint,
  a matching selection, and selection of a different persisted gap.
- Produce an API and regression harness that the later behavioral fix can use
  without redesigning the ownership boundary.

## Non-Goals

- Do not change `TimelineGapRepairTrigger`, descriptor selection, repair
  continuation, or pagination direction.
- Do not block room rendering or delay actor creation while waiting for sync.
- Do not add product state, WebView DTOs, React logic, or user-visible errors.
- Do not persist the shadow checkpoint across process restart. A later fix may
  decide whether persistence is needed after the live reproduction is known.
- Do not infer convergence from rendered rows, row count, or render ACK.
- Do not introduce a forward-pagination API in this slice.

## Ownership

### Matrix SDK Event Cache

The room event cache owns one in-memory, monotonically sequenced observation of
each non-empty sync timeline it commits:

```rust
pub struct RoomTimelineSyncObservation {
    sequence: u64,
    limited: bool,
    event_count: usize,
    prev_batch_present: bool,
    newest_event_id: Option<OwnedEventId>,
    inserted_gap: Option<RoomTimelineGapDescriptor>,
}
```

The observation is finalized only after linked-chunk mutation and persistence
complete. `newest_event_id` is the newest raw event in the sync batch, including
relations or events that do not become standalone timeline rows.
`inserted_gap` identifies the exact persisted gap introduced by the same sync
batch. Tokens and linked-chunk identifiers remain private.

`RoomEventCache::latest_sync_observation()` returns the latest observation for
RoomListService correlation. The type has a redacted `Debug` implementation.
An empty room response does not reuse an earlier observation.

### Matrix SDK UI RoomListService

RoomListService owns a monotonically increasing room-subscription generation.
The existing API remains available, while Core uses a generation-returning
variant:

```rust
pub async fn subscribe_to_rooms_with_generation(
    &self,
    room_ids: &[&RoomId],
) -> RoomSubscriptionGeneration;
```

At subscription mutation, RoomListService records the current event-cache sync
sequence for every requested room. Its sync loop snapshots the subscription
generation before awaiting the next sliding-sync result. A response is eligible
only when:

- the generation is unchanged when the result completes;
- `UpdateSummary.rooms` contains the room; and
- the room is still in the active subscription set.

For an eligible room, RoomListService publishes:

```rust
pub struct RoomSubscriptionCheckpoint {
    subscription_generation: RoomSubscriptionGeneration,
    room_id: OwnedRoomId,
    timeline: Option<RoomTimelineSyncObservation>,
}
```

The timeline observation is present only when its sequence is newer than the
subscription baseline. If the response contains the room without a timeline
mutation, `timeline` is `None`; a stale observation is never attached.

RoomListService keeps the latest checkpoint per room and exposes a subscriber.
This handles a response arriving before Core finishes constructing the actor.
Only the latest generation for each room is retained.

### Koushi SDK Adapter

`koushi-sdk` wraps the upstream checkpoint and opaque gap descriptor in a
token-free contract. It exposes closed coarse accessors for counts and flags,
plus an internal equality operation that can answer whether two opaque handles
refer to the same snapshot-scoped gap. `Debug` must not expose room IDs, event
IDs, tokens, or chunk identities.

### Core TimelineManager and TimelineActor

TimelineManager owns one checkpoint observer task for the active
RoomListService. Replacing or stopping the sync backend aborts the old observer
and clears its retained checkpoints.

`build_timeline_actor_handle` passes the generation returned by the room
subscription call into the new actor. TimelineManager routes a checkpoint only
when room, subscription generation, and active actor generation match. It also
replays the latest retained checkpoint immediately after actor registration so
an early response is not lost.

TimelineActor stores at most one checkpoint. It performs shadow comparison at
the same point where current gap inspection chooses a repair descriptor. It
does not alter the selected descriptor or scheduler state.

## Data Flow

```text
Core requests room subscription generation G
  -> RoomListService records per-room event-cache sequence baseline
  -> sliding-sync request includes the room subscription
  -> SDK processes the room timeline and persists linked chunks
  -> event cache records sync observation S and its introduced gap
  -> sliding sync returns UpdateSummary containing the room
  -> RoomListService publishes checkpoint (G, S)
  -> TimelineManager generation-fences and routes the checkpoint
  -> TimelineActor compares checkpoint gap with current LiveEdge selection
  -> diagnostics record only the coarse shadow decision
```

If `LiveEdge` inspection starts before the checkpoint arrives, the actor records
`RepairStartedBeforeCheckpoint`. When the checkpoint later arrives it is stored
for subsequent comparison, but the active repair is not cancelled or changed.

## Shadow Decisions

```rust
enum LiveEdgeShadowDecision {
    AwaitingSubscriptionResponse,
    NoTimelineUpdate,
    CheckpointAnchored,
    CheckpointGapMatchesSelection,
    SelectedDifferentGap,
    CheckpointGapNotInTopology,
    RepairStartedBeforeCheckpoint,
    UnsupportedBackend,
    Stale,
}
```

- `AwaitingSubscriptionResponse`: the actor exists but no current checkpoint
  has arrived.
- `NoTimelineUpdate`: the room was present in the response but no newer
  timeline observation followed the subscription baseline.
- `CheckpointAnchored`: the checkpoint introduced no gap and its newest raw
  event is already in actor-owned event coverage.
- `CheckpointGapMatchesSelection`: the current selection is the gap introduced
  by the current subscription response.
- `SelectedDifferentGap`: repair selected another persisted gap.
- `CheckpointGapNotInTopology`: the checkpoint gap was already joined, removed,
  or made stale before comparison.
- `RepairStartedBeforeCheckpoint`: live-edge repair acquired scheduler
  ownership before current sync provenance was available.
- `UnsupportedBackend`: legacy sync has no RoomListService subscription
  generation in this slice.
- `Stale`: any room, subscription, actor, or topology generation fence failed.

## Diagnostics and Privacy

Diagnostics use source `core.live_catchup_shadow` with closed stage and decision
tokens. Allowed fields are:

- backend kind;
- decision token;
- subscription, actor, and topology generation numbers;
- limited, `prev_batch`, timeline update, and checkpoint-gap booleans;
- event, persisted-gap, repair-batch, and projection-batch counts;
- direction token;
- whether checkpoint observation preceded repair acquisition.

Forbidden fields are room, event, user, transaction, chunk, and gap IDs;
message bodies; display names; pagination tokens; homeserver names; and raw SDK
errors. All new private types use redacted custom `Debug` implementations.

Shadow diagnostics are observational. Missing checkpoints, unsupported backend,
stale results, actor replacement, logout, and observer lag do not change
`AppState`, scheduler ownership, continuity state, or render ACK behavior.
No fixed-delay retry or timeout is added. The ordering fact is recorded when a
repair starts without a checkpoint and when the actor later receives or never
receives a matching checkpoint.

## Verification

### Matrix SDK Fork

Add integration coverage in `matrix-sdk` and `matrix-sdk-ui` for:

- a limited subscribed-room response producing an observation after cache
  persistence;
- observation `newest_event_id` using the newest raw relation event;
- the observation gap matching persisted gap inspection;
- a subscription generation change rejecting an older delayed response;
- a room response without a new timeline producing `timeline=None` rather than
  reusing the baseline observation;
- latest-per-room checkpoint replay for a late subscriber; and
- redacted `Debug` output for observations and checkpoints.

### Koushi Core

Add pure tests for every `LiveEdgeShadowDecision`, generation fencing,
early-checkpoint replay, observer lifecycle, non-interference with the current
repair result, and private-safe diagnostics.

Add a deterministic real-actor characterization fixture with a delayed
room-subscription response:

1. seed older persisted room topology;
2. request a room subscription;
3. delay the response containing the current limited timeline;
4. acknowledge the actor's initial projection;
5. prove current `LiveEdge` repair starts before the checkpoint;
6. release the response and prove the checkpoint is routed to the same actor;
7. preserve the existing repair outcome and render fence unchanged.

The test passes by asserting the current defect token
`repair_started_before_checkpoint`. The later behavioral fix must reverse this
expectation rather than retain a characterization test that blesses the bug.

### Local Headless QA

Extend `timeline_reconnect` with an unsubscribed-room stage:

1. A and B join a room and A seeds one locally known anchor;
2. A unsubscribes the room timeline;
3. A's proxy is disabled;
4. B sends more than the room-subscription timeline limit;
5. A reconnects and then opens the room;
6. the harness observes a current subscription checkpoint and a closed shadow
  decision without printing private identifiers.

The standard token is `live_catchup_shadow_checkpoint=ok`. When the local
server reproduces the ordering defect, it may additionally print
`live_catchup_shadow_repair_before_checkpoint=ok`; the deterministic mock test,
not server timing, is the required regression gate.

## Delivery

This work crosses two repositories and is delivered in order:

1. `shinaoka/matrix-rust-sdk-work`: event-cache observation,
   RoomListService generation/checkpoint stream, SDK tests, and private API
   documentation.
2. `shinaoka/koushi-matrix`: update every Matrix SDK Cargo revision and the
   `vendor/matrix-rust-sdk` submodule to the same SDK commit; add the adapter,
   manager/actor shadow wiring, headless scenario, canon, and tests.

The Koushi change must include `Cargo.toml`, the submodule pointer,
`crates/koushi-sdk/src/lib.rs`, and any feature declarations in external review
input. SDK and Koushi PRs remain behavior-neutral and do not close #271.

## Acceptance Criteria

- A deterministic test proves a room can begin current `LiveEdge` repair before
  its room-subscription response is observed.
- Every checkpoint identifies its exact subscription generation and, when
  present, the exact sync-created persisted gap.
- Delayed or cancelled response generations cannot be attributed to a newer
  subscription or actor.
- Empty/no-timeline responses cannot reuse stale event-cache observations.
- Current repair behavior, budgets, ACKs, state, and UI output are unchanged.
- Production-safe diagnostics distinguish checkpoint match, mismatch, and
  repair-before-checkpoint without private data.
- Both repositories have reproducible focused tests and the local headless
  stage reports only closed token evidence.
