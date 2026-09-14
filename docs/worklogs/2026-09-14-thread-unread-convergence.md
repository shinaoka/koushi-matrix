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
- Headless Core QA binary tests: 105 passed (unit tests, not a live homeserver
  scenario run).
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

## Follow-up: residual notification after installation

The next supplied diagnostic showed Room navigation unread/newer counts at zero
and successful room and thread receipt requests, but SDK notification/mention
counts still at one. The first fix therefore did not resolve the reported
notification badge. It was insufficient to test only direct thread replies.

A new failing SDK test reproduced the exact count combination using a notifying
edit of a thread reply after the main read boundary. `m.replace` has no direct
thread relation; its target determines its thread ownership. SDK topic
`9aac22df2` adds that lookup to the Room filter, including a target evicted from
the loaded window but present in the event store. Main edits and unknown targets
retain their notifications. Thread notification counting and explicit receipt
boundaries remain unchanged. The focused receipt suite passes 22 tests and an
independent final review approved the actual diff. This is synthetic behavioral
evidence; real-account badge convergence still needs observation after update.


The follow-up build 2733.0 was installed and inspected through native
accessibility: the real-account badge still showed one notification/mention.
Therefore the edit-ownership fix is not claimed to resolve the reported badge.
The SDK cache suite passed 76 tests and Core passed 1,072 (9 ignored).
Additional privacy-preserving diagnostics record cache push flags and indexed
relation targets, plus the Room active receipt position and local counts at
actor startup. These are startup snapshots, not continuous recount telemetry;
a Thread actor label still observes the Room cache. No message content or raw
identifiers are added. Independent review approved the diagnostic fields.


## Cached count reconciliation follow-up

SDK `a9e655491` recomputes persisted RoomInfo counts when a room cache is
subscribed with a complete loaded suffix from the active receipt. Missing
boundaries and gaps preserve existing counts. The synthetic RED test returned
notifications=1 instead of 0 before the change; focused GREEN and the expanded
77-test event-cache suite passed. Independent final diff review approved. The
test seeds restored state; it does not simulate a process restart.

Before installing this build, native inspection already showed room unread=0
and all six thread replies in the previous app. Therefore the disappearance
cannot be attributed to the new subscription repair; later sync can already
trigger the corrected filter. The repair addresses recovery without new sync.


Build 2736.0 (desktop `1a8867d2`, SDK `a9e655491`) was signed, installed
to `/Applications/Koushi.app`, and relaunched. Signature verification and
source/installed executable SHA-256 equality passed. Core: 1,072 passed,
9 ignored. Frontend and app build passed. Native UI after relaunch showed
room unread=0. The thread summary showed six replies; accessibility initially
exposed two event action groups and four after reopening, so this observation
does not establish that all six replies are simultaneously rendered or resolve
the separate one-visible-reply report. Old application backup and installation
manifest are under `artifacts/cache-recount-2736/` (local, untracked).
