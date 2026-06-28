# Timeline Viewport Determinism Design

Status: draft for review. Date: 2026-06-29.

## Goal

Make room startup and room switching deterministic. Given the same persisted
viewport state, same timeline data, same settings, and same startup event
sequence, Koushi must land on the same visible timeline range every time.

This design specifically addresses the observed failures:

- Restarting while the room was at the latest messages sometimes lands at the
  latest area, sometimes jumps backward, and sometimes shows an incomplete
  thread-summary projection until the user scrolls.
- Programmatic backfill/materialize, live-edge correction, saved-anchor restore,
  and virtual-row measurement can all write `scrollTop` during startup.
- A latest-reply thread ordering setting can require showing a very old thread
  root near the latest timeline position when a recent reply targets that root.

## Current Root Cause

The current persisted state only represents an event anchor:

```ts
room_scroll_anchors: Record<roomId, TimelineScrollAnchor>
```

That is not enough. It turns two different user intents into the same shape:

- "I am at the live edge; keep following the projected timeline bottom."
- "I am reading this specific event; keep this event edge at this pixel offset."

When the user is at the live edge, `observeViewport` can still capture the last
visible event as a durable `TimelineScrollAnchor`. On restart, React treats that
anchor as a fixed reading position. If that anchor is no longer present,
materialize/backfill can start. At the same time, initial live-edge scrolling,
virtualized height correction, prepend anchor restoration, and event targeting
can also write `scrollTop`. The final position depends on async ordering rather
than state.

The bug is therefore not primarily "which backfill threshold should we use"; it
is that viewport intent is under-modeled and scroll ownership is distributed.

## Required Invariants

1. Viewport restore starts from one explicit mode: `LiveEdge`, `Anchored`, or
   `Targeting`.
2. Only one component issues scroll commands. React effects may report facts,
   but they must not independently decide competing scroll writes.
3. During startup restore, `observeViewport` may report visible facts for
   navigation/read-marker projection, but it must not persist a replacement
   viewport mode until the startup restore is settled.
4. `LiveEdge` startup never uses viewport-driven backward pagination or anchor
   materialization.
5. `Anchored` startup is the only restore path that may materialize/backfill an
   anchor event.
6. Thread latest-reply projection is a data/projection concern, not a viewport
   backfill concern.
7. Repeated startup with the same persisted state must produce the same visible
   event range and same scroll command sequence.

## Viewport Model

Replace the event-only persisted shape with an explicit mode. Backward
compatibility with legacy `room_scroll_anchors` is not required for this cleanup;
old anchors may be ignored or dropped at schema migration time.

```ts
type PersistedTimelineViewport =
  | {
      kind: "liveEdge";
      updatedAtMs: number;
    }
  | {
      kind: "anchored";
      eventId: string;
      edge: "top" | "bottom";
      offsetPx: number;
      updatedAtMs: number;
    };
```

Runtime-only state adds a third mode:

```ts
type RuntimeViewportMode =
  | { kind: "liveEdge" }
  | {
      kind: "anchored";
      eventId: string;
      edge: "top" | "bottom";
      offsetPx: number;
    }
  | {
      kind: "targeting";
      eventId: string;
      source: "activity" | "search" | "timeline-navigation" | "read-receipt" | "manual";
      block: "center" | "end";
      settleTo: "anchored" | "liveEdgeIfBottom";
    };
```

`Targeting` is not persisted. It is a bounded transition used when the user asks
to jump to a specific event. Once the event is visible and the scroll command
has settled, the machine commits either `Anchored` or `LiveEdge`.

`Anchored` is also the runtime representation for user free-scroll away from the
bottom. The controller keeps updating the anchor after settled user scrolls, but
it does not create a separate durable "pixel scrollTop" mode. The persisted
unit remains event-relative because raw pixel scroll positions are not stable
across preview/image/thread-summary hydration.

### Anchored To LiveEdge Transition

The transition from free-scroll/`Anchored` back to `LiveEdge` must be based on
user intent, not on incidental layout changes.

Allowed transitions to `LiveEdge`:

