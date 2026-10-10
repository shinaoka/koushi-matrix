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

## Re-entry lifecycle correction after follow-up

The initial patch was too narrow to establish that the reported behavior was
fixed. The follow-up describes edits/reactions, no visible deletion, and an
intermittent divider at the root after reopening. No deletion attribution is
established by that observation; the independently reproduced SDK redaction
counter bug above remains a candidate, not proof of the residual badge's cause.

The renderer retained timeline rows at App scope but discarded NavigationUpdated
in that store. Navigation lived only in the mounted TimelineView. A replay
received before the panel listener mounted was lost; cached rows then suppressed
the subscription fallback. The view borrowed the room's fully-read marker for
the Thread, reproducing a divider under the root. Retain the verbatim Rust
navigation projection in the same per-key row mirror, clear it on resync or
actor/timeline generation replacement, and preserve it on same-generation replay.
Remove the component-local owner and the Thread room-marker fallback. Focused
and Room fallback behavior is unchanged. The existing Core emission generation
gate and FIFO InitialItems-before-Navigation delivery remain authoritative.

Core projection now chooses the newer loaded local/confirmed boundary and maps
it once to a visible in-scope predecessor before unread processing. This replaces
both the older special case and this task's initial hidden-event-only fallback.
The own-message-tail rule remains separate and preserves its existing behavior.
No receipt is advanced and no unread count is hidden by the renderer.

Verification:

- Renderer RED: four assertions showed the root divider instead of the requested
  thread position or no divider; fixtures contain only synthetic edited replies
  and reactions.
- Core RED: hidden confirmed receipt followed by unread content failed with no
  display anchor instead of the visible predecessor.
- GREEN: 277 tests across all TimelineView suites and timelineStore; 60 Core
  navigation tests; the retired actor emission gate regression passed separately.
- Existing frontend navigation fixtures now follow production's InitialItems then
  NavigationUpdated order; their behavioral assertions are unchanged.
- Typecheck passed. Independent read-only design review approved the mirror and
  projection consolidation. Integrated review identified an unintended Focused
  fallback change; it was corrected to exclude only Thread. Tests additionally
  cover account/root switches, missing navigation, resync and actor/generation
  replacement.

User requested local installation of this revision. Dependency lockfile audit
passed the high/critical gate (one low advisory); local packaging and installation
results will be recorded separately. No release or upstream publication is
implied. The notification badge source remains unconfirmed until fresh diagnostics
from a build with the added event-type boolean are available.

## Local installation

With user authorization, packaged source commit `58b74d16` as macOS app/DMG
(version 0.20.3, bundle 3525.0) using the repository build entry point. App and
DMG generation completed; `codesign --verify --deep --strict` passed for the
built and installed app. The installed executable's SHA-256 matches the built
one. The prior app was preserved in a temporary backup, the installed app was
replaced, and a new process launched successfully. No application data was
modified by the installation procedure. This local build is signed, not
notarized or published. The real-account badge outcome remains to be observed.
