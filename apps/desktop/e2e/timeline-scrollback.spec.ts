/**
 * Headless-Chromium DOM tier of the headless UI test layer (QA Model
 * layer 4): proves the REAL-DOM scrollback contract that the Vitest logic
 * tier cannot — actual scroll positions in a real layout engine.
 *
 * Scenario (per the Phase 7 exit review):
 *  (a) InitialItems fills the viewport,
 *  (b) the user scrolls near the top (triggering one auto-backfill request),
 *  (c) a prepend ItemsUpdated batch arrives via the mock event stream,
 *  (d) the previously-visible anchor item's viewport position is restored
 *      within a bounded pixel delta,
 *  (e) no further auto-backfill request is recorded until restoration
 *      completed; EndReached then stops further auto-pagination entirely.
 *
 * The harness page (harness.html) mounts the real TimelineView component
 * against the mock IPC transport; no Tauri process, no native window. The
 * Vite server is owned (started/stopped) by Playwright on port 5183.
 */

import { expect, test, type Page } from "@playwright/test";

const ROOM_ID = "!harness-room:example.invalid";
const ACCOUNT_KEY = "@harness-user:example.invalid";
const ITEM_HEIGHT_PX = 48; // pinned by harness.html CSS
const ANCHOR_PIXEL_TOLERANCE = 2;

function timelineKey() {
  return { account_key: ACCOUNT_KEY, kind: { Room: { room_id: ROOM_ID } } };
}

function makeItem(id: string, body: string) {
  return {
    id: { Event: { event_id: id } },
    sender: "@sender:example.invalid",
    body,
    timestamp_ms: 1_800_000_000_000,
    in_reply_to_event_id: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: []
  };
}

async function pushInitialTimelineItems(page: Page, count: number) {
  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: count }, (_, i) =>
        makeItem(`$m${String(i).padStart(2, "0")}`, `message ${i}`)
      )
    }
  );
}

async function expectTimelineScrolledToBottom(container: ReturnType<Page["locator"]>) {
  await expect
    .poll(async () =>
      container.evaluate((node) =>
        Math.abs(node.scrollHeight - node.clientHeight - node.scrollTop)
      )
    )
    .toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
}

async function waitAnimationFrames(page: Page, count: number) {
  for (let i = 0; i < count; i += 1) {
    await page.evaluate(
      () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
    );
  }
}

function makeVariableHeightItem(index: number) {
  const body =
    index >= 880
      ? Array.from({ length: 36 }, (_, line) => `message ${index} line ${line}`).join("\n")
      : `message ${index}`;
  return makeItem(`$vh${String(index).padStart(3, "0")}`, body);
}

function makeTallItem(index: number) {
  // 10 lines makes each row taller than the 72px default estimate, so the
  // initial scroll-to-bottom based on estimated heights lands above the real
  // live edge before measurement settles.
  const body = Array.from({ length: 10 }, (_, line) => `tall ${index} line ${line}`).join("\n");
  return makeItem(`$tall${String(index).padStart(3, "0")}`, body);
}

test("initial timeline load and remount start at the live edge", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");

  await pushInitialTimelineItems(page, 30);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(30);
  await expectTimelineScrolledToBottom(container);

  await page.reload();
  await page.waitForSelector("[data-testid=timeline-view]");

  await pushInitialTimelineItems(page, 30);

  const remountedContainer = page.locator("[data-testid=timeline-view]");
  await expect(remountedContainer.locator("[data-item-id]")).toHaveCount(30);
  await expectTimelineScrolledToBottom(remountedContainer);
});

