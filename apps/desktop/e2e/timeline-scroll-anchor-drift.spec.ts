/**
 * Real-browser regression for free-scroll stability.
 *
 * `.timeline-view` deliberately disables browser scroll anchoring
 * (`overflow-anchor: none`) so Koushi owns event-based viewport stability.
 * jsdom cannot prove real scroll positions or wheel behavior, so this spec uses
 * Playwright/Chromium and the real TimelineView harness.
 */

import { expect, test, type Page } from "@playwright/test";
import { SCROLL_EDGE_TOLERANCE_PX } from "../src/components/timeline/TimelineViewportObservation";
import { roomTimelineKey } from "../src/domain/coreEvents";

const HARNESS_ROOM_ID = "!harness-room:example.invalid";
const HARNESS_ACCOUNT_KEY = "@harness-user:example.invalid";
const HARNESS_SEED_EVENT_ID = "$seed-event:example.invalid";
const ANCHOR_PIXEL_TOLERANCE = 3;

const TIMELINE_KEY = roomTimelineKey(HARNESS_ACCOUNT_KEY, HARNESS_ROOM_ID);

function makeEventItem(index: number, body?: string) {
  const eventId = `$scroll-anchor-item-${String(index).padStart(4, "0")}:example.invalid`;
  return {
    id: { Event: { event_id: eventId } },
    sender: HARNESS_ACCOUNT_KEY,
    sender_label: "Harness User",
    body: body ?? `scroll anchor message ${index}\nThis row has stable text height.`,
    timestamp_ms: 1_800_000_000_000 + index * 1000,
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

async function gotoReadyApp(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reply to message" }).first()).toBeVisible();
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

  await expect
    .poll(
      async () =>
        page.evaluate(
          async ({ key, nextItems }) => {
            const itemDomIds = nextItems.flatMap((item) => {
              const eventId = (item as { id: { Event?: { event_id: string } } }).id.Event?.event_id;
              return eventId ? [eventId] : [];
            });
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
            await new Promise((resolve) => setTimeout(resolve, 25));
            return itemDomIds.some((id) =>
              document.querySelector(`[data-item-id="${CSS.escape(id)}"]`)
            );
          },
          { key: TIMELINE_KEY, nextItems: items }
        ),
      { timeout: 10_000, intervals: [25, 50, 100, 250] }
    )
    .toBe(true);
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

  await page.evaluate(() => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
    if (!container) return;
    container.scrollTop = 0;
    container.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 120 }));
    container.dispatchEvent(new Event("scroll", { bubbles: true }));
  });

  const anchor = page.locator(`[data-event-id="${anchorEventId}"]`);
  // Move through the virtualized range in bounded, frame-separated steps. A
  // single estimated-height jump can mount the overscanned row while its
  // measured range still places it far outside the viewport; stepping lets the
  // production range model settle before the anchor oracle begins. Each step
  // is shorter than the viewport (the harness viewport is ~420 px): a step of a
  // whole screen or more can carry a row from just below the viewport to just
  // above it between two checks, depending on when measured heights replace
  // estimates, and the walk then runs on to the live edge (seen on CI runners).
  for (let step = 0; step < 300; step += 1) {
    const inViewport = await page.evaluate((eventId) => {
      const row = document.querySelector<HTMLElement>(
        `[data-event-id="${CSS.escape(eventId)}"]`
      );
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
      if (!row || !container) return false;
      const rowRect = row.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      return rowRect.bottom > containerRect.top && rowRect.top < containerRect.bottom;
    }, anchorEventId);
    if (inViewport) break;
    await page.evaluate(() => {
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
      if (!container) return;
      const delta = Math.max(100, Math.floor(container.clientHeight * 0.75));
      container.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: delta }));
      container.scrollTop += delta;
      container.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await waitAnimationFrames(page, 2);
  }

  await expect(anchor).toBeVisible({ timeout: 5000 });
  await expect(anchor).toBeInViewport();

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

