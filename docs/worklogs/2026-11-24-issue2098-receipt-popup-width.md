# The read-receipt popup hugs its content (#2098)

Reported: with one reader the popup was a mostly empty box — `[T] Takahiro Misawa` on the
left, the timestamp pushed to the far right, and the panel far wider than either.

Two independent causes, both old:

1. `ReceiptReaders.tsx` passed `inlineSize: RECEIPT_POPUP_INLINE_SIZE_PX` (**420**) to
   `useFloatingPlacement`, and `floatingPlacementStyle` writes that number onto the panel as
   an inline style. The popup was 420px wide whatever it contained.
2. `.receipt-reader-name` was `flex: 1 1 auto`, so the name grew to absorb every pixel the
   pinned width left over and the timestamp landed on the far edge.

## What changed

- The popup measures itself (`contentRef: popupRef`) instead of being handed a constant
  width, and the width comes from CSS: `inline-size: max-content` bounded by
  `max-inline-size: min(--receipt-tooltip-max-inline-size, --receipt-tooltip-max-viewport-inline-size)`
  — the two existing tokens, applied to the panel they were named for. No new token.
- It also passes `false` to `floatingPlacementStyle`, which is the other half of the same
  mechanism: with the default the layer writes its measurement back onto the panel as inline
  styles, and the next measurement then reads that pin instead of the content. The receipt
  popup opens on the compact publication (capped at 3 readers) and the reader window arrives
  afterwards, so the pinned box stayed at the height of the first one or two rows — 38px tall
  with twelve rows inside it. `Shell.tsx` already calls the same helper the same way for its
  content-measured menu.
- `.receipt-reader-name` is `flex: 0 1 auto`: it hugs its text, shrinks, and truncates. The
  timestamp follows at the row's fixed `gap: 6px` instead of being pushed right.
- `RECEIPT_POPUP_INLINE_SIZE_PX` and the `receiptPopupBlockSize` row-count arithmetic are
  gone. The height still follows the reader count because each row is a fixed 20px,
  `gap: 3px` separates them, and `align-content: start` stops residual slack from stretching
  a row (#360). The measured box is that height, so the pin and the content agree.

Kept, unchanged: one reader per line, ellipsis instead of wrapping, the reserved
`padding-inline-end: 28px` for the close control, `max-block-size: min(60vh, 420px)` with
`overflow-y: auto` for long lists, and the pane/viewport boundary clamp in
`resolveFloatingPlacement`.

Rows with several readers keep their own natural width and the popup takes the widest of
them; the shorter rows therefore have empty space after their timestamp, which is the
requested behaviour (timestamps are not column-aligned).

## Measured (chromium, 1280×800, one reader "Ann" + a timestamp)

| | before | after |
| --- | --- | --- |
| popup width | 420.0 px | 218.3 px |
| name box vs its own text width | 224.9 px vs 23.3 px | 23.3 px vs 23.3 px |
| name → timestamp gap | 6 px, with the timestamp at the far edge | 6 px, immediately after the name |
| empty space after the timestamp | — (the name absorbed it) | 0 px |
| `max-inline-size` | `none` | 280 px |
| height with 12 rows after opening at 1 row | 38 px (pinned; rows scrolled) | 291 px (twelve 20px rows + gaps + padding) |

The popup width is now exactly its content: `chrome + occupied` where `chrome` is read from
`getComputedStyle` (10px padding start, the 28px close-control reserve, 2px of borders) and
`occupied` is measured from the row's first child to the end of the timestamp. Both come from
the DOM, so no assertion depends on the runner's fonts.

## RED → GREEN

Three assertions in `apps/desktop/e2e/receipt-popup-layout.spec.ts`, all numeric and
font-independent (the name's own text width comes from a `Range` over its text node, so no
expectation depends on the runner's fonts):

- short name: popup width equals `chrome + row content` within 1px, the name box equals its
  text width, the name→timestamp gap equals the row's `column-gap` within 1px, and the
  width is below 220px.
- long name: `max-inline-size` is a length below 420px, the popup respects it, the name's
  `scrollWidth` exceeds its box (ellipsised), the row stays 20px (one line).
- several readers: the popup equals `chrome + widest row`, the short row's name box is
  narrower than the long row's, and the popup does not scroll.
- rows that arrive after the popup opened: it grows to twelve rows (291px) instead of staying
  at the pinned one-row height, and does not scroll.

Against the pre-change code these fail with `420` vs `chrome + content`, `max-inline-size:
none` (`NaN`), the name box measuring `224.9px` for a `23.3px` name, and a 38px popup holding
twelve rows. The two existing `1`/`2`-row tests were **not** changed — the new width rule
still satisfies them.

`styles.contract.test.ts` now expects the `.receipt-tooltip` block to read
`--receipt-tooltip-max-inline-size` and `--receipt-tooltip-max-viewport-inline-size`.

## Canon

`docs/agents/state-ownership.md` said a floating-layer popup's size is owned by the
measurement, never by CSS, and named `receiptPopupBlockSize` as the way to keep the height
proportional. Both were true only for a panel handed a constant size. The rule now says a
content-sized panel passes `contentRef` **and** `false` to `floatingPlacementStyle` (so CSS
owns the box, the measurement follows it, and the layer does not pin its own reading back),
keeps the warning about a stylesheet size on a caller-sized panel, and states the #360
invariant by its mechanism (fixed-height rows, `align-content: start`) instead of the
deleted helper.

No user-guide page describes the popup's geometry, so no help change was needed
(`docs/help/` has no reader-popup entry).

## Verification

- `npm --prefix apps/desktop run typecheck`, `run lint`, `test -- --run` — 217 files,
  2729 tests.
- `npx playwright test e2e/receipt-popup-layout.spec.ts --project=chromium --project=webkit
  --repeat-each=2` — 10 passed.
- `npx playwright test e2e/renderer-perf-budget.spec.ts e2e/receipt-popup-layout.spec.ts
  --project=chromium` — 13 passed, 1 skipped. No budget value was raised.

## Not covered

- The layer no longer narrows the panel, so the pane clamp now comes from CSS alone: the cap
  is 280px and 80vw, both wider than the panes this app allows
  (`MIN_RIGHT_PANEL_WIDTH = 320`, window `minWidth: 760`), so nothing reachable relies on the
  clamp. A future pane narrower than 280px would let the popup extend past it rather than
  shrink, which is the one behaviour this change gives up.
- A very tall reader list near the top of a short pane is no longer shortened to fit either;
  `max-block-size: min(60vh, 420px)` with `overflow-y: auto` bounds it instead. If that ever
  overlaps its trigger, `shrinkToAnchor: false` is the documented option for a self-scrolling
  panel (#1839), not a second sizing path.
- The review's font-dependence point was applied: the absolute `width < 220px` assertion is
  gone, because the timestamp's width is font metrics and the pixel-exact `chrome + occupied`
  assertion already states the claim.
- The screenshot in the report was not re-taken on macOS; the numbers above are from the
  headless harness at 1280×800.