test("initial short load grows to the live edge on later PushBack", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 5 }, (_, i) =>
        makeItem(`$short${String(i).padStart(2, "0")}`, `short ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(5);

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: items.map((item) => ({ PushBack: { item } }))
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 50 }, (_, i) =>
        makeItem(`$grow${String(i).padStart(2, "0")}`, `grow ${i}`)
      )
    }
  );

  await expect(container.locator("[data-item-id]")).toHaveCount(55);
  await expectTimelineScrolledToBottom(container);
});

test("short initial load stays stable when the user scrolls up slightly", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 5 }, (_, i) =>
        makeItem(`$short${String(i).padStart(2, "0")}`, `short ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(5);

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: items.map((item) => ({ PushBack: { item } }))
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 50 }, (_, i) =>
        makeItem(`$grow${String(i).padStart(2, "0")}`, `grow ${i}`)
      )
    }
  );

  await expect(container.locator("[data-item-id]")).toHaveCount(55);
  await expectTimelineScrolledToBottom(container);

  const maxScrollTop = await container.evaluate(
    (node) => node.scrollHeight - node.clientHeight
  );
  const targetScrollTop = Math.max(0, maxScrollTop - 100);

  // Simulate a small user scroll up from the live edge.
  await container.evaluate((node, target) => {
    node.scrollTop = target;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  }, targetScrollTop);

  await page.waitForTimeout(500);

  const finalScrollTop = await container.evaluate((node) => node.scrollTop);
  // If the component snaps back to the bottom or jumps to the top quarter,
  // this assertion fails.
  expect(Math.abs(finalScrollTop - targetScrollTop)).toBeLessThanOrEqual(10);
});

test("short variable-height load stays stable when the user scrolls up slightly", async ({ page }) => {
  await page.goto("/harness.html?variableHeights=true");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 5 }, (_, i) =>
        makeItem(`$short${String(i).padStart(2, "0")}`, `short ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(5);

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: items.map((item) => ({ PushBack: { item } }))
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 800 }, (_, i) => makeVariableHeightItem(i))
    }
  );

  await expect(container).toHaveAttribute("data-virtualized", "true");
  await expectTimelineScrolledToBottom(container);

  // The live edge must actually be rendered, not just scrolled to an empty
  // padding area with stale viewport metrics.
  await expect(page.locator('[data-item-id="$vh799"]')).toBeVisible();

  const maxScrollTop = await container.evaluate(
    (node) => node.scrollHeight - node.clientHeight
  );
  const targetScrollTop = Math.max(0, maxScrollTop - 100);

  await container.evaluate((node, target) => {
    node.scrollTop = target;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  }, targetScrollTop);

  await page.waitForTimeout(500);

  const finalScrollTop = await container.evaluate((node) => node.scrollTop);
  expect(Math.abs(finalScrollTop - targetScrollTop)).toBeLessThanOrEqual(10);
});

test("tall variable-height initial load snaps to the real live edge after measurement", async ({ page }) => {
  await page.goto("/harness.html?variableHeights=true");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 5 }, (_, i) =>
        makeItem(`$short${String(i).padStart(2, "0")}`, `short ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(5);

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: items.map((item) => ({ PushBack: { item } }))
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 800 }, (_, i) => makeTallItem(i))
    }
  );

  await expect(container).toHaveAttribute("data-virtualized", "true");
  await expectTimelineScrolledToBottom(container);

  // The real live edge must be rendered. Before the fix, the viewport landed
  // in the middle of the list because estimated heights were too low.
  await expect(page.locator('[data-item-id="$tall799"]')).toBeVisible();
});

