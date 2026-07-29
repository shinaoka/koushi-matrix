# Activity Edit Identity and Anchored Navigation Repair

## Problem

Activity combines timeline-observed rows with each room's latest-event summary.
For an edited message those sources use different Matrix event IDs: the timeline
uses the original event while latest-event can use the `m.replace` event. Exact
ID deduplication therefore renders the same logical message twice and can make
navigation target a relation event that is not a standalone timeline item.

Focused navigation also commits `main_timeline_anchor` after transport
acknowledgement alone. An accepted projection may contain zero items or omit the
requested target, producing a blank anchored pane with only “Jump to latest”.

## Design

### Canonical Activity identity

Carry `relation_type` and `relation_event_id` from
`MatrixRoomLatestEventSummary` into the Rust-owned `RoomLatestEventSummary`.
Define the displayed identity as:

- `m.replace`: the non-empty relation target ID;
- `m.annotation`: no standalone Activity row;
- otherwise: the event's own ID.

The latest-event append path compares this displayed identity with
timeline-derived rows. If the timeline row exists, it wins because it carries
the original timestamp and profile-enriched avatar. If no timeline row exists,
the synthesized latest-event row uses the displayed identity, so opening it
targets the renderable original event.

Unread calculations continue to use the latest source event for room recency,
while row identity, clearing, and navigation use the displayed event ID.

### Projection evidence and fallback

The WebView acknowledgement of an `InitialItems` projection must include
privacy-safe evidence derived from the canonical timeline store:

- projected item count;
- whether the requested focused target is present.

Core accepts the acknowledgement only for the exact request/key/generation
owner as today, but commits `EnterAnchoredTimeline` only when the target is
present. A matching acknowledgement without the target closes the focused
context and leaves the already-selected live room timeline active. The desktop
command treats that explicit fallback as a successful room navigation rather
than waiting for an anchor until timeout.

The target-presence check uses event IDs only in memory; diagnostics report
counts and outcome tokens, not raw room/event IDs.

### Focused context integrity

Preserve `TimelineEventFocusThreadMode::Automatic {
hide_threaded_events
}` across the matrix-sdk-ui/event-cache boundary. The event-cache enum and cache
key include this boolean, and the room-focused filter uses it instead of
unconditionally hiding threaded events.

An empty cached focused context must not be permanently sticky. The cache
boundary gains a bounded refresh path used by a new focused subscription when
the cached vector is empty. It performs at most one `/context` recreation for
that open request and then projects either target-present items or an explicit
target-missing fallback.

## Diagnostics

Add private-data-free `core.activity_navigation` lifecycle records for:

- focused projection acknowledged (`item_count`, `target_present`);
- anchor committed;
- live fallback (`target_missing` or `empty_projection`);
- focused cache refresh attempted and its count/outcome.

## Verification

Behavioral tests cover:

1. original timeline row plus `m.replace` latest summary yields one enriched row
   keyed by the original event;
2. annotation latest summaries do not create standalone rows;
3. accepted projections without the target cannot commit an anchor and select
   the live fallback;
4. target-present projections commit the anchor once;
5. `hide_threaded_events: false` survives the SDK boundary;
6. an existing empty focused cache is refreshed once rather than reused forever.
