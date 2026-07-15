# State-Driven Timeline Backfill Design

**Date:** 2026-07-15
**Status:** Approved for implementation planning

## Context

Room entry no longer floods the desktop with every persisted timeline gap, but
an intermittent follow-up failure remains: an eligible automatic history load
can stay idle until another interaction, most visibly the header's live-edge
down-arrow, changes viewport state. The interaction does not call backward
pagination directly. It scrolls to the live edge and reports the new viewport,
so its ability to appear to “unstick” loading points to a missed scheduler
wake-up rather than a missing button handler.

The current decision is split across mutable refs, layout effects, scroll
handlers, Core actor messages, pagination terminal events, and repair render
acknowledgements. In particular:

- `TimelineView` retains a sticky
  `autoBackfillRequiresUserScrollRef` after restoring an in-session room
  anchor. Only a genuine user scroll reliably clears it.
- underfilled loading and near-top loading use separate call sites and do not
  share one complete eligibility decision.
- a blocked evaluation is retried only if one of the dependencies or handlers
  that happens to contain a retry path runs later.
- Core updates `viewport_observation` on `ObserveViewport`, but the message is
  not itself a gap-repair scheduler wake-up.
- diff delivery, pagination completion, projection acknowledgement, and repair
  render acknowledgement can incidentally wake the scheduler. Whether one of
  those events arrives after eligibility changes depends on timing.

This explains why the failure is intermittent: identical final state can
produce different work depending on event order.

## Goals

- Make automatic timeline work a deterministic function of settled state, not
  of which event happened to arrive last.
- Guarantee that every transition which can change eligibility re-runs the
  appropriate owner’s evaluator.
- Keep at most one backward pagination and one actor-owned gap operation in
  flight under their existing mutual-exclusion rules.
- Preserve the render-settled pacing and bounded repair budgets introduced by
  the room-entry gap-repair fix.
- Make an ineligible or stalled decision diagnosable without reproducing it
  under a debugger.
- Keep the live-edge control a navigation action, never a special fetch action.

## Non-Goals

- Merging ordinary backward pagination and persisted-gap repair into one
  network implementation.
- Moving viewport or virtualization ownership from `TimelineView` to Core.
- Repairing unprojected off-screen gaps automatically.
- Increasing the automatic repair cache-chunk budget above zero.
- Changing the manual Room-info repair action.
- Changing sidebar filters, room naming, read-marker semantics, or anchor
  persistence.
- Adding a polling timer or periodic watchdog.

## Ownership

The existing ownership boundary remains explicit:

| Owner | Work | Authoritative inputs |
| --- | --- | --- |
| `TimelineView` | ordinary backward pagination | committed virtual layout, viewport metrics, pagination state, user settings |
| Core Room timeline actor | persisted-gap inspection and repair | SDK gap descriptors, canonical timeline items, viewport observation, actor scheduler state |
| SDK | cache/network gap operation and causal projection tags | event-cache linked chunks and pagination tokens |

No layer may infer another layer’s private facts. The desktop does not inspect
SDK gap descriptors, and Core does not infer DOM settlement.

## Trigger Model

Each owner exposes one `evaluate` entry point. Events do not decide or start
automatic work themselves; they update state and invoke the owner’s evaluator.

### Desktop evaluation triggers

The ordinary-pagination evaluator runs after:

- the initial timeline projection commits;
- projection and virtual-range settlement completes;
- room-anchor restoration completes or falls back to live edge;
- a genuine user scroll commits a new viewport range;
- a backward pagination terminal state arrives;
- a prepend batch commits and anchor compensation completes;
- a resync replay clears `awaitingResync`;
- the automatic-load setting changes; and
- a timeline key or generation reset installs its initial state.

Programmatic scroll events remain classified and ignored as scroll *triggers*.
Their owning operation instead invokes evaluation after its settlement point.
This prevents duplicate requests without requiring a sticky “wait until the
user scrolls” flag.

### Core evaluation triggers

The gap-repair evaluator runs after:

- the initial projection acknowledgement is accepted;
- a relevant SDK diff batch updates canonical items;
- ordinary pagination completes;
- anchor restoration releases scheduler ownership;
- a repair projection receives its matching render acknowledgement;
- a no-diff repair operation completes; and
- a settled viewport observation changes the projected gap candidate.

