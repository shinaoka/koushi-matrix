/**
 * Upward reading across Core's bounded display window must not move the
 * reader's row in any painted frame by more than the reader's own scrolling.
 *
 * Core keeps a 240-row display window and slides it by a page when the reader
 * nears its older end: one `ItemsUpdated` batch removes the newest rows and
 * inserts older ones in front (the diff shape `finalize_display_projection_diffs`
 * builds for that slide), followed by `PaginationStateChanged { Idle,
 * prepend_expected: false }`. While the reader scrolls, every row the window
 * mounts above them is laid out at its real height, not the estimate the spacer
 * reserved for it, and that difference has to be compensated before the frame
 * is painted (#278 local invariant). A compensation one frame late shows the
 * reader a frame of content shifted by the estimate error: the flicker.
 *
 * Input is task-timed like native wheel input: a `wheel` event, then a 30 px
 * scrollTop step, every 16 ms.
 *
 * The oracle samples painted frames. A probe's own rAF runs in registration
 * order with the app's and can read the DOM before the app's compensation of
 * the same frame, so it would report frames nobody sees. Every rAF and
 * ResizeObserver callback is wrapped instead and the frame re-sampled after
 * each one; the last sample of a frame is the state that reaches paint.
 */

import { expect, test, type Page } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_SEED_EVENT_ID = "$seed-event:example.invalid";
const TIMELINE_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);

/** Core's `WINDOW_SLOTS` and `PAGE_ROWS` (display_projection/bounded_window.rs). */
const WINDOW_ROWS = 240;
const PAGE_ROWS = 60;
const HISTORY_ROWS = 1200;
const STEP_PX = 30;
const STEP_INTERVAL_MS = 16;
const ANCHOR_TOLERANCE_PX = 2;
const REQUIRED_SLIDES = 2;

function eventId(index: number): string {
  return `$window-slide-${String(index).padStart(5, "0")}:example.invalid`;
}

/** Rows of one, two and three lines, so a mounted row rarely matches the estimate. */
function makeEventItem(index: number) {
  const lines = (index * 7) % 3;
  return {
    id: { Event: { event_id: eventId(index) } },
    sender: HARNESS_ACCOUNT_KEY,
    sender_label: "Harness User",
    body: `window slide message ${index}${"\nsynthetic continuation line".repeat(lines)}`,
    // Twenty minutes apart: a date divider every few dozen rows.
    timestamp_ms: 1_800_000_000_000 + index * 1_200_000,
    in_reply_to_event_id: null,
    thread_root: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [],
    send_state: null,
    thread_summary: null
  };
}

type PaintedFrame = {
  scrollTop: number;
  readerTotalPx: number;
  /** The previous painted frame's centre row, where this frame painted it. */
  tracked: { id: string; top: number | null } | null;
  centre: { id: string; top: number } | null;
  spacerPx: number;
};

type PaintedFrameProbe = {
  frameTs: number;
  currentTs: number;
  readerTotalPx: number;
  current: PaintedFrame | null;
  finals: PaintedFrame[];
  active: boolean;
  sample(): void;
};