test("Koushi-owned anchoring keeps virtualized free-scroll viewport stable", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 720);

  const { anchorEventId, originalOffset } = await placeAnchorInViewport(page, 220);
  await growRenderedRowAboveViewport(page);
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);
});

test("a 147-row room keeps mounting bounded while scrolling and measuring rows", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 147);

  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-virtualized", "true");
  const { anchorEventId, originalOffset } = await placeAnchorInViewport(page, 70);
  expect(await timeline.locator("[data-item-id]").count()).toBeLessThanOrEqual(80);
  await growRenderedRowAboveViewport(page);
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);
});

test("virtualized date dividers have the same modeled and physical height", async ({ page }) => {
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await gotoReadyApp(page);
  const items = Array.from({ length: 115 }, (_, index) => makeEventItem(index));
  for (let index = 17; index >= 0; index -= 1) {
    const timestamp = 1_800_000_000_000 + index * 86_400_000;
    items.splice(index * 6, 0, {
      ...makeEventItem(1000 + index),
      id: { Synthetic: { synthetic_id: `date-divider-${timestamp}` } },
      timestamp_ms: timestamp
    } as unknown as ReturnType<typeof makeEventItem>);
  }
  await seedTimelineWithItems(page, items);
  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-total-items", "133");
  await timeline.evaluate((node) => {
    node.scrollTop = 0;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await waitAnimationFrames(page, 4);
  const mismatch = await page.evaluate(() => {
    const list = document.querySelector<HTMLElement>(".timeline-item-list");
    if (!list) throw new Error("Missing timeline list");
    const children = Array.from(list.children) as HTMLElement[];
    const sum = children.reduce((height, child) => height + child.getBoundingClientRect().height, 0);
    return Math.abs(list.getBoundingClientRect().height - sum);
  });
  expect(mismatch).toBeLessThanOrEqual(1);
  const { anchorEventId, originalOffset } = await placeAnchorInViewport(page, 70);
  for (let reversal = 0; reversal < 20; reversal += 1) {
    await timeline.evaluate((node) => {
      node.scrollTop += 400;
      node.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await waitAnimationFrames(page, 2);
    await timeline.evaluate((node) => {
      node.scrollTop -= 400;
      node.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await waitAnimationFrames(page, 2);
  }
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);
  await timeline.hover();
  for (let reversal = 0; reversal < 60; reversal += 1) {
    await page.mouse.wheel(0, reversal % 2 === 0 ? 480 : -380);
    await waitAnimationFrames(page, 2);
    await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
    expect(await timeline.locator("[data-frame-item-id]").count()).toBeLessThanOrEqual(80);
  }
  expect(pageErrors).toEqual([]);
});

test("a backfill prepend keeps the reading row mounted inside a bounded window", async ({ page }) => {
  await gotoReadyApp(page);
  // Every room is windowed now, including lists shorter than a page of
  // history. This room used to mount all 79 rows because it was under the
  // retired 120-row threshold.
  await seedTimelineWithItems(
    page,
    Array.from({ length: 79 }, (_, index) => makeEventItem(index + 52))
  );

  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-virtualized", "true");
  await expect(timeline).toHaveAttribute("data-total-items", "79");
  // Bring the reading row into the mounted window before the prepend: the room
  // opens at the live edge, so row 70 is not mounted yet.
  const { anchorEventId, originalOffset } = await placeAnchorInViewport(page, 70);
  await page.evaluate((eventId) => {
    const row = document.querySelector(`[data-event-id="${eventId}"]`);
    const list = document.querySelector(".timeline-item-list");
    if (!row || !list) throw new Error("missing initial anchor");
    document.documentElement.dataset["anchorRemovedDuringPrepend"] = "false";
    let mountedRows = list.querySelectorAll(".timeline-item-frame").length;
    let peakMountedRows = mountedRows;
    // Publish the starting count too: a prepend that the window absorbs without
    // touching the DOM inserts no node, so the observer never reports and the
    // assertion below would compare NaN instead of the mount count it is about.
    document.documentElement.dataset["peakMountedRowsDuringPrepend"] = String(peakMountedRows);
    new MutationObserver((records) => {
      for (const record of records) {
        for (const node of record.removedNodes) {
          if (node === row || (node instanceof Element && node.contains(row))) {
            document.documentElement.dataset["anchorRemovedDuringPrepend"] = "true";
          }
          if (node instanceof Element && node.classList.contains("timeline-item-frame")) mountedRows -= 1;
        }
        for (const node of record.addedNodes) {
          if (node instanceof Element && node.classList.contains("timeline-item-frame")) mountedRows += 1;
        }
        peakMountedRows = Math.max(peakMountedRows, mountedRows);
      }
      document.documentElement.dataset["peakMountedRowsDuringPrepend"] = String(peakMountedRows);
    }).observe(list, { childList: true, subtree: true });
  }, anchorEventId);

  await page.evaluate(({ key, items }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key,
          generation: 2,
          batch_id: 1,
          diffs: items.map((item) => ({ PushFront: { item } }))
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: TIMELINE_KEY,
    items: Array.from({ length: 52 }, (_, index) => makeEventItem(index)).reverse()
  });

  await expect(timeline).toHaveAttribute("data-total-items", "131");
  await expect(timeline).toHaveAttribute("data-virtualized", "true");
  await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.dataset["anchorRemovedDuringPrepend"])).toBe("false");
  expect(Number(await page.evaluate(() => document.documentElement.dataset["peakMountedRowsDuringPrepend"]))).toBeLessThanOrEqual(100);
  expect(await timeline.locator("[data-frame-item-id]").count()).toBeLessThanOrEqual(80);
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);

  // Backfill again after virtualization is already active. The previous
  // window's indices now refer to older rows, so this transition must also
  // preserve the reading row and its bounded DOM window.
  await page.evaluate(({ key, items }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key,
          generation: 2,
          batch_id: 2,
          diffs: items.map((item) => ({ PushFront: { item } }))
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: TIMELINE_KEY,
    items: Array.from({ length: 52 }, (_, index) => makeEventItem(index - 52)).reverse()
  });
  await expect(timeline).toHaveAttribute("data-total-items", "183");
  expect(await page.evaluate(() => document.documentElement.dataset["anchorRemovedDuringPrepend"])).toBe("false");
  expect(Number(await page.evaluate(() => document.documentElement.dataset["peakMountedRowsDuringPrepend"]))).toBeLessThanOrEqual(100);
  expect(await timeline.locator("[data-frame-item-id]").count()).toBeLessThanOrEqual(80);
  await expectAnchorStayedPut(page, anchorEventId, originalOffset);

  await timeline.hover();
  for (let reversal = 0; reversal < 48; reversal += 1) {
    await page.mouse.wheel(0, reversal % 2 === 0 ? -520 : 420);
    await waitAnimationFrames(page, 2);
    await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
    expect(await timeline.locator("[data-frame-item-id]").count()).toBeLessThanOrEqual(80);
  }
});

test("repeated scroll reversals across variable-height rows keep the application mounted", async ({ page }) => {
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await gotoReadyApp(page);
  await seedTimelineWithItems(
    page,
    Array.from({ length: 180 }, (_, index) =>
      makeEventItem(
        index,
        index % 7 === 0
          ? Array.from({ length: 12 }, (_, line) => `message ${index} line ${line} ${"long text ".repeat(12)}`).join("\n")
          : index % 3 === 0
            ? `message ${index} ${"medium text ".repeat(10)}`
            : `message ${index}`
      )
    )
  );

  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-virtualized", "true");
  await timeline.hover();
  for (let reversal = 0; reversal < 60; reversal += 1) {
    await page.mouse.wheel(0, reversal % 2 === 0 ? -680 : 540);
    await waitAnimationFrames(page, 2);
    await expect(page.getByRole("main", { name: "Conversation timeline" })).toBeVisible();
  }
  expect(pageErrors).toEqual([]);
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
  await waitAnimationFrames(page, 8);

  const timeline = page.locator("[data-testid=timeline-view]");
  await timeline.hover();
  const beforeScrollTop = await timelineScrollTop(page);
  await page.mouse.wheel(0, 180);
  // WebKit may deliver the compositor's wheel scroll after the next several
  // animation frames on a busy CI runner. Observe the resulting position.
  await expect.poll(() => timelineScrollTop(page)).toBeGreaterThan(beforeScrollTop + 20);
});

test("virtualized free-scroll viewport accepts small user wheel movement toward older messages", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 720);
  await placeAnchorInViewport(page, 220);
  await waitAnimationFrames(page, 8);

  const timeline = page.locator("[data-testid=timeline-view]");
  await timeline.hover();
  const beforeScrollTop = await timelineScrollTop(page);
  await page.mouse.wheel(0, -180);
  await expect.poll(() => timelineScrollTop(page)).toBeLessThan(beforeScrollTop - 20);
});