Core coalesces triggers while inspection, repair, pagination, anchor restore,
projection correlation, or render settlement owns the scheduler. Releasing an
owner immediately evaluates the coalesced trigger.

## Desktop Ordinary-Pagination Evaluator

### Pure decision

A pure domain function receives an immutable snapshot and returns one of:

- `request(demand)`;
- `blocked(reason)`; or
- `idle(reason)`.

The snapshot contains:

- timeline key and generation;
- Room/Thread presentation kind;
- initialized and awaiting-resync state;
- automatic-load setting;
- pagination direction state;
- request-in-flight state;
- projection, virtual-range, prepend-anchor, and room-anchor settlement state;
- item revision and projected total height;
- `scrollTop`, `scrollHeight`, and `clientHeight`;
- effective near-top threshold; and
- evaluation trigger kind.

The evaluator has no refs, timers, transport calls, React state setters, or
diagnostic side effects.

### Demand

The evaluator recognizes three ordinary-pagination demands:

1. `underfilled`: automatic loading is enabled, the layout is settled, and
   both DOM and height-model projections do not fill the viewport.
2. `near_top_prefetch`: automatic loading is enabled and the settled viewport
   is within the existing prefetch threshold.
3. `explicit_top_scroll`: a genuine user scroll reaches the exact top while
   automatic prefetch is disabled, preserving the current manual-scroll
   behavior.

A live-edge jump is never a demand. If its completed layout happens to be
underfilled, the normal post-settlement evaluation observes that fact; the
button itself still starts no fetch.

### Blockers

The evaluator blocks a demand while any of these facts is true:

- the timeline is not initialized or is awaiting resync;
- pagination UI is suppressed for the presentation;
- projection, virtual range, prepend-anchor compensation, or room-anchor
  restoration is unsettled;
- a backward request is already in flight; or
- backward pagination is `Paginating` or `EndReached`.

`Paginating` or an observed prepend proves that Core accepted the request. Core
also compares the observable oldest event before and after the SDK call and
attaches `prepend_expected` to the accepted terminal after releasing task
ownership. `prepend_expected=false` settles without waiting for a UI diff, so a
filtered or aggregation-only page can continue automatically. An `Idle` terminal
without either acceptance proof means the command was not accepted by the active
scheduler. `Failed`, unaccepted `Idle`, and transport rejection clear the local
in-flight token and re-evaluate only on the next external state transition; a
gap-position projection explicitly supplies that wake after gap-scheduler work.
They do not spin.

### Request token and deduplication

The effectful wrapper assigns a local request epoch when it sends a `request`
decision. `Paginating` or the prepend projection marks that epoch accepted. An
accepted epoch with `prepend_expected=true` remains active until the matching
backward pagination terminal and prepend have both arrived, or until reset. A
confirmed no-prepend terminal settles it directly. Promise resolution alone
does not complete the operation because Core projection may still be in transit.

After a prepend-producing terminal, the next evaluation waits for projection
and anchor compensation to settle. A no-prepend terminal re-evaluates directly.
If the viewport is still underfilled, a new epoch may request one additional
page. This permits paced filling to `EndReached` without concurrent or
same-epoch duplicates.

The sticky `autoBackfillRequiresUserScrollRef` is removed. Restore-generated
scroll echoes are already suppressible by programmatic-scroll classification;
room-anchor settlement becomes the explicit wake-up.

## Core Gap-Repair Evaluator

Core retains the existing `TimelineGapRepairTracker`, causal projection
correlation, five-second observable-settlement timeout, and render fence. The
change is to make scheduler wake-ups explicit and candidate-aware.

The actor retains a private snapshot of the latest inspection’s projected gap
positions and their inspection/item revision. A viewport observation is
relevant only when it changes the projected candidate selected by the existing
viewport-first, live-edge-nearest policy. Raw scroll ticks that keep the same
candidate do not inspect again.

If a relevant trigger arrives while another owner is active, the tracker keeps
one coalesced automatic inspection request. Manual continues to outrank
automatic. When ownership is released, the same `evaluate` entry point begins
at most one inspection.

The following safety properties remain unchanged:

- automatic repair uses an event limit of 64 and a cached-chunk limit of zero;
- manual repair uses an event limit of 64 and a cached-chunk limit of one;
- an unprojected gap has no invented index-zero position;
- a diff-producing repair waits for its exact SDK causal projection and
  desktop render acknowledgement;
