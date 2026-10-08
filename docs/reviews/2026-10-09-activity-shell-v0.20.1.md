# Activity shell containment and v0.20.1

Refs #1222.

## Cause and change

The room access indicator change in `5bb28804` added absolutely positioned
screen-reader descriptions to room rows. Without a row containing block,
those descriptions were positioned against `.app-grid` and escaped the
sidebar clipping boundary. A long room list enlarged `.desktop`'s scrollable
overflow, and focused timeline `scrollIntoView` also scrolled that shell.

Set `position: relative` on `.room-item` to contain its description inside
the sidebar. Preserve the accessible description and the timeline's target
centering. Synchronize the desktop version files to 0.20.1 without dependency
changes. Add the Activity shell regression to both Chromium and WebKit.

## Review

Consulted REPOSITORY_RULES (root-cause discipline, QA, privacy, review),
verification, engineering gate policy, i18n layout ownership, and the desktop
release runbook. CSS owns this presentation defect; no Rust state machine or
wire contract changes are needed. The production RoomButton description and
TimelineView focused-anchor scroll paths match the exercised DOM path.

The help pages still describe the same Activity navigation and room access
information. No user action, label, prerequisite, or menu placement changes,
so no help text update is needed. `user-help.mjs --check` passed.

## Verify-first evidence

The new headless check uses 35 synthetic sidebar rows and an Activity target
at the end of a 35-message focused timeline. Before the CSS change, Chromium
reported shell scrollTop 126 and WebKit 127; both reported scrollHeight 1663
for an 800px viewport. The same test passes after the change with scrollTop 0
and scrollHeight 800, while asserting the timeline scrolls to the target and
the sidebar remains internally scrollable. Native GUI confirmation is optional.
