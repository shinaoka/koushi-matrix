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

## Release gate observer repair (#1229)

Integrated main's Space rail fixes (`fbdb7211`) without conflicts. Post-merge
frontend typecheck, lint, build, 1771 Vitest tests and 14 affected DOM checks
passed. The dedicated full browser run passed 476 checks; Linux native
signed-out smoke and approved real-homeserver QA also passed with cleanup.

CI run 37847153289 exposed an existing test observer defect: the one-second
intent lifecycle waiter returned failure on its first 20ms quiet interval.
A new real-command test delivers the correlated outcome after an 80ms quiet
interval; it failed before the repair and passes after it. Awaiting the
remaining absolute deadline fixes the observer without extending the flood
test's one-second limit or changing product scheduling. All nine tests in
`runtime_intent_lifecycle` pass, including the original flood test. This is
an oracle repair under the existing deadline contract, not a state-machine
change; no canon or wire amendment is needed. The failed CI record remains
available rather than being hidden by an unchanged rerun.
