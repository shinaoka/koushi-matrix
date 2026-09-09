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
- Full core tests: 1,019 passed; state tests including doctests: 792 passed.
  Frontend typecheck and 1,272 tests passed. Local Tuwunel and Synapse basic
  operations and timeline reconnect scenarios passed. Initial PR CI passed all
  nine jobs.
- The required real-server gate exposed a stale QA admission order: the runner
  waited for LoggedIn before performing the verification that admits login.
  Adapt the QA runner to existing-identity recovery before sync, collect both
  terminal events and Ready under one deadline, and revoke provisional sessions
  on admission failure through the same connection. Product authentication and
  verification conditions remain unchanged.
- QA admission regression failed before the adaptation and passes afterwards;
  tests cover both completion orders for login/restore, request/account fences,
  unsupported methods, one recovery submission, and timeout.
- Real-server space compatibility QA passed, including recovery, sync,
  send/edit/redact/search, store restore, room/space leave+forget, logout, and
  post-logout restore rejection. The failed initial run's pending device was
  reused in the successful run and cleaned up.
- Native sleep/wake validation has not been performed. The installed app has
  not been replaced by this source change.
