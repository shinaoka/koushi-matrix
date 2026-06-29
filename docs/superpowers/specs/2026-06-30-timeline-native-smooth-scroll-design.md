# Design: Timeline native-smooth virtual scrolling (issue #158)

Status: written design pending user review
Date: 2026-06-30
Issue: https://github.com/shinaoka/koushi-matrix/issues/158

## Goal

Make the desktop timeline scroll like a native surface while preserving the
existing Rust-owned Matrix contracts. During user scroll, already mounted rows
should move with browser scrolling. React should not recompute the virtual
window for every `scrollTop` tick, and dynamic row measurement should not move
the visible anchor while the user is actively reading.

This is a GUI presentation and verification design. Timeline item order,
timeline diffs, room/thread semantics, backfill, read receipts, read markers,
media metadata, link-preview policy, and notification state remain Rust-owned.
React may own viewport metrics, DOM measurements, scroll anchors, and temporary
scroll-write bookkeeping.

## Context

The current `TimelineView` on `origin/main` has the failure shape described in
the issue:

- `viewportMetrics.scrollTop` is React state.
- `virtualWindow` depends on `viewportMetrics`, so scroll events can recompute
  `startIndex`, `endIndex`, `paddingTop`, `paddingBottom`, and the sliced row
  array.
- The virtual row measurement layout effect updates `itemHeightByDomIdRef` and
  bumps `measuredHeightVersion` without an active-scroll guard.
- Ready image media rows render a real `<img>` after download without a stable
  pre-ready aspect-ratio box for the same final image footprint.
- `.timeline-view` currently allows browser `overflow-anchor: auto`, while the
  app also performs explicit event-anchor correction.

Recent timeline work fixed deep anchor restore, offscreen anchor drift, and
cache restore behavior. This design does not replace those contracts. It adds a
scroll-pipeline layer around the existing virtualized view so the browser owns
steady active scrolling and the app applies scroll-affecting corrections only at
semantic boundaries.

## Primary Source Notes

- Element Web `ScrollPanel` models either bottom-stickiness or a tracked scroll
  token, not only an absolute offset:
  https://github.com/element-hq/element-web/blob/develop/apps/web/src/components/structures/ScrollPanel.tsx
- Element's scrolling notes say setting `scrollTop` during active scrolling can
  interrupt ongoing scroll and can read stale values, especially on macOS. The
  BACAT path waits for a short idle window before height recalculation and uses
  relative `scrollBy` for compensation:
  https://github.com/element-hq/element-web/blob/develop/docs/scrolling.md
- TanStack Virtual's chat guidance emphasizes stable keys, end anchoring,
  scroll direction, `isScrolling`, and conditional dynamic-size correction:
  https://tanstack.com/blog/tanstack-virtual-chat
- Virtuoso Message List treats prepend, item changes, and auto-scroll-to-bottom
  as explicit scroll modifiers rather than undifferentiated array updates:
  https://virtuoso.dev/virtuoso-message-list/scroll-modifier/
- Stream Chat's virtualized list documentation calls out default item-height
  estimation quality as a direct scroll-stability factor:
  https://getstream.io/chat/docs/sdk/react/components/core-components/virtualized-list/
- Browser `overflow-anchor` can automatically adjust scroll position when
  content changes. If Koushi keeps explicit event-anchor correction, that
  browser behavior must be measured and controlled:
  https://developer.mozilla.org/en-US/docs/Web/CSS/overflow-anchor

The Element Web reference repository was inspected locally at
`/tmp/koushi-reference/element-web`, commit `84c0d2c`.

## Diagnosis

The diagnosis is coherent but must be quantified before fixing. There are two
related failure classes:

1. Active-scroll jank: steady scroll causes React commits and virtual-window
   recomputation even while the viewport stays inside the already mounted
   overscan window.
