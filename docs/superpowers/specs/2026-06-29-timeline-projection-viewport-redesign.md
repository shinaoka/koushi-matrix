# Timeline Projection and Viewport Redesign

Date: 2026-06-29
Status: design ready for implementation planning

## Context

Koushi has accumulated timeline bugs that point to an architectural problem rather
than isolated UI defects:

- timeline scroll position is non-deterministic after restart or room switches;
- mid-history viewports drift when previews, receipts, reactions, or thread
  summaries change row height;
- live-edge and Activity-target jumps can be pulled away by later layout work;
- missing thread roots can trigger confusing materialization/backfill behavior;
- reaction counts can fail to update when the current user adds a reaction key
  already used by another user;
- repeated fixes have added viewport flags without creating a single
  authoritative model.

This design was reviewed twice with Claude Code at maximum effort. The second
review broadened the scope from viewport-only to the full timeline projection
pipeline.

## Goals

1. Make timeline viewport behavior deterministic across restart, room switch,
   backfill, preview expansion, and live updates.
2. Separate canonical timeline data from derived presentation and from viewport
   control.
3. Ensure all programmatic scroll writes pass through one tested controller.
4. Make relation aggregate updates, especially reactions, observable and tested
   at the SDK/Rust boundary.
5. Stop feedback loops where persisted viewport observations become restore
   commands for the same mounted view.
6. Provide implementation slices small enough to delegate safely.

## Non-Goals

- Do not change the product decision for thread ordering in this redesign.
  Thread ordering must be represented in timeline projection, not viewport code.
- Do not solve every reaction UI enhancement in this pass. The immediate goal is
  delivery correctness for canonical reaction aggregates.
- Do not add more ad hoc flags to `TimelineView.tsx` to mask current symptoms.

## Architecture

The timeline must be split into three layers with explicit ownership.

### Layer A: Canonical Timeline Data

Rust SDK timeline diffs are the source of truth for timeline content:

- event items;
- edits and redactions;
- send state;
- media and link-preview data attached to an item;
- thread summary content;
- event-scoped relation aggregates such as reactions.

Rust projects SDK items into DTOs. A DTO replacement for an item must contain the
complete current aggregate state for that item, not only the field that changed.

React applies `InitialItems` and `DiffBatch` to an immutable canonical item
store keyed by stable item identity. The store must not reconstruct reaction
counts from button clicks or component-local state.

The current index-based diff application is a risk. The redesign must choose one
of these contracts:

1. Prefer stable-id application for item replacement/removal when the SDK diff
   target identity is available.
2. If index application remains, assert and test that the SDK vector and the
   canonical item vector have identical cardinality and ordering. Synthetic rows
   such as date dividers and read markers must not be part of that canonical
   vector.

The render list may add synthetic rows later, but canonical diff application
must not depend on render-list indexes.

### Layer B: Derived Presentation

React derives rows from canonical timeline items plus independent presentation
state:

- profiles;
- settings;
- selection/focus;
- live signals such as read receipts, typing, and presence;
- local visual state such as preview-expanded/collapsed display.

The canonical item store is authoritative for item identity, content, and
relation aggregates. Snapshot/live-signal state is authoritative only for
signals and markers. Neither stream should overwrite the other's domain.

If Layer B adds memoization, its semantic signatures must include every field
that affects content or layout:

- reaction key, total count, own reaction state/event id, sender preview;
- thread latest reply count, latest sender, and preview text;
- preview visibility and rendered dimensions;
- receipt/reaction display changes that alter row height.

Rows are allowed to defer non-critical visual expansion while the user is
actively scrolling, but canonical data must update immediately.

### Layer C: Viewport Controller

The viewport controller owns all programmatic scroll writes and layout
compensation. It does not understand Matrix semantics beyond stable row ids,
event ids, row stability, and whether an update can affect layout.

The controller exposes commands such as:

