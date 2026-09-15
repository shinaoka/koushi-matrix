# Thread receipt and notification evidence for upstream submission

Recorded: 2026-09-14. Status: local evidence packet; no upstream issue or PR submitted.

## Scope and confidence

Three distinct SDK changes must remain separate in review. Tests below ran in
the maintained fork, not an otherwise unmodified upstream checkout. Source
comparison establishes that the first regression was introduced by an upstream
commit included in the September upgrade. It does not establish that current
upstream HEAD still has the bug. Before submission, port the synthetic tests to
current upstream and record its exact revision and RED/GREEN results.

No account dumps, message bodies, screenshots, credentials, or real event/room
identifiers are included. The test events use synthetic example.org identifiers.
The desktop notification badge has **not** yet been confirmed fixed in the
installed app; SDK unit-test success is not evidence of that final outcome.

## Revision provenance

| Role | SDK revision |
| --- | --- |
| Fork immediately before upstream merge | `a04792c7ab1c38faedc58e4c8e63bee721a1fb09` |
| Upstream receipt refactor, July 28 | `cff13f7046209a8b6ce4410cde235c1c3baac989` |
| Upstream imported September 12 | `6602de58ea9222f058997960e1c33e7d11a1bc6c` |
| Fork merge commit | `c1c08979f38c0db714dba7abeb93ef6bc5358956` |
| Fork base for first reproduction/fix | `f9d55baf7c1665ba3a4b5d06236a10b7fff5a886` |
| Receipt-boundary fix | `b65d72ba8d29605b06335e625889c08edadbf24e` |
| Thread-edit ownership fix | `9aac22df2862b7e36ad449d3d7bd6fab564d1d55` |

The first fix restores old behavior; it is not evidence that a fork-only patch
was accidentally deleted. The thread-edit filter and subscription recount were
absent in the immediately preceding fork revision too.

## 1. Preserve explicit unthreaded receipt boundaries on thread replies

Suggested PR title: `fix(event-cache): preserve unthreaded receipt boundaries on thread replies`

With threading enabled, a valid explicit room receipt can refer to a thread
reply. The receipt is a chronological boundary even though the reply does not
contribute to the room unread count. The upstream refactor applies the room
filter before both receipt selection and the boundary search, hiding that ID.
This can retain notifications for messages that precede the explicit receipt.

The old implementation iterated all event IDs in `select_best_receipt`, applying
the thread exclusion only to implicit own-message receipts. It also passed all
events to `find_and_process_events`, excluding threads during counting. Commit
`cff13f704` instead introduced `event_filter.filter(event)` before those two
searches. Its ancestry in the imported upstream commit was checked locally.

Minimal reproduction: a main event followed by a thread reply, threading enabled,
and an explicit unthreaded receipt on the reply. Assert that the main unread and
notification counters clear while the boundary remains the reply. Cover active,
sync-delivered, and stored receipts; keep implicit thread replies from marking
unseen main messages as read.

Tests in `crates/matrix-sdk/src/event_cache/caches/read_receipts.rs`:

- `unthreaded_receipt_on_thread_reply_preserves_room_read_boundary`
- `unthreaded_reply_receipt_is_matched_from_sync_and_store`

RED: first test failed with actual 1, expected 0 before production edits.
GREEN: the receipt suite passed 20 tests after `b65d72ba8`.

## 2. Attribute notifying thread edits to their thread

Suggested PR title: `fix(event-cache): exclude thread-related edits from room notifications`

Minimal reproduction: a read main root, a thread reply, and a notifying,
highlighting `m.replace` targeting that reply. The edit has no direct `m.thread`
relation. Excluding only direct replies therefore leaves room unread=0 but
notification=1 and mention=1. The edit must retain its notification in its own
thread until read there.

The fix determines one-hop ownership from loaded events or an already-locked
event store, matching the thread aggregator convention. It preserves main and
unknown-target notifications. It introduces no network request. Review separately
whether this ownership policy should be shared with upstream aggregation code.
The pre-upgrade implementation also only excluded direct thread replies: this
is a previously unhandled case, not a demonstrated regression from that merge.

