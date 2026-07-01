# Design: Image attachment hover actions + full-size viewer (issue #163)

Status: proposed
Date: 2026-07-01
Issue: https://github.com/shinaoka/koushi-matrix/issues/163

## Goal

Inline image attachments should prioritize the image itself; metadata and
download controls must not permanently crowd or overlap it. Image-scoped actions
appear on hover/focus, and clicking an image opens a larger viewer/lightbox
without losing timeline position. Keyboard focus exposes the same actions as
mouse hover.

## Context

- `TimelineMediaAttachment` (`apps/desktop/src/components/TimelineView.tsx`) uses
  a 3-column inline grid `icon | main(title + meta) | download` (`.message-media`
  in `styles.css`), so the image is squeezed into a frame alongside filename,
  size, mimetype, encryption badge, and the download button.
- A lightbox already exists: `MediaViewer` (`apps/desktop/src/components/
  mediaLists.tsx`) is a modal with zoom and prev/next, backed by Rust-owned
  `TimelinePaneState.media_gallery`, opened today from `RoomMediaGallery` via
  `onOpenItem(index)` (viewer index is React-local state in `panes.tsx`).
- Media rendering is DTO-only: React reads `TimelineItem.media` and the
  `TimelineMediaDownloadState` (`ready` carries a converted `source_url`); MXC
  URIs are not render URLs. React must not parse Matrix media events or
  synthesize upload/download lifecycle.
- CSS uses hardcoded px literals (image max 420×260, 28px buttons, radii); the
  repo prefers named CSS custom-property tokens.

## Decided approach — layout "A" (Element-style), image-scoped, single-image viewer

Scope: **Image kind only.** File/Audio/Video attachments keep the current block.

### Inline presentation (Image kind)

- The image preview is the primary block (rendered only when download state is
  `ready`, using the existing aspect box so wide/tall/small/pending/missing keep
  a stable footprint — ties into the #158 aspect-ratio box).
- **Actions overlay**: on `:hover` and `:focus-within`, a small chrome appears in
  the image's top-right with **info (ⓘ)** and **download (⬇)** (plus optional
  overflow). Hidden by default; positioned relative to the image so it does not
  detach during reflow/virtualization.
- **Metadata hidden by default**: filename / size / mimetype are shown via the
  ⓘ action (popover) and in the viewer chrome — not as a persistent bar.
- **Encrypted**: a small **persistent** lock badge remains visible (security
  signal), even though other metadata is hidden.
- **Download pending**: progress overlay on the image (`role="progressbar"`).
  **Failed**: retry affordance overlay.

### Click → viewer (single-image mode)

- Clicking the image opens the existing `MediaViewer` in a **single-item mode**
  (prev/next hidden) showing that event's `ready` `source_url`.
- The viewer renders as a **modal overlay over the current timeline**;
  `TimelineView` stays mounted, so timeline scroll position is preserved. Do not
  switch primary view / open the gallery pane.
- Viewer open/close is **React-local presentation state** (like a tooltip); no
  new Rust product state, and no dependency on `media_gallery` coverage.
- Viewer chrome exposes download + details; `Escape` closes; add a focus trap
  (currently missing) and restore focus to the triggering image on close.

## Ownership

DTO-only rendering is preserved: React maps `TimelineItem.media` +
`TimelineMediaDownloadState` to nodes and controls. Overlay visibility and
viewer open/close are presentation-only. No MXC parsing, no synthesized
lifecycle, no new Rust media state.

## Accessibility

- Overlay actions revealed on `:hover` **and** `:focus-within`; all controls are
  real buttons/links reachable by keyboard with existing `aria-label`s.
- Viewer: `role="dialog"`, focus trap, `Escape` to close, focus return to the
  trigger, image `alt` = filename.

## CSS

Tokenize the image/button/radius px literals into named CSS custom properties
(e.g. `--media-image-max-inline`, `--media-image-max-block`,
`--media-action-size`), using logical properties; keep fixed-format control
dimensions behind tokens per repo rule. No new ad hoc px literals in TSX.

## Testing (verify-first)

- Browser-headless (Playwright): inline image shows no permanent metadata
  overlapping the preview; hover **and** keyboard focus reveal the action
  overlay; clicking the image opens the viewer overlay with the image; `Escape`
  closes it and the timeline scroll position is unchanged; download is available
  both inline and in the viewer; encrypted shows the persistent lock badge and
  pending shows the progress overlay.
- Reuse the Linux `--scenario=local-media` lane (staging → timeline media row →
  viewer open/close) with synthetic fixtures. After frontend changes, run one
  lane without `--skip-build` before reusing `--skip-build`.
- Synthetic fixtures only; no real/private media bytes, filenames, MXC URIs, or
  paths in evidence.

## Scope / non-goals

- Images only; File/Audio/Video keep the current block.
- No prev/next browsing (single-image viewer); no new Rust media/product state.

## Batch integration (#161–#163, single PR)

Shares `TimelineView.tsx` and `styles.css` with #161. On the single
implementation branch, land **after** #161 to avoid concurrent edits to those
hot files. #162 is parallel-safe.
