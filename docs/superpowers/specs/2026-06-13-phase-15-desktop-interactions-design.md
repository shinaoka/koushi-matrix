# Phase 15 Desktop Interactions Design

Date: 2026-06-13
Status: accepted design for Phase 15 implementation.

Canon: [overview.md](../../architecture/overview.md) and
[engineering-rules.md](../../policies/engineering-rules.md). This spec narrows
the Phase 15 roadmap item; canon remains the authority.

## Scope

Phase 15 makes the app feel native while keeping every behavior verifiable by
headless or Linux virtual-display QA:

- desktop notifications for notification-worthy Matrix changes
- dock/taskbar badge count and unread window-title hint
- native window-state persistence evidence on the Linux lane
- minimal accessibility pass over the three-pane shell
- shortcut parity table completion

The phase does not introduce new Matrix commands for notification settings or
per-room notification rules. It consumes notification counts and unread state
already projected by the SDK/core pipeline.

## Notification Payload Contract

Core/state may expose a notification decision surface as serializable UI data.
The payload is intentionally smaller than timeline or room state.

Allowed fields:

- `room_display_name`: the same visible room label already allowed in
  `AppState`; for QA tokens this may be replaced by a redacted label
- `kind`: `mention`, `dm`, or `message`
- `notification_count` and `highlight_count`
- coarse unread total used for badges and title hints

Forbidden fields:

- message body or formatted body
- sender display name or Matrix ID
- room ID, event ID, transaction ID, thread root ID
- access tokens, recovery material, store/search keys, raw SDK errors

The adapter may show a redacted OS notification such as "New mention" with the
allowed room label. It must not include message content by default. A future
settings phase may add an explicit opt-in body preview, but Phase 15 does not.

## Decision Rules

The desktop attention decision is pure and testable:

1. Compute total unread from `AppState` room summaries.
2. Compute highest priority as `mention` when any room has highlight count,
   otherwise `dm` when any DM room has unread notifications, otherwise
   `message` when any room has unread notifications.
3. Emit a notification candidate only when the unread/highlight signal
   increases and the room is not the focused room.
4. Browser/headless tests assert candidates through a mocked adapter; Linux
   GUI smoke asserts an observable native/QA token without using private
   screenshots.

Badge counts and window-title hints derive from the same unread total. QA title
tokens stay private-data-free and are allowed to include `unread=N`,
`badge=N`, and `notify=<kind|none>`.

## Adapter Contract

React owns DOM focus, accessibility, and visual unread affordances. Tauri owns
native capabilities:

- badge count through the Tauri window API
- OS notification through a small adapter wrapper that is mockable in
  headless tests
- persisted native window state through the existing Tauri window-state code

If Linux DBus notification observation is unavailable under Xvfb, the phase may
fall back to a deterministic QA title token plus a documented gap, but only
after attempting a native assertion in the Linux lane.

## Accessibility Minimum

The three-pane shell must expose stable landmarks or labelled regions for:

- spaces/room navigation
- room timeline main area
- contextual side panel when open

Keyboard-only walkthrough must prove focus can move through primary
application controls without trapping or skipping the composer/search/panel
entry points.