2. Anchor jumps: row heights change after measurement, image download/decode,
   link preview update, receipt update, reaction update, thread summary update,
   local echo settle, or prepend. If those height changes alter spacer sizes or
   trigger scroll writes during active scroll, the visible event moves without a
   matching user input.

The largest missing baseline is row-height estimate error by row kind. A bad
estimate can produce incorrect spacer sizes and scrollbar geometry even after
scroll-event render churn is fixed. Phase 0 therefore measures both commit churn
and estimate error before behavior changes.

## Recommended Approach

Refactor the existing local virtualizer first. Do not replace it with TanStack
Virtual, Virtuoso Message List, or another external virtualizer in the first
pass.

Reasons:

- The current passing behavior includes Matrix-specific prepend compensation,
  room-switch viewport memory, live-edge stickiness, read-signal observation,
  focused-event jumps, and Rust-owned timeline event application. Replacing the
  virtualizer would rewrite the most fragile working layer.
- The core defect is architectural: scroll position drives React state, and
  measurement commits run during active scrolling. That can be fixed locally.
- TanStack is a plausible later fallback because it is headless, but Koushi
  would still own the chat-specific anchor, prepend, read-signal, and Matrix
  boundary behavior.
- Virtuoso Message List has useful semantic scroll-modifier ideas, but adopting
  it wholesale would hand too much scroll semantics to a library before Koushi's
  own contracts are measured.

Use the Phase 0 instrumentation as the fallback gate. If Phases 1 and 2 cannot
make the local virtualizer pass the active-scroll and deferred-measurement tests
without weakening anchor behavior, evaluate TanStack in a separate design using
the same tests as acceptance criteria.

## Architecture

Introduce a small viewport-controller layer inside `TimelineView` first. It may
later move to a colocated helper module if it becomes large enough to test in
isolation.

The controller owns presentation-only data:

- latest raw DOM metrics in refs: `scrollTop`, `scrollHeight`, `clientHeight`,
  `listOffsetTop`
- active scroll state: active, idle, last input time, and last scroll direction
- range state: virtualized, start index, end index, top spacer, bottom spacer
- pending measured heights that are observed during active scroll but not yet
  committed to the height model
- the current viewport intent: live edge, anchored reading position, target
  scroll, room restore, startup restore, backfill compensation, measurement
  flush, or free scroll
- scroll-write reason tokens used to distinguish programmatic scroll echoes from
  user input
- private-data-free diagnostic counters and samples

The controller must not own Matrix product semantics. `reportViewportObservation`
must continue to notify Rust of the first/last visible event and bottom state,
but this observation cadence is decoupled from React render cadence. Reducing
render commits must not drop viewport observations.

## Phase 0: Measurement Harness

Add test-only instrumentation before changing scroll behavior. Production
diagnostics stay off by default and private-data-free.

Measurements per scroll frame or scroll transaction:

- `scrollTop`, `scrollHeight`, `clientHeight`
- viewport intent
- scroll activity: active or idle
- user input pending
- programmatic scroll reason and token kind
- virtual range: start index and end index
- `paddingTop` and `paddingBottom`
- measured-height version
- changed measured row count
- height delta above, inside, and below the viewport
- anchor event classification without logging the event id
- anchor top delta in pixels
- render or commit count
- range update count
- row-height estimate error buckets by row kind: text, formatted/code, reply,
  reaction/receipt/thread-summary, link preview, media, redacted/system

The instrumentation must avoid Matrix identifiers, event ids, room ids, user ids,
message bodies, filenames, MXC URIs, raw SDK errors, local paths, and account
data in normal logs, QA titles, screenshots, or test artifacts.

RED checks:

1. Active scroll inside overscan: script a scroll that remains within the
   already rendered overscan window. The test records multiple scroll events but
   expects no virtual range commit and no height-model commit during the active
   burst. This is expected to fail on current `origin/main`.
2. Deferred measurement: mutate a visible row's measured height during active
   scroll. The test expects `paddingTop` and the anchor-relative top to remain
   stable until idle, then exactly one measurement flush. This is expected to
   fail on current `origin/main`.
