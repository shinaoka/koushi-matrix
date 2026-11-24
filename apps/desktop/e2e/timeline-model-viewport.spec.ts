/**
 * #1890 item 5: the viewport's own questions come from the height model.
 *
 * The reader's visible range, the gap ids in it and the scroll anchor used to be
 * read by looping over every mounted row with `getBoundingClientRect`. #1889 cut
 * that to a binary search plus the visible run, which is bounded by the viewport
 * but still one forced layout per probe and per visible row — so any write that
 * dirtied the layout before a scroll frame put the whole window back on the
 * critical path. The model answers the same questions from the cumulative
 * heights the spacers were written from, so the row reads stop being a function
 * of the row count at all.
 *
 * Two things this file pins, both from outside the app:
 *
 * - a scroll pays the same row reads in a 600-row room as in a 120-row one,
 * - the range the app reports is the range on screen (±1 row: rounded heights can
 *   move a boundary row in or out), after the open, after a scroll, and after the
 *   window is re-sliced by a jump.
 *
 * `installLayoutReadProbe` buckets reads by element, so `.timeline-item-frame`
 * reads — the measurement pass that *feeds* the model, untouched by this change —
 * are counted apart and never asserted here.
 *
 * Measured on a 1280x800 viewport, one open plus the scripted scroll: 1379 row
 * reads over 59 passes before the change, 47 over 50 after — and the 600-row
 * room reads the same 47 over 50 as the 120-row one.
 */

import { expect, test, type Page } from "@playwright/test";
import { gotoReadyShell, seedTimelineItems } from "./support/basicOperations";
import {
  installLayoutReadProbe,
  layoutBudgetItem,
  layoutReads,
  resetLayoutReads,
  seedSwitchableRoom,
  switchableRoomKey,
  type LayoutReadCounts
} from "./support/roomSwitch";

const SMALL_ROOM_ID = "!model-small:example.invalid";
const SMALL_ROOM_NAME = "Model Small";
const LARGE_ROOM_ID = "!model-large:example.invalid";
const LARGE_ROOM_NAME = "Model Large";
const SMALL_ROWS = 120;
const LARGE_ROWS = 600;

/**
 * Row reads one scripted scroll may cost, whatever the room holds. The scripted
 * scroll below moves the viewport ten times and settles twice afterwards; a pass
 * reads the anchor row and the row the room anchor is taken from, so the whole
 * run is a small multiple of the passes. Before #1890 item 5 the same scroll read
 * every mounted row on every pass (~23 rows a pass on this viewport).
 */
const ROW_READ_CEILING = 80;
/**
 * The 600-row room may pay what the 120-row room pays, plus the passes the extra
 * frames run. Deliberately tight: the point is that the cost does not follow the
 * item count, and 5x the rows must not mean more reads per pass.
 */
const ROW_READ_SLACK = 24;
/** Frames of scripted scrolling per measurement. */
const SCROLL_FRAMES = 10;
const SCROLL_EVENTS_PER_FRAME = 5;
const SCROLL_STEP_PX = 40;

function items(prefix: string, count: number): unknown[] {
  return Array.from({ length: count }, (_, index) =>
    layoutBudgetItem(`$${prefix}-${index}:example.invalid`, index)
  );
}

async function settleFrames(page: Page, frames = 3): Promise<void> {
  await page.evaluate(
    (count) =>
      new Promise<void>((resolve) => {
        let remaining = count;
        const step = () => {
          remaining -= 1;
          if (remaining <= 0) {
            resolve();
            return;
          }
          requestAnimationFrame(step);
        };
        requestAnimationFrame(step);
      }),
    frames
  );
}

