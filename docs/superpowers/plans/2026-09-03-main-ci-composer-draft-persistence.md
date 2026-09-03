# Main CI — deterministic composer-draft persistence fence

## Failure

Main CI run `33697529398` failed only `koushi-core-testkit/tests/runtime_timeline.rs::composer_drafts_persist_after_debounce_and_load_on_restart`: after sleeping `COMPOSER_DRAFT_PERSIST_DEBOUNCE * 2`, the restarted runtime did not load the draft. The identical commit's PR run passed in 0.53s; main failed at 1.68s. This is a test synchronization race, not Issue #779 product behavior.

## Root cause

The test observes reducer state, sleeps for a wall-clock multiple of the debounce, and immediately drops the runtime. The deadline only makes persistence eligible; it does not prove the AppActor scheduled the blocking save or that atomic persistence completed. Under scheduling delay the runtime can be dropped before save completion.

Production already has a feature-gated `ComposerDraftIoBarrierForTesting` with save-start, release, and save-completed signals. Other persistence concurrency tests use this explicit seam.

## Verify-first and fix

The failing main run is RED evidence. In the test, install the existing I/O barrier after initial restore and before submitting the draft. Replace the sleep with:

1. await save-start behind the established 3-second deadlock guard;
2. release the blocked save;
3. await save-completed behind the same deadlock guard;
4. only then drop/restart and assert restoration.

Do not change production debounce, wait limits, retries, runtime state, or persistence behavior. The test remains an end-to-end debounce→save→restart proof and becomes event-driven rather than timing-dependent.

## Gates

Run the exact test repeatedly, then full `runtime_timeline`, Core tests, format/diff, and required GitHub CI. Merge as a separate maintenance PR, then verify main CI green before considering Issue #779 complete.

## Review record

- Design review: `reviewer-flash` **Correct-to-merge**. Its one Minor recommendation was incorporated: barrier awaits use the established 3-second deadlock guards.
- Verify-first evidence: main CI run `33697529398` failed before the change. The event-driven exact test passed, then its compiled binary passed 20/20 repetitions; full `runtime_timeline` passed 22/22.
- Implementation review: `reviewer-flash` **Correct-to-merge**, no code findings. The unrelated generated `Cargo.lock` drift observed by the reviewer was removed before commit.
