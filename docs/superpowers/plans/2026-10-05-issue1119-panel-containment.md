# Right-Panel And Composer Containment (#1119, #1121 Phase B)

Date: 2026-10-05. Canon: engineering rules "Right Panel And Composer
Containment" and i18n "Catalog And Formatting Rules" (truncation). This plan
records the work; it does not override the canon.

## Problem

With two signed-in accounts the composer toolbar names the sending account by
Matrix ID. `.thread-pane` is a grid with explicit rows but no explicit column,
so its implicit `auto` column took the intrinsic minimum width of its items.
A long ID in `.composer-tools` widened that column (about 650-850px inside a
390px panel). The thread header, timeline, and composer ran past the panel, and
the close and send buttons left the window. The panel's own rectangle still
fitted, so the #452 outer-fit checks (`right-panel-clipping.spec.ts`) passed.

## Fix

- Shared shell: `.thread-pane` declares `grid-template-columns: minmax(0, 1fr)`
  and its direct children get `min-inline-size: 0`. Every right-panel mode
  renders this shell, so all modes inherit the boundary.
- Shared composer toolbar: `.composer-tools` shrinks (`min-inline-size: 0`) and
  wraps (`flex-wrap: wrap`, `min-block-size` instead of a fixed height). The
  identity is the yielding item (`flex: 1 1 10ch`, `min-inline-size: 0`, the ID
  ellipsized); the math switch and send button are `flex: none`. At the 320px
  minimum panel width the math switch wraps to a second row instead of running
  past the toolbar edge.
- No breakpoint, overlay-inset, or global overflow changes. Native geometry
  stays in Rust/Tauri.

## Verification

- `apps/desktop/e2e/right-panel-containment.spec.ts` (new). Each case opens the
  thread with the reply-pill click, measures the panel and descendant rects
  plus the toolbar, footer, and header rows, hit-tests close and send with
  `elementFromPoint`, and clicks close at its measured centre, asserting the
  `close_thread` command. It covers:
  - widths 800, 1100, 1190, 1200 and 1400;
  - a boundary walk 1400 down to 760;
  - 800 -> 1400 -> 800;
  - a 1 -> 2 -> 1 account switch at 1100 and 1400;
  - a right-panel resizer drag to the 320px minimum;
  - the main composer;
  - the room info, people and threads-list modes;
  - a nine-row pairwise matrix of density x locale (LTR, accented pseudo,
    RTL bidi pseudo) x inline/overlay layout.
- `apps/desktop/e2e/support/panelGeometry.ts` is the shared geometry helper,
  now used by the #452 outer-fit spec as well.
- Negative controls: removing the shell column fails 19 cases. Removing only
  the toolbar wrap fails the resizer-drag case on the toolbar-row check.

## Not covered headlessly

- Real WebView zoom and OS font-size settings. Headless coverage treats zoom as
  a CSS-width change through the width matrix.
- Native macOS window confirmation. This is a manual check.
- The combined reply-quote plus panel scenario of #1121 Phase C. It depends on
  the Phase A quote work landing.
