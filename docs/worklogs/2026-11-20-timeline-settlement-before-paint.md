# Timeline scroll flicker: anchor settlement lands a painted frame late (2026-11-20)

Refs #1653, #1892, #1938.

## Report

Scrolling the desktop timeline by a screen or more, mostly upward while older
history loads, flickers. An installed-app measurement (0.36.2, WebKitGTK, a
scripted 30 px step with a dispatched `wheel` before each `scrollTop` write)
counted 7 centre-row jumps of 21–130 px in 6 s of upward scrolling and 16
app-written `scrollTop` corrections, the largest 6641 px during history loads.

## Reproduction

`apps/desktop/e2e/timeline-window-slide-anchor.spec.ts`, Chromium and WebKit.
It opens a 240-row window at the live edge, answers each
`paginate_timeline_backwards` the way Core's cached backward window extension
does (one `ItemsUpdated` with the newest page removed and the next older page
inserted, then `PaginationStateChanged { Idle, prepend_expected: false }`), and
scrolls up 30 px every 16 ms through two window slides. It asserts on every
painted frame that the row at the previous frame's centre moved by the reader's
scroll delta and nothing else (2 px tolerance), and that no virtual spacer is
visible.

Two measurement traps decided the shape of that spec:

- **A probe's own rAF is not the painted frame.** rAF callbacks run in
  registration order, so a probe registered before the app's compensation frame
  reads the DOM before it. The first version of this probe reported 9,000 px
  "jumps" at every window slide that were corrected later in the same frame and
  never painted. The spec wraps every rAF and ResizeObserver callback and
  re-samples after each one; the last sample of a frame is what reaches paint.
  The installed-app measurement used a rAF sampler and may over-count for the
  same reason.
- **Input dispatched from a rAF is not native input.** Native wheel input is a
  task; its `scroll` event and the range-commit frame it schedules share one
  rendering update. A driver that dispatches `wheel` inside its own rAF (ahead of
  the app's frame callback) cancels that frame every time, and the window never
  follows: the viewport went blank for 1,100 frames. That is an artefact of the
  driver, not the reported symptom, so the spec drives input from tasks.

With task-timed input and painted-frame sampling, `main` fails in both engines:
every row the window mounts above the reader paints one frame displaced by the
row's estimate error (−36, −61, −86 px for the spec's one-, two- and three-line
rows) and is corrected in the next frame (about 430 corrections in one run, all
of them a frame late), and occasionally a window slide paints its whole insert
uncompensated (+9,171 px) for a frame.

## Cause

Both are the same defect: a free-scroll settlement waits for a scheduled frame
although the layout that displaced the reader is already committed.

- **Row mounts.** Since #1890 the scroll frame commits the virtual range inside
  the rAF (`flushSync`), so the newly mounted row is laid out in that rendering
  update. The measurement pass opens a `waiting-measurement` transaction and the
  compensation effect schedules `attempt()` for a frame, which is the next
  rendering update: the current one is painted first.
- **Window slides.** The slide commits in a task and schedules settlement for
  the next frame. A wheel task that arrives first runs `advanceViewportEpoch()`,
  which cancels the pending settlement. The rebased transaction is re-armed only
  by a re-render in a continuous lane, after the frame is painted.

The #278 design ("Anchor Compensation", local invariant) already requires the
measurement and the scroll write to occur before paint, "without waiting for the
list `ResizeObserver` or a scheduled animation frame". The code met that only on
the live-edge path.

## Change

`TimelineView.tsx`: the layout effect that already runs a range-prepared
settlement after the measurement pass now also runs a settlement armed by the
same commit (`projectionCommitted` with its frame still pending). It runs after
that commit's measurement pass, as #837 requires, and before paint. The
scheduled frame stays as the retry for a settlement that cannot complete yet
(pending deferred measurements, an anchor not mounted). No new owner, timer or
state is added; the transaction controller and its fences are unchanged.

## Verification

- New spec RED on `main`: Chromium and WebKit, about 650 painted-frame jumps
  each. GREEN with the change: Chromium, WebKit ×4.
- Full Playwright DOM tier, frontend typecheck, vitest and ESLint: see the PR.

## Not covered

- The installed-app measurement on the reporter's real room was not re-run here
  (private data; disposable homeservers only). The equivalent headless run in
  WebKit shows zero painted jumps.
- Input that cancels the scroll frame's range commit (a `wheel` landing between
  a `scroll` event and its frame) leaves the window behind the reader. Native
  input should not hit that ordering; it was seen only with the rAF driver.
