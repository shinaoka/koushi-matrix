/**
 * Viewport observation and row isolation (#1892).
 *
 * The issue measured a scroll that ran at 3.5-20 fps on WebKitGTK with 5.2-6.9 s
 * of `getBoundingClientRect` in a 6-8 s run, and the Rust core taking 2 % of it.
 * Every scroll event asked the viewport question again: the visible range was
 * read, the range was compared, and the observation was sent — several times per
 * painted frame, for a range that can only change once per painted frame.
 *
 * The three things this file pins are the ones a reader can check from outside:
 * at most one observation per frame, the anchor written once when the scroll
 * settles rather than once per pixel, and a shell-level update leaving rows that
 * did not change alone. The closed-popover budget (#1894) rides along with the
 * scroll, because a popover hook that reads layout when it is closed is the same
 * defect class.
 */

import { expect, test, type Page } from "@playwright/test";

import {
  installRendererBudget,
  readRendererBudget,
  resetRendererBudget,
  settleRendererBudget
} from "./support/rendererBudget";
import {
  BUDGET_ROOMS,
  gotoBudgetHarness,
  openBudgetRoom,
  seedBudgetRooms
} from "./support/rendererBudgetFixtures";

const VIEWPORT = { width: 1280, height: 800 };

/** A real scroll delivers many events per painted frame; this spec says how many. */
const BURST = {
  frames: 12,
  eventsPerFrame: 10,
  stepPx: 40
} as const;

test.beforeEach(async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installRendererBudget(page);
});

async function gotoApp(page: Page): Promise<void> {
  await gotoBudgetHarness(page);
  await seedBudgetRooms(page);
}

function observationCount(page: Page): Promise<number> {
  return page.evaluate(() => window.__harness.invocationsOf("observe_timeline_viewport").length);
}

function anchorWriteCount(page: Page): Promise<number> {
  return page.evaluate(
    () => window.__harness.invocationsOf("update_navigation_scroll_anchor").length
  );
}

/**
 * Move the scrollport `frames` times, dispatching `eventsPerFrame` scroll events
 * in each moved frame, and report how many observations landed in each frame.
 *
 * A native wheel would put engine-specific input handling between the test and
 * the app (#1507), so the scrollport is moved directly and the scroll events the
 * app listens for are dispatched — many of them, as a real scroll does.
 */
async function burstScroll(
  page: Page
): Promise<{ perFrame: number[]; total: number }> {
  return page.evaluate(async (burst) => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
    if (!container) {
      throw new Error("viewport observation: timeline viewport is missing");
    }
    const observed = () =>
      window.__harness.invocationsOf("observe_timeline_viewport").length;
    const nextFrame = () => new Promise<number>((resolve) => requestAnimationFrame(resolve));
    const start = observed();
    const perFrame: number[] = [];
    let previous = start;
    for (let frame = 0; frame < burst.frames; frame += 1) {
      for (let event = 0; event < burst.eventsPerFrame; event += 1) {
        container.scrollTop = Math.max(0, container.scrollTop - burst.stepPx);
        container.dispatchEvent(new Event("scroll", { bubbles: true }));
      }
      await nextFrame();
      const current = observed();
      perFrame.push(current - previous);
      previous = current;
    }
    return { perFrame, total: previous - start };
  }, BURST);
}

test("a scroll burst sends at most one viewport observation per frame", async ({ page }) => {
  await gotoApp(page);
  await openBudgetRoom(page, BUDGET_ROOMS.long);
  await settleRendererBudget(page);
  await resetRendererBudget(page);

  const { perFrame, total } = await burstScroll(page);
  const read = await readRendererBudget(page);

  // Before #1892 each of the ten scroll events in a frame could ask again; the
  // signature dedupe only suppressed the ones that landed inside the same range.
  expect(
    perFrame.filter((count) => count > 1),
    `a frame sent more than one viewport observation: ${perFrame.join(",")}`
  ).toEqual([]);
  // Ten events per frame is more than a range can change, so the coalescer has to
  // be dropping asks rather than forwarding each one.
  expect(total, "no viewport observation was sent at all").toBeGreaterThan(0);
  expect(
    total,
    `${BURST.eventsPerFrame} scroll events per frame produced ${total} observations`
  ).toBeLessThanOrEqual(BURST.frames);

  // One observation per distinct range: the burst changes the range several
  // times, and the same payload must never be sent twice in a row. The dedupe is
  // what keeps a repeat from becoming a whole-shell snapshot push per frame.
  const payloads = await page.evaluate(() =>
    window.__harness
      .invocationsOf("observe_timeline_viewport")
      .map((invocation) => JSON.stringify(invocation.args))
  );
  const repeatedPayloads = payloads.filter(
    (payload, index) => index > 0 && payload === payloads[index - 1]
  );
  expect(repeatedPayloads, "the same observation was sent twice in a row").toEqual([]);

  // #1894: a closed receipts popover listens to nothing, observes nothing and
  // reads no layout while the timeline scrolls underneath it, and it does not
  // mount anything in the document.
  expect(
    read.floatingLayerReads,
    `a closed popover measured layout; top callers: ${read.layoutReadBuckets
      .slice(0, 5)
      .map((bucket) => `${bucket.key} (${bucket.count})`)
      .join(", ")}`
  ).toBe(0);
});

test("the scroll anchor is written once after the scroll settles, not per event", async ({
  page
}) => {
  await gotoApp(page);
  await openBudgetRoom(page, BUDGET_ROOMS.long);
  await settleRendererBudget(page);
  await resetRendererBudget(page);

  const before = await anchorWriteCount(page);
  // One frame's worth of scroll events, read in the same turn: a burst that is
  // still moving must not write, and reading after a round trip would race the
  // settle timer instead of measuring it.
  const duringScroll = await page.evaluate(async () => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
    if (!container) throw new Error("viewport observation: timeline viewport is missing");
    const written = () => window.__harness.invocationsOf("update_navigation_scroll_anchor").length;
    const start = written();
    for (let event = 0; event < 10; event += 1) {
      container.scrollTop = Math.max(0, container.scrollTop - 40);
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
    }
    await new Promise<number>((resolve) => requestAnimationFrame(resolve));
    return written() - start;
  });
  expect(duringScroll, "the anchor was written while the scroll was still moving").toBe(0);

  await settleRendererBudget(page);
  const afterSettle = (await anchorWriteCount(page)) - before;
  expect(afterSettle, "the settled range was never written").toBeGreaterThan(0);
  // The settled position can land twice: the anchor the scroll left and the one
  // the measurement commit recaptures differ by a pixel of row height. The
  // contract this pins is that ten events in one frame cost at most that, not
  // ten writes; an immediate write per event is what #1894 removed.
  expect(afterSettle, `${afterSettle} anchor writes for a settled 10-event frame`).toBeLessThanOrEqual(
    2
  );
});
