# Unlocated Foreground Gap Diagnostics Implementation Plan

**Goal:** Add private-safe evidence showing why a foreground room with SDK gaps does not begin repair when none of those gaps can be projected into the rendered timeline.

**Architecture:** Keep the repair state machine and selection behavior unchanged. Instrument three boundaries: SDK-gap projection, foreground demand admission, and repair candidate selection. Model boundary presence and diagnostic selection as pure functions so short unit tests cover the real-account failure shape without a slow homeserver scenario.

**Tech stack:** Rust, `koushi-diagnostics`, existing `koushi-core` timeline unit tests, Cargo, npm desktop build.

## Constraints

- Never record Matrix room, event, user, transaction, or gap identifiers, message content, timestamps, tokens, descriptor ordinals, or raw errors.
- Do not add a repair fallback or room-switch cancellation in this change.
- Do not run the long Conduit scenario while iterating; run short focused tests, then one consolidated verification pass.

## Task 1: Explain gap projection

**File:** `crates/koushi-core/src/timeline.rs`

1. Add a pure boundary-presence counter for descriptors whose newer/older SDK boundary events are both present, only one is present, or neither is present in `navigation_items`.
2. Add a RED unit test for the real-account shape: four descriptors, zero projected, all boundaries absent.
3. Add `core.timeline_gap_projection / inspection` with gap count, projected count, boundary-presence counts, navigation-event count, foreground-demand state/epoch, and scheduler phase.
4. Wire it into `emit_gap_positions` without changing its returned positions.
5. Run the focused projection and privacy tests.

## Task 2: Trace foreground demand and selection

**File:** `crates/koushi-core/src/timeline.rs`

1. Add an actor-local diagnostic-only foreground-demand flag and expose the existing demand revision as its epoch.
2. Add `core.timeline_gap_demand / activate` when `BeginGapRepairDemand` is handled. Keep the existing visible-gap admission gate unchanged so the log proves whether an inspection was requested.
3. Add a pure selection-decision classifier with tokens `explicit_visible`, `viewport`, `nearest_live_edge`, `foreground_unlocated`, and `blocked`.
4. Add RED tests showing an active foreground demand plus SDK gaps and zero projections classifies as `foreground_unlocated`, while existing projected candidates retain their relation.
5. Add `core.timeline_gap_selection / inspection` immediately after candidate selection, including whether repair starts and the scheduler phase; record no identifiers.
6. Run the focused demand, selection, and privacy tests.

## Task 3: Consolidated verification and collection handoff

1. Run `cargo fmt --check` and the complete short `koushi-core` test suite once.
2. Run `cargo clippy -p koushi-core --all-targets -- -D warnings` if that is part of the repository's normal short gate.
3. Run the desktop build once.
4. Inspect the diff for accidental identifiers and verify `git diff --check`.
5. Provide a single collection procedure: select DM A, select DM B, return to DM A, then export diagnostics. The expected evidence is projection counts, demand activation, and a `foreground_unlocated` or other explicit selection decision.