async function installPaintedFrameProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const probe: PaintedFrameProbe = {
      frameTs: -1,
      currentTs: -1,
      readerTotalPx: 0,
      current: null,
      finals: [],
      active: false,
      sample() {
        if (!probe.active) return;
        const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
        if (!container) return;
        if (probe.frameTs !== probe.currentTs) {
          if (probe.current) probe.finals.push(probe.current);
          probe.currentTs = probe.frameTs;
        }
        const box = container.getBoundingClientRect();
        const trackedId = probe.finals.at(-1)?.centre?.id ?? null;
        let tracked: PaintedFrame["tracked"] = null;
        if (trackedId !== null) {
          const row = container.querySelector<HTMLElement>(
            `[data-item-id="${CSS.escape(trackedId)}"]`
          );
          tracked = { id: trackedId, top: row ? row.getBoundingClientRect().top - box.top : null };
        }
        const middle = box.top + container.clientHeight / 2;
        let centre: PaintedFrame["centre"] = null;
        for (const row of container.querySelectorAll<HTMLElement>("[data-item-id]")) {
          const rect = row.getBoundingClientRect();
          if (rect.top <= middle && rect.bottom > middle) {
            centre = { id: row.dataset["itemId"]!, top: rect.top - box.top };
            break;
          }
        }
        let spacerPx = 0;
        for (const spacer of container.querySelectorAll<HTMLElement>(".timeline-virtual-spacer")) {
          const rect = spacer.getBoundingClientRect();
          if (rect.height <= 0) continue;
          spacerPx = Math.max(
            spacerPx,
            Math.min(rect.bottom, box.bottom) - Math.max(rect.top, box.top)
          );
        }
        probe.current = {
          scrollTop: container.scrollTop,
          readerTotalPx: probe.readerTotalPx,
          tracked,
          centre,
          spacerPx
        };
      }
    };
    (window as unknown as { __paintProbe: PaintedFrameProbe }).__paintProbe = probe;
    const nativeRequestAnimationFrame = window.requestAnimationFrame.bind(window);
    window.requestAnimationFrame = (callback) =>
      nativeRequestAnimationFrame((ts) => {
        probe.frameTs = ts;
        try {
          callback(ts);
        } finally {
          queueMicrotask(() => probe.sample());
        }
      });
    const NativeResizeObserver = window.ResizeObserver;
    window.ResizeObserver = class extends NativeResizeObserver {
      constructor(callback: ResizeObserverCallback) {
        super((entries, observer) => {
          try {
            callback(entries, observer);
          } finally {
            queueMicrotask(() => probe.sample());
          }
        });
      }
    };
    // Every frame is sampled, including one in which the app has no callback.
    const tick = () =>
      nativeRequestAnimationFrame((ts) => {
        probe.frameTs = ts;
        queueMicrotask(() => probe.sample());
        tick();
      });
    tick();
  });
}

