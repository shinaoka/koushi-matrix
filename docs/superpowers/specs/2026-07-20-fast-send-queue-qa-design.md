# Fast SendQueue QA Design

## Goal

Make the normal SendQueue development loop finish within 60 seconds, and never
require the multi-minute full Conduit scenario for each implementation edit.
Keep real SDK, persistence, and homeserver coverage in slower checkpoint lanes.

## Three lanes

### Fast lane (default during implementation)

Run the production SendQueue state machine with deterministic test-controlled
network availability and retry timing. Cover:

- local echo creation and authoritative completion;
- offline failure, retry, and FIFO ordering;
- cancellation;
- connection drop followed by ordered `CoreRuntime::shutdown`;
- reopening the same data directory and restoring an unsent item;
- removal of the pending echo after authoritative completion; and
- absence of duplicate pending/remote rows.

The lane must not create a real homeserver, wait for SDK backoff, or use blind
sleeps. It should fail on event- or state-scoped bounded waits and complete in
at most 60 seconds on a normal development machine.

### Focused Conduit lane (checkpoint)

Add an early SendQueue route that bootstraps only user A, captures the recovery
secret, shuts down that runtime through the ordered barrier, and invokes the
existing standalone `run_send_queue_stage`. It must not construct user B or run
the generic room/space, two-user timeline, pagination, navigation, edit, or
redaction stages.

This lane retains real SDK, Conduit, encrypted identity recovery, sync,
send-queue persistence, and proxy-controlled reconnect coverage. Its target is
3–6 minutes; the existing 300-second reconnect timeout remains an exceptional
failure budget rather than the normal development loop.

### Full E2E lane (PR gate)

Keep the existing full scenario for cross-stage integration. Run it once after
the implementation and focused lanes are green, before PR submission. It
continues to own room/space projection, user B login/join, two-user delivery,
pagination/navigation, edit/redact behavior, and final restore/logout cleanup.

## Boundaries

- Production ordering and persistence code is shared by every lane.
- Only clocks, retry scheduling, and network outcomes may be controlled by the
  fast test boundary.
- Do not add a second SendQueue implementation or a boolean identity-gate mode.
- Reuse the typed `QaParticipantLoginGate`, identity helpers, and existing
  SendQueue stage helpers.
- Every same-data-directory reopen must follow: drop connections, await
  `CoreRuntime::shutdown`, then reopen. A source contract guards this ordering.
- No test may rely on a blind sleep to prove lifecycle completion.

## Developer workflow

1. During edits, run the fast lane only.
2. When the behavior is complete, run the short unit/check/fmt suite and the
   focused Conduit lane once.
3. Run the full Conduit E2E once before creating the PR.

If the focused or full lane finds a failure, reproduce it first as a fast,
deterministic regression whenever the failing boundary can be controlled
locally. Long lanes are evidence gates, not debugging loops.

## Success criteria

- The fast command has a documented invocation and completes within 60 seconds.
- Its failure output identifies the exact phase and authoritative state/event.
- The focused SendQueue route does not execute the generic two-user timeline
  flow.
- The focused and full lanes continue to exercise the same production
  SendQueue and ordered-shutdown paths.
- Existing headless QA tests, core tests, formatting, and compile checks remain
  green.
