# Live-Edge Timeline Gap Repair Design

**Date:** 2026-07-16  
**Status:** Approved for implementation  
**Issue:** #269

## Context

The persisted event cache can prove that a Room timeline is gapped even when
neither raw boundary event has a standalone rendered `TimelineItem`. Reactions
and replacements are the important case: the SDK aggregates the relation into
its owning message row, so Core cannot project the raw boundary event ID.

The room-entry pacing policy intentionally makes ordinary `Automatic` repair
stop when no gap is projected. That protects historical off-screen cache from
being materialized during room entry, but it also prevents the newest gap from
being repaired. A room can therefore show an older cached window and later
live events while permanently omitting the interval between them.

## Goals

- Make an opened Room timeline converge to its complete live edge without a
  jump-to-latest click.
- Repair the newest persisted gap when its boundary is a non-rendered relation.
- Keep viewport-driven historical repair zero-cache and bounded.
- Preserve SDK-owned descriptors, non-destructive repair, causal projection
  correlation, render acknowledgements, and all generation fences.
- Stop live-edge recovery on no progress, stale topology, or a small batch
  ceiling, with private-data-free diagnostics.

## Non-Goals

- Automatically repairing every historical off-screen gap.
- Turning jump-to-latest into a fetch command.
- Inventing a synthetic position for an unprojected gap.
- Exposing gap handles, tokens, boundary IDs, or live-edge event IDs outside
  the Room timeline actor.
- Replacing ordinary backward pagination or manual repair.

## Considered Approaches

### 1. Distinct `LiveEdge` intent (selected)

Add `LiveEdge` between `Automatic` and `Manual` in scheduler priority. It uses
the existing actor task and fences, prefers a projected candidate, and falls
back only to the newest SDK descriptor. This preserves the historical policy
while repairing the product-critical live edge.

### 2. Let all automatic repair fall back to the newest descriptor

This is smaller, but it erases the distinction between viewport demand and
room-entry continuity. Every viewport wake could reveal off-screen cached
chunks and recreate the room-entry saturation fixed by #263.

### 3. Make relation events projectable as synthetic rows

This couples SDK relation aggregation to presentation and would expose rows the
timeline intentionally does not render. It also does not solve other kinds of
unprojected live-edge boundaries.

## Ownership and Trigger Model

The Core Room `TimelineActor` owns live-edge recovery. At actor creation it
captures the newest rendered event-row identity as an actor-private target and
queues `LiveEdge` only after the initial projection acknowledgement. For an
aggregated edit or reaction, the SDK timeline row already carries the owning
event identity, so the target is the relation owner rather than the raw
relation event.

After an accepted live diff batch, Core recomputes that target. A changed
target queues one coalesced `LiveEdge` inspection; an unchanged target does not.
Manual remains highest priority. Thread and focused timelines do not run this
Room-only policy.

The target proves the actor has a rendered live edge; it is never logged or
serialized. Empty Room timelines retain ordinary continuity behavior and do
not use an unconstrained newest-gap fallback.

## Descriptor Selection

Inspection keeps SDK order, which is oldest to newest.

1. `Automatic`: select only the viewport-first projected candidate, then the
   projected candidate nearest the live edge. No projection means `offscreen`.
2. `LiveEdge`: use the same projected selection first. If none is projectable
   and the actor has a live-edge target, select `gaps.last()` and record
   `live_edge_fallback`.
3. `Manual`: preserve the projected selection and existing newest-descriptor
   fallback.

An unprojected descriptor still produces no WebView gap row.

## Budgets and Progress

- `Automatic`: 64 events, zero cached chunks per request, existing global
  batch ceiling.
- `LiveEdge`: 64 events, at most one cached chunk per request, at most four
  repair batches for one actor generation.
- `Manual`: 64 events, one cached chunk per request, existing global ceiling.

The SDK adapter exposes only the descriptor's coarse topology revision to
Core; handles, tokens, and IDs remain private. Before a live-edge repair Core
records `(revision, ordinal)`. On the next live-edge inspection, the same
selection after a completed batch is `no_progress` and terminates. SDK
`Stale`, `Deferred { cached_chunks_loaded: 0 }`, and zero-event `Progress`
outcomes also terminate without immediate requeue. A changed revision or a
positive bounded outcome may continue after its exact projection/render fence.

Reaching four live-edge batches records `budget_exhausted` and leaves the
timeline retryably incomplete. A later changed live-edge target may start a
fresh bounded attempt; an unchanged target cannot spin.

## Projection and Failure Semantics

All successful diff-producing operations retain the current causal path:

1. SDK publications carry actor/repair/publication identity.
2. Core correlates the final publication to its exact desktop batch.
3. React renders and acknowledges the matching actor/timeline/repair/batch
   fence.
4. Only then may Core re-inspect.

No-diff results are evaluated for progress before reinspection. Failures,
stale descriptors, no progress, budget exhaustion, actor replacement, logout,
and generation changes preserve existing events and release scheduler
ownership. No normal path removes or resets the room cache.

## Diagnostics and Privacy

`core.timeline_gap_repair` uses trigger token `live_edge` and coarse outcomes:
`projected`, `live_edge_fallback`, `no_progress`, and `budget_exhausted`.
Diagnostics may include counts, ordinals, revisions, generations, and batch
counts. They must not include room/event/user/transaction IDs, message bodies,
pagination tokens, display names, or raw SDK errors.

## Verification

Verification is test-first:

- Core policy tests prove trigger priority, the independent budget, newest-gap
  fallback only with a live-edge target, unchanged-topology termination,
  target-change rearming, and exact render fences.
- SDK adapter tests prove that topology revision is available without exposing
  tokens or identifiers through `Debug`.
- A deterministic actor/SDK fixture models an older cached event, a newest
  persisted gap with an unprojected relation boundary, a missing cross-client
  event, and a newer live event. The final projection contains the missing
  event exactly once in order.
- Existing tests continue to prove that ordinary automatic repair does not
  materialize an unprojected historical gap.

## Acceptance Criteria

- Opening a gapped Room repairs the missing live-edge interval without user
  interaction, including non-rendered relation boundaries.
- Historical unprojected gaps remain outside ordinary automatic repair.
- Repair is asynchronous, non-destructive, ordered, and duplicate-free.
- No-progress, stale, and budget-exhausted paths terminate and release the
  scheduler.
- Diagnostics distinguish viewport projection from live-edge fallback and its
  stop reasons without leaking private data.
