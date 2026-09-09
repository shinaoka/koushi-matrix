# Full-reader surface and concrete payload proposal

Status: **Correct-to-implement** as part of Sol's full
[reader vertical](2026-09-06-receipt-reader-vertical.md) re-review, including these
payload/UX choices and the [lifecycle](2026-09-06-scoped-view-lifecycle.md).
This records design approval, not compiled implementation or passed UI tests.

## Payload boundary

Use these concrete concepts, with final declarations in the existing protocol/Core
layers rather than a UI-framework crate:

- `ReceiptSourceRef`: existing TimelineKey, InitialItems projection RequestId,
  public TimelineGeneration, event ID. Core resolves the private actor-generation
  owner; a renderer never fabricates that private gate value.
- `ReceiptCompactSummary`: exact total, bounded reader rows (all through four,
  three from five), exact overflow, and the source reference needed to open the
  full surface. It belongs to the scoped TimelineReceipts model, not TimelineItem
  or the old room-wide map.
- `ReaderRow`: user ID as stable row identity, Rust-resolved display label and
  original-name provenance where the current policy requires it, Rust-resolved
  initials, optional validated timestamp with Rust-selected locale, and existing
  portable avatar readiness. Adapters format only delivered timestamps using the
  fixed native medium-date/short-time policy; no JavaScript runtime is required.
  No MXC-demand input, local path or image bytes. The precise range, locale mapping
  and errors follow [the approved formatting amendment](2026-09-06-receipt-formatting-amendment.md).
- `ReaderWindow`: source reference, exact total, window origin, bounded rows,
  accepted window-request sequence, accepted source/order revision and projection
  dependency revision. The enclosing model has the scope ID and final revision.
- `ReaderView`: Loading, Ready(ReaderWindow), or a terminal source/session/consumer
  retirement reason. Capacity rejection is an operation result, not a fake empty
  reader list. Source unavailability never becomes a zero-reader snapshot.
- Window requests carry the installed revision and either an index interval or
  stable anchor identity. Rust caps the interval and owns order/count/selection.
  Observations name that installed revision and actually visible row identities;
  model ACK never substitutes for visibility.

Proposed declaration shape (documentation, not compiled production code):

```rust
struct ReceiptSourceRef {
    key: TimelineKey,
    projection_request_id: RequestId,
    generation: TimelineGeneration,
    event_id: String,
}
struct ReaderRow {
    user_id: String,
    display_label: String,
    original_display_label: String,
    initials: String,
    timestamp: Option<ReceiptTimestamp>,
    avatar: Option<AvatarThumbnailState>,
}
struct ReceiptCompactSummary {
    source: ReceiptSourceRef,
    total_count: u64,
    overflow_count: u64,
    readers: Vec<ReaderRow>, // bounded to four, or three when overflowing
}
struct ReaderWindow {
    source: ReceiptSourceRef,
    total_count: u64,
    start: u64,
    rows: Vec<ReaderRow>,
    window_sequence: u64,
    source_revision: u64,
    dependency_revision: u64,
    resolved_anchor: ResolvedReaderAnchor,
}
enum ResolvedReaderAnchor {
    Row { user_id: String, index: u64 },
    NoSurvivingInstalledRow,
    NotRequested,
}
// TimelineSourceRef is ReceiptSourceRef without the individual event_id.
// Selection includes the observed display revision and at most 256 event IDs.
enum ViewSpec {
    TimelineReceipts { source: TimelineSourceRef, selection: ReceiptEventSelection },
    ReceiptReaders { source: ReceiptSourceRef, start: u64, limit: u16 },
}
enum ViewModel {
    TimelineReceipts { source: TimelineSourceRef, summaries: Vec<ReceiptCompactSummary> },
    ReaderLoading { source: ReceiptSourceRef },
    ReaderReady(ReaderWindow),
}
enum ViewRetirement {
    SourceUnavailable,
    SessionRetired,
    ConsumerRetired,
    RuntimeStopped,
}
enum ViewDelivery {
    Model { scope: ViewScopeId, revision: u64, model: ViewModel },
    Retired { scope: ViewScopeId, reason: ViewRetirement },
}
```

All identity/revision/sequence u64s use the reviewed canonical-decimal-string wire
encoding; bounded counts/indices follow existing numeric DTO conventions. Scope IDs
are opaque as specified in the lifecycle document. Models/rows carry no raw avatar
MXC URI: `AvatarThumbnailState` reuses the existing portable readiness reference.
Enums use explicit tagged DTO forms and all mirrors/decoders are updated together.
Public types get privacy-safe Debug and required doctests. No open JSON payload or
caller-selected field path is introduced.

Timeline receipt selections are capped in EVENT rows; each summary has at most
four reader rows. Account for this multiplier explicitly in memory/byte budgets.
Validate selected IDs/order against the actor's current Rust display model, including
hidden/ignored policy; do not infer SDK positions from renderer indices. Visibility
and end evidence must also carry the installed receipt-model revision, as specified
in the vertical. Both concrete ViewSpec variants have production consumers in this
change; no unused future scope variants are declared.