/** Open a seeded room, settle it, and scroll it away from the live edge. */
async function openAndScroll(
  page: Page,
  roomName: string,
  roomId: string,
  prefix: string,
  count: number
): Promise<LayoutReadCounts> {
  await page.locator(`.room-item[aria-label="${roomName}"]`).click();
  await expect(page.locator(`.room-item.is-active[aria-label="${roomName}"]`)).toHaveCount(1);
  await seedTimelineItems(page, items(prefix, count), 2, switchableRoomKey(roomId));
  await expect(page.locator('[data-testid="timeline-view"]')).toHaveAttribute(
    "data-total-items",
    String(count)
  );
  await settleFrames(page, 3);
  // One frame's worth of scroll events per step, as a wheel delivers them.
  await page.evaluate(
    async ({ frames, events, step }) => {
      const container = document.querySelector<HTMLElement>('[data-testid="timeline-view"]');
      if (!container) {
        throw new Error("model viewport: timeline viewport is missing");
      }
      for (let frame = 0; frame < frames; frame += 1) {
        for (let event = 0; event < events; event += 1) {
          container.scrollTop = Math.max(0, container.scrollTop - step / events);
          container.dispatchEvent(new Event("scroll", { bubbles: true }));
        }
        await new Promise((resolve) => requestAnimationFrame(resolve));
      }
    },
    { frames: SCROLL_FRAMES, events: SCROLL_EVENTS_PER_FRAME, step: SCROLL_STEP_PX }
  );
  await settleFrames(page, 2);
  return layoutReads(page);
}

