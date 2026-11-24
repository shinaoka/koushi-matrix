# Recover from a startup Secure Backup failure (2026-11-24, #2086)

Reported: with sync working and only the key-backup endpoint failing at launch, the
composer went read-only, Koushi asked the endpoint six times in the first 14 seconds and
then never again, and the lock outlived the endpoint's recovery by nine minutes until a
button at the bottom of **Settings › Sessions** was clicked. Meanwhile **Settings ›
Encryption** read "Key backup: Ready", and the warning text promised "while Koushi
retries".

Three gaps, fixed as A (recovery detection) and B (one truth, actionable notice).

## A. A retryable blocking failure keeps retrying

`SecureBackupGateState::BlockedFailed` is the state that closes sending before this
session has established Secure Backup authority. In `AccountActor`'s inspection
completion it was the one terminal that scheduled **nothing** — neither the backoff retry
nor the 30-minute periodic monitor — so the only recovery was the explicit typed retry.
It now keeps the same bounded retry the post-authority `DegradedRetrying` path already
used: jittered exponential backoff from 5 seconds, capped at 5 minutes
(`secure_backup_retry_delay_from`), with the attempt counter preserved across retries and
reset by any settle that arms no retry (a ready or inconclusive answer, or a
non-retryable failure such as the 401 above) as well as by a proven-connectivity
recovery edge. Without that reset a user who fixed the cause after ten minutes of
503s would wait the 5-minute ceiling for the first retry of the fixed endpoint.

Guarded by the existing structured detail: `BlockedFailed` whose
`secure_backup_failure_is_retryable` is false (rejected credential, key or backup
mismatch, incomplete secret storage) arms no monitor, and a deferred inspection that
expired because connectivity was never proven keeps its #860 behaviour (no monitor, the
bounded deadline is the recovery).

**No new unblock path.** Admission is still
`SecureBackupGateState::backup_is_ready()` and is still only set from a completed
inspection's `recommended_gate_state()`. A retry that answers "no server backup",
"recovery required", "unauthorized", or another non-ready state leaves sending closed;
the new tests assert exactly that on the retrying path.

No new persistence: the attempt counter already lived in the actor. The extra triggers
needed no new Core event either:

- **Network recovery** already exists: a proven connectivity edge admits an inspection,
  or schedules the retry monitor when a recovery epoch is open.
- **Window focus** is a renderer concern, so it reuses the existing
  `retry_secure_backup_inspection` command through `secureBackupFocusRecheckDue`, at most
  once per 15 seconds, only while the gate is a retryable blocking failure, and never
  while a manual re-check is still settling — a second command during a settling
  inspection makes Rust discard that inspection and look again.
- **Manual re-check** is the same command, now reachable from the notice and from
  Settings.

## B. One truth, and a notice that acts

- `SecureBackupSendNotice` is the one component for the send-blocked notice: the reason
  plus **Re-check** and **Encryption settings**. It replaces three copies of the bare
  string (main composer, thread composer, scheduled-send panel) and is passed down as an
  element, so the workspace panes render the same actions. `.composer-read-only-notice`
  keeps its class and `role="status"` and holds only the reason — the read-only shell specs
  read that element's role and exact text as a contract — while the buttons sit in a
  sibling `.composer-read-only-notice-actions` inside the `.composer-read-only-notice-row`
  strip, so a status announcement does not contain the controls it announced.
- `secureBackupRecheckCanHelp` is the one answer to "will looking again help?", used by
  the warning sentence and the notice's button: no recorded failure (still checking) and a
  retryable failure both offer a re-check and keep the "… while Koushi retries" wording; a
  recorded non-retryable failure says Koushi cannot reach it on its own and hides the
  button, because a 401 never becomes a 200 by being asked again.
- The Encryption page's **Key backup** row and the Sessions page's row now read
  `secure_backup_gate` (`secureBackupGateOperational` mirrors Rust's
  `backup_is_ready()`), instead of a session-status reading that can predate a failed
  inspection. A non-operational gate shows "Not ready" with the re-check action; an
  operational one shows "Ready", so the row cannot claim readiness while sending is closed
  or deny it while sending works.
- The runtime warning picks its sentence from the recorded cause: retryable failures keep
  "… while Koushi retries" (now true), non-retryable ones say Koushi cannot reach it on
  its own, and the appended cause uses `secureBackupFailurePresentation`, so an immediate
  HTTP 503 is no longer described as a timeout.

## Canon

- `docs/architecture/overview.md` (Secure Backup admission): the pre-authority rule now
  reads "blocking, with a bounded re-inspection on the same jittered backoff", states that
  only a confirmed ready inspection raises admission, and names the notice, Settings, and
  window-return re-checks as the exposed surface.