// #1507: WebKit stores scrollTop in whole pixels, so a fractional anchor
// correction can never land exactly. A leftover above the 0.5 px tolerance used
// to restart stabilization every frame, rewriting scrollTop ~50 times a second
// and swallowing the reader's wheel input. Fractional growth steps make at least
// one leftover exceed the tolerance whatever the starting offset.
test("a sub-pixel anchor leftover settles instead of rewriting the viewport", async ({ page }) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 720);
  await placeAnchorInViewport(page, 220);
  await waitAnimationFrames(page, 8);
  await page.evaluate(() => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const descriptor = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop")!;
    const writes = { count: 0 };
    (window as unknown as { __scrollWrites: { count: number } }).__scrollWrites = writes;
    Object.defineProperty(container, "scrollTop", {
      configurable: true,
      get() { return descriptor.get!.call(this); },
      set(value: number) { writes.count += 1; descriptor.set!.call(this, value); }
    });
  });
  const writeCount = () => page.evaluate(() => (window as unknown as { __scrollWrites: { count: number } }).__scrollWrites.count);

  for (const growth of [180.2, 180.45, 180.7, 180.95]) {
    await page.evaluate((padding) => {
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
      const containerTop = container.getBoundingClientRect().top;
      const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-event-id]"))
        .filter((row) => row.getBoundingClientRect().bottom < containerTop);
      rows.at(-1)!.style.paddingBlockEnd = `${padding}px`;
    }, growth);
    await page.waitForTimeout(300);
    const settledAt = await writeCount();
    await page.waitForTimeout(500);
    expect(await writeCount() - settledAt, `scrollTop rewritten after ${growth}px growth settled`).toBe(0);
  }

  const timeline = page.locator("[data-testid=timeline-view]");
  await timeline.hover();
  const beforeScrollTop = await timelineScrollTop(page);
  await page.mouse.wheel(0, 180);
  await expect.poll(() => timelineScrollTop(page)).toBeGreaterThan(beforeScrollTop + 20);
});