3. Semantic scroll writes: script live-edge follow, jump-to-event, backfill
   prepend, room-switch restore, and measurement flush. Each programmatic write
   must carry one private-data-free reason token, and the next scroll event must
   be classified as an echo rather than user input.
4. Estimate histogram: load a mixed synthetic timeline and assert that
   diagnostics include row-kind buckets and aggregate estimate-error values.
   This is a measurement gate, not a smoothness pass/fail gate.

Likely files:

- `apps/desktop/src/components/TimelineView.tsx`
- `apps/desktop/src/components/TimelineView.test.tsx`
- `apps/desktop/e2e/timeline-scrollback.spec.ts`
- `apps/desktop/src/test/harnessMain.tsx`
- `apps/desktop/src/domain/diagnostics.ts`
- `apps/desktop/src/domain/qaTitle.ts`

## Phase 1: Native Scroll First

Stop treating every `scrollTop` tick as React state.

Design:

- Keep raw scroll metrics in refs.
- Schedule scroll processing with `requestAnimationFrame`.
- Compute the next virtual range from refs and the current height model.
- Commit React range state only when `startIndex` or `endIndex` changes, or
  when a committed height-model transaction changes spacer sizes. Raw
  `scrollTop` movement inside the already mounted overscan window must not
  update React range state.
- Do not recompute row slices for scroll events that remain inside the mounted
  overscan window.
- Keep viewport observation and auto-backfill checks on the scroll path, but
  do not require a React render for those checks.

Acceptance:

- The active-scroll-inside-overscan RED test turns GREEN.
- Existing virtualized timeline tests still prove bounded DOM count, bottom
  reveal, live-edge snap, prepend anchor restore, and room-switch restore.
- Read receipt and viewport observation command shapes remain covered by
  browser-headless tests.

## Phase 2: Deferred Dynamic Measurement

Measurements may be read during active scroll, but scroll-affecting height model
updates must wait.

Design:

- Track `scrollActivity=active` from user input and scroll events.
- Use a 100ms idle window modeled after Element's wait. The value must be a
  named test-visible constant and deterministic under fake timers.
- Store observed height changes in a pending map during active scroll.
- On idle, apply all pending height changes as one transaction.
- If the viewport is live-edge, follow the bottom only for live-edge intent.
- If the viewport is anchored, preserve the selected anchor using one
  compensation write after the height model transaction.
- Prefer relative `scrollBy` for compensation where the browser supports it;
  use direct `scrollTop` assignment only where a target scroll requires an
  absolute position.
- Add a max-defer cap so programmatic scroll echoes cannot starve measurement
  flush indefinitely.

Acceptance:

- The deferred-measurement RED test turns GREEN.
- Manual anchored reading position does not move under receipt, reaction,
  preview, thread summary, or local echo settle updates.
- Live edge continues to follow append and content growth only while the intent
  is live edge.
- User scroll away from bottom clears live-edge follow before deferred
  measurement flush can yank the viewport back.

## Phase 3: Media Height Reservation

Reduce dynamic height changes at the source.

Design:

- For media rows with Rust-projected width and height, reserve a clamped
  aspect-ratio box before image download/decode is ready.
- The reserved box must match the eventual `.message-media-image` constraints:
  the current design caps media at `max-width: 420px` and `max-height: 260px`.
- Unknown dimensions use a conservative fallback that is measured through the
  deferred measurement path.
- Link-preview image boxes already have fixed dimensions; keep them stable and
  include them in the estimate histogram.
- Do not move filename, caption, MXC URI, media key, or raw media error data
  into logs or QA output.

Acceptance:

- Image rows with known dimensions have no measurable decode/download height
  shift beyond 1 CSS pixel in the deterministic browser harness.
- Unknown-dimension media shifts only through the deferred measurement path.
- Existing media send/download tests and message rendering tests remain green.

