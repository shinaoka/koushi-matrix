# 2026-10-22 — Emoji picker frame resizing (#1583)

## Scope

The emoji picker popup's own frame, not native window resizing. #1522 shipped a
single placement-following corner; #1583 makes every edge and every corner a
resize target with the opposite side pinned.

## Canon consulted

- `REPOSITORY_RULES.md` — headless-first verification, presentation-state
  ownership, logical CSS properties, user-guide consistency.
- `docs/architecture/state-machine.md` "Window settings" — the
  `emoji_picker_size` preference stays the only persisted size state; the
  rectangle a drag pins is per-open presentation state and was added to that
  bullet.
- `docs/architecture/i18n.md` — no product text added: the eight frame zones
  reuse `composer.emojiResize` for the one keyboard-operable handle and are
  `aria-hidden` for the pointer-only strips.
- `docs/agents/verification.md` — RED-then-GREEN, no manual GUI evidence.

## Change

- `apps/desktop/src/components/floatingLayer.tsx`: `FloatingPlacementResult`
  now returns the containment `bounds` it clamped into, and
  `pinnedPlacementStyle` clamps a user-pinned rectangle into those same bounds
  while only narrowing the rendered size (the saved preference is untouched).
- `apps/desktop/src/components/EmojiPicker.tsx`: one frame descriptor table
  drives eight handles (four edges, four corners). Each drag records the panel
  rectangle at pointer-down and moves only its own edges, so the anchor-derived
  placement and its flip heuristic never move the panel under the pointer and
  the opposite edge stays put. Logical insets come from the zone, the cursor
  names the physical edge pair, and the root `dir` decides which inline side the
  physical left/right strip lands on (the previous code flipped the drag sign in
  RTL). The marked corner is still the only tab stop and keeps the keyboard
  arrow resizing.
- `apps/desktop/src/styles.css`: the panel owns an 8px frame gutter, so the
  strips and 14px corner squares never cover the search field, the close
  button, the tabs, the grid, or the scrollbar.

## Verification

- `apps/desktop/e2e/emoji-picker-resize.spec.ts` (RED first against `main`:
  every zone probe returned `null`): hit-tests all eight zones and their
  cursors, proves search/close/tabs/grid/body are not covered, then drags all
  four edges and all four corners from the composer (above / inline-start),
  from reaction menus on another user's and on the user's own message (below /
  inline-end), and under RTL, measuring the rendered box and the pinned
  opposite edge. It also proves a drag inside the body does not resize.
- `src/components/EmojiPicker.resize.test.tsx` covers the eight zones, the
  single keyboard handle, and its arrow-key commits.
