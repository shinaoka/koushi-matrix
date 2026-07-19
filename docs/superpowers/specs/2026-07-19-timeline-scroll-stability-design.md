# Timeline Scroll Stability Design

Status: approved for implementation from issues #277 and #278.

## Goal

Keep the visible timeline position stable while image and link-preview content
becomes ready and while older variable-height rows are prepended and measured.

## Scope

This change is presentation-only. It changes the React timeline's reserved
boxes, virtual height commits, and scroll anchoring. It does not change Matrix
event semantics, Rust state, Tauri DTOs, or the timeline coordinate system.

Issue #277 stays local to `TimelineMediaAttachment`, the link-preview JSX, and
`styles.css`. Issue #278 stays local to the timeline viewport and virtual-height
logic in `TimelineView.tsx` plus its browser-headless tests.

## Alternatives

### Selected: reserved boxes plus synchronous local-anchor compensation

Reserve the final media and preview geometry before asynchronous content is
ready. For remaining height-model changes, preserve a nearby visible row and
its pixel offset by applying only that row's measured relative movement in the
same layout pass as the model commit. Defer prepend/measurement commits during
active user input and reduce speculative backfill to about two viewports.

This follows the shared principle used by Element Web and Element X: layout
completion must not replace a stable local viewport coordinate with a global
estimate.

### Rejected: media CSS only

Making media layout-neutral removes a common height change, but it leaves
variable text, reactions, edits, and prepend estimates on the asynchronous
`ResizeObserver` correction path. It cannot satisfy issue #278.

### Rejected: reverse or bottom-anchor the entire list

A reversed list or bottom-relative coordinate system can make prepend stability
natural, but it rewrites virtualization, keyboard navigation, markers, jump
anchors, and existing tests. That risk and scope are unnecessary for these two
regressions.

## Media and Link-Preview Geometry (#277)

### Image attachments

`TimelineMediaAttachment` renders one `.message-media-figure` in every image
download state. The outer figure receives one immutable inline/block size from
event metadata. Download completion swaps only the figure's inner placeholder
for the image and overlay actions.

When valid width and height metadata are present, the existing 420 by 260 pixel
clamp determines the reserved box. When either dimension is missing or invalid,
the box is the fixed 4:3 fallback within the same clamp: 347 by 260 pixels. A
download result's dimensions never resize the already committed timeline row.

Non-image file attachments retain their compact row layout.

### Link previews

Every preview occupies one fixed-height `.link-preview-card`. Pending previews
render skeleton cards immediately from the already projected pending entries;
the `pendingCount` passed to `onLoadLinkPreviews` remains request diagnostics,
not a second source of product state. Ready and failed states replace content
inside the same card box. Image presence, title length, and metadata arrival do
not alter the card's outer height; text is clamped and overflow is hidden.

## Anchor Compensation (#278)

### Local invariant

In free-scroll mode, the first suitable visible row and its viewport-relative
top offset form the authoritative anchor. Before committing a new measured
height map, capture that anchor. React commits the height model, and the
following layout effect measures the same row and changes `scrollTop` by only
the row's relative top delta. The measurement and scroll write therefore occur
before paint, without waiting for the list `ResizeObserver` or a scheduled
animation frame.

If the exact row is unavailable, the code does not manufacture a correction
from global 72-pixel estimates. Existing explicit room/jump restoration remains
responsible for its own fallback path.

The diagnostics field `heightDeltaAboveViewportPx` records the signed same-pass
correction and the normal scroll-write counter records the corresponding
`backfillCompensation` write.

### Active input gating

Wheel/touch/pointer scroll input marks the timeline active. Row measurements
and a newly projected prepend remain pending while input is active. The existing
100 ms idle timer releases them together; the existing 500 ms maximum defer is
a safety bound and also applies the same anchor compensation. No new timer or
parallel scroll state machine is introduced.

### Backfill threshold

Near-top prefetch uses two current viewport heights, with the existing 80 pixel
minimum. It no longer derives 7,200 pixels from 100 estimated rows. Explicit
top scrolling and underfilled initial timelines retain their existing demand
rules.

## Failure and Cancellation Behavior

- Missing image metadata uses the fixed fallback box; it never falls back to an
  unreserved icon row.
- Failed image downloads keep the same figure and expose retry inside it.
- Failed link previews keep or replace their reserved card without collapsing
  the row during the asynchronous request.
- Timeline key/room changes invalidate pending height and anchor work through
  the existing measurement epoch and timeline-key guards.
- Live-edge intent continues to pin the bottom; local-anchor compensation is
  free-scroll-only.

## Verification

The browser-headless gate extends
`apps/desktop/e2e/timeline-scrollback.spec.ts` before implementation and proves:

1. An image row has the same measured height before and after ready state when
   dimensions are known.
2. The same invariant holds when event width/height metadata is absent.
3. Link-preview pending skeletons and ready cards have the same row height.
4. A prepend containing rows whose measured heights differ materially from the
   72-pixel estimate keeps the visible anchor within the existing pixel
   tolerance during prepend and measurement commit.
5. The height commit records a non-zero `heightDeltaAboveViewportPx` together
   with a same-pass `backfillCompensation` scroll write.
6. Active scroll input defers the corrected commit until idle, and the near-top
   threshold is approximately two viewport heights.

Focused unit tests cover media fallback geometry and backfill-threshold
calculation. The final gate runs the focused Playwright scrollback spec, desktop
typecheck, the full frontend unit suite, lint/boundary checks, and production
build once after the coherent implementation is complete.