async function openAtLiveEdge(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.locator(`[data-event-id="${HARNESS_SEED_EVENT_ID}"]`)).toBeVisible();
  const first = HISTORY_ROWS - WINDOW_ROWS;
  const items = Array.from({ length: WINDOW_ROWS }, (_, offset) => makeEventItem(first + offset));
  // appHarnessMain emits its own seed during boot; retry until this window wins.
  await expect
    .poll(
      async () =>
        page.evaluate(
          async ({ key, nextItems, probeId }) => {
            await window.__harness.pushCoreEvent({
              kind: "Timeline",
              event: {
                InitialItems: { request_id: null, key, generation: 2, items: nextItems }
              }
              // eslint-disable-next-line @typescript-eslint/no-explicit-any
            } as any);
            await new Promise((resolve) => setTimeout(resolve, 25));
            return document.querySelector(`[data-item-id="${CSS.escape(probeId)}"]`) !== null;
          },
          { key: TIMELINE_KEY, nextItems: items, probeId: eventId(HISTORY_ROWS - 1) }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
  await expect(page.locator("[data-testid=timeline-view]")).toHaveAttribute(
    "data-total-items",
    String(WINDOW_ROWS)
  );
  for (let frame = 0; frame < 8; frame += 1) {
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
  }
}

/** Answer each backward page the way Core extends its window from cached history. */
async function installWindowSlidingCore(page: Page): Promise<void> {
  await page.evaluate(
    ({ key, windowRows, pageRows, historyRows, items }) => {
      type Harness = {
        currentSnapshot(): unknown;
        setCommandResponse(command: string, response: () => unknown): void;
        pushCoreEvent(event: unknown): Promise<void> | void;
      };
      const harness = (window as unknown as { __harness: Harness }).__harness;
      const state = { first: historyRows - windowRows, batch: 10, slides: 0 };
      (window as unknown as { __slideState: typeof state }).__slideState = state;
      harness.setCommandResponse("paginate_timeline_backwards", () => {
        // Core answers on its own task, a little after the request
        // (navigation.rs, the cached backward window extension).
        setTimeout(() => {
          if (state.first > 0) {
            const count = Math.min(pageRows, state.first);
            const older = items.slice(state.first - count, state.first);
            state.first -= count;
            state.slides += 1;
            const diffs = [
              ...Array.from({ length: count }, () => ({ Remove: { index: windowRows - count } })),
              ...older.map((item, index) => ({ Insert: { index, item } }))
            ];
            void harness.pushCoreEvent({
              kind: "Timeline",
              event: { ItemsUpdated: { key, generation: 2, batch_id: ++state.batch, diffs } }
            });
          }
          void harness.pushCoreEvent({
            kind: "Timeline",
            event: {
              PaginationStateChanged: {
                request_id: null,
                key,
                direction: "Backward",
                state: "Idle",
                prepend_expected: false
              }
            }
          });
        }, 30);
        return harness.currentSnapshot();
      });
    },
    {
      key: TIMELINE_KEY,
      windowRows: WINDOW_ROWS,
      pageRows: PAGE_ROWS,
      historyRows: HISTORY_ROWS,
      items: Array.from({ length: HISTORY_ROWS }, (_, index) => makeEventItem(index))
    }
  );
}

type ScrollResult = {
  steps: number;
  slides: number;
  appCorrections: number;
  jumps: Array<{ frame: number; extraPx: number }>;
  spacerFrames: number;
};

/** Scroll up until Core's window has slid `slides` times, then a little more. */
async function scrollUpAcrossSlides(page: Page, slides: number): Promise<ScrollResult> {
  return page.evaluate(
    ({ slides, stepPx, intervalMs, tolerancePx }) =>
      new Promise<ScrollResult>((resolve) => {
        const probe = (window as unknown as { __paintProbe: PaintedFrameProbe }).__paintProbe;
        const slideState = (window as unknown as { __slideState: { slides: number } }).__slideState;
        const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
        probe.active = true;
        let steps = 0;
        let settleSteps = -1;
        let stepsAtTop = 0;
        const finish = () => {
          probe.active = false;
          const result: ScrollResult = {
            steps,
            slides: slideState.slides,
            appCorrections: 0,
            jumps: [],
            spacerFrames: 0
          };
          for (let index = 1; index < probe.finals.length; index += 1) {
            const previous = probe.finals[index - 1];
            const current = probe.finals[index];
            const readerDeltaPx = current.readerTotalPx - previous.readerTotalPx;
            if (Math.abs(current.scrollTop - (previous.scrollTop - readerDeltaPx)) > 0.5) {
              result.appCorrections += 1;
            }
            if (current.spacerPx > 0.5) result.spacerFrames += 1;
            if (!previous.centre || current.tracked?.top == null) continue;
            const extraPx = current.tracked.top - (previous.centre.top + readerDeltaPx);
            if (Math.abs(extraPx) > tolerancePx) {
              result.jumps.push({ frame: index, extraPx: Math.round(extraPx) });
            }
          }
          resolve(result);
        };
        const step = () => {
          if (slideState.slides >= slides && settleSteps < 0) settleSteps = 120;
          if (settleSteps >= 0) settleSteps -= 1;
          stepsAtTop = container.scrollTop > 0 ? 0 : stepsAtTop + 1;
          if (settleSteps === 0 || stepsAtTop > 120 || steps > 4000) {
            finish();
            return;
          }
          container.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -stepPx }));
          const before = container.scrollTop;
          container.scrollTop -= stepPx;
          probe.readerTotalPx += before - container.scrollTop;
          steps += 1;
          setTimeout(step, intervalMs);
        };
        setTimeout(step, intervalMs);
      }),
    {
      slides,
      stepPx: STEP_PX,
      intervalMs: STEP_INTERVAL_MS,
      tolerancePx: ANCHOR_TOLERANCE_PX
    }
  );
}

test("upward reading across bounded-window slides keeps the reader's row in every painted frame", async ({
  page
}) => {
  // Over a thousand 16 ms steps through two window slides.
  test.setTimeout(120_000);
  await installPaintedFrameProbe(page);
  await openAtLiveEdge(page);
  await installWindowSlidingCore(page);
  await page.locator("[data-testid=timeline-view]").hover();

  const result = await scrollUpAcrossSlides(page, REQUIRED_SLIDES);
  await test.info().attach("window-slide-scroll", {
    contentType: "application/json",
    body: JSON.stringify({ ...result, jumps: result.jumps.slice(0, 40) }, null, 2)
  });

  // The premises: the reader crossed Core's window edge, and rows mounted at a
  // height other than the estimate, so the app had positions to correct.
  expect(result.slides).toBeGreaterThanOrEqual(REQUIRED_SLIDES);
  expect(result.appCorrections).toBeGreaterThan(0);
  // Every correction reached the frame that needed it.
  expect(result.jumps).toEqual([]);
  expect(result.spacerFrames).toBe(0);
});