test("auto-backfill after short non-virtualized growth keeps the viewport stable", async ({ page }) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 5 }, (_, i) =>
        makeItem(`$short${String(i).padStart(2, "0")}`, `short ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(5);

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: items.map((item) => ({ PushBack: { item } }))
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 100 }, (_, i) =>
        makeItem(`$grow${String(i).padStart(2, "0")}`, `grow ${i}`)
      )
    }
  );

  await expect(container.locator("[data-item-id]")).toHaveCount(105);
  await expectTimelineScrolledToBottom(container);

  const maxScrollTopBefore = await container.evaluate(
    (node) => node.scrollHeight - node.clientHeight
  );

  // A small scroll up from the live edge in a short list should NOT trigger
  // auto-backfill; this was causing a jarring viewport jump when older
  // messages arrived immediately after a tiny scroll gesture.
  const slightScrollTop = Math.max(0, maxScrollTopBefore - 50);
  await container.evaluate((node, target) => {
    node.scrollTop = target;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  }, slightScrollTop);
  await page.waitForTimeout(200);
  const backfillAfterSlightScroll = await page.evaluate(
    () => window.__harness.invocationsOf("paginate_timeline_backwards").length
  );
  expect(backfillAfterSlightScroll).toBe(0);

  // Scrolling near the top of the short list still triggers the intended
  // prefetch behavior.
  await container.evaluate((node) => {
    node.scrollTop = 40;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("paginate_timeline_backwards").length
      )
    )
    .toBeGreaterThanOrEqual(1);

  // Simulate core responding with older messages.
  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 2,
            diffs: items.map((item) => ({ PushFront: { item } }))
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 50 }, (_, i) =>
        makeItem(`$older${String(i).padStart(2, "0")}`, `older ${i}`)
      )
    }
  );

  await expect(container.locator("[data-item-id]")).toHaveCount(155);
});

test("short timeline does not auto-backfill on a small scroll up from the live edge", async ({ page }) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");

  // 50 items × 48px = 2400px total height; the desired 100-item prefetch
  // window (7200px) is larger than the whole list. A small scroll up from
  // the live edge must not immediately request older messages, otherwise the
  // prepend + anchor restoration causes a jarring viewport jump.
  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 50 }, (_, i) =>
        makeItem(`$short${String(i).padStart(2, "0")}`, `short ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(50);
  await expectTimelineScrolledToBottom(container);

  const maxScrollTop = await container.evaluate(
    (node) => node.scrollHeight - node.clientHeight
  );
  const slightScrollTop = Math.max(0, maxScrollTop - 120);

  await container.evaluate((node, target) => {
    node.scrollTop = target;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  }, slightScrollTop);

  await page.waitForTimeout(200);

  const backfillCount = await page.evaluate(
    () => window.__harness.invocationsOf("paginate_timeline_backwards").length
  );
  expect(backfillCount).toBe(0);

  // Scrolling near the top of the short list still triggers backfill.
  await container.evaluate((node) => {
    node.scrollTop = 40;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect
    .poll(() =>
      page.evaluate(
        () => window.__harness.invocationsOf("paginate_timeline_backwards").length
      )
    )
    .toBeGreaterThanOrEqual(1);
});

test("variable-height initial load starts at the live edge", async ({ page }) => {
  await page.goto("/harness.html?variableHeights=true");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 1_000 }, (_, index) => makeVariableHeightItem(index))
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-virtualized", "true");
  await expectTimelineScrolledToBottom(container);
});

test("virtualized jump remains stable after variable-height rows are measured", async ({
  page
}) => {
  await page.goto("/harness.html?variableHeights=true");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 1_000 }, (_, index) => makeVariableHeightItem(index))
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-virtualized", "true");
  await container.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator('[data-item-id="$vh999"]')).toBeVisible();

  await page.evaluate(
    ({ key }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key,
            snapshot: {
              read_marker_event_id: "$vh099",
              read_marker_display_event_id: "$vh099",
              first_unread_event_id: "$vh100",
              unread_event_count: 1,
              unread_position: "aboveViewport",
              newer_event_count: 0,
              can_jump_to_bottom: false
            }
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: timelineKey() }
  );

  await page.getByRole("button", { name: "Jump to first unread, 1 unread" }).click();
  await waitAnimationFrames(page, 4);

  const targetOffset = await container.evaluate((node) => {
    const target = node.querySelector<HTMLElement>('[data-item-id="$vh100"]');
    if (!target) {
      return null;
    }
    const containerRect = node.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    return Math.abs(
      targetRect.top + targetRect.height / 2 - (containerRect.top + node.clientHeight / 2)
    );
  });
  expect(targetOffset).not.toBeNull();
  expect(targetOffset!).toBeLessThanOrEqual(96);
});