## Phase 4: Browser Scroll Anchoring Policy

Choose one anchoring owner for the timeline scroller.

Design:

- Measure whether browser `overflow-anchor` adjusts the timeline while Koushi
  also applies event-anchor correction.
- If double correction occurs or cannot be ruled out, set the timeline scroller
  to `overflow-anchor: none` and keep `overflow-anchor: none` on virtual spacers.
- If browser anchoring is retained, document the exact DOM anchor candidates and
  remove conflicting custom correction for that case.

Recommendation: keep Koushi's explicit event-anchor correction and disable
browser anchoring on the timeline scroller, because Koushi already has
Matrix-event anchors and semantic scroll-write reasons.

Acceptance:

- Browser-headless test proves one correction path for a controlled row-growth
  case.
- Spacer elements are not browser anchor candidates.
- Anchor restore and deferred measurement compensation do not double-apply.

## Test Matrix

Fast checks:

- `npm --prefix apps/desktop run test -- --run src/components/TimelineView.test.tsx`
- `npm --prefix apps/desktop run test -- --run src/domain/diagnostics.test.ts src/domain/qaTitle.test.ts`
- `npm --prefix apps/desktop exec -- playwright test e2e/timeline-scrollback.spec.ts --workers=1`

Focused browser-headless additions:

- smooth active scroll inside one overscan window
- deferred measurement during active scroll
- media known-dimension height stability
- manual anchor stability under non-message row updates
- live-edge follow vs user scroll-away
- backfill prepend one-shot compensation
- startup and room-switch deterministic restore
- scroll-write reason token classification

Existing local core QA gates for Rust timeline behavior are not replaced by
these browser checks. If implementation touches Rust command/event/snapshot DTOs
or timeline actor behavior, add the relevant Rust tests and local headless QA
scenario gates in the implementation plan.

## Risks And Edge Cases

- macOS inertial scrolling can report stale scroll values while visual scrolling
  continues off the main thread. Headless Chromium cannot fully prove this; it
  can prove render/range churn and anchor math, but native-platform evidence may
  still be needed before claiming macOS smoothness.
- Anchor restore and measurement flush can both write scroll position. Reason
  tokens must serialize these writes so compensation does not double-apply.
- Programmatic scroll echoes can keep idle timers alive. The max-defer cap must
  guarantee eventual measurement flush.
- Prepend rows begin unmeasured. Anchor compensation must account for the
  post-measurement correction, not only the initial estimated restore.
- Local echo settlement can change a row key from transaction id to event id,
  remounting and remeasuring the row. That must not move the visible anchor.
- Thread timelines and timelines below the virtualization threshold share much
  of the component. Non-virtualized behavior must remain stable.
- Reducing React render cadence must not reduce viewport observation cadence,
  read receipt dispatch, auto-backfill triggering, or live-edge state updates.
- External virtualizer adoption remains a fallback, not an initial step. Any
  later adoption needs a separate design and must pass the same measurement
  harness before replacing local behavior.

## Non-Goals

- Hiding flicker with fixed sleeps, arbitrary `scrollTop` thresholds, or larger
  overscan alone.
- Rendering the full timeline to avoid virtualization.
- Moving Matrix product semantics into React.
- Changing thread ordering, room timeline membership, or Rust-owned read-signal
  semantics.
- Claiming real macOS inertial-scroll behavior is fixed from headless Chromium
  evidence alone.

## Completion Criteria

- The measurement harness demonstrates that active scroll no longer causes
  unnecessary React virtual-window commits.
- Active-scroll height observations are buffered and flushed once after idle.
- Manual anchored reading position and live-edge follow are clearly separated.
- Known-dimension media rows reserve stable height before decode/download.
- The timeline uses exactly one anchoring owner for row-growth correction.
- Browser-headless regression tests cover the major scroll, measurement,
  media, live-edge, prepend, and restore cases without private data.