Do not serialize all index records in a compact or window model. No obsolete
full-reader room-map or all-reader `details[]` compatibility path remains after
cutover. Keep typing and fully-read room state independent of this change.

## Opening and closing

Keep the compact stack's bounded hover/focus preview; it contains only the compact
rows and a localized overflow indication. An actual button opens the full-reader
surface using its source reference. Enter/Space have native button behavior;
hover alone does not open a full-reader subscription. The button has a Rust/localized
accessible name including the exact total and `aria-haspopup="dialog"`.

The full surface uses the existing FloatingLayer/boundary placement, a named dialog
and a close button. Escape closes and returns focus to the opener if it still
exists, otherwise the owning timeline's focusable container. Source retirement
clears the rows immediately on its priority signal, closes the dialog, and announces
the localized reason without attempting to focus a removed row. Account/window
replacement drops the subscription. No modal focus trapping is implied for this
non-modal dialog; keyboard users can Tab to its close control and out of it.
On opening, focus the named list container while Loading. When the first ready
window installs, focus its first real row only if focus remains in that initial
loading container and no newer user input intervened. That row is the sole roving
`tabindex=0`; other rows are `-1`. While Loading, the container is the one list tab
stop. A close-button dismissal follows the same opener/timeline fallback as Escape.
Outside-pointer dismissal must not steal focus from the user's clicked destination;
if focus would remain in the removed dialog, use the same fallback. Tab moving out
does not forcibly close the non-modal dialog. Retirement announcements use the
existing host-level persistent live region, never a node inside the removed popup.

## One bounded reader-window renderer

The list uses chronological-independent reader order supplied by Rust. Its rows
are single-line name/time cells, with ellipsis for visual overflow but complete
accessible text. Bidi-isolate cells, preserve `dir=auto` for names and test RTL.
Use a font-relative row pitch; a single shared row/probe ResizeObserver measures
that pitch on font/zoom changes. Do not introduce per-reader height caches.

Use an ordered list with bounded mounted rows, `aria-setsize` and `aria-posinset`
on each real list item. Top/bottom presentation spacers represent unmounted
indices; spacers and the sizing probe are hidden from accessibility. The visible
height is capped to the available pane/viewport, not 1,500 times the row height.
Default initial window is 32 rows; the common maximum remains 256. Native geometry
requests the visible interval plus a Rust-selected prefetch allowance (at most
eight extra identities total, not eight at each end). Profile reads may prepare
that bounded window; avatar requests require accepted visibility/prefetch demand.

One reader-window adapter owns this surface's row pitch, mounted range and explicit
scroll/focus realization. It is not a second timeline engine and does not reuse
or extend the superseded timeline correction scheduler. It consumes Rust's row
order; it does not sort/filter readers or compute receipt counts.

A scrollbar jump outside the installed window requests the destination interval
and shows one localized loading region until it arrives. Do not generate fake
person rows or label an unloaded region as empty. A newer native input invalidates
an older local positioning attempt. Core window-request sequences also reject
late preparation results; local input tokens are presentation-only.

## Keyboard and changing data

Real list items use roving focus, not selection semantics; do not add `aria-selected`
or imply a listbox operation. ArrowUp/Down move to the adjacent index, Home/End to
the first/last, PageUp/Down by the measured visible row count. An unloaded target
requests its bounded window, then focuses the real row only if the input token and
accepted request sequence still match. Tab leaves the list rather than visiting
1,500 tab stops. No new person-profile activation behavior is introduced here.

While reading, retain a visible stable row/offset through source updates; Rust
resolves its new position and a surviving adjacent row if it disappears. The single
window adapter realizes that anchor only absent newer user input. Window replacement
must not replay an earlier scroll command. Resize/font changes preserve the same
visible identity with the new measured pitch; they do not alter Matrix read state.

An anchor request contains the installed revision and anchor user ID, not a pixel
offset. Core validates it against its bounded installed identity mapping. Resolve
the same ID if it survives; otherwise choose the first surviving successor in that
installed order, then the nearest surviving predecessor. Return the explicit
`resolved_anchor` ID/index above. If no installed row survives, return
`NoSurvivingInstalledRow`: focus the list container, announce the changed context,
and await explicit navigation rather than jumping to an estimated distant row.
The adapter alone retains the pixel offset/input token and applies a returned row
anchor only for the current request/input. It never reconstructs the fallback order.

## Verify-first migration

Before replacement, map and retain the existing compact hover/focus, popup clipping,
small-list sizing and label/locale tests. Add only the changed boundary coverage:

- 1,500 retained readers, exact total, compact three and correct overflow, bounded
  mounted full-reader window; keyboard End reaches the actual last reader.
- Old window/profile result after a newer scrollbar/keyboard request cannot move
  focus or replace the newer window; retirement bypasses a missing model ACK.
- Font scaling/resize and RTL keep complete accessible labels and usable controls;
  opening in Thread/Focused remains inside the correct floating boundary.
- Headless Core profile lookup RED turns green without discarding reader records.
  Actual Tuwunel/Synapse avatar request/cancel/dedup evidence remains mandatory.

These are design assertions, not passed results. Native macOS wheel/trackpad
qualification remains only the explicitly deferred exact-build handoff.
