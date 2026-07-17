# Legacy Live Catch-Up Continuity Design

## Status

Approved design for issue #275. The GitHub issue's 2026-07-17 “Final
implementation plan” comment is the authoritative product decision for this
slice.

## Problem

After SyncService fails before connectivity is proven, Core can start the
legacy `/sync` fallback and open a room before the first legacy response has
committed its timeline into the event cache. The TimelineActor then runs
`LiveEdge` inspection against stale persisted topology. A later legacy response
and later live event can restore live delivery without repairing the preceding
offline interval.

The existing RoomListService checkpoint solves this ordering race only for the
SyncService backend. Legacy actors have no current-response provenance and
therefore fall through `LiveCatchupGate::Unsupported` to an arbitrary persisted
gap heuristic. Relay publication and render acknowledgement fences can also
outlive the correlation that created them, permanently blocking a queued
follow-up inspection.

## Goals

- Make committed room-timeline provenance a backend-neutral Core contract.
- Prevent `LiveEdge` repair from acquiring scheduler ownership before the
  active backend's current room response is committed.
- Select only the opaque gap created by that committed response.
- Recover deterministically from lost relay correlation and missing render
  acknowledgement while preserving queued `LiveEdge` work.
- Project `Running` only after the active backend has produced a successful
  response.
- Prove contiguous, duplicate-free recovery with deterministic Linux tests.

## Non-Goals

- Do not change ordinary viewport/manual historical-gap selection.
- Do not optimize member lazy-loading or persisted-history scanning without
  measurement from the functional regression.
- Do not move Matrix continuity decisions into React or Tauri.
- Do not persist response checkpoints across process restart.
- Do not expose room IDs, event IDs, gap identities, tokens, bodies, or raw SDK
  errors in diagnostics or normal `Debug` output.

## Architecture

### Backend-neutral checkpoint

Core uses one closed `MatrixCommittedRoomTimelineCheckpoint` adapter type for
both backends. It contains:

- a Core-owned backend epoch;
- the active actor generation and room routing key outside normal `Debug`;
- whether the committed response mutated the room timeline;
- whether that mutation introduced a gap; and
- the exact opaque SDK gap descriptor when present.

SyncService continues to obtain the underlying committed observation from its
RoomListService subscription checkpoint. Legacy `/sync` publishes an
equivalent checkpoint only after the event cache has committed the room update.
The SDK fork supplies a retained, monotonically sequenced committed-room
observation stream so a consumer that subscribes after a fast commit can replay
the latest per-room value. Empty/no-timeline responses publish a checkpoint
without reusing an older observation. SyncService response identity combines
the subscription generation with the room event-cache observation sequence,
because one subscription generation covers multiple sync responses.

TimelineManager owns the active backend epoch and the sole observation task.
Backend replacement aborts the old task, increments the epoch, clears retained
checkpoints, and fences queued delivery. Manager routing requires backend
epoch, room, and actor generation to match. Timeline construction and initial
projection remain non-blocking.

### Live-edge gate

A room actor queues `LiveEdge` on open but does not begin inspection until the
current checkpoint arrives. Once it arrives:

- the exact current-response gap is the only descriptor eligible to start a
  repair chain;
- that opaque descriptor remains retained until the first bounded repair is
  admitted, even when later responses are empty or also limited;
- after the exact first batch makes progress, continuation follows only the
  newest gap in the updated topology until its boundaries join, then returns
  to normal automatic policy rather than touching unrelated history;
- a response with no timeline mutation or no gap closes this `LiveEdge`
  decision without selecting historical topology;
- a stale/absent descriptor causes one re-inspection, then an explicit closed
  stale decision that clears the old checkpoint so the next committed response
  can be admitted; the latest checkpoint delivered during that attempt is
  retained separately and promoted without waiting for another response; and
- actor/backend replacement discards the checkpoint and preserves a fresh
  `LiveEdge` trigger for the replacement generation.

The `Unsupported` bypass is removed for production backends. A missing
provenance producer is a pending/recoverable state, not permission to guess.

### Relay and render settlement recovery

Each relay/render fence owns actor, timeline, and repair generations plus a
bounded actor timer. Completion, actor replacement, unsubscribe, logout, or
matching timeout cancels the timer.

On relay overflow, stale generation, missing tagged publication, or timeout,
the actor clears the obsolete relay correlation and pending projection,
preserves the highest-priority queued trigger, and schedules an authoritative
resync followed by re-inspection. On render-ACK timeout or generation change it
clears the obsolete render fence, queues the saved trigger, and re-inspects the
authoritative SDK topology. A newer live event may queue work but never settles
continuity by itself.

### Truthful lifecycle

Launching a legacy task leaves actor lifecycle and projected state in
`Starting` (or `Reconnecting` after a proven run). The first successful legacy
stream item sends an internal success notification to SyncActor. Only that
notification sets actor lifecycle to `Running`, completes the room/timeline
dependent handoff, then projects `SyncLifecycleStatus::Running` and emits
`SyncEvent::Running`. Idempotent `Start` re-emits `Running` only after that
proof. After a proven run, generation-fenced backend controls move the actor to
`Reconnecting` before projecting that state and return it to `Running` only
after a successful recovery response; idempotent `Start` cannot synthesize a
recovery.

## Error Handling

- Observation stream lag triggers a versioned resubscribe/full checkpoint
  replay; it never substitutes an arbitrary gap.
- A checkpoint for a stale backend epoch, actor generation, room, or topology
  is discarded without mutating the active repair.
- Lost relay/render settlement becomes a private-data-free coarse failure and
  a queued retry, never a permanent owner.
- Repeated repair with unchanged topology/zero progress closes under the
  existing bounded batch rules.

## Verification

The primary RED test is a deterministic real-actor fixture with an unrelated
old gap, automatic SyncService-to-legacy fallback, delayed first legacy commit,
an offline interval larger than the live limit, and one later live event. It
asserts that stale topology is never selected, the exact committed-response gap
is repaired, all expected event IDs occur once, and scheduler state returns to
idle.

Focused coverage also includes SDK retained observation replay, no-timeline
responses, privacy-safe `Debug`, backend/actor generation rejection, relay
overflow, missing render ACK, lifecycle before/after first success, the existing
SyncService checkpoint regression, and both backend headless reconnect lanes.

macOS remains the final production-state validation environment after Linux
automation passes; no real-account identifiers or content enter artifacts.