- `docs/architecture/state-machine.md`: the pre-authority paragraph now separates a
  retryable `BlockedFailed` (keeps the bounded retry) from a non-retryable one and from a
  connectivity-deadline expiry (#860), and says no retry raises admission.

Both replace the previous normative sentence "instead of entering an automatic retry
loop", which is what produced the report.

## Verification

- RED first: `a_retryable_blocking_failure_retries_with_backoff_and_clears_the_lock_when_ready`
  failed on `has_monitor_task` before the Core change ("a retryable blocking failure must
  own a retry, not wait for a click"). The frontend single-truth test was verified red by
  re-rendering with the gate override disabled.
- New: `a_non_retryable_blocking_failure_arms_no_retry` (401 arms no monitor),
  `SecureBackupSendNotice` (both actions, no second re-check while settling),
  `Secure Backup gate presentation (#2086)` (operational predicate, retryable predicate,
  focus-recheck decision and its interval), and
  `Encryption reads the Secure Backup gate instead of a stale ready reading` (blocked
  gate overrides a ready session reading, re-check dispatches, operational gate restores
  the row).
- `cargo fmt --check`, `cargo clippy --profile ci -p koushi-core -p koushi-state
  --all-targets --locked -D warnings`, `cargo test -p koushi-core --profile ci`
  (2088 passed, 11 ignored), `cargo test -p koushi-state --profile ci` (147 + 1248 + 3
  passed).
- `npm --prefix apps/desktop run typecheck`, `run lint`, `test -- --run` (217 files,
  2722 tests). The seven `scripts/check-*.mjs` gates CI runs for these areas.

## Review round

One independent review (GLM, read-only) on the staged diff; two IMPORTANT and four MINOR
findings, all verified against the code and fixed here:

- The warning offered "Koushi cannot reach it on its own … re-check" for a gate with no
  recorded failure, i.e. on every ordinary startup while the first inspection runs. Now
  one predicate decides the sentence and the button, and the no-failure case keeps the
  retrying wording.
- The new Core test shortened the retry base to 20 ms, so the owners round trip could lose
  the race to the retry timer; the base is now 1 s and the 5 s probe poll still observes
  the retry.
- The attempt counter no longer survives a settle that arms no retry.
- The notice hid no re-check for a non-retryable failure, so two surfaces disagreed.
- The buttons moved out of the `role="status"` region. The first attempt moved the role
  itself onto the sentence span, which broke five `readable-provisional-shell.spec.ts`
  contracts in CI (`.composer-read-only-notice` must carry `role="status"` and exactly
  `SEND_BLOCKED_COPY`); the fix keeps the element and moves only the actions.
- The focus handler now skips while a manual re-check is in flight.

Declined, with the reason: the review's suggestion to delete the focus re-check entirely
(it is a required trigger in #2086's task, and it is now a single throttled command that
cannot open sending) and to make two optional props required (they keep the panes usable
without a gate, and the fallback string is still exercised by the scheduled-send panel
test).

## Not covered

- The recording's exact request pattern (six probes in 14 seconds) came from the
  inspection stages and the pre-change admission path; this change does not reproduce it
  against a real homeserver, so it is not fixed by name — what changes is that the last
  failure is no longer terminal.
- The retry interval ceiling stays at the existing 5 minutes. A permanently failing
  endpoint therefore costs one request every ≤5 minutes for the session; the task's
  "widen while idle" is satisfied by that floor being the ceiling of the existing curve,
  not by a second curve.
- Marking the request pattern as QA evidence would need the disposable-homeserver proxy
  the reporter used; no QA scenario was added, since the Core behaviour is covered by
  actor tests that do not depend on wall-clock multi-second waits.
- `--scenario=all` and the release gates are untouched.
- Only the blocking direction overrides the Key backup row. The reverse (`degradedRetrying`
  with a stale session reading of "disabled") still shows the session reading: a usable
  gate means sending is admitted, not that a server backup exists, and
  `security-e2ee.spec.ts:199` drives exactly that pair — gate `ready` with
  `key_backup: disabled` — to press **Enable**. Overriding it there removed the Enable
  action and failed the spec, which is the correct answer rather than a stale one.
- The trust **chip** at the top of the Encryption page still comes from
  `trustOverallStatus` (identity verification plus the session reading), so a blocked gate
  can sit beside a "Verified" chip. That is a different question — whether the identity is
  verified, not whether a backup is usable — and the backup row carries its own state; if a
  report shows the pair reading as a contradiction, the fix is a gate-aware chip with a
  new string, which this change does not add.
