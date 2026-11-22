# Grouping consecutive notices into one line (#2041 P2)

A burst of state events used to fill the timeline: four profile-picture changes in
a row were four notice lines, each with a sender name and a sentence that said
almost the same thing. P1 (#2044) turned a notice into one muted information line;
this change makes a burst of the same notice one line with a count.

## Where the decision lives

Rust owns row visibility and order (`timelineStore.ts` carries the same rule for
the display-policy acknowledgement, #1110), so the grouping is a display-projection
concern in Core, not a render-time filter over whatever React happens to hold.

`group_consecutive_notices` runs at the end of `project_display_items`, after every row
has its `TimelineDisplayMetadata`, and writes three additive fields onto it:

- `group_count` — how many notices the row stands for; absent (or 1) on the wire
  means an ordinary row,
- `group_position` — 0-based position, so the head is the row the reader sees,
- `group_id` — `notice-group:<head event id>`, stable while the group grows.

Every notice keeps its own row in the canonical timeline, in `navigation_items`
and in the DTO stream. Grouping changes metadata only, so search, read state,
pins, reactions and receipts still address the individual event.

## What breaks a run

The pass walks the display items in order and closes the run on anything that is
not a groupable notice: a message, a membership line (it carries no localized
notice kind, so it is never counted — a join or a leave stays its own line), a
call, a date divider, the hidden `read-marker` synthetic row, the `timeline-start`
row, an undecryptable or unsupported event, or the end of the window. Two more
rules:

- **Same sentence** — the grouping key is `sender` plus the structured
  `notice_i18n` value (kind and its parameters), never the rendered body. Two
  different renames never collapse into one line.
- **Five minutes** — `NOTICE_GROUP_MAX_GAP_MS` is the single threshold, read by
  the tests instead of repeated. A timestamp that runs backwards does not join.

Notices whose row carries per-event work are never counted: an undecryptable
message keeps its key-request control, a call and a poll keep their own wording,
and a malformed or unsupported event keeps its explanation. A positive "state
notice only" list would rot as new notice kinds appear; the exclusion names the
rows that must stay individually reachable.

Because the date divider and the read marker are rows of their own, a group can
never span a day boundary or the unread divider — the divider keeps the row it
anchors to. Thread and focused timelines do not group (`room_window` gate): a
focused pane exists to show one event.

## What the renderer does

`projectNoticeGroups` keeps a collapsed group's head and drops its members;
`TimelineView` holds the expanded group ids in session memory. A group also opens
on its own when it holds a row the view is pointed at — the navigation target
(`inPlaceTarget`, the pinned/focused target), the unread divider's event, or the
read marker's — because those rows must exist to be scrolled to and to carry their
divider. The pane keys this view by room, so a room switch is a remount and starts
collapsed again; a test pins that with a keyed child, not by assumption.

The head line keeps the `· 4` count as a disclosure button (`aria-expanded`) whose
accessible name comes from the message catalogue in all five locales. The open set
comes back from the same projection that drops the rows, so a group drawn open
because it holds the navigation target also reports `aria-expanded="true"` — a
head that reported itself closed would answer the button with no visible change.
Opening the line shows each event's own time, which a collapsed line deliberately
does not carry — that is the one part of P1's "no timestamp" rule this change
narrows.

Row counts only fall, so no virtualization budget moves; `renderer-perf-budget`
and `room-switch-layout-budget` pass unchanged.

## Evidence

- Core: `display_projection/issue_2041_tests.rs` — four consecutive profile
  changes are one group; a pin between them splits 2/1/1; a message splits; five
  minutes joins and five minutes plus a millisecond splits; a date divider and the
  read marker split; different senders and undecryptable/call notices do not
  group; a window starting mid-burst makes its own head.
- Desktop: `TimelineView.notice-groups.test.tsx` (one line with the count, opens to
  four rows with their own times, closes again, a replaced view starts collapsed,
  a single notice offers no count) and `timelineDisplayProjection.test.ts` for the
  collapse rule.
- e2e: `timeline-notice-grouping.spec.ts` on the full-app harness, plus the two
  budget specs.

## Known limits

- A search or pin jump to a notice that a collapsed group hides lands on nothing,
  the same way a jump to a hidden row does today. The target's group opens
  automatically when the target arrives as `inPlaceTarget`; a jump that resolves
  to an event Core has not published as a row is unchanged by this work.
- The group head is the oldest loaded notice, so a bounded window that starts in
  the middle of a burst shows a smaller count than the room history holds. When
  older notices load the head moves, so the row set changes at the top and the
  backfill loses its pure-prepend shape for that batch; the reader's row is still
  anchored by the ordinary viewport transaction.
- A repair gap can fall inside a burst, because the desktop inserts the gap
  placeholder after Core projects the rows. The placeholder itself stays visible
  (it is not a group member), so the hole is still signposted, but the count on
  the line spans it.
