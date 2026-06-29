/**
 * Real-browser regression for free-scroll stability.
 *
 * TimelineView no longer runs a retained-anchor post-settle correction loop.
 * In free-scroll mode, layout changes above the viewport are expected to be
 * handled by native browser scroll anchoring (`overflow-anchor: auto`). jsdom
 * cannot prove that, so this spec uses Playwright/Chromium and the real
 * TimelineView harness.
 */

import { expect, test, type Page } from "@playwright/test";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_SEED_EVENT_ID = "$seed-event:example.invalid";
const ANCHOR_PIXEL_TOLERANCE = 3;

const TIMELINE_KEY = {
  account_key: HARNESS_ACCOUNT_KEY,
  kind: { Room: { room_id: HARNESS_ROOM_ID } }
};

function makeEventItem(index: number, body?: string) {
  const eventId = `$scroll-anchor-item-${String(index).padStart(4, "0")}:example.invalid`;
  return {
    id: { Event: { event_id: eventId } },
    sender: "@sender:example.invalid",
    body: body ?? `scroll anchor message ${index}\nThis row has stable text height.`,
    formatted: null,
    timestamp_ms: 1_800_000_000_000 + index * 1000,
    in_reply_to_event_id: null,
    reply_quote: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: [],
    send_state: null,
    actions: null,
    thread_summary: null,
    media: null,
    sender_label: `Sender ${index}`,
    original_display_label: ""
  };
}

async function gotoReadyApp(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
}

async function seedTimeline(page: Page, itemCount: number): Promise<void> {
  await seedTimelineWithItems(
    page,
    Array.from({ length: itemCount }, (_, index) => makeEventItem(index))
  );
}

async function seedTimelineWithItems(
  page: Page,
  items: ReturnType<typeof makeEventItem>[]
): Promise<void> {
  // appHarnessMain emits its own seed item during boot. Wait for it before
  // replacing the timeline, otherwise the boot retry loop can overwrite this
  // test's InitialItems between Playwright assertions.
  await expect(page.locator(`[data-event-id="${HARNESS_SEED_EVENT_ID}"]`)).toBeVisible();

  await page.evaluate(
    async ({ key, nextItems }) => {
      await window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: {
            request_id: null,
            key,
            generation: 2,
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            items: nextItems as any[]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: TIMELINE_KEY, nextItems: items }
  );
  const lastIndex = items.length - 1;
  const lastEventId = `$scroll-anchor-item-${String(lastIndex).padStart(4, "0")}:example.invalid`;
  await expect(page.locator(`[data-event-id="${lastEventId}"]`)).toBeAttached();
}

async function waitAnimationFrames(page: Page, count: number): Promise<void> {
  for (let index = 0; index < count; index += 1) {
    await page.evaluate(
      () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
    );
  }
}

async function placeAnchorInViewport(
  page: Page,
  targetIndex: number
): Promise<{ anchorEventId: string; originalOffset: number }> {
  const anchorEventId = `$scroll-anchor-item-${String(targetIndex).padStart(4, "0")}:example.invalid`;

  await page.evaluate(
    ({ target }) => {
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
      if (!container) return;
      container.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -120 }));
      container.scrollTop = Math.max(0, target * 90);
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
    },
    { target: targetIndex }
  );

  const anchor = page.locator(`[data-event-id="${anchorEventId}"]`);
  await expect(anchor).toBeVisible({ timeout: 5000 });

  const originalOffset = await anchorOffsetFromContainer(page, anchorEventId);
  expect(originalOffset).not.toBeNull();
  return { anchorEventId, originalOffset: originalOffset! };
}

async function anchorOffsetFromContainer(page: Page, eventId: string): Promise<number | null> {
  return page.evaluate(
    ({ targetEventId }) => {
      const anchor = document.querySelector<HTMLElement>(`[data-event-id="${targetEventId}"]`);
      const container = anchor?.closest<HTMLElement>("[data-testid=timeline-view]");
      if (!anchor || !container) return null;
      return anchor.getBoundingClientRect().top - container.getBoundingClientRect().top;
    },
    { targetEventId: eventId }
  );
}

async function growRenderedRowAboveViewport(page: Page): Promise<void> {
  await page.evaluate(() => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
    if (!container) return;
    const containerTop = container.getBoundingClientRect().top;
    const rows = Array.from(
      container.querySelectorAll<HTMLElement>("[data-event-id]")
    ).filter((row) => row.getBoundingClientRect().bottom < containerTop);
    const target = rows.at(-1);
    if (!target) {
      throw new Error("No rendered row above viewport to grow");
    }
    target.style.paddingBlockEnd = "180px";
  });
  await page.waitForTimeout(100);
}

async function expectAnchorStayedPut(
  page: Page,
  eventId: string,
  originalOffset: number
): Promise<void> {
  await expect
    .poll(
      async () => {
        const nextOffset = await anchorOffsetFromContainer(page, eventId);
        if (nextOffset === null) return Number.POSITIVE_INFINITY;
        return Math.abs(nextOffset - originalOffset);
      },
      { timeout: 3000, intervals: [30, 50, 100, 200] }
    )
    .toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
}

async function timelineScrollTop(page: Page): Promise<number> {
  return page.evaluate(() => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
    return container?.scrollTop ?? 0;
  });
}

test("native scroll anchoring keeps non-virtualized free-scroll viewport stable", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 80);

  const { anchorEventId, originalOffset } = await placeAnchorInViewport(page, 8);
  await growRenderedRowAboveViewport(page);
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);
});

test("native scroll anchoring keeps virtualized free-scroll viewport stable", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 720);

  const { anchorEventId, originalOffset } = await placeAnchorInViewport(page, 220);
  await growRenderedRowAboveViewport(page);
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);
});

test("virtualized free-scroll viewport stays stable while row measurements settle", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimelineWithItems(
    page,
    Array.from({ length: 720 }, (_, index) =>
      makeEventItem(
        index,
        Array.from(
          { length: index % 7 === 0 ? 9 : 2 },
          (_, line) => `scroll anchor message ${index} line ${line}`
        ).join("\n")
      )
    )
  );

  const { anchorEventId, originalOffset } = await placeAnchorInViewport(page, 220);
  await waitAnimationFrames(page, 8);
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);
});

test("virtualized free-scroll viewport accepts small user wheel movement", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 720);
  await placeAnchorInViewport(page, 220);

  const timeline = page.locator("[data-testid=timeline-view]");
  await timeline.hover();
  const beforeScrollTop = await timelineScrollTop(page);
  await page.mouse.wheel(0, 180);
  await waitAnimationFrames(page, 8);
  const afterScrollTop = await timelineScrollTop(page);

  expect(afterScrollTop).toBeGreaterThan(beforeScrollTop + 20);
});

test("virtualized free-scroll viewport accepts small user wheel movement toward older messages", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 720);
  await placeAnchorInViewport(page, 220);

  const timeline = page.locator("[data-testid=timeline-view]");
  await timeline.hover();
  const beforeScrollTop = await timelineScrollTop(page);
  await page.mouse.wheel(0, -180);
  await waitAnimationFrames(page, 8);
  const afterScrollTop = await timelineScrollTop(page);

  expect(afterScrollTop).toBeLessThan(beforeScrollTop - 20);
});