test("scrollback prepend keeps the anchor item visually stable and gates auto-backfill", async ({
  page
}) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");

  // ---- (a) InitialItems filling the viewport (30 × 48px ≫ 400px) ----
  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 30 }, (_, i) =>
        makeItem(`$m${String(i).padStart(2, "0")}`, `message ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(30);

  // The list must overflow the 400px viewport.
  const overflowing = await container.evaluate(
    (node) => node.scrollHeight > node.clientHeight
  );
  expect(overflowing).toBe(true);

  // Start at the bottom (live edge), like a real session.
  await container.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
  });

  // ---- (b) scroll near the top → exactly one auto-backfill request ----
  await container.evaluate((node) => {
    node.scrollTop = 50; // below the 80px threshold
  });
  await expect
    .poll(async () =>
      page.evaluate(
        () => window.__harness.invocationsOf("paginate_timeline_backwards").length
      )
    )
    .toBe(1);

  // Core acknowledges: Backward pagination is now running (spinner shows).
  await page.evaluate(
    ({ key }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key,
            direction: "Backward",
            state: "Paginating"
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: timelineKey() }
  );
  await expect(page.locator("[data-testid=timeline-spinner]")).toBeVisible();

  // Capture the anchor: first visible item id + pixel offset from the
  // container top (same definition the component uses).
  const anchorBefore = await container.evaluate((node) => {
    const containerTop = node.getBoundingClientRect().top;
    for (const child of node.querySelectorAll<HTMLElement>("[data-item-id]")) {
      const rect = child.getBoundingClientRect();
      if (rect.bottom > containerTop) {
        return {
          itemId: child.dataset["itemId"] ?? "",
          offsetTop: rect.top - containerTop
        };
      }
    }
    return null;
  });
  expect(anchorBefore).not.toBeNull();

  // ---- (c) prepend batch arrives via the mock event stream ----
  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: items.map((item) => ({ PushFront: { item } }))
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 10 }, (_, i) =>
        makeItem(`$old${String(i).padStart(2, "0")}`, `older message ${i}`)
      )
    }
  );
  await expect(container.locator("[data-item-id]")).toHaveCount(40);

  // ---- (d) anchor item's viewport position restored (bounded delta) ----
  const anchorAfter = await container.evaluate(
    (node, anchorId) => {
      const containerTop = node.getBoundingClientRect().top;
      const child = node.querySelector<HTMLElement>(
        `[data-item-id="${anchorId}"]`
      );
      if (!child) return null;
      return child.getBoundingClientRect().top - containerTop;
    },
    anchorBefore!.itemId
  );
  expect(anchorAfter).not.toBeNull();
  expect(Math.abs(anchorAfter! - anchorBefore!.offsetTop)).toBeLessThanOrEqual(
    ANCHOR_PIXEL_TOLERANCE
  );

  // Restoration also means scrollTop moved DOWN by the prepended height
  // (10 × 48px), i.e. the viewport did not jump to the new items.
  const scrollTopAfter = await container.evaluate((node) => node.scrollTop);
  expect(scrollTopAfter).toBeGreaterThanOrEqual(
    50 + 10 * ITEM_HEIGHT_PX - ANCHOR_PIXEL_TOLERANCE
  );

  // ---- (e) no further auto-backfill until restoration completed ----
  // The prepend + restoration must not have fired a second request: the
  // count is still exactly 1 (pagination was Paginating during apply, and
  // the restored scrollTop is far from the top edge).
  const countAfterPrepend = await page.evaluate(
    () => window.__harness.invocationsOf("paginate_timeline_backwards").length
  );
  expect(countAfterPrepend).toBe(1);

  // Core reports EndReached: scrolling to the very top must NOT trigger
  // another auto-backfill request.
  await page.evaluate(
    ({ key }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          PaginationStateChanged: {
            request_id: null,
            key,
            direction: "Backward",
            state: "EndReached"
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: timelineKey() }
  );
  await expect(page.locator("[data-testid=timeline-start]")).toBeVisible();

  // Capture the timeline generation BEFORE the next store mutation, so the
  // poll below has a concrete value to wait past instead of a fixed sleep.
  const previousGeneration = await page
    .locator("[data-timeline-generation]")
    .first()
    .getAttribute("data-timeline-generation");

  // Scroll to the very top under EndReached: the synchronous onScroll handler
  // must NOT dispatch another auto-backfill (Backward state is EndReached).
  await container.evaluate((node) => {
    node.scrollTop = 0;
  });

  // Drive a concrete, observable store mutation (a new generation via
  // InitialItems) and poll the generation attribute until it changes. This
  // replaces a fixed wait: once the new generation is rendered, the earlier
  // scroll event has already been processed. InitialItems preserves the
  // Backward=EndReached state, so auto-backfill stays suppressed.
  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 2, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 40 }, (_, i) =>
        makeItem(`$gen2-${String(i).padStart(2, "0")}`, `gen2 message ${i}`)
      )
    }
  );
  await expect
    .poll(() =>
      page
        .locator("[data-timeline-generation]")
        .first()
        .getAttribute("data-timeline-generation")
    )
    .not.toBe(previousGeneration);

  const countAfterEndReached = await page.evaluate(
    () => window.__harness.invocationsOf("paginate_timeline_backwards").length
  );
  expect(countAfterEndReached).toBe(1);
});

test("timeline does not auto-backfill unless the setting enables it", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 30);

  const container = page.locator("[data-testid=timeline-view]");
  await container.evaluate((node) => {
    node.scrollTop = 40;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });

  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.invocationsOf("paginate_timeline_backwards").length)
    )
    .toBe(0);
});

test("timeline prefetches older messages one batch before the top edge", async ({ page }) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 180);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-total-items", "180");

  // 3,500px is far above the old 80px top-edge trigger, but inside one
  // 100-item batch in the deterministic harness (100 * 48px).
  await container.evaluate((node) => {
    node.scrollTop = 3_500;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });

  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.invocationsOf("paginate_timeline_backwards").length)
    )
    .toBe(1);
});

test("large timelines keep only the viewport window in the DOM", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 1_000);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-virtualized", "true");
  await expect(container).toHaveAttribute("data-total-items", "1000");
  await expect
    .poll(() => container.locator("[data-item-id]").count())
    .toBeLessThan(220);

  await container.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(page.locator('[data-item-id="$m999"]')).toBeVisible();
  await expect
    .poll(() => container.locator("[data-item-id]").count())
    .toBeLessThan(220);
});

test("active scroll inside mounted overscan does not recompose the virtual window", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 1_000);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-virtualized", "true");

  await container.evaluate((node) => {
    node.scrollTop = 20_000;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await waitAnimationFrames(page, 3);

  await page.evaluate(() => window.__harness.resetScrollDiagnostics());

  await container.evaluate((node) => {
    for (let index = 0; index < 12; index += 1) {
      node.scrollTop += 4;
      node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 4 }));
      node.dispatchEvent(new Event("scroll", { bubbles: true }));
    }
  });
  await waitAnimationFrames(page, 5);

  const diagnostics = await page.evaluate(() => window.__harness.scrollDiagnostics());
  expect(diagnostics).not.toBeNull();
  expect(diagnostics?.scrollFrames ?? 0).toBeGreaterThanOrEqual(1);
  expect(diagnostics?.latestFrame?.userInputPending).toBe(true);
  expect(diagnostics?.rangeCommits ?? 0).toBe(0);
  expect(diagnostics?.heightModelCommits ?? 0).toBe(0);
  expect(diagnostics?.renderCommits ?? 0).toBeLessThanOrEqual(1);
});

test("timeline keeps SDK diff order and ignores duplicate update batches", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: [
        { ...makeItem("$jun17", "Jun 17"), timestamp_ms: 1_797_460_000_000 },
        { ...makeItem("$jun20", "Jun 20"), timestamp_ms: 1_797_720_000_000 }
      ]
    }
  );

  await page.evaluate(
    ({ key, item }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: [{ PushBack: { item } }]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      item: { ...makeItem("$jun13", "Jun 13"), timestamp_ms: 1_797_120_000_000 }
    }
  );

  await expect(page.locator("[data-item-id]")).toHaveCount(3);
  await expect(page.locator("[data-item-id]").nth(0)).toHaveAttribute("data-item-id", "$jun17");
  await expect(page.locator("[data-item-id]").nth(1)).toHaveAttribute("data-item-id", "$jun20");
  await expect(page.locator("[data-item-id]").nth(2)).toHaveAttribute("data-item-id", "$jun13");

  await page.evaluate(
    ({ key, items }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 2, items }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: [makeItem("$a", "a"), makeItem("$b", "b"), makeItem("$c", "c")]
    }
  );
  await expect(page.locator("[data-timeline-generation]")).toHaveAttribute(
    "data-timeline-generation",
    "2"
  );

  const repeatedRemoveBatch = {
    kind: "Timeline",
    event: {
      ItemsUpdated: {
        key: timelineKey(),
        generation: 2,
        batch_id: 2,
        diffs: [{ Remove: { index: 1 } }]
      }
    }
  };
  await page.evaluate((payload) => {
    window.__harness.pushCoreEvent(payload as any);
    window.__harness.pushCoreEvent(payload as any);
  }, repeatedRemoveBatch);

  await expect(page.locator("[data-item-id]")).toHaveCount(2);
  await expect(page.locator("[data-item-id]").nth(0)).toHaveAttribute("data-item-id", "$a");
  await expect(page.locator("[data-item-id]").nth(1)).toHaveAttribute("data-item-id", "$c");
});

test("timeline navigation renders Rust-owned unread controls and sends viewport facts", async ({
  page
}) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 30);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(30);

  await container.evaluate((node) => {
    node.scrollTop = 0;
  });

  await expect
    .poll(
      () =>
        page.evaluate(() =>
          window.__harness.invocationsOf("observe_timeline_viewport").at(-1)?.args
        ),
      { timeout: 1_000 }
    )
    .toMatchObject({
      roomId: ROOM_ID,
      firstVisibleEventId: "$m00",
      atBottom: false
    });

  await page.evaluate(
    ({ key }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          NavigationUpdated: {
            key,
            snapshot: {
              read_marker_event_id: "$m24",
              read_marker_display_event_id: "$m24",
              first_unread_event_id: "$m25",
              unread_event_count: 3,
              unread_position: "belowViewport",
              newer_event_count: 5,
              can_jump_to_bottom: true
            }
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: timelineKey() }
  );

  await expect(page.getByRole("separator", { name: "Unread messages" })).toBeVisible();

  await page.getByRole("button", { name: "Jump to first unread, 3 unread" }).click();
  await expect(page.locator('[data-item-id="$m25"]')).toBeInViewport();

  await page.getByRole("button", { name: "Jump to bottom, 5 new messages" }).click();
  await expect
    .poll(() =>
      container.evaluate((node) =>
        Math.abs(node.scrollHeight - node.clientHeight - node.scrollTop)
      )
    )
    .toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
});

test("scrolling to bottom marks the latest readable event", async ({ page }) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 40);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(40);

  await container.evaluate((node) => {
    node.scrollTop = 0;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__harness.invocationsOf("observe_timeline_viewport").at(-1)?.args
      )
    )
    .toMatchObject({ atBottom: false });

  await container.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });

  await expect
    .poll(() =>
      page.evaluate(() => window.__harness.invocationsOf("set_fully_read").at(-1)?.args)
    )
    .toEqual({
      roomId: ROOM_ID,
      eventId: "$m39"
    });
});

test("timeline jump-to-date dispatches Rust timestamp resolution", async ({ page }) => {
  await page.goto("/appHarness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 8);

  await page.getByRole("button", { name: "Jump to date" }).click();
  await expect(page.getByRole("dialog", { name: "Jump to date" })).toBeVisible();
  await page.getByRole("textbox", { name: "Jump to date" }).fill("2026-06-16T12:34");
  await page.getByRole("button", { name: "Open date in timeline" }).click();

  const expectedTimestamp = await page.evaluate(() =>
    new Date("2026-06-16T12:34").getTime()
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__harness.invocationsOf("open_timeline_at_timestamp")[0]?.args
      )
    )
    .toEqual({
      roomId: ROOM_ID,
      timestampMs: expectedTimestamp
    });
});

test("timeline jump-to-date reads the submitted input value", async ({ page }) => {
  await page.goto("/appHarness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 8);

  await page.getByRole("button", { name: "Jump to date" }).click();
  await expect(page.getByRole("dialog", { name: "Jump to date" })).toBeVisible();
  await page.getByRole("textbox", { name: "Jump to date" }).evaluate((node) => {
    (node as HTMLInputElement).value = "2026-06-16T12:35";
  });
  await page.getByRole("button", { name: "Open date in timeline" }).click();

  const expectedTimestamp = await page.evaluate(() =>
    new Date("2026-06-16T12:35").getTime()
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__harness.invocationsOf("open_timeline_at_timestamp")[0]?.args
      )
    )
    .toEqual({
      roomId: ROOM_ID,
      timestampMs: expectedTimestamp
    });
});