- The user explicitly invokes jump-to-bottom / latest.
- The header navigation button whose semantic action is "jump to latest" or
  "jump to bottom" is equivalent to jump-to-bottom / latest, even if the visual
  icon changes. It transitions to `LiveEdge` directly as a user command, not as
  a side effect of a later scroll observation.
- Concretely, the down-arrow latest button performs two actions as one command:
  scroll to the latest projected timeline position and set the viewport mode to
  `LiveEdge`.
- The user performs a real scroll gesture or keyboard scroll and the settled
  viewport is within the live-edge enter threshold.
- A local send action explicitly requests live-edge follow, because sending a
  new message is a user command to participate at the latest position.

Forbidden transitions to `LiveEdge`:

- A programmatic scroll echo lands near the bottom.
- Preview, image, receipt, avatar, or thread-summary hydration changes row
  heights and makes an anchored viewport appear near the bottom.
- Startup live-edge correction, anchor restoration, or targeting completion
  fires a scroll event.
- Backfill/prepend restoration changes `scrollTop`.

Use hysteresis:

- Enter `LiveEdge` only after a user-driven settled observation with
  `distanceToBottom <= LIVE_EDGE_ENTER_PX`.
- Exit `LiveEdge` only after a user-driven settled observation with
  `distanceToBottom > LIVE_EDGE_EXIT_PX`.
- `LIVE_EDGE_EXIT_PX` must be larger than `LIVE_EDGE_ENTER_PX` so small layout
  jitter cannot flap the mode.

The exact thresholds are implementation constants, but the contract is
testable: identical non-user layout changes must never change `Anchored` to
`LiveEdge`.

## Single Scroll Owner

Introduce a viewport controller that is the only source of scroll commands:

```ts
type ViewportCommand =
  | { kind: "scrollToBottom"; reason: "startup-live-edge" | "live-edge-growth" | "manual-jump" }
  | { kind: "scrollToAnchor"; eventId: string; edge: "top" | "bottom"; offsetPx: number }
  | { kind: "scrollToTarget"; eventId: string; block: "center" | "end" }
  | { kind: "requestAnchorMaterialize"; eventId: string; maxBatches: number; eventCount: number }
  | { kind: "requestProjectionHydrate"; reason: "latest-reply-root"; eventIds: string[] };
```

`TimelineView` executes commands but does not decide between modes. Layout
effects report observations back to the controller:

- rendered item ids
- measured row heights
- viewport dimensions
- visible first/last event ids
- whether DOM is currently at bottom
- whether a command's target is mounted

The controller emits at most one active scroll command per frame and records a
command sequence id. Later layout events may refine the same command, but they
must not introduce a competing mode.

## Startup Algorithm

On room mount or restart:

1. Load `PersistedTimelineViewport` for the room.
2. Initialize the runtime controller:
   - `liveEdge` if persisted mode is `liveEdge` or absent.
   - `anchored` if persisted mode is `anchored`.
3. Suppress viewport persistence until startup settlement completes.
4. Wait for `InitialItems` or equivalent store replay for the room key.
5. If mode is `LiveEdge`:
   - Render the latest projection from the current timeline store.
   - Issue `scrollToBottom(startup-live-edge)`.
   - Do not request viewport-driven backward pagination.
   - Do not request anchor materialization.
   - After row measurement, image/preview size changes, or thread-summary
     hydration, re-issue bottom correction only if the mode is still `LiveEdge`.
6. If mode is `Anchored`:
   - If the anchor event is present, issue `scrollToAnchor`.
   - If absent, request anchor materialization/backfill through the core
     materialize path.
   - While materialization is pending, no live-edge correction is allowed.
7. Startup settles when the selected mode's command has reached a stable DOM
   target for two animation frames or a bounded timeout expires.
8. After startup settlement, enable viewport persistence and normal user-driven
   transitions.

This makes the result deterministic because the mode is selected before any
scroll command is issued, and other systems cannot rewrite the target mode.

## Thread Latest-Reply Projection

