# Issue #802 degraded-network recovery design

Status: **Correct-to-merge design** — `reviewer-flash` round 2. Round 1 found two Important and three Minor issues; all were fixed before round 2. Round 2 added two non-blocking precision notes, addressed below. (`reviewer-flash` selected: low cost, fast, strong independent correctness review; `reviewer-gpt` is slower/costlier and strongest for broad architectural review; `reviewer-flash-opencode-go` is similarly fast but redundant with the selected model family).

## Problem and evidence

Dogfood diagnostics show a valid ready session surviving a transport outage. Sliding Sync recovered before the user pressed **Retry**, but a timed-out current-session inspection remained projected as a terminal session error. Secure-backup inspection separately ran a fixed 30 s request + 5 s retry cycle. The Matrix SDK send queue already retains local echoes and exposes recoverable transport failure/retry/cancel states; the `send_queue_fast` proxy tests cover offline local echoes and ordered recovery, so #802 must not replace that owner.

## Ownership and invariants

- `AppState.sync` is the one Rust-owned account connectivity signal. Only a committed `SyncState::Running` proves connectivity; stopped, starting, failed, and reconnecting are unproven.
- The reducer owns current-session projection and automatic recovery admission. React only renders the Rust DTO and dispatches the existing typed manual/open command.
- `AccountActor` owns secure-backup inspection tasks and retry cadence. It consumes reducer connectivity effects, never infers connectivity from React or independent browser probes.
- Session inspection remains informational: it must not change session readiness or verification admission. Generation/request fences remain authoritative.
- A transport outage must not erase the last successful session facts or look like authentication invalidation.

## State-machine changes

### Current-session status

1. Extend `SessionStatusRefreshTrigger` with `Recovery`.
2. Extend `Checking` and `Failed` with optional `last_known_details`, populated from the preceding `Ready`, `Checking`, or `Failed` state.
3. Add coarse failure `ConnectivityUnavailable`. It carries no SDK/server text and is intentionally distinct from existing `Unavailable`, which means the session/current device required for inspection is absent rather than that the network path is unproven. Classify SDK request failures into coarse `Authentication`, `Network`, `Server`, or stage-specific `Sdk`; `Network` participates in recovery refresh while authentication/server outcomes remain explicit non-transport failures.
4. On open/manual refresh while sync is not `Running`, do not emit a network effect. Project `Failed(ConnectivityUnavailable)` immediately and preserve last-known facts.
5. On a real timeout, preserve last-known facts in `Failed(TimedOut)`.
6. Only `handle_sync_status_changed` may emit recovery work. On its accepted transition from an unproven state to `Running`, if current-session status is `Failed(TimedOut | ConnectivityUnavailable)`, enter one correlated `Checking(trigger=Recovery)` and emit one refresh effect. The compatibility/test-only `handle_sync_started` and `handle_sync_recovered` paths do not emit it. Use the accepted sync lifecycle generation as the reducer-owned correlation id; current-session request correlation is a separate namespace and the actor still rejects stale generation/request completions.
7. A manual retry arriving immediately after recovery does not replace or restart the already-admitted automatic request. Runtime emits a correlated `IntentLifecycle::BenignNoOp(AlreadyActive)` for the manual command's full `RequestId`, while the automatic request remains the only network owner. This deterministically covers the observed 125 ms ordering without leaving an opaque waiter or making the manual action appear to repair connectivity.
8. An open/manual refresh deferred because connectivity is unproven commits the `ConnectivityUnavailable` projection but emits no network effect; runtime treats that state projection as committed rather than inferring acceptance only from the presence of a network effect.
9. UI renders preserved details while checking/failed, and presents connectivity/timeout as a warning with connection-specific copy rather than a session-auth error.

### Secure-backup inspection

1. Add an `AppEffect::SyncConnectivityChanged { proven }` on every accepted sync connectivity edge. Runtime forwards it to `AccountActor`.
2. While connectivity is unproven, abort any nonessential secure-backup inspection/monitor and coalesce one pending inspection; direct retry/open requests do not start a probe.
3. Treat consecutive connectivity flaps as one backup-recovery epoch until a backup inspection succeeds. The first `Running` edge in that epoch resets the retry attempt and schedules one immediate coalesced inspection; later unproven→Running flaps in the same epoch preserve the accumulated attempt instead of repeatedly collapsing cadence to 5 s.
4. Automatic health retries exist only after backup authority was previously operational (`secure_backup_ready`, projecting `DegradedRetrying`). While sync remains proven, recoverable network/rate-limit/timeout failures use bounded exponential backoff: 5 s, 10 s, 20 s, … capped at 5 min, with deterministic bounded 0–20% jitter derived from the actor monitor serial. A successful backup inspection closes the recovery epoch and resets the attempt. A pre-authority inconclusive inspection remains `BlockedFailed` and has no automatic monitor; only the explicit typed retry can re-admit it.
5. Periodic 60 s monitoring resumes only after an authoritative successful inspection. No monitor is scheduled for `BlockedFailed`.
6. Diagnostics record connectivity edge, admission/defer decision, retry attempt, scheduled delay, timeout/result, and elapsed request time using only coarse tokens/counts.

