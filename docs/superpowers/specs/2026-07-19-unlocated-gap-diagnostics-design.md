# Unlocated Foreground Gap Diagnostics Design

## Problem

Real-account diagnostics from 2026-07-18 show the same failure shape in two
selected DM rooms:

- Core inspection finds four persisted gaps;
- no gap can be projected into the current navigation items;
- the UI consequently reports no visible gap IDs;
- no selection, attempt admission, or repair batch starts;
- the inspection terminates as offscreen.

The current records prove that repair did not start, but they do not distinguish
why each descriptor was unprojectable, whether foreground demand was active, or
whether a later room selection cancelled that demand. The next real-account run
must expose those boundaries without changing repair behavior.

## Scope

Add private-data-free diagnostics for three lifecycle boundaries:

1. SDK gap descriptors to Core display placement;
2. selected-room foreground demand activation;
3. repair candidate selection and room-transition cancellation.

This diagnostic phase must not change candidate selection, scheduling, repair,
pagination, room selection, or cancellation behavior. A later design and commit
will use the captured evidence to repair the state machine.

## Diagnostic Events

### `core.timeline_gap_projection`

Emit once for each completed, gapped inspection immediately after Core attempts
to map descriptors into `navigation_items`.

Fields:

- `stage=inspection`
- `gap_count`
- `projected_count`
- `boundary_both_count`: descriptors whose older and newer boundary events are
  both present in the current navigation items
- `boundary_one_count`: descriptors with exactly one present boundary
- `boundary_none_count`: descriptors with neither boundary present
- `navigation_event_count`: event-backed navigation items available for mapping
- `foreground_demand_active`: whether the actor currently owns selected-room
  demand
- `foreground_demand_epoch`: identity-free monotonic actor-local epoch

The three boundary counters must sum to `gap_count`; `projected_count` must not
exceed `gap_count`.

### `core.timeline_gap_demand`

Emit when a committed room selection reaches its actor and when that actor is
explicitly deactivated or cancelled by a later room transition.

Fields:

- `stage=activate | deactivate`
- `foreground_demand_epoch`
- `foreground_demand_active`
- `visible_gap_count`
- `projected_gap_count`
- `scheduler_phase=idle | queued | inspecting | repairing | awaiting_projection_ack`
- `reason=room_selected | room_reselected | room_switched | unsubscribed`

For this diagnostic-only phase, existing production behavior may lack a real
deactivation message on ordinary room switch. In that case the absence of a
`deactivate` record, together with a later activation and multiple active room
actors, is intentional evidence. Do not invent cancellation behavior merely to
emit the record.

### `core.timeline_gap_selection`

Emit once per completed gapped inspection before the existing selection branch
returns or starts repair.

Fields:

- `stage=evaluation`
- `trigger=automatic | live_edge | live_tail_snapshot | manual`
- `decision=explicit_visible | viewport | nearest_live_edge | foreground_unlocated | blocked`
- `repair_started`
- `gap_count`
- `projected_gap_count`
- `visible_gap_count`
- `foreground_demand_active`
- `foreground_demand_epoch`
- `has_live_edge_target`
- `scheduler_phase`

`foreground_unlocated` means foreground demand is active, persisted gaps exist,
and none can be projected. During this phase it remains observational: the
existing selection result is unchanged and `repair_started` remains false unless
the current production path independently starts repair.

## Ownership and Data Flow

The timeline actor owns the counters and foreground-demand diagnostic state.
The SDK remains responsible for opaque descriptors and boundary metadata. The
UI remains a producer of viewport facts only.

The manager may report a coarse active-room-actor count in an existing manager
diagnostic, but actor diagnostics must not receive room IDs, event IDs, or any
cross-room registry merely for logging. Diagnostic state must not become a
second scheduler or a source of production decisions.

## Privacy Contract

Allowed values are fixed tokens, booleans, and bounded counters. The new records
must not include or derive:

- room, event, user, transaction, or account identifiers;
- descriptor handles, topology revisions, ordinals, chunk identifiers, or
  pagination tokens;
- message bodies, timestamps, sender data, URLs, or raw SDK errors.

Debug output and exported diagnostics must contain the same coarse fields. Tests
must inspect collected structured records and assert that forbidden field names
and synthetic private marker values are absent.

## Testing

Use test-driven development and direct behavioral helpers rather than source-text
assertions.

Required RED/GREEN cases:

1. Four descriptors with no boundary events produce
   `boundary_none_count=4`, `projected_count=0`, and
   `decision=foreground_unlocated` while repair behavior remains unchanged.
2. Both/one/none boundary mixtures produce exact counters whose sum equals the
   descriptor count.
3. Foreground activation increments the actor-local epoch and emits a private-safe
   demand record without requiring a visible gap ID.
4. Re-selection emits another activation epoch; ordinary room switching does not
   falsely claim cancellation if production has not sent one.
5. The structured collector and formatted debug/export representation contain no
   forbidden identifiers or private marker values.

Run focused Core tests, the full `timeline_gap_repair_tracker_tests` module,
`cargo check -p koushi-core`, Rust formatting, and `git diff --check`. Do not run
the long Conduit scenario during diagnostic implementation.

## Real-Account Collection

After the diagnostic-only commit and short verification:

1. build and launch the Mac worktree;
2. select one affected DM and leave it open until inspection settles;
3. switch to the second affected DM and repeat;
4. return to the first DM;
5. export one Koushi diagnostic snapshot.

The snapshot is sufficient when it contains projection, demand, and selection
records for the observed transitions. The next repair design will be based on
that snapshot; this diagnostic phase itself does not claim the gap is fixed.