When `threadRootOrder = latestReply`, a recent reply to an old root means the
root/summary row should appear near the latest projected timeline position. This
must not be solved by viewport-driven backfill.

Required behavior:

- `LiveEdge` means "follow the bottom of the current projected timeline", not
  "follow the newest raw event id".
- If the projected latest area contains a thread root whose full event item is
  not currently hydrated, the projection layer may request bounded data
  hydration for that root.
- Hydration must not move the viewport. It can replace a placeholder/summary row
  with the full row once data arrives.
- While the full root is missing, the timeline may render a stable summary-only
  row with the latest reply summary. It must have a stable height estimate and
  stable item id so virtualization does not oscillate.
- Viewport-driven backward pagination remains forbidden in `LiveEdge`.

This separates two user expectations:

- The latest thread activity should be visible at the live edge.
- The viewport should not jump backward or start an anchor restore just because
  that activity references an old root.

## Backfill vs Hydration

Use two distinct operations:

### Anchor Materialization

Purpose: make an explicitly anchored or targeted event available so the
viewport can scroll to it.

Allowed modes:

- `Anchored`
- `Targeting`

Forbidden mode:

- `LiveEdge`

### Projection Hydration

Purpose: complete rows that the current projection has already decided are
visible or near-visible, such as latest-reply thread roots.

Allowed modes:

- `LiveEdge`
- `Anchored`
- `Targeting`

Rules:

- It never writes `scrollTop`.
- It never changes the viewport mode.
- It has bounded request counts and private-data-safe diagnostics.
- If hydration changes row heights, the active mode decides the correction:
  `LiveEdge` corrects to bottom; `Anchored` corrects to the anchor edge/offset;
  `Targeting` corrects to the target block.

## Persistence Contract

`observeViewport` should report facts separately from persisted mode updates.

Suggested command/data split:

```rust
TimelineViewportObservation {
    first_visible_event_id: Option<String>,
    last_visible_event_id: Option<String>,
    at_bottom: bool,
}

TimelineViewportPersistence {
    room_id: String,
    viewport: PersistedTimelineViewport,
}
```

React may still send observation facts often. Persistence should be committed
only when the viewport controller says the current mode is user-settled:

- `LiveEdge` is persisted when the user reaches bottom or explicitly jumps to
  bottom.
- `Anchored` is persisted when the user scrolls away from live edge and the
  controller captures a stable anchor.
- During startup restore, targeting, anchor materialization, and command echo
  suppression, persistence is blocked.

This prevents startup observations from replacing a clean persisted mode with an
accidental event anchor.

## Headless And Local Homeserver Tests

Testing must cover both pure determinism and real Matrix timeline behavior.

### Pure State Machine Tests

Add tests for the viewport controller/reducer:

- `LiveEdge` startup emits only `scrollToBottom`; no anchor materialize or
  backward pagination command is emitted.
- `Anchored` startup emits `scrollToAnchor` when the event is loaded.
- `Anchored` startup emits `requestAnchorMaterialize` when the event is absent.
- During startup settlement, observations do not persist a replacement mode.
- Once settled, user scroll away from bottom captures `Anchored`.
- User scroll to bottom captures `LiveEdge` only after a user-driven settled
  observation within `LIVE_EDGE_ENTER_PX`.
- Explicit latest/bottom navigation captures `LiveEdge` immediately through the
  command path, without waiting for scroll-observation inference.
- Programmatic scroll echoes, startup restore, backfill/prepend correction, and
  layout-only height changes never transition `Anchored` to `LiveEdge`, even if
  the viewport becomes numerically close to the bottom.
- `LiveEdge` exits only after a user-driven settled observation beyond
  `LIVE_EDGE_EXIT_PX`, with `LIVE_EDGE_EXIT_PX > LIVE_EDGE_ENTER_PX`.
- `Targeting` transitions to `Anchored` after the target event is mounted and
  scrolled into position.
- Row-height changes re-emit only the active mode correction command.

### React Headless DOM Tests

Use the existing `TimelineView` jsdom/headless tests to verify command effects:

- Same initial `LiveEdge` state + same `InitialItems` produces the same
  `scrollTop` and visible event ids across repeated runs.
