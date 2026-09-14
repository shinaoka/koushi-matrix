# Thread unread convergence — 2026-09-14

## Scope

Started from origin/main `526314fa` in an isolated checkout. Existing changes in
the other checkout were left untouched. No real-account contents are included
in fixtures or this report.

Room navigation counted hidden thread replies even after their thread was read.
A hidden reply at the canonical tail could also prevent a visible main event
from becoming the room's viewed boundary. Both now respect the displayed main
conversation, while retaining canonical positions for receipt lookup.

The SDK pin is advanced from `f9d55baf7c1665ba3a4b5d06236a10b7fff5a886` to
the local fix commit `b65d72ba8`. Explicit unthreaded receipts may point at a
thread reply left by an older client. Filtering replies before matching the
receipt discarded that chronological boundary. Match explicit receipts against
all events, but continue excluding replies from room counts and implicit own
message read advancement. The tests cover active, sync-delivered, and persisted
receipts, including notification and mention count clearing.

The app's stale-edit notification repair now uses the SDK-local notification
counter, matching the room list. It no longer promotes private room receipts
to unseen thread replies. Existing edit repair remains in place.

## Element X comparison

Element X iOS `TimelineProxy.sendReadReceipt` and Android
`RustTimeline.sendReadReceipt` delegate to the SDK Timeline receipt API:

- <https://github.com/element-hq/element-x-ios/blob/develop/ElementX/Sources/Services/Timeline/TimelineProxy.swift>
- <https://github.com/element-hq/element-x-android/blob/develop/libraries/matrix/impl/src/main/kotlin/io/element/android/libraries/matrix/impl/timeline/RustTimeline.kt>

Koushi intentionally retains its direct Room receipt API, as required by the
repository's stale-count convergence contract: Timeline deduplication can
suppress a needed resend. This change aligns counting and receipt scope without
replacing that mechanism. The SDK filtering regression was reproduced with
synthetic data; it does not prove the cause of every notification in the user's
anonymized diagnostic stream.

## Verification

Before production edits, new headless tests failed for room reply recounting,
hidden-tail viewed-boundary progress, SDK receipt-boundary loss, private receipt
promotion, and local-versus-server notification selection. After the fixes:

- Core library: 1,072 passed, 9 ignored, no failures.
- SDK event-cache read receipt suite: 20 passed.
- Related TimelineView thread and settings UI tests: 47 passed.
- Frontend typecheck/build, release configuration preflight, agent-doc checks,
  and whitespace checks passed.
- npm lockfile/full audits: no high or critical findings; two moderate Vitest
  advisories remain. Runtime-only audit: no vulnerabilities.
- Independent design and final integrated reviews: approved, no blockers.

The ordering-setting regression test preserves all six thread replies when
switching LatestReply/RootEvent. It does not reproduce or establish a fix for
the reported one-visible-reply scroll/fetch symptom. That symptom remains
unconfirmed without post-toggle viewport/pagination evidence.

The lockfile's desktop package version was synchronized to the existing 0.9.0
manifest by Cargo. No product version bump was introduced by this fix.
