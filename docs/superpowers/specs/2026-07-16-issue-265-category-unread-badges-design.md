# Category Unread Badges Design (#265)

## Goal

Show simultaneous, accessible unread and highlight attention for the DMs and
Rooms category tabs without recomputing Matrix semantics in React.

## Design

`Sidebar` passes each category's total length plus the corresponding
`SidebarModel` unread/highlight aggregates to `RoomListControls`:

- DMs: `global_dms.length`, `dm_unread_count`, `dm_highlight_count`;
- Rooms: `space_rooms.length`, `space_unread_count`,
  `space_highlight_count`.

Those Rust projections already implement Home/active-space scoping, marked
unread, mute exclusion, and highlight counts. React only formats counts for
display (`99+` above 99). The total stays in its existing quiet count capsule;
nonzero unread appears in a separate attention capsule, with a highlight class
when the projected highlight count is nonzero. The button accessible name
states category, unread count, total count, and highlight count when present.
Zero unread is omitted visually but remains explicit in the accessible name.

CSS keeps the two meanings distinct in selected/unselected, focus, hover,
light/dark, forced-colors, and narrow-width states. Category selection and its
local persistence are unchanged.

## Verification

Browser-headless tests seed Rust-shaped sidebar snapshots and prove both
categories remain visible and live-update while either category is selected,
including zero/one-sided/both, highlight, `99+`, mark-read deltas, active-space
scope, accessible names, and persisted selection. Existing Rust sidebar tests
remain the authority for mute and marked-unread aggregation.

