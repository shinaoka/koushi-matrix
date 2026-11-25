# The rail's Account Settings button shows the account, not a painted dot (#2088)

The button at the bottom of the Space rail was a green circle with a smaller green dot,
painted by one CSS rule (`.user-presence::before`, `styles.css`: `background: var(--success)`
plus `box-shadow: 15px 13px 0 -8px var(--success)`). Nothing in the drawing came from the
account, the connection or presence, so the only way to learn what the button does was the
tooltip or a click — and the shape read as a status light, because the same green is the ready
dot on an account tab and the online dot on a direct message avatar.

## What changed

- `WorkspaceRail` takes the selected account tab (`accountTab`) and draws its avatar in the
  button: the profile image when Core has a renderable `avatarSourceRef`, otherwise the
  account's initial on its fallback colour. The source, the initial and the colour class come
  from the rules the account tab strip already uses (`renderableThumbnailSourceUrl`,
  `avatarInitial`, `avatarColorClass`). What the two do not share is the glyph colour: the rail
  sets `var(--avatar-contrast)`, while the tab strip inherits `--text-muted` from `.account-tab`,
  which is a pre-existing contrast defect on the tab side and is not this issue's surface.
- The image goes through the shared `useRecoverableImageSource`, the same hook every other
  avatar uses: a broken image falls back to the initial, and the source is retried on focus,
  online and after 10s.
- A tab with no bound account renders an avatar belonging to nobody, so it gets a generic
  account glyph (`UserRound`) instead. In today's app the rail only renders for a ready session
  (`AccountContent` returns `AuthScreen` first), so this branch covers a parent that has not
  published the tab yet, and keeps the button honest if an unbound tab is ever selectable there.
- `App.tsx` passes the tab: `accountTabs?.tabs.find((tab) => tab.id === accountTabId)`. No new
  DTO and no new demand path — `#2083` already publishes `avatarSourceRef`, and
  `useAccountTabAvatarDemands` already requests the thumbnail for every tab.
- CSS: only the `::before` painting is removed. `.rail-account-avatar` is a circle inside the
  44px control, `overflow: hidden`, `object-fit: cover` for the image and `var(--avatar-contrast)`
  for the initial. The button gets `padding: 0`, because the UA keeps 6px inline and 1px block
  of its own and the avatar's percentages resolve against that content box: measured with padding
  the avatar was 24x34, an ellipse. With the padding zeroed, `calc(100% - 8px)` is 36px wide and
  30px in compact mode, and it follows the button's own size token without repeating it. The button keeps `--workspace-control-size` and
  `--r-md`, so the rail's layout, the compact-mode sizes and the drag/overflow geometry do not
  change.
- `docs/design/ui-design-language.md`: the Space rail line said "account avatar (presence
  ring)"; there is no presence ring now, so it says what is drawn.

`aria-label` (`workspace.userSettings`), the tooltip, the click handler and the account context
menu are untouched.

## Testing

- `Shell.test.tsx`, new `rail account avatar (#2088)` block: the ready image, the initial on the
  tab's own colour class, the image failure returning to the initial, the generic glyph for an
  unbound tab, and the unchanged click / context-menu calls. Against the pre-change sources the
  first four fail and the last one passes, which is what pins "only the drawing changed".
- `styles.contract.test.ts`: no `.user-presence::before`, no `var(--success)` inside a
  `.user-presence` block, no `box-shadow: 15px 13px 0 -8px`, and `.rail-account-avatar` keeps
  its clip, circle, contrast colour and `object-fit: cover`.
- `e2e/workspace-rail-overflow.spec.ts` on both engines: 28 passed (the new wiring case included).
  No budget was raised.
- The wiring case is the only thing that pins `App.tsx`'s prop lookup: every unit test injects
  `accountTab` directly, so with the `App.tsx` line reverted the case fails (`element(s) not
  found` for `.rail-account-avatar`) while the whole unit suite stays green.
- `apps/desktop-shell` (the tracked prototype) drew the same green circle and dot; it now draws
  the avatar circle too, so the design doc and the prototype agree.

## Left alone

`.account-tab-avatar` has no `color` of its own, so the tab strip's initial inherits
`--text-muted` on the account colour (about 1.2:1 on `--avatar-1`). The rail sets
`var(--avatar-contrast)`. Fixing the tab is its own change, not part of drawing the avatar on
this button.

The button still has no hover treatment of its own (it never had one); the rail's other tiles
draw hover and focus with their own rules. Adding one here is a separate design question, not
part of replacing the presence drawing with the account's avatar.