test.beforeEach(async ({ page }) => {
  await installLayoutReadProbe(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await gotoReadyShell(page);
  await seedSwitchableRoom(page, SMALL_ROOM_ID, SMALL_ROOM_NAME);
  await seedSwitchableRoom(page, LARGE_ROOM_ID, LARGE_ROOM_NAME);
});

test("a scroll reads the same rows in a 600-row room as in a 120-row one", async ({ page }) => {
  await resetLayoutReads(page);
  const small = await openAndScroll(page, SMALL_ROOM_NAME, SMALL_ROOM_ID, "model-small", SMALL_ROWS);
  await resetLayoutReads(page);
  const large = await openAndScroll(page, LARGE_ROOM_NAME, LARGE_ROOM_ID, "model-large", LARGE_ROWS);

  expect(small.box, "the small room ran no observation pass").toBeGreaterThan(0);
  expect(large.box, "the large room ran no observation pass").toBeGreaterThan(0);
  expect(
    small.row,
    `${small.row} row reads over ${small.box} passes in a ${SMALL_ROWS}-row room`
  ).toBeLessThanOrEqual(ROW_READ_CEILING);
  expect(
    large.row,
    `${large.row} row reads over ${large.box} passes in a ${LARGE_ROWS}-row room`
  ).toBeLessThanOrEqual(ROW_READ_CEILING);
  // The claim is the absence of a per-row cost, so the five-fold room may not
  // cost five times the reads.
  expect(
    large.row,
    `${large.row} row reads in the ${LARGE_ROWS}-row room against ${small.row} in the ${SMALL_ROWS}-row one`
  ).toBeLessThanOrEqual(small.row + ROW_READ_SLACK);
});

interface ViewportTruth {
  /** Event ids of the mounted rows in display order. */
  order: string[];
  firstIndex: number;
  lastIndex: number;
}

/** The rows whose boxes actually cross the viewport band, read by the spec. */
async function visibleTruth(page: Page): Promise<ViewportTruth> {
  return page.evaluate(() => {
    const container = document.querySelector<HTMLElement>('[data-testid="timeline-view"]');
    if (!container) {
      throw new Error("model viewport: timeline viewport is missing");
    }
    const bandTop = container.getBoundingClientRect().top;
    const bandBottom = bandTop + container.clientHeight;
    const nodes = [
      ...container.querySelectorAll<HTMLElement>("[data-activity-event-id]")
    ];
    const order = nodes.map((node) => node.dataset["activityEventId"] ?? "");
    let firstIndex = -1;
    let lastIndex = -1;
    nodes.forEach((node, index) => {
      const rect = node.getBoundingClientRect();
      if (rect.bottom > bandTop && rect.top < bandBottom) {
        if (firstIndex < 0) firstIndex = index;
        lastIndex = index;
      }
    });
    return { order, firstIndex, lastIndex };
  });
}

/** The range the last observation reported, as indices in the mounted order. */
async function observedRange(
  page: Page
): Promise<{ firstIndex: number; lastIndex: number; calls: number }> {
  return page.evaluate(() => {
    const calls = window.__harness.invocationsOf("observe_timeline_viewport");
    const last = calls.at(-1)?.args as
      | { firstVisibleEventId: string | null; lastVisibleEventId: string | null }
      | undefined;
    const order = [
      ...document.querySelectorAll<HTMLElement>('[data-testid="timeline-view"] [data-activity-event-id]')
    ].map((node) => node.dataset["activityEventId"] ?? "");
    return {
      firstIndex: last?.firstVisibleEventId ? order.indexOf(last.firstVisibleEventId) : -1,
      lastIndex: last?.lastVisibleEventId ? order.indexOf(last.lastVisibleEventId) : -1,
      calls: calls.length
    };
  });
}

/**
 * The observation names the rows on screen. Rounded heights can put a boundary
 * row on the other side of the edge from the layout, so the tolerance is one row
 * — which is invisible to a read receipt and to the reader.
 */
async function expectObservationMatchesViewport(page: Page, label: string): Promise<void> {
  const truth = await visibleTruth(page);
  expect(truth.firstIndex, `${label}: no mounted row crossed the viewport`).toBeGreaterThanOrEqual(
    0
  );
  await expect
    .poll(
      async () => {
        const observed = await observedRange(page);
        if (observed.calls === 0) {
          return "no observation was sent";
        }
        const firstOff = Math.abs(observed.firstIndex - truth.firstIndex);
        const lastOff = Math.abs(observed.lastIndex - truth.lastIndex);
        if (observed.firstIndex < 0 || observed.lastIndex < 0) {
          return `the observation named a row that is not mounted: ${observed.firstIndex},${observed.lastIndex}`;
        }
        if (firstOff > 1 || lastOff > 1) {
          return `observed rows ${observed.firstIndex}..${observed.lastIndex}, on screen ${truth.firstIndex}..${truth.lastIndex}`;
        }
        return "ok";
      },
      { message: `${label}: the observation does not name the rows on screen` }
    )
    .toBe("ok");
}

test("the reported range is the range on screen after an open, a scroll and a jump", async ({
  page
}) => {
  await page.locator(`.room-item[aria-label="${LARGE_ROOM_NAME}"]`).click();
  await expect(page.locator(`.room-item.is-active[aria-label="${LARGE_ROOM_NAME}"]`)).toHaveCount(1);
  await seedTimelineItems(
    page,
    items("model-large", LARGE_ROWS),
    2,
    switchableRoomKey(LARGE_ROOM_ID)
  );
  await expect(page.locator('[data-testid="timeline-view"]')).toHaveAttribute(
    "data-total-items",
    String(LARGE_ROWS)
  );
  await settleFrames(page, 3);
  await expectObservationMatchesViewport(page, "after the open");

  // A wheel-like scroll, many events per frame.
  await page.evaluate(async ({ events, step }) => {
    const container = document.querySelector<HTMLElement>('[data-testid="timeline-view"]');
    if (!container) {
      throw new Error("model viewport: timeline viewport is missing");
    }
    for (let event = 0; event < events; event += 1) {
      container.scrollTop = Math.max(0, container.scrollTop - step / events);
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
    }
    await new Promise((resolve) => requestAnimationFrame(resolve));
  }, { events: 10, step: 400 });
  await settleFrames(page, 2);
  await expectObservationMatchesViewport(page, "after a scroll");

  // A jump far up the history re-slices the mounted window in one step.
  await page.evaluate(async () => {
    const container = document.querySelector<HTMLElement>('[data-testid="timeline-view"]');
    if (!container) {
      throw new Error("model viewport: timeline viewport is missing");
    }
    container.scrollTop = Math.max(0, container.scrollTop - 5000);
    container.dispatchEvent(new Event("scroll", { bubbles: true }));
    await new Promise((resolve) => requestAnimationFrame(resolve));
  });
  await settleFrames(page, 3);
  await expectObservationMatchesViewport(page, "after the window was re-sliced");
});
