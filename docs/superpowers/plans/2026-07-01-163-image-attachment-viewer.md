# #163 Image attachment hover actions + full-size viewer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Inline image attachments prioritize the image; actions appear on hover/focus as an overlay; clicking opens the existing `MediaViewer` in single-image mode without losing timeline position. Images only.

**Architecture:** Restructure `TimelineMediaAttachment` (Image kind) to an image-first layout with a hover/focus action overlay and hidden metadata (persistent lock badge for encrypted). Click opens `MediaViewer` in a new single-item mode as a modal overlay over the mounted timeline (React-local open state). Tokenize CSS px literals; add viewer focus trap.

**Tech Stack:** React `TimelineView.tsx`, `mediaLists.tsx`, `panes.tsx`, `styles.css`, Playwright, Linux `local-media` lane.

## Global Constraints

- Media rendering is DTO-only: read `TimelineItem.media` + `TimelineMediaDownloadState`; MXC URIs are not render URLs (only `ready.source_url`). No Matrix event parsing, no synthesized lifecycle, no new Rust media state. (verbatim: AGENTS.md media rules)
- Overlay visibility and viewer open/close are presentation-only (like tooltips).
- No ad hoc px literals in TSX; repeated/semantic dimensions go in named CSS custom properties. (verbatim: AGENTS.md Live Signals Phase B)
- Actions revealed on `:hover` AND `:focus-within`; viewer `role="dialog"`, focus trap, Escape, focus return.
- Scope: Image kind only; File/Audio/Video keep the current block. Synthetic media fixtures only.

---

### Task 1: `MediaViewer` single-item mode (RED→GREEN)

**Files:**
- Modify: `apps/desktop/src/components/mediaLists.tsx` (`MediaViewer`)
- Test: Vitest spec for `MediaViewer`

**Interfaces:**
- Produces: `MediaViewer` accepts a single-item context (e.g. `items.length === 1` or an explicit `singleItem` prop) → prev/next controls hidden/disabled; zoom + download + details + close remain; focus trap + Escape + focus-return added.

- [ ] Step 1: RED — render `MediaViewer` with one item; assert prev/next are absent, Escape triggers `onClose`, focus is trapped.
- [ ] Step 2: run vitest → FAIL.
- [ ] Step 3: Implement single-item mode + focus trap/Escape/focus-return.
- [ ] Step 4: run vitest + `npm --prefix apps/desktop run typecheck` → PASS.
- [ ] Step 5: Commit — `feat(media): MediaViewer single-image mode with focus trap (#163)`

---

### Task 2: Inline image-first layout + hover/focus action overlay (RED→GREEN)

**Files:**
- Modify: `apps/desktop/src/components/TimelineView.tsx` (`TimelineMediaAttachment`, Image kind branch)
- Modify: `apps/desktop/src/styles.css` (new `.message-media-image*` overlay classes + tokens)
- Test: Playwright `apps/desktop/e2e/*` (media spec)

**Interfaces:**
- Consumes: existing `media`, `downloadState`, `onDownload`, `onOpenViewer(eventId)` (Task 3 wires the handler).
- Produces: Image kind renders image as the primary block; `.message-media-actions` overlay (info + download [+ overflow]) shown on hover/focus; metadata hidden by default; persistent lock badge when `media.source.encrypted`; pending progress overlay; failed retry overlay.

- [ ] Step 1: RED — Playwright: for an image row, metadata is not visible by default (no overlap), hover AND keyboard focus reveal the action overlay, encrypted shows a persistent lock badge.
- [ ] Step 2: run spec → FAIL.
- [ ] Step 3: Implement the image-first layout; move filename/size/mimetype behind the info action; keep File/Audio/Video branch unchanged. Add CSS tokens (`--media-image-max-inline`, `--media-image-max-block`, `--media-action-size`, radii); no TSX px literals.
- [ ] Step 4: run spec (isolated) + typecheck → PASS.
- [ ] Step 5: Commit — `feat(gui): image-first inline attachment with hover/focus actions (#163)`

---

### Task 3: Click image → open viewer as overlay (position preserved) (RED→GREEN)

**Files:**
- Modify: `apps/desktop/src/components/panes.tsx` (viewer open state usable from a timeline image click, not only the gallery; render viewer as overlay over the timeline)
- Modify: `apps/desktop/src/components/TimelineView.tsx` (image click / Enter dispatches `onOpenViewer(eventId)`)
- Test: Playwright media spec

**Interfaces:**
- Consumes: Task 1 single-item `MediaViewer`; the clicked event's `ready.source_url`.
- Produces: clicking an inline image opens the single-image viewer overlay; timeline stays mounted and scroll position is unchanged after open+close; download available inline and in the viewer.

- [ ] Step 1: RED — Playwright: click an image → viewer overlay appears with the image; record timeline `scrollTop` before open and after close, assert unchanged; Escape closes.
- [ ] Step 2: run spec → FAIL.
- [ ] Step 3: Implement open-from-timeline (React-local presentation state) rendering the single-item viewer without unmounting the timeline / without opening the gallery pane.
- [ ] Step 4: run spec (isolated) + typecheck → PASS.
- [ ] Step 5: Commit — `feat(gui): open single-image viewer from timeline, preserving position (#163)`

---

### Task 4: Linux `local-media` lane proof

**Files:** `scripts/desktop-linux-gui-qa.mjs` `--scenario=local-media` (add hover-action + click-to-viewer assertions; keep existing gallery/viewer tokens).

- [ ] Step 1: Extend the lane: hover reveals actions, click opens viewer, close preserves the row; synthetic fixture only.
- [ ] Step 2: Run once WITHOUT `--skip-build` (frontend changed), then confirm tokens `gui_local_media=ok` / `gui_local_media_viewer=ok` (+ any new hover token).
- [ ] Step 3: Commit — `test(qa): local-media hover actions + click-to-viewer (#163)`

## Self-Review

- Spec coverage: single-item viewer (T1), image-first + hover/focus overlay + hidden metadata + lock badge (T2), click→viewer position-preserving (T3), lane proof (T4). ✓
- No new Rust/product state (presentation-only). ✓
- Type consistency: `onOpenViewer(eventId)` defined in T2 interfaces, consumed in T3.
- Ordering note: lands AFTER #161 on the single branch (shared `TimelineView.tsx`/`styles.css`).