Tests in the same file:

- `thread_edit_does_not_leave_room_notification_after_thread_is_read`
- `related_thread_filter_preserves_main_notifications_and_explicit_receipts`

RED: notifying edit left actual 1, expected 0 before the ownership fix.
GREEN: receipt suite passed 22 tests after `9aac22df2`. Coverage includes stored
and loaded targets, main/unknown edits, own edits, threading disabled, explicit
boundaries, and thread notification retention.

## 3. Reconcile persisted counts when opening a cached room

Local follow-up commit `a9e655491`; independently reviewed with no blocking findings. Not submitted upstream.
Do not present this as an upstream regression proven by historical bisect.

Correcting a filter does not necessarily repair already persisted RoomInfo
counts: subscription previously returned cached events without recomputation,
and a completely empty sync update can return without receipt post-processing.
Duplicate-only updates are different: they already run post-processing.

The synthetic test seeds cached events and stale RoomInfo counts, then opens a
subscription without another sync. This models restored state; it is **not** a
full process restart/persistent-store reopen test. RED on `9aac22df2` plus the
new test: actual notifications 1, expected 0. Candidate GREEN: one focused test;
expanded event-cache suite: 77 passed, 0 failed.

Test in `crates/matrix-sdk/src/event_cache/caches/room/mod.rs`:
`test_subscribe_recounts_cached_thread_edit_notifications`.

The candidate recomputes only if the active marker is loaded with no later gap.
It preserves missing-boundary/gapped counts, retains valid unread main events,
and checks repeated subscriptions. A single write guard covers recount,
snapshot, and subscriber registration. RoomInfo save failure is logged by the
existing helper, so subscribe success alone is not a durability guarantee.

## Reproduce and export

Run these from the SDK repository, using an external CARGO_TARGET_DIR if desired:

```sh
cargo test -p matrix-sdk --lib unthreaded_receipt_on_thread_reply_preserves_room_read_boundary
cargo test -p matrix-sdk --lib unthreaded_reply_receipt_is_matched_from_sync_and_store
cargo test -p matrix-sdk --lib thread_edit_does_not_leave_room_notification_after_thread_is_read
cargo test -p matrix-sdk --lib related_thread_filter_preserves_main_notifications_and_explicit_receipts
cargo test -p matrix-sdk --lib test_subscribe_recounts_cached_thread_edit_notifications
cargo test -p matrix-sdk --lib event_cache::caches
```

For a RED run, transplant only the named synthetic tests onto the corresponding
pre-fix revision, without the production hunks. For the first two fixes, export
individual commits rather than the entire fork or desktop changes:

```sh
git format-patch -1 b65d72ba8 --stdout > receipt-boundary.patch
git format-patch -1 9aac22df2 --stdout > thread-edit-ownership.patch
```

The second commit follows the first; adapt or retain that dependency explicitly.
Port to current upstream APIs before proposing either patch. Keep the Core
viewport/read-state changes and the diagnostic instrumentation out of SDK PRs.

Historical source checks, independent of compiling old workspaces:

```sh
git show cff13f704 -- crates/matrix-sdk/src/event_cache/caches/read_receipts.rs
git show a04792c7a:crates/matrix-sdk/src/event_cache/caches/read_receipts.rs
git show a04792c7a:crates/matrix-sdk/src/event_cache/caches/room/mod.rs
git merge-base --is-ancestor cff13f704 6602de58e
```

## Preserved evidence

[Outcome-only logs](evidence/2026-09-14-thread-unread/) contain the synthetic
RED/GREEN assertions and suite results. [Source log hashes](evidence/2026-09-14-thread-unread/source-log-hashes.md)
identify the temporary originals. These excerpts omit compiler machine paths,
timestamps, process IDs, and unrelated output; they are not full raw logs.

Before opening upstream PRs, record a clean upstream reproduction, full target
revision, final ported commit IDs, and review results. The third change also needs a durable restart test if restart recovery is claimed.
