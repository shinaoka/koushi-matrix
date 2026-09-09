# Reconnect reconciliation delay

## Problem and evidence

After intermittent connectivity and sleep, SDK sync returned to Running and
committed a room response. The local room-list reconciliation exceeded its
10-second acknowledgement deadline. Core classified this as an internal sync
failure and stopped the observer, although the authoritative room projection
completed roughly half a second later. Explicit restart restored operation.
This timing summary excludes account identifiers and message content.

## Contract and change

Consulted REPOSITORY_RULES.md, docs/agents/verification.md,
docs/policies/engineering-rules.md, and the architecture overview/state machine.
Local projection delay is not proof of sync-owner failure. Preserve one pending
acknowledgement; diagnose the delay once without ending sync. Keep SDK state,
encryption readiness, and explicit cancellation observable while waiting.
Retire pending work on owner replacement and retain generation/sequence fences.
Channel closure and invalid acknowledgement remain failures.

The independent design review rejected an indefinite in-branch wait: the room
observer may await an SDK update that requires the blocked supervisor to
restart the SDK. Polling the wait in the outer select avoids this dependency.
Pause committed-response consumption while one reconciliation is pending, using
the existing retained SDK observable rather than adding a request queue.

Scope: core sync observer, focused behavioral tests, and its normative docs.
No vendored SDK, authentication, frontend, or public DTO changes are intended.

## Verification

- A paused-clock regression drives the real reconciliation request/ack path
  past the old deadline, then supplies a matching acknowledgement.
- Cover cancellation during ack wait and saturated-channel submission,
  lifecycle/encryption signal handling while pending, and invalid/closed ack.
- Run focused sync tests and the existing runtime room-list reconnect test.
- Review the integrated diff independently before reporting the fix ready.

Results:

- RED: `cargo test -p koushi-core --lib sync::reconcile_tests --release`
  exited 101 on the unmodified production path: the slow-projection assertion
  failed after advancing past ten seconds.
- GREEN: `cargo test -p koushi-core --lib sync:: --release` exited 0;
  26 passed and one existing diagnostic subprocess fixture was ignored.
- SDK gitlink guard, Rust test-structure checker, agent-doc checker, and
  `git diff --check` passed.
- Independent design and final integrated source reviews approved the change.
  Review found and the implementation fixed two added concurrency hazards:
  a pending ack surviving another network loss, and an old room response
  completing recovery for a newer replacement encryption generation. Regression
  tests now cover both, including repeated loss after a partial room proof.
- `cargo test -p koushi-core-testkit --test runtime_room_list_sync --release`
  exited 0: all four integration tests passed, including SDK reconnect and
  reuse of the same sync engine.
- No real-account or native sleep/wake validation has been performed. The
  installed app has not been replaced by this source change.
