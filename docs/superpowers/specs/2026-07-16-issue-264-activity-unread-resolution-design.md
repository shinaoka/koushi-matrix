# Activity Unread Resolution Design (#264)

## Goal

Make Home → Activity → Unread converge from room-level attention to actual
event-backed message rows, including stale/encrypted history, without moving
unread semantics or message synthesis into React.

## Root Cause

The current implementation has two independent gaps:

1. `ActivityProjection::snapshot` computes `unread_row` for observed and
   room-summary latest events but appends those rows only to `recent`. It then
   appends a `RoomUnread` placeholder for every unread room, so even known
   event rows cannot appear in the Unread stream.
2. `ActivityRowsObserved` is produced only by subscribed room
   `TimelineActor`s. `PaginateActivity` merely republishes `snapshot()` and
   performs no history request, so a stale unread room that was never opened
   can never acquire event rows.

## State Contract

`ActivityStream` gains a coarse `resolution` field:

```rust
pub enum ActivityResolutionState {
    Idle,
    Resolving { generation: u64, unresolved_room_count: u32 },
    Failed {
        generation: u64,
        unresolved_room_count: u32,
        failure_kind: OperationFailureKind,
    },
}
```

The field exposes no room/event/user ids, content, cursor, or raw SDK error.
`RoomUnread` remains a transient typed placeholder while `Resolving`, and may
remain as context beside a `Failed` state. It is never presented as completed
Unread content. A fully resolved stream is `Idle` and contains no placeholder
for a room with authoritative unread attention.

Generation is bumped for open/retry/session replacement. Late results are
ignored. Close, logout, lock, account switch, and a newer generation cancel the
active task. Retry is an explicit typed `RetryActivityResolution` command;
`PaginateActivity` remains stream pagination and no longer masquerades as a
resolver.

## Projection Rules

Observed event rows and `RoomSummary.latest_event` rows are inserted into both
Recent and Unread when the existing Rust unread-boundary calculation marks them
unread. A room placeholder is synthesized only when no event-backed unread row
exists for that room.

When Activity is open and unresolved rooms exist, `AppActor` sends a private
internal resolution request to `AccountActor`. The request carries the room id,
fully-read marker, minimum unread count, and generation; its `Debug` output is
redacted. `AccountActor` owns the Matrix session and a single cancellable
resolution task. Each generation resolves at most 16 rooms serially and uses
the shared account `/messages` backpressure gate. Per-room successes are emitted
even when another room fails; the remaining placeholder count stays retryable.

The SDK resolver builds a decrypted room timeline, consumes cached items first,
then paginates backward in pages of 50 until it reaches the fully-read marker,
reaches the server start, or consumes 32 pages. It merges live diffs received
during pagination by event id. A missing marker with remaining budget is
retryable; budget exhaustion is a coarse timeout failure. Network/SDK failures
retry twice with bounded backoff before becoming `Failed`. No fixed sleep is
used for correctness checks.

Successful rows are ingested through the same `ActivityProjection` path as
normal TimelineActor observations. The projection re-evaluates current room
counts, mute/low-priority policy, markers, cleared events, and profile labels,
so stale results cannot manufacture unread state. Recent remains unchanged
except for gaining legitimate shared event infrastructure rows. Viewing or
resolving Unread never marks anything read.

## GUI Contract

React renders event rows exactly as today. During `Resolving`, placeholders are
replaced visually by one loading/status surface; during `Failed`, they are
replaced by a retryable error surface. React dispatches only
`retry_activity_resolution` and does not join room state, inspect timelines, or
construct message rows.

## Verification

- State tests cover generation guards, resolving/failed/retry transitions,
  session clearing, and wire/Debug privacy.
- Core tests first reproduce the known-row-to-placeholder defect, then cover
  stale bounded backfill, encrypted rows, live-during-resolution dedupe,
  reconnect generation replacement, mark-room/all-read, cancellation, and
  terminal retryable failure.
- Browser tests assert sender, context, preview, timestamp, unread/highlight,
  loading, failure, and retry command shape; completed known unread content may
  not consist only of `roomUnread` rows.
- Local headless QA records token-only resolution evidence.
