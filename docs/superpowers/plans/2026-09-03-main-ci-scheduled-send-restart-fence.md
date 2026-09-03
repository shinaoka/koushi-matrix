# Main CI — event-fence cancelled scheduled-send restart test

## Failure

Main CI run `33708842268` failed only `cancelled_local_fallback_scheduled_send_does_not_resurrect_on_restart` at `runtime_scheduled_send.rs:683`: `state predicate was not satisfied within 200ms`.

## Root cause

After restarting the runtime and injecting ready-room actions, the test polls snapshots every 5ms for at most 200ms. The asserted condition is causally tied to an observable state transition (`SessionState::Ready` plus selected timeline room and empty scheduled-send projections), not to elapsed time. Under CI scheduling, the actor/store restoration pipeline can legitimately publish after this arbitrary polling window.

The shared test support already provides `wait_for_state_event`: it rechecks the current snapshot, then waits for versioned state publication with a 5-second deadlock guard. That is the repository's deterministic contract for actor transitions and cannot miss a transition between polling intervals.

## Deterministic fix

Replace only this 200ms polling call with `wait_for_state_event` and the identical predicate. This is not a retry or product-timeout increase: it removes wall-clock polling as the progress mechanism and uses publication events; 5 seconds is only the shared deadlock guard. Keep the negative no-resurrection assertions and all production scheduled-send persistence behavior unchanged.

Do not change scheduled-send timers, persistence, retries, runtime state, or the general helper's other two call sites in this task.

## Verify first and gates

The failed main run is RED. Run the exact test 50 times, full `runtime_scheduled_send`, Core tests, format/diff, independent review, PR CI, merge, and main CI. If another unrelated deterministic failure appears, diagnose it separately rather than retrying or widening limits.

## Review record

- Design review: `reviewer-flash` **Correct-to-merge**. It verified the scheduled-send load is structurally complete before the room-selection predicate can pass, so the negative remains non-vacuous. The other polling helper call waits for a real retry timer and remains intentionally time-bounded.
- Verify-first evidence: main CI `33708842268` failed before the change. The event-driven exact test passed, its compiled binary passed 50/50 repetitions, and full `runtime_scheduled_send` passed 12/12.
- Implementation review: `reviewer-flash` **Correct-to-merge**, no findings.