- scroll to live edge;
- scroll to target event;
- compensate a layout mutation;
- consume startup restore once;
- report visible range for persistence and coverage.

Components and effects do not write `scrollTop` directly.

## Viewport State Model

Use orthogonal axes rather than one large mode enum.

```ts
type Stickiness = "Live" | "Free";

type Seek =
  | { kind: "None" }
  | {
      kind: "Target";
      target: { eventId: string; align: "center" | "end"; source: string };
      settleTo: Stickiness;
      generation: number;
    }
  | {
      kind: "Restore";
      key: string;
      anchor: ViewportAnchor;
      settleTo: Stickiness;
      generation: number;
    };

type ScrollActivity = "Idle" | "Active";

type ViewportAnchor = {
  rowId: string;
  eventId?: string;
  edge: "bottom";
  offsetPx: number;
};
```

Controller state:

```ts
type ViewportControllerState = {
  stickiness: Stickiness;
  seek: Seek;
  scrollActivity: ScrollActivity;
  programmaticToken: number | null;
  layoutTransaction: LayoutTransaction | null;
  anchor: ViewportAnchor | null;
};
```

Do not include wall-clock timestamps in anchor identity. Persisted viewport
records may contain a timestamp for staleness only.

## Controller Invariants

1. Observation is not command.
   Persisted viewport observations are written outward and consumed only once on
   room/timeline mount or explicit restore. A prop echo for the same mounted
   timeline must not re-arm restore.

2. Programmatic scroll echo is token-based.
   Before any controller-owned scroll write, assign a monotonic
   `programmaticToken`. Classify scroll events by token/generation, not by exact
   `scrollHeight` and `scrollTop` equality.

3. User scroll activity is durable.
   `scrollActivity` becomes `Active` on wheel, touch, pointer, keyboard scroll,
   or non-programmatic scroll. It returns to `Idle` only after a debounce that
   covers inertial/momentum scroll.

4. Active user scroll owns the viewport.
   During `scrollActivity = Active`, no absolute programmatic scroll write is
   allowed except explicit user commands such as Activity click, search result,
   jump-to-date, or latest button.

5. Live stickiness is intentional.
   The latest/down button enters `stickiness = Live` and scrolls to the live
   edge. Briefly passing through the exact bottom during inertial scroll must not
   promote to Live unless the user intentionally remains near the live edge.

6. Seeking remains latched.
   Targeting does not settle after one animation frame. It settles only after
   scroll idle plus measured confirmation that the target is visible at the
   requested alignment.

7. Reset and resync supersede in-flight work.
   Restore/seek generations increment on reset, resync, room change, and
   timeline-key change. Work from older generations is ignored.

## Anchor Policy

Use one persistent anchor type:

- bottom edge of the bottom-most fully visible, height-stable row;
- fallback to the nearest visible stable row if no fully visible row exists;
- never anchor to a row that is being mutated in the same layout transaction.

This anchor is used for Free stickiness and persisted viewport restore.

Prepend compensation should not rely on re-resolving this anchor. For prepends,
prefer `newScrollHeight - oldScrollHeight` compensation, which is more robust
when virtualization unmounts the anchor row.

## Layout Mutation Protocol

All height-affecting changes are processed as batched layout transactions.

Examples:

- link preview expansion;
- image decode or natural-size reveal;
- read receipt display changes;
- reaction chip changes;
- thread summary text changes;
- font, density, or window-size changes;
- older-event prepends.

Protocol:

1. Open a transaction for the frame.
2. Capture one stable bottom-edge anchor and the relevant scroll metrics.
3. Apply all queued render/store updates for that frame.
4. Measure once.
5. Compensate once:
   - `Stickiness = Live`: if still live, scroll to bottom by relative delta.
   - `Stickiness = Free`: preserve the captured anchor by relative
     `scrollBy(delta)`.
   - `Seek != None`: target alignment wins; unrelated compensation waits until
     seek settles.

