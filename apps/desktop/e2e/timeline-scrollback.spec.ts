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

import { expect, test } from "@playwright/test";

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
    timestamp_ms: 1_800_000_000_000
  };
}

test("scrollback prepend keeps the anchor item visually stable and gates auto-backfill", async ({
  page
}) => {
  await page.goto("/harness.html");
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

  await container.evaluate((node) => {
    node.scrollTop = 0;
  });
  // Give the scroll handler a beat, then assert the count did not move.
  await page.waitForTimeout(250);
  const countAfterEndReached = await page.evaluate(
    () => window.__harness.invocationsOf("paginate_timeline_backwards").length
  );
  expect(countAfterEndReached).toBe(1);
});