/**
 * #1890: the mounted window is sized in screens, not in rows. These three
 * checks pin the consequences: what is mounted on open, when older history is
 * requested, and that the reported visible set is the physical one.
 */
test("a live-edge open mounts the visible screen plus one screen of history", async ({ page }) => {
  await gotoReadyApp(page);
  // Record every row ever added, not only the final window: a live-edge open
  // that mounts the top screen first and then snaps to the bottom shows up
  // here even though the final DOM looks right.
  await page.evaluate(() => {
    const seen = new Set<string>();
    (window as unknown as { __mountedRowIds: Set<string> }).__mountedRowIds = seen;
    new MutationObserver((records) => {
      for (const record of records) {
        for (const node of record.addedNodes) {
          if (!(node instanceof Element)) continue;
          const rows = node.matches("[data-item-id]")
            ? [node]
            : Array.from(node.querySelectorAll("[data-item-id]"));
          for (const row of rows) {
            const id = row.getAttribute("data-item-id");
            if (id) seen.add(id);
          }
        }
      }
    }).observe(document.body, { childList: true, subtree: true });
  });
  await seedTimeline(page, 120);
  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-total-items", "120");
  await waitAnimationFrames(page, 8);

  const bound = await page.evaluate(() => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const containerRect = container.getBoundingClientRect();
    const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-item-id]"));
    const visible = rows.filter((row) => {
      const rect = row.getBoundingClientRect();
      return rect.bottom > containerRect.top && rect.top < containerRect.bottom;
    });
    const rowHeight = rows[0]?.getBoundingClientRect().height || 1;
    return {
      visible: visible.length,
      screensAbove: Math.ceil(container.clientHeight / rowHeight)
    };
  });
  const everMounted = await page.evaluate(() =>
    Array.from((window as unknown as { __mountedRowIds: Set<string> }).__mountedRowIds)
  );

  // Row height follows the host font, so the bound is computed from the
  // rendered rows rather than assumed.
  expect(everMounted.length).toBeLessThanOrEqual(bound.visible + bound.screensAbove + 2);
  // The first row of a 120-row room was mounted here before windowing.
  expect(everMounted).not.toContain("$scroll-anchor-item-0000:example.invalid");
});

