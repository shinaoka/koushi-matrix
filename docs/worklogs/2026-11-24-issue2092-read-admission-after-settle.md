# Re-attempt a read admission the opening pagination superseded (2026-11-24, #2092)

A room displayed at the live edge with the newest message visible kept its unread badge
and "Jump to bottom" bar. The diagnostics showed `navigation unread=9 newer=9 bottom=true`
and no `core.read_state stage=admission` line at all, so no admission was ever attempted —
not rejected, not deferred. The renderer never asked again.

## Cause

The renderer reports one viewport observation per settled frame, naming the display window
revision it holds (`apps/desktop/src/components/TimelineView.tsx:2973-3048`,
`runViewportObservation`). The actor drops an observation whose revision is not current,
before touching the read state:

`crates/koushi-core/src/timeline/actor.rs:2756-2761` (before this change)

```rust
if observation.display_window_revision != Some(self.display_projection.window_revision) {
    return;
}
```

`window_revision` advances on every window rebuild, rematerialization and bounded-window
reconcile (`display_projection.rs:318,:335`, `bounded_window.rs:216`, `:1239`), and the
opening backward pagination rebuilds the window while the reader sits at the bottom. The
observation the renderer settles with therefore arrives naming the previous revision and
is discarded. There is no re-attempt: the drop returned before `self.viewport_observation`
was stored, so `observe_local_viewed_boundary` (`read_state.rs:2114`) never ran and the
read boundary that feeds admission never moved. The renderer has no reason to report the
same span twice (`lastViewportObservationRef`), so the room stayed unread.

The renderer side of the settle chain was verified as already working, in both arrival
orders, by `TimelineView.window.test.tsx` — a suppressed report is retried when the rows
and the window converge. The hole was the silent drop on the Core side.

## Change

- `display_projection/bounded_window.rs`: `supersedes_observation` — the observation may
  be re-anchored when its endpoints are still projected in the reported order **and** its
  end is still the window's newest visible row, i.e. a prepend moved the window's start
  above the reader, not the end past them. `observed_positions` is shared with
  `observation_is_current` instead of being duplicated.
- `timeline/actor.rs`: the room branch re-anchors such an observation to the current
  revision instead of returning. Everything after the guard runs unchanged, including
  `at_bottom &= window_reaches_live_end(..)` and the read-boundary gate.

Safety conditions, and why they still hold:

- **Never admit while scrolled up.** `at_bottom` is the renderer's, untouched by the
  re-anchor; `observe_local_viewed_boundary` requires it, and the actor still intersects
  it with `window_reaches_live_end`.
- **A new message makes the room unread again.** The re-anchor needs the reported end to
  *be* the current end. A row that arrived after the observation moves that end, so the
  observation is rejected exactly as before and cannot read a row the reader never saw.
- **A read covers the newest row at that moment.** The read target is the observed end,
  which the rule pins to the newest row; the actor test asserts the target.
- **Inactive window / hidden room rules** are a different gate (subscription and read
  workers) and are not touched.

## Tests

- `timeline/actor/issue_2092_tests.rs`: `an_observation_superseded_by_a_prepend_still_reaches_the_read_boundary`
  — live-edge room, opening backward page lands, the settled observation (previous
  revision) must produce `LocalReadBoundaryObserved`, and the target must be the newest
  row. **RED** before the change: the boundary message never arrives (`#2092: the
  superseded observation never reached the read boundary`); proven by restoring the early
  `return`.
- `display_projection/bounded_window_tests.rs`: `a_superseded_observation_is_reanchored_only_at_the_same_end`
  — a prepend is re-anchored; an observation ending below the newest row is rejected; an
  observation whose first row is no longer projected is rejected.
- `apps/desktop/src/components/TimelineView.window.test.tsx`: asks again once the rows and
  the display window converge, with the window arriving after the rows (the order that
  leaves no other trigger). Green before and after: it pins the renderer half of the
  chain.

Gates: `cargo fmt --all -- --check`; `cargo test -p koushi-core --profile ci` 2137 passed,
11 ignored; `cargo test -p koushi-state --profile ci` 1398 passed; `cargo clippy --profile
ci -p koushi-core -p koushi-state --all-targets --locked -- -D warnings` (only the
pre-existing vendored `ControlledMemoryStore` documentation warning);
`npm --prefix apps/desktop run typecheck && run lint && test -- --run` 2717 passed;
`node scripts/check-leaf-crate-boundaries.mjs`.

## Not done

The field evidence is the reported log, not a captured failing run: the drop is proven by
the actor test, but nothing here reproduces the exact renderer/actor timing that produced
it in the app. If a room still opens unread after this change, the next step is a
diagnostic on the drop path (a `debug!` naming the superseded revision) rather than another
retry.
