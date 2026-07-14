# Room Timeline Gap Repair Design

## Status

Approved for issue #260. This revision supersedes the earlier design that
permitted a destructive manual room-cache reset.

## Goal

Detect and repair SDK-represented room timeline discontinuities without
deleting known events, copying pagination tokens into Koushi, or inferring
continuity from timestamps, event IDs, or visually adjacent rows.

## Ownership

- The Matrix SDK owns persisted linked chunks, pagination tokens, gap
  serialization, deduplication, cache mutation, and proof that a selected gap
  joined its boundaries or reached the authoritative room start.
- `koushi-sdk` maps the SDK API into an app-owned, token-free Rust contract.
  SDK gap descriptors remain actor-private and never enter `AppState`, IPC,
  diagnostics, or Koushi persistence.
- Core owns one per-room scheduler: trigger coalescing, priority, bounded work,
  retry/backoff, cancellation, and account/timeline-generation guards.
- `koushi-state` owns the user-visible continuity state. React renders this
  projection and dispatches repair/navigation commands only.
- The timeline event stream owns a content-free positioned gap row. React does
  not derive gaps by comparing neighboring events.

## SDK Contract

The pinned SDK fork exposes:

- `inspect_timeline_gaps`, returning `Unknown`, `Gapped`, or `Complete` plus
  gaps ordered from oldest to newest;
- an opaque, snapshot-scoped descriptor for each gap, with Rust-only boundary
  information and no exposed pagination token;
- `repair_timeline_gap(descriptor, budget)`, returning `Stale`, `Deferred`,
  `Failed`, `Progress`, `BoundariesJoined`, or `StartReached`.

The SDK revalidates a descriptor before and after network I/O. `Stale` makes
Core re-inspect; it never attempts repair at an old location. Descriptors and
tokens are not persisted by Koushi. Restart recovery begins with inspection.

`Complete` is the only SDK result that proves continuity. An empty gap list
with no persisted evidence is `Unknown`, not healthy. A successful live sync
does not prove that older internal gaps are absent.

## Rust-Owned State Machine

Each active room has a monotonically generated continuity projection:

- `Unknown`: no authoritative continuity proof exists;
- `Inspecting`: SDK inspection is in flight;
- `Healthy`: SDK returned `Complete`;
- `Incomplete`: one or more explicit gaps exist;
- `Repairing`: bounded work is in flight, with coarse gap/batch progress;
- `FailedIncomplete`: repair or inspection failed while the projection remains
  known to be incomplete.

Completions apply only to matching account, timeline actor, and room-local
repair generations. Logout, account replacement, or actor replacement cancels
work and discards late completions. Reconnect and restart re-enter inspection.

An SDK `Unknown` inspection returns to `Unknown`; it does not invent a gap or a
start marker. A failed inspection after an explicit gap was known becomes
`FailedIncomplete`. A missing or corrupt token uses a supported SDK anchored
recovery path only when continuity can subsequently be proven. Until such a
path exists or succeeds, the room remains `Unknown` or `FailedIncomplete` and
all existing events stay visible.

## Scheduling Policy

Automatic inspection is requested on room subscription, limited sync,
recovery/reconnect, relevant cache/timeline updates, a new live event, and
known-event navigation. Manual **Repair room timeline** uses the same scheduler.

For one room, duplicate triggers and repeated manual clicks coalesce. Work
prioritizes the visible gap when Core has a matching viewport fact; otherwise
it chooses the gap nearest the live edge. One operation uses a bounded SDK
event and cached-chunk budget. `Progress` and `Deferred` re-inspect before the
next batch. `Stale` immediately re-inspects. Retries use the executor-backed
clock and a bounded backoff; no actor uses fixed sleeps.

Only one targeted repair operation runs for a room. SDK-side serialization also
prevents races with normal, automatic, and cache-only pagination. Live events
may arrive during repair; the SDK owns merge and deduplication.

## Projection And UI

An explicit timeline gap row is positioned by the Rust timeline projection at
the discontinuity. It contains only a stable presentation key, coarse state,
and retry affordance. It contains no room ID, event ID, boundary, token, or
message content. Pending and failed gaps remain visible until a fresh Rust
projection removes them.

Repair diffs use the existing timeline stream. The frontend's viewport helper
preserves the visible anchor when repaired events are inserted. React may
measure the viewport and report which projected gap is visible; it must not
choose a pagination token or infer continuity.

**Start of conversation** is rendered only after Rust receives SDK
`Complete`/`StartReached` proof for the active generation. Local emptiness,
pagination `EndReached`, or the absence of a visible gap row is insufficient.

Reply quotes remain clickable and keyboard-accessible. Navigation to a
known-but-absent event requests inspection/repair in addition to the existing
anchored navigation flow; fetching one event never marks its surrounding
interval healthy.

## Manual Repair And Failure Semantics

Room info replaces **Reset room timeline cache** with **Repair room timeline**.
The command never unsubscribes solely to delete cache and never calls
`EventCacheStore::remove_room`. Network error, timeout, cancellation, stale
generation, process restart, or unsupported anchored recovery preserves every
currently projected event and the SDK cache. Failure is explicit and retryable.
Credentials, E2EE state, drafts, settings, search data, and unrelated rooms are
outside the mutation scope.

## Diagnostics

Diagnostics use coarse enums and counts only:

- trigger: `limited_sync`, `cache_gap`, `missing_known_event`,
  `empty_nonempty_room`, or `manual`;
- stage and room-local generation;
- gap count and processed batch count;
- whether boundaries joined or authoritative start was reached;
- terminal success, retry, timeout, cancellation, failure, or stale rejection.

They never contain room/user/event IDs, pagination tokens, message bodies,
decrypted content, filenames, raw SDK errors, or secrets. Tests assert behavior
on state/events/DOM; diagnostics are not correctness evidence.

## Verification Contract

The implementation first adds RED headless checks for reducer guards and Core
scheduling, then turns those checks GREEN before GUI wiring. Coverage includes
limited-sync and internal gaps, multi-batch progression, live events during
repair, stale descriptors/generations, cancellation/restart, network failure,
manual-request coalescing, no `remove_room` path, authoritative start, known
event navigation, viewport preservation, and unrelated-room isolation. The
local homeserver scenario emits private-data-free success tokens only.

## Non-Goals

- Guessing a missing cursor from timestamps, event IDs, or array adjacency.
- A parallel raw `/messages` implementation in Koushi.
- Persisting SDK descriptors or pagination tokens in first-party state.
- Claiming arbitrary missing-token recovery without SDK continuity proof.
- Any room- or account-wide cache deletion as a repair strategy.