test("the observed viewport matches the physically visible rows", async ({ page }) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 120);
  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-total-items", "120");

  const compareObservation = async () => {
    const physical = await page.evaluate(() => {
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
      const containerRect = container.getBoundingClientRect();
      const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-event-id]")).filter(
        (row) => {
          const rect = row.getBoundingClientRect();
          return rect.bottom > containerRect.top && rect.top < containerRect.bottom;
        }
      );
      return {
        first: rows[0]?.getAttribute("data-event-id") ?? null,
        last: rows.at(-1)?.getAttribute("data-event-id") ?? null
      };
    });
    const observed = await page.evaluate(() => {
      const call = window.__harness.invocationsOf("observe_timeline_viewport").at(-1);
      return {
        first: (call?.args?.["firstVisibleEventId"] as string | null | undefined) ?? null,
        last: (call?.args?.["lastVisibleEventId"] as string | null | undefined) ?? null
      };
    });
    return { physical, observed };
  };

  // On open, after a user scroll, and after a prepend changes the window.
  await expect
    .poll(async () => (await compareObservation()).observed, { timeout: 10_000 })
    .toEqual((await compareObservation()).physical);

  await timeline.hover();
  await page.mouse.wheel(0, -400);
  await waitAnimationFrames(page, 4);
  await expect
    .poll(async () => (await compareObservation()).observed, { timeout: 10_000 })
    .toEqual((await compareObservation()).physical);

  await page.evaluate(({ key, items }) => {
    window.__harness.pushCoreEvent({
      kind: "Timeline",
      event: {
        ItemsUpdated: {
          key,
          generation: 2,
          batch_id: 1,
          diffs: items.map((item) => ({ PushFront: { item } }))
        }
      }
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any);
  }, {
    key: TIMELINE_KEY,
    items: Array.from({ length: 40 }, (_, index) => makeEventItem(index - 40))
  });
  await expect(timeline).toHaveAttribute("data-total-items", "160");
  await waitAnimationFrames(page, 6);
  await expect
    .poll(async () => (await compareObservation()).observed, { timeout: 10_000 })
    .toEqual((await compareObservation()).physical);
});

/** The app harness control surface the request probe drives. */
type RequestProbeHarness = {
  invocationsOf(command: string): readonly unknown[];
  currentSnapshot(): unknown;
  setCommandResponse(command: string, response: () => unknown): void;
};

type RequestProbe = {
  /** Every row height the row really had while it passed through the window. */
  heights: Map<string, number>;
  requests: Array<{
    firstVisibleId: string | null;
    scrollTop: number;
    clientHeight: number;
    listOffsetTop: number;
  }>;
  release: Array<() => void>;
};