Use instant programmatic scrolls for compensation and restore. Smooth scrolling
is reserved for deliberate user navigation only.

The scroll container must use `overflow-anchor: none` so Chromium's native
scroll anchoring does not fight manual compensation.

## Visual Update Policy While Scrolling

Canonical data updates immediately. Presentation can choose one of three
strategies, in this priority order:

1. Reserve stable space when dimensions are known.
2. Render immediately with transaction compensation when the change is cheap and
   predictable.
3. Defer unknown-height non-critical expansion while `scrollActivity = Active`.

Deferred visual work must have a deterministic flush trigger:

- scroll idle;
- explicit navigation completion;
- a bounded max-defer timeout to avoid stranded updates.

This directly addresses the current mid-history symptom: while the user is
resting at an arbitrary timeline position, preview expansion or similar state
changes must not continuously move the first or bottom visible message.

## Coverage and Backfill

Backfill is data coverage, not viewport state.

The viewport controller may emit that a desired visible range is not renderable
from loaded data. Coverage code decides whether to request older/newer data.

Request dedup should be coarse and deterministic, based on:

- timeline generation;
- loaded content signature;
- visible range bucket;
- coverage mode.

Do not key backfill requests by exact pixel scroll position.

Live stickiness does not automatically backfill old thread roots because a
summary references them. Explicit thread open or context expansion may
materialize the root.

When older events are prepended, apply them through the layout mutation protocol
so the visible content does not move.

## Thread Summary and Root Policy

Thread ordering belongs to timeline projection/render modeling, not viewport
code.

If the product chooses "latest thread reply lifts the root," represent that with
a stable sort key or virtual-row policy in Layer A/B. The viewport controller
only sees rows.

If a visible thread summary references an unloaded root, render a stable
placeholder/summary. Materialize the root only for explicit thread open or
explicit context view. Missing roots must not trigger random backfill or scroll
drift.

## Reaction Update Policy

The reaction count bug is likely a delivery issue, not a viewport issue.

Requirement:

- reaction count and own-reacted state are canonical item aggregate fields;
- `toggle_reaction` is a command, not the source of the final visible count;
- final reaction display comes from SDK diff -> Rust DTO -> React canonical
  item replacement -> render row.

The local echo lifecycle must be handled:

1. A remote user has reaction key K on event E.
2. The current user toggles K on E.
3. Local echo may temporarily update own reaction state.
4. Remote echo resolves to an event id.
5. The aggregate count is not double-counted.
6. If sending fails, the optimistic state is rolled back by canonical diff.

Required diagnostic boundary:

- Add a Rust/SDK boundary RED test or headless diagnostic proving that own
  reaction on an existing reaction key emits a canonical item update carrying
  total count `N -> N + 1` and own reaction state `false -> true`.

React memo signatures for reactions are a regression guard for future Layer B
memoization. They are not the presumed fix for the current bug.

## Persistence, Quit, and Restart

Persist viewport only on:

- scroll idle;
- room switch;
- app quit;
- explicit navigation settle.

Do not synchronously persist every scroll event.

Persisted value:

- stable anchor identity;
- desired stickiness;
- timestamp for staleness only.

Startup restore consumes the persisted value exactly once per
`(roomId, timelineKey, restoreGeneration)`.

On app quit, flush pending viewport observation so restart is deterministic.

If the persisted anchor cannot be materialized within budget, fallback must be
deterministic:

1. nearest loaded newer/older anchor if available;
2. otherwise live edge if desired stickiness was Live;
3. otherwise current loaded edge with an explicit recorded restore failure.

## Test Strategy

### Reducer and Controller Tests

These can run in jsdom if geometry is abstracted behind a measurement port.

