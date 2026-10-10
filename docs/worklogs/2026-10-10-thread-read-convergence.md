# Thread read divider and residual notification count

## Evidence and scope

The report describes a thread read divider disappearing on Space re-entry and
a room notification badge remaining after reading the thread. Sanitized
diagnostics show a successful threaded receipt, zero thread unread events, and
a confirmed boundary present in the canonical window without a display anchor.
Separately, the SDK room counter reports zero unread messages and one
notification. The retained cache has a deleted event followed by a notifying
event without a relation. Existing diagnostics do not identify that latter
event's type, so attribution of the reported badge to a redaction remains an
inference; the new `redaction_event` boolean permits direct confirmation.

Two deterministic failures reproduce these shapes:

- A hidden confirmed receipt with no current local viewport observation leaves
  the display marker unset, so a returning thread cannot render its divider.
- A redaction carrying cached notify/highlight actions contributes counts even
  though it contributes no unread message. The previous guard rejected the
  deleted message but admitted the redaction event itself.

## Changes and contracts

Base: `62c2d810`, with SDK `55e51ffe8`. Canon: root repository rules,
state-machine sections "Unread Source Of Truth" and "Navigation", and the
verification instructions. Changes are in an isolated worktree; the existing
development checkout is untouched.

- Core navigation resolves a hidden confirmed boundary to the nearest visible
  in-scope predecessor when no display anchor is available. It retains the
  actual confirmed receipt, unread count, and local viewed state. Existing own
  message anchoring and visible receipt fallback remain unchanged.
- SDK receipt counting excludes redaction events as well as redacted events.
  They remain valid ordering boundaries, and subsequent unread messages keep
  their counters. No room-wide receipt is sent merely by reading a thread.
- Core cache diagnostics add only a boolean distinguishing redaction events;
  no identifiers or event content are added to diagnostics.
- State-machine notes clarify both invariants. No user-guide workflow changes
  are needed: this restores existing read/notification behavior.

## Verification

- Before implementation, the new Core re-entry regression failed with no
  display anchor instead of the visible reply (exit 101).
- Before implementation, the new SDK redaction regression failed with
  `(unread, notifications, mentions) = (0, 1, 1)` instead of `(0, 0, 0)`
  (exit 101).
- After implementation, the Core navigation suite passed 58 tests, including
  re-entry, missing-boundary and subsequent-unread guards (exit 0).

- SDK `event_cache::caches::read_receipts` suite: 24 passed (exit 0), including
  the formerly failing redaction test, deleted-message counters, threaded edit
  scope and unthreaded receipt ordering.
- Core `timeline::diagnostics` suite: 13 passed, 1 intentionally ignored helper
  (exit 0); the producer-path test runs that helper in its child process.
- Scoped Rust formatting checks (Core and SDK), Rust test structure checker,
  agent-document checker, and whitespace checks passed.

Core commands used `cargo test --profile ci -p koushi-core --lib <filter>`.
SDK commands used `cargo test --manifest-path vendor/matrix-rust-sdk/Cargo.toml
-p matrix-sdk --features testing --lib <filter>`, with test debug information
and incremental compilation disabled. Both reused the existing shared Cargo
target directory. The SDK standalone suite uses the SDK workspace dependency
resolution; Core also compiled the patched SDK with the application lockfile.

Self-review confirmed that no local receipt is advanced, unseen replies are not
acknowledged, and event ordering is retained when notification counts are
filtered. No real-account writes, desktop replacement, release build, or
homeserver QA has been performed. The application symptom still needs
confirmation in a build containing these changes.