/**
 * #1890 §2.4/§2.5: the fetch trigger is a distance from the *resident* top, so the
 * distance it reads has to be the reader's true one. The request is sampled at the
 * moment the page command is invoked — v1's form sampled `scrollTop` two frames
 * later, which conflates the trigger with the compensation that follows it — and
 * the true offset of the row that was first visible at that moment is rebuilt from
 * the heights every row was actually laid out with on its way past the window.
 */
test("older history is requested within two screens of the resident top", async ({ page }) => {
  await gotoReadyApp(page);
  // Rows that are not all the same height: the real app lays bodies out at their
  // content height, so a room whose history is uniform would not exercise the
  // measurement this contract rests on (the 2a honesty caveat).
  const itemCount = 120;
  const seededItems = Array.from({ length: itemCount }, (_, index) =>
    makeEventItem(
      index,
      index % 4 === 3 ? `${"a longer body line\n".repeat(5)}end ${index}` : undefined
    )
  );
  await seedTimelineWithItems(page, seededItems);
  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-total-items", String(itemCount));
  await waitAnimationFrames(page, 6);

  await page.evaluate(() => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const harness = (window as unknown as { __harness: RequestProbeHarness }).__harness;
    const probe: RequestProbe = { heights: new Map(), requests: [], release: [] };
    (window as unknown as { __requestProbe: RequestProbe }).__requestProbe = probe;

    const record = (node: Element) => {
      const id = (node as HTMLElement).dataset["itemId"];
      if (!id) return;
      const height = node.getBoundingClientRect().height;
      if (height > 0) probe.heights.set(id, height);
    };
    const observer = new ResizeObserver((entries) =>
      entries.forEach((entry) => record(entry.target))
    );
    const observeMounted = () => {
      for (const node of container.querySelectorAll("[data-item-id]")) {
        if ((node as unknown as { probed?: boolean }).probed) continue;
        (node as unknown as { probed?: boolean }).probed = true;
        observer.observe(node);
        record(node);
      }
    };
    observeMounted();
    new MutationObserver(observeMounted).observe(container, { childList: true, subtree: true });

    // Held, not answered: the request stays in flight, which is what lets the spec
    // assert that no second one follows it.
    harness.setCommandResponse("paginate_timeline_backwards", () => {
      const viewportTop = container.getBoundingClientRect().top;
      const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-item-id]"));
      const firstVisible =
        rows.find((row) => row.getBoundingClientRect().bottom > viewportTop + 0.5) ?? null;
      probe.requests.push({
        firstVisibleId: firstVisible?.dataset["itemId"] ?? null,
        scrollTop: container.scrollTop,
        clientHeight: container.clientHeight,
        listOffsetTop: container.querySelector<HTMLElement>(".timeline-item-list")?.offsetTop ?? 0
      });
      return new Promise((resolve) => probe.release.push(() => resolve(harness.currentSnapshot())));
    });
  });

  const requests = () =>
    page.evaluate(() => window.__harness.invocationsOf("paginate_timeline_backwards").length);
  const clientHeight = await timeline.evaluate((node) => Math.round(node.clientHeight));
  const resetLedger = () =>
    page.evaluate(() => {
      const probe = (window as unknown as { __requestProbe: RequestProbe }).__requestProbe;
      probe.release.forEach((release) => release());
      probe.release = [];
      probe.requests = [];
      window.__harness.clearInvocations();
    });
  const distanceToBottom = () =>
    page.evaluate(() => {
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
      return container.scrollHeight - container.clientHeight - container.scrollTop;
    });

  await timeline.hover();
  let requested = 0;
  // A room whose first window laid out can already have asked for history, and a
  // page that never comes back keeps the next request blocked. Answering it and
  // replacing the timeline is how this test starts from a state it owns; the second
  // attempt is insurance for that state, never for the distance the reader travelled.
  for (let attempt = 0; attempt < 2 && requested === 0; attempt += 1) {
    if (attempt > 0) {
      await page.evaluate(
        async ({ key, nextItems }) => {
          await window.__harness.pushCoreEvent({
            kind: "Timeline",
            event: {
              InitialItems: { request_id: null, key, generation: 3, items: nextItems }
            }
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
          } as any);
        },
        { key: TIMELINE_KEY, nextItems: seededItems }
      );
      await waitAnimationFrames(page, 8);
    }
    await page.evaluate(() => {
      const probe = (window as unknown as { __requestProbe: RequestProbe }).__requestProbe;
      probe.release.forEach((release) => release());
      probe.release = [];
      probe.requests = [];
      window.__harness.clearInvocations();
    });
    // Up to the resident top, one screen per step. The bound is the content, not a
    // constant: a slower or shorter window simply needs more steps, and a step that
    // runs out would look like a request that never came.
    for (let step = 0; step < 200; step += 1) {
      if ((await timelineScrollTop(page)) <= 0) break;
      if ((await requests()) > 0) break;
      await page.mouse.wheel(0, -clientHeight);
      await waitAnimationFrames(page, 3);
    }
    requested = await requests();
  }
  expect(requested).toBe(1);

  // One at a time: while that page is in flight, more scrolling to the same place
  // must not start another request.
  for (let step = 0; step < 2; step += 1) {
    // eslint-disable-next-line no-await-in-loop
    await page.mouse.wheel(0, -Math.round(clientHeight / 2));
    // eslint-disable-next-line no-await-in-loop
    await waitAnimationFrames(page, 3);
  }
  expect(await requests()).toBe(1);

  // Answer the page, then bring the rows above the sampled one into the window so
  // each of them has been laid out at least once and its real height is recorded.
  await page.evaluate(() => {
    const harness = (window as unknown as { __harness: RequestProbeHarness }).__harness;
    const probe = (window as unknown as { __requestProbe: RequestProbe }).__requestProbe;
    probe.release.forEach((release) => release());
    probe.release = [];
    harness.setCommandResponse("paginate_timeline_backwards", () => harness.currentSnapshot());
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    container.scrollTop = 0;
    container.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await waitAnimationFrames(page, 8);

  const measurement = await page.evaluate(() => {
    const probe = (window as unknown as { __requestProbe: RequestProbe }).__requestProbe;
    const request = probe.requests[0]!;
    const sampledIndex = request.firstVisibleId
      ? Number(/-([0-9]{4}):/.exec(request.firstVisibleId)?.[1] ?? Number.NaN)
      : Number.NaN;
    let trueOffset = 0;
    let unmeasured = 0;
    for (let index = 0; index < sampledIndex; index += 1) {
      const id = `$scroll-anchor-item-${String(index).padStart(4, "0")}:example.invalid`;
      const height = probe.heights.get(id);
      if (height === undefined) unmeasured += 1;
      else trueOffset += height;
    }
    return {
      sampledIndex,
      trueOffset: Math.round(trueOffset),
      unmeasured,
      clientHeight: request.clientHeight,
      scrollTop: request.scrollTop,
      listOffsetTop: request.listOffsetTop
    };
  });

  // The premise of the measurement: the sampled row is not the first row (a
  // request from the top edge proves nothing), and every row above it was really
  // laid out at some point, so their sum is the reader's true distance.
  expect(measurement.sampledIndex).toBeGreaterThan(0);
  expect(measurement.unmeasured).toBe(0);
  expect(measurement.trueOffset).toBeLessThanOrEqual(
    2 * measurement.clientHeight + SCROLL_EDGE_TOLERANCE_PX
  );
});