- `LiveEdge` with later preview/image/receipt/thread-summary height changes
  stays at bottom and does not request backward pagination.
- `Anchored` with height changes preserves event edge/offset.
- `Targeting` from Activity scrolls once to the target and then settles to an
  anchor.
- Programmatic scroll echo does not persist a new viewport mode.
- Anchored/free-scroll restores do not automatically reattach to `LiveEdge`
  unless a real user scroll gesture settles inside the live-edge enter
  threshold.

### Local Matrix Homeserver QA

Extend `scripts/desktop-headless-local-qa.mjs` and the Rust QA binary with a
`timeline_viewport_determinism` scenario. Run it against the disposable local
homeserver path already used by `qa:headless-local`.

Scenario A: Live edge with latest reply to old root.

1. Start a local homeserver.
2. Create a room.
3. Send an old thread root message.
4. Send enough normal messages that the root is outside the initial loaded
   latest range.
5. Send a thread reply to the old root as the latest activity.
6. Configure `threadRootOrder = latestReply`.
7. Persist viewport mode as `LiveEdge`.
8. Start Koushi and subscribe to the room.
9. Assert:
   - the runtime viewport mode remains `LiveEdge`;
   - no viewport-driven backward pagination is requested;
   - no anchor materialization is requested;
   - the latest-reply thread summary/root placeholder or hydrated row appears in
     the latest projected range;
   - the visible first/last event ids and command sequence are stable across
     repeated startup runs.

Scenario B: Anchored old event.

1. Use the same room history.
2. Persist viewport mode as `Anchored(oldRoot, bottom, offset)`.
3. Start Koushi.
4. Assert:
   - anchor materialization/backfill is requested if the root is not loaded;
   - the anchor event lands at the requested edge/offset;
   - no live-edge bottom correction runs while anchoring.

Scenario C: Non-determinism detector.

Run Scenario A and B 10-30 times with the same stored profile. Record
private-data-safe tokens:

- viewport mode
- scroll command kinds and counts
- pagination/materialize/hydration request counts
- visible range ordinal hashes
- whether thread latest-reply row/placeholder is present

The run fails if any token differs between iterations.

## Diagnostics

Add private-data-safe diagnostics for the transition:

- `viewport.mode=liveEdge|anchored|targeting`
- `viewport.command=scrollToBottom|scrollToAnchor|scrollToTarget|materialize|hydrate`
- `viewport.persist=blocked|liveEdge|anchored`
- `viewport.startup=restoring|settled|timeout`
- `timeline.projectionHydrate.reason=latestReplyRoot count=N`
- `timeline.anchorMaterialize.reason=anchoredRestore|targeting count=N`

Diagnostics must not include raw room ids, event ids, user ids, message bodies,
paths, homeserver URLs, or SDK error bodies.

## Migration

Compatibility is intentionally not a constraint for this cleanup. On schema
change:

- Drop legacy `room_scroll_anchors`.
- Initialize rooms without a stored viewport as `LiveEdge`.
- Persist the new explicit viewport mode after the first user-settled
  observation.

This avoids converting ambiguous old anchors into new deterministic modes.

## Out Of Scope

- Changing Matrix read-marker semantics.
- Automatically opening rooms at first unread on startup.
- Replacing the Matrix SDK timeline implementation.
- Persisting full DOM scroll positions or cached DOM nodes.
- Making thread latest-reply root hydration perfect in the first slice; a stable
  summary placeholder is acceptable as long as the viewport is deterministic.

## Acceptance Criteria

- Restarting a room at `LiveEdge` is deterministic across repeated local runs.
- A latest-reply thread row whose root is old appears near the projected latest
  range without viewport-driven backward pagination.
- Restarting an `Anchored` room deterministically restores the anchor or reports
  a bounded not-found state.
- `scrollTop` writes are traceable to one viewport command stream.
- Startup observations cannot overwrite the persisted viewport mode before
  settlement.
- Unit, React headless, and local homeserver QA all cover the old-root
  latest-reply case.