- user scroll away from Live enters Free;
- inertial scroll without new wheel events remains Active and Free until idle;
- latest/down button enters Live and scrolls to bottom;
- older/up/backfill action does not enter Live;
- token-based programmatic echo survives height changes;
- a user scroll landing on a previous coordinate is not classified as echo;
- persisted anchor echo with a new timestamp does not re-arm restore;
- startup restore is consumed once only;
- seek remains latched until idle and measured target alignment;
- reset/resync cancels stale seek/restore generation;
- backfill dedup is deterministic and not pixel-band based.

### Geometry Tests

Use Playwright or an injected `LayoutMeasurementPort`. jsdom alone is not
enough for scroll geometry.

- mid-history preview expansion above the viewport preserves visible anchor
  within device-pixel tolerance;
- visual expansion during active scroll does not oscillate;
- older-event prepend preserves visible content;
- window resize/font/density change compensates Free view and sticks Live view;
- Activity jump aligns target bottom/end and does not drift after preview/render
  changes;
- no direct `scrollTop` writes exist outside the controller-owned path.

### Canonical Data Tests

- SDK/Rust DTO boundary emits reaction aggregate update for own reaction on an
  existing reaction key;
- local echo to remote echo does not double-count;
- send failure rolls reaction aggregate back;
- hidden/redacted/synthetic rows do not corrupt canonical diff application;
- React canonical item replacement produces a new item object and rerenders the
  row.

### Integration Tests

Use a local Matrix homeserver where practical.

- room with latest messages plus old-root thread reply near live edge:
  restart at Live is deterministic; visible summary does not auto-backfill the
  old root; explicit thread open materializes root;
- two users react with the same key: own reaction increments visible count and
  survives restart;
- self-react while scrolled mid-history: count updates and viewport does not
  move;
- muted/unread/activity updates do not affect viewport controller state.

## Implementation Slices

### Slice 1: Scroll Ownership Foundation

This is the first safe task for DS V4 Flash.

- Introduce monotonic `programmaticToken`.
- Add durable inertial-aware `scrollActivity`.
- Route all `scrollTop` mutations through controller-owned `scrollTo`/`scrollBy`.
- Delete coordinate-equality echo detection.
- Delete the overlapping scroll-capture suppression window if token ownership
  makes it redundant.
- Add invariant tests preventing direct scroll writes outside the controller.

No transaction protocol, no backfill rewrite, no reaction fix in this slice.

### Slice 2: Layout Transaction Batch

- Add frame-batched layout transaction capture/measure/compensate.
- Add `overflow-anchor: none`.
- Handle preview/image/receipt/reaction/thread-summary height changes through
  the protocol.
- Add Playwright or measurement-port tests for anchor stability.

### Slice 3: Restore and Persistence Cleanup

- Consume persisted viewport only once on mount/timeline generation.
- Remove wall-clock timestamps from restore identity.
- Flush viewport on room switch and quit.
- Make fallback deterministic when an anchor cannot be materialized.

### Slice 4: Coverage and Backfill Boundary

- Move backfill state out of viewport controller.
- Use range-based coverage requests and deterministic dedup.
- Ensure prepends use transaction compensation.

### Slice 5: Canonical Reaction Delivery

- Add RED test at SDK/Rust DTO boundary for same-key own reaction.
- Fix the delivery layer identified by that test.
- Add local echo success/failure coverage.

### Slice 6: Thread Projection Cleanup

- Represent thread ordering policy in canonical projection/render sort keys.
- Ensure missing roots use stable placeholders.
- Materialize roots only on explicit context/thread operations.

## Acceptance Criteria

- During active user scroll, no non-command path writes absolute `scrollTop`.
- A persisted viewport observation cannot become a restore command for the same
  mounted timeline.
- Mid-history visual updates do not move the selected visible anchor beyond
  device-pixel tolerance.
- Restart restores the same persisted viewport deterministically.
- Activity target alignment remains stable through later layout changes.
- Same-key own reactions update count and own state through canonical data
  delivery, not component-local counters.
- Tests include at least one real-layout or measurement-port suite; jsdom-only
  geometry claims are not accepted.