/**
 * #1890 §2.2: with the window sized in screens, a row that measures shorter than
 * the estimate above the reader has to publish its height in the same frame as
 * the anchor compensation. If it waits for the scroll to settle, the modelled
 * gap between the reader and the spacer grows and the spacer itself can come
 * into view during the wheel.
 */
test("a round trip through history keeps the reader's row and never shows a spacer", async ({
  page
}) => {
  await gotoReadyApp(page);
  await seedTimeline(page, 120);
  const timeline = page.locator("[data-testid=timeline-view]");
  await expect(timeline).toHaveAttribute("data-total-items", "120");
  await waitAnimationFrames(page, 6);

  // The live-edge open puts the last row at the bottom; keep its distance from
  // the container's bottom edge as the reader's position.
  const readerRow = page.locator(
    '[data-event-id="$scroll-anchor-item-0119:example.invalid"]'
  );
  const offsetFromBottom = async (): Promise<number> =>
    page.evaluate(() => {
      const row = document.querySelector<HTMLElement>(
        '[data-event-id="$scroll-anchor-item-0119:example.invalid"]'
      );
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
      if (!row || !container) return Number.POSITIVE_INFINITY;
      return (
        container.getBoundingClientRect().bottom - row.getBoundingClientRect().bottom
      );
    });
  await expect(readerRow).toBeVisible();
  const beforeOffset = await offsetFromBottom();
  expect(Number.isFinite(beforeOffset)).toBe(true);

  // Count every frame that ends with a spacer on screen, from here until the
  // end. The read is registered from the container's own scroll listener: that
  // runs after the app's, so this rAF runs after the one that commits the window
  // and samples the frame as it is painted. A chain started from here runs a
  // frame earlier and would sample the pre-commit DOM.
  await page.evaluate(() => {
    const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const state = { spacerFrames: 0, maxSliverPx: 0, running: true, pending: false };
    (window as unknown as { __spacerWatch: typeof state }).__spacerWatch = state;
    const readFrame = () => {
      state.pending = false;
      const box = container.getBoundingClientRect();
      let sliver = 0;
      for (const spacer of Array.from(
        container.querySelectorAll<HTMLElement>(".timeline-virtual-spacer")
      )) {
        const rect = spacer.getBoundingClientRect();
        // A spacer with no height is the list's own end, not a visible gap.
        if (rect.height <= 0) continue;
        const visible = Math.min(rect.bottom, box.bottom) - Math.max(rect.top, box.top);
        if (visible > 0) sliver = Math.max(sliver, visible);
      }
      // The last read of a frame wins: a spacer that the same frame's commit
      // removes was never painted.
      if (sliver > 0) {
        state.spacerFrames += 1;
        state.maxSliverPx = Math.max(state.maxSliverPx, sliver);
      }
    };
    const scheduleRead = () => {
      if (!state.running || state.pending) return;
      state.pending = true;
      requestAnimationFrame(readFrame);
    };
    container.addEventListener("scroll", scheduleRead, { passive: true });
    (window as unknown as { __spacerWatchStop?: () => void }).__spacerWatchStop = () => {
      state.running = false;
      container.removeEventListener("scroll", scheduleRead);
    };
    scheduleRead();
  });

  const clientHeight = await timeline.evaluate((node) => node.clientHeight);
  await timeline.hover();
  // Up to the top of the loaded history, one screen at a time.
  for (let step = 0; step < 40; step += 1) {
    const scrollTop = await timelineScrollTop(page);
    if (scrollTop <= 0) break;
    await page.mouse.wheel(0, -Math.round(clientHeight));
    await waitAnimationFrames(page, 3);
  }
  // And back down to the live edge.
  for (let step = 0; step < 40; step += 1) {
    await page.mouse.wheel(0, Math.round(clientHeight));
    await waitAnimationFrames(page, 3);
    const distanceToBottom = await page.evaluate(() => {
      const container = document.querySelector<HTMLElement>("[data-testid=timeline-view]");
      if (!container) return 0;
      return container.scrollHeight - container.clientHeight - container.scrollTop;
    });
    if (distanceToBottom <= 1) break;
  }
  await waitAnimationFrames(page, 4);

  const afterOffset = await offsetFromBottom();
  expect(Math.abs(afterOffset - beforeOffset)).toBeLessThanOrEqual(1);
  const spacerWatch = await page.evaluate(() => {
    const stop = (window as unknown as { __spacerWatchStop?: () => void }).__spacerWatchStop;
    stop?.();
    const state = (window as unknown as { __spacerWatch: { spacerFrames: number } })
      .__spacerWatch;
    return state.spacerFrames;
  });
  expect(spacerWatch).toBe(0);
});
