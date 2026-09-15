# Redacted notifications and read-divider evidence (2026-09-15)

## SDK reproduction

Base SDK: `a9e655491`. The synthetic regression
`redacted_event_does_not_retain_notification_counts` creates a notifying,
highlighted message and applies the production
`RoomEventCacheState::apply_redaction_to_event` operation. Before the fix,
recount reports `(unread, notifications, mentions) = (0, 1, 1)` instead of
`(0, 0, 0)`. `TimelineEvent::replace_raw` retains cached push actions, while
`marks_as_unread` alone excludes redacted events.

The fix excludes redacted events at `ReadReceipts::process_event`, before all
three counters. It also repairs recounts of persisted entries without rewriting
the cache. The test covers serialization/restoration, a redacted receipt
boundary, and a later legitimate notification. Non-redacted reactions, edits,
and state-event push actions are not blanket-filtered. The full event-cache
suite passes: 78 tests, zero failures.

This needs an SDK patch: the application consumes SDK-owned aggregate counts;
its public API cannot distinguish a stale redacted contribution and subtract
it without duplicating cache ownership. Upstreaming intent: submit this minimal
counter fix and synthetic test to matrix-rust-sdk. This is currently a fork
patch, not an accepted upstream fix.

## Core divider reproduction

A local viewed boundary can precede a later confirmed receipt. The old Core
projection always preferred the local boundary, causing the divider to appear
before an already confirmed reply. The regression
`confirmed_thread_receipt_cannot_display_before_an_older_local_boundary`
failed with the older local event instead of the later confirmed event.
The fix compares positions within the same canonical timeline and chooses the
newer boundary, mapping hidden events to a visible row at or before it.
Tests also cover reversed/equal boundaries, missing positions, and hidden edits.
The server receipt and unread-count semantics are unchanged.

## Runtime and historical limits

Private diagnostics showed a successful thread receipt and zero thread
attention, while the room retained zero unread events and one notification.
A remaining non-thread notifying cache entry aligned with a redacted row in
the supplied screenshot. No private message contents, account identifiers,
room identifiers, or raw diagnostic attachments are included here.

Old SDK `a04792c7a` already contained both the split counter logic and the
redaction/replace_raw path. The older Core also preferred local display
boundaries. These are reproduced defects, but the precise change that made
them visible during upstream alignment is not established. In particular,
this evidence does not prove a removed historical patch or that every observed
divider movement has this cause. Native confirmation remains a separate step.

Sanitized diagnostic fields now identify redacted cache entries and whether
local/display boundaries precede a confirmed canonical position, without
logging event IDs or message contents.

## Follow-up: returning subscribers miss the read snapshot

After initial native success, the user and a second native check observed the
divider above the latest reply. Reopening/restarting restored it. The new private
diagnostic showed thread `replay_initial_emitted` without a corresponding new
navigation publication; it did not establish a backward server receipt update.

The real-actor regression
`replay_initial_items_republishes_unchanged_read_navigation` creates a new
subscriber after a confirmed snapshot, requests a replay, and requires
`InitialItems` followed by the same `NavigationUpdated`. It failed before the
fix: only items arrived. A returning frontend clears its view-local navigation
and can then fall back to the room marker instead of the thread marker.

The successful replay branch now forces publication of the current navigation
snapshot. Ordinary incremental deduplication and the actor generation guard
remain in force. This follow-up changes no SDK receipt-authority semantics.