- stale actor, timeline, repair, and batch generations are rejected; and
- a zero-budget deferred automatic repair stops until a meaningful cache,
  pagination, diff, or candidate transition occurs.

## State Flow

For either owner, the scheduling contract is:

1. Apply the incoming transition to authoritative state.
2. Evaluate the complete immutable snapshot.
3. If idle or blocked, record the deduplicated reason and do nothing.
4. If eligible and no owner is active, claim one operation epoch and start one
   operation.
5. On completion, apply the terminal state before releasing ownership.
6. Wait for required projection/layout settlement.
7. Re-evaluate once from the final settled state.

No timer is required. A blocked condition must name the transition that will
wake it when the condition clears.

## Diagnostics and Privacy

Both evaluators record coarse, private-data-free decisions.

Desktop entries use source `timeline.backfill_evaluation` and include:

- `trigger`;
- `decision=request|blocked|idle`;
- `demand=underfilled|near_top_prefetch|explicit_top_scroll|none`;
- one stable `reason` token;
- item count and rounded viewport/height-model metrics;
- pagination state; and
- request epoch.

Core entries use source `core.timeline_gap_repair` with stage `evaluation` and
include:

- trigger kind;
- decision and blocker;
- projected gap count;
- whether the candidate changed;
- scheduler phase; and
- generation/batch counters already permitted by policy.

Repeated identical decisions are deduplicated by a diagnostic signature. Logs
must not contain room, event, user, or transaction IDs; message bodies;
pagination tokens; display names; or raw SDK errors.

## Failure Handling

- Transport rejection releases only the desktop request epoch, records a
  retryable failure, and blocks another attempt until a new external transition.
- Core inspection/repair failure follows the existing typed continuity failure
  path and releases actor scheduler ownership.
- Projection or render timeout remains bounded and retryable.
- Resync, unsubscribe, account replacement, timeline-key change, and actor
  replacement invalidate request epochs, candidates, and pending evaluations.
- A diagnostic callback failure cannot affect eligibility or scheduler state.

## Testing

### Pure desktop policy

Table-driven tests cover every demand and blocker, including:

- restored anchor settled near the top;
- restored anchor settled in the middle;
- underfilled initial projection;
- height-model overflow with transient DOM underfill;
- automatic loading disabled with user scroll at exact top;
- `Paginating`, `EndReached`, `Failed`, and resync states; and
- live-edge jump as a non-demand trigger.

### TimelineView integration

Tests use real timeline events and layout frames to prove:

- a restored, underfilled room requests exactly one page without requiring a
  subsequent user scroll;
- a programmatic room-restore scroll echo does not duplicate that request;
- pagination terminal arriving before or after a prepend diff produces the
  same next decision;
- a second page is requested only after prepend anchor restoration settles;
- a live-edge click does not directly call `paginateBackwards`;
- resync blocks evaluation until replayed `InitialItems`; and
- diagnostics expose the exact blocker for every skipped eligible demand.

### Core actor policy

Tests prove:

- all scheduler-release transitions invoke one evaluator;
- viewport observations with the same candidate do not re-inspect;
- a changed visible candidate queues one coalesced inspection;
- manual trigger precedence remains intact;
- zero-budget automatic deferral does not spin;
- render ACK identity fences remain exact; and
- candidate and pending state are discarded on generation replacement.

### Event-order permutations

A deterministic permutation test feeds the same final facts in representative
orders: projection then viewport, viewport then projection, pagination
terminal before diff, diff before terminal, and render ACK before or after an
unrelated viewport update. Every order must converge to the same request count,
active owner, and final decision.

## Acceptance Criteria

- An eligible underfilled or near-top timeline cannot remain idle solely
  because no later incidental diff arrives.
- Restoring a room anchor never requires an unrelated user action to wake
  automatic loading after settlement.
- The header live-edge arrow initiates zero pagination or repair operations
  directly.
- No more than one backward pagination request is active for a timeline.
- Automatic gap repair retains zero cached-chunk hydration and render-ACK
  pacing.
- Event-order permutation tests converge deterministically.
- A captured diagnostic report identifies the last evaluator trigger,
  decision, and blocker even when the user cannot reproduce the issue again.