## Acceptance-criteria traceability

- Simulated latency/loss/interface switching: sync is the sole connectivity authority; transient failure preserves session readiness and last-known facts.
- No repeated probes while unproven: reducer defers session inspection and AccountActor aborts/coalesces backup inspection.
- Automatic stale-failure clearing: the sole accepted unproven→Running edge admits exactly one `Recovery` refresh.
- Bounded cadence: post-authority backup retries use capped exponential backoff with jitter and one reset per recovery epoch.
- Local echo: existing SDK send-queue ownership and `send_queue_fast` outage/recovery tests remain the gate.
- Recovery-before-manual ordering: the automatic request stays authoritative and the later manual request gets a correlated benign no-op.
- Diagnostics distinction: existing sync diagnostics retain auth/HTTP/transport classification; new session/backup events add timeout, defer, recovery, and retry-cadence evidence without secrets.

## Existing behavior retained as acceptance evidence

- Local echo/retry state stays owned by the Matrix SDK send queue. Existing `send_queue_fast` tests exercise recoverable transport failure, multiple offline local echoes, ordering, retry, cancel, and success projection.
- Sliding Sync diagnostics already distinguish auth failure, HTTP status/error source, transport failure, timeout/retryability, lifecycle restart, and first committed responses without secrets. Session-status diagnostics add only admission/defer/recovery correlation and do not duplicate raw network errors.
- No client recreation or retry/timeout inflation is introduced.

## Verify-first tests

1. Reducer RED test: ready facts → timed-out failure → reconnecting → running emits exactly one recovery refresh; an immediate manual request is ignored; success restores ready facts.
2. Reducer RED test: open/manual while starting/reconnecting emits no inspection and preserves prior facts as connectivity-unavailable.
3. Core RED tests: secure-backup probes are deferred while unproven; recovery resets the backoff and admits one probe; stale monitor wakeups remain rejected.
4. Pure RED tests: exponential delay is monotonic, bounded, jittered, and resettable.
5. Component RED test: stale facts remain visible with connection-specific warning and retry remains available only through the typed command.
6. Runtime RED test: a manual refresh coalesced behind `Checking(Recovery)` emits a full-request-id correlated benign no-op; an offline deferred projection is committed without starting an actor probe.
7. Update `docs/architecture/state-machine.md` current-session and sync guard diagrams for `Recovery`, `ConnectivityUnavailable`, preserved details, the single `SyncStatusChanged` emission site, and the correlated manual coalescing outcome.
8. Re-run focused Rust/Vitest tests, send-queue transport tests, state/core suites, typecheck, lint, format, docs/secret/boundary checks, and full GitHub CI.

## Gate record

- Design review: `reviewer-flash` round 1 findings fixed; round 2 **Correct-to-merge**.
- Verify-first RED: the recovery reducer test failed to compile on the missing `Recovery`, `SyncConnectivityChanged`, and `last_known_details` contracts before implementation.
- Local GREEN evidence: current-session/sync reducer suites; full `koushi-state`; full `koushi-core --lib`; full `koushi-sdk --lib`; focused Tauri recovery-trigger boundary; full Vitest; session-status and profile-settings Playwright; send-queue outage/recovery integration; typecheck, lint, build, secret scan, format/diff, SDK/submodule and boundary guards.
- Implementation diff review: `reviewer-flash` returned **Correct-to-merge** with three Minor findings; all three were fixed (duplicate flap probe, non-transport stale-green trust tone, and no-op semantic narrowing), then re-reviewed **Correct-to-merge** with no remaining findings.

## Non-goals

- Do not create a second network monitor, recreate the Matrix client, alter authentication/session readiness, or reimplement the SDK send queue.
- Do not make backup or session inspection success prove Sliding Sync connectivity.
- Do not add retry-count promises to the public DTO.
