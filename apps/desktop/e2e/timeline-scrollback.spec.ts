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
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOM_ID = "!harness-room:example.invalid";
const OTHER_ROOM_ID = "!other-room:example.invalid";
const ACCOUNT_KEY = "@harness-user:example.invalid";
const ITEM_HEIGHT_PX = 48; // pinned by harness.html CSS
const ANCHOR_PIXEL_TOLERANCE = 2;
const DESKTOP_STYLES_PATH = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../src/styles.css"
);

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
    thread_root: null,
    thread_summary: null,
    can_react: false,
    is_redacted: false,
    is_hidden: false,
    can_redact: false,
    is_edited: false,
    can_edit: false,
    reactions: []
  };
}

function makeImageItem(id: string) {
  return {
    ...makeItem(id, "image body"),
    media: {
      kind: "Image",
      filename: "synthetic.png",
      source: {
        mxc_uri: "mxc://example.invalid/synthetic",
        encrypted: false,
        encryption_version: null
      },
      mimetype: "image/png",
      size: 12345,
      width: 2048,
      height: 1188,
      thumbnail: null
    }
  };
}

function makeImageItemWithoutDimensions(id: string) {
  return {
    ...makeImageItem(id),
    media: {
      ...makeImageItem(id).media,
      width: null,
      height: null
    }
  };
}

function makePendingLinkPreviewItem(id: string) {
  return {
    ...makeItem(id, "See https://example.invalid/preview"),
    link_previews: [
      {
        url: "https://example.invalid/preview",
        state: "pending"
      }
    ]
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
      () =>
        new Promise<void>((resolve) => {
          let resolved = false;
          const finish = () => {
            if (resolved) {
              return;
            }
            resolved = true;
            resolve();
          };
          requestAnimationFrame(finish);
          setTimeout(finish, 16);
        })
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

test("stale session anchor cannot win when a large prepend lands during room re-entry", async ({
  page
}) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");

  const staleAnchorEventId = "$stale-anchor";
  await page.evaluate(
    ({ roomId, otherRoomId, anchorEventId }) => {
      window.__harness.seedTimelineViewportSessionAnchor(roomId, {
        event_id: anchorEventId,
        edge: "bottom",
        offset_px: 0,
        updated_at_ms: Date.now() - 10 * 60 * 1_000
      });
      window.__harness.showRoom(otherRoomId);
    },
    { roomId: ROOM_ID, otherRoomId: OTHER_ROOM_ID, anchorEventId: staleAnchorEventId }
  );
  await waitAnimationFrames(page, 2);
  await page.evaluate((roomId) => {
    window.__harness.showRoom(roomId);
    window.__harness.clearDiagnosticLogs();
  }, ROOM_ID);
  await waitAnimationFrames(page, 2);
  await page.evaluate(
    ({ roomId, anchorEventId }) => {
      const key = {
        account_key: "@harness-user:example.invalid",
        kind: { Room: { room_id: roomId } }
      };
      const makeHarnessItem = (eventId: string, body: string) => ({
        id: { Event: { event_id: eventId } },
        sender: "@sender:example.invalid",
        body,
        timestamp_ms: 1_800_000_000_000,
        in_reply_to_event_id: null,
        thread_root: null,
        thread_summary: null,
        can_react: false,
        is_redacted: false,
        is_hidden: false,
        can_redact: false,
        is_edited: false,
        can_edit: false,
        reactions: []
      });
      const liveItems = Array.from({ length: 40 }, (_, index) =>
        makeHarnessItem(`$reentry-live-${index}`, `live ${index}`)
      );
      const historicalItems = Array.from({ length: 64 }, (_, index) =>
        makeHarnessItem(
          index === 0 ? anchorEventId : `$reentry-old-${index}`,
          `historical ${index}`
        )
      );

      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          InitialItems: { request_id: null, key, generation: 1, items: liveItems }
        }
      } as any);
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: historicalItems.map((item) => ({ PushFront: { item } }))
          }
        }
      } as any);
    },
    { roomId: ROOM_ID, anchorEventId: staleAnchorEventId }
  );

  const container = page.locator("[data-testid=timeline-view]");
  await expect.poll(() => container.locator("[data-item-id]").count()).toBeGreaterThan(0);
  await expectTimelineScrolledToBottom(container);
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__harness
          .diagnosticLogs()
          .filter(
            (entry) =>
              entry.source === "timeline.scroll" &&
              entry.message.includes("stage=room_reentry_restore")
          )
      )
    )
    .toEqual([
      {
        source: "timeline.scroll",
        message:
          "stage=room_reentry_restore session_mode=anchor anchor_age=stale " +
          "anchor_is_live=false path=cleared_to_live_edge",
        timestampMs: expect.any(Number)
      }
    ]);
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
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -100 }));
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
  await expect(page.getByTestId("timeline-view")).toHaveCSS("overflow-anchor", "none");

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

  const anchorBefore = await container.evaluate((node, target) => {
    node.scrollTop = target;
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -100 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
    const top = node.getBoundingClientRect().top;
    for (const row of node.querySelectorAll<HTMLElement>("[data-item-id]")) {
      if (row.getBoundingClientRect().bottom > top) {
        return {
          itemId: row.dataset["itemId"] ?? "",
          offsetTop: row.getBoundingClientRect().top - top
        };
      }
    }
    return null;
  }, targetScrollTop);
  expect(anchorBefore).not.toBeNull();

  await page.waitForTimeout(500);

  await expect
    .poll(async () => {
      const offset = await container.evaluate((node, anchorId) => {
        const row = node.querySelector<HTMLElement>(`[data-item-id="${anchorId}"]`);
        return row
          ? row.getBoundingClientRect().top - node.getBoundingClientRect().top
          : null;
      }, anchorBefore!.itemId);
      return offset === null ? Number.POSITIVE_INFINITY : Math.abs(offset - anchorBefore!.offsetTop);
    })
    .toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
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

test("auto-backfill keeps a slight live-edge scroll stable before near-top prefetch", async ({ page }) => {
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
      items: Array.from({ length: 105 }, (_, i) =>
        makeItem(`$grow${String(i).padStart(3, "0")}`, `grow ${i}`)
      )
    }
  );

  const container = page.locator("[data-testid=timeline-view]");
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

test("virtualized navigation hides the first-unread control after variable-height rows are measured", async ({
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
              local_viewed_event_id: "$vh099",
              server_confirmed_read_event_id: "$vh099",
              read_state_sync: "synced",
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

  await expect(page.getByRole("button", { name: "Jump to first unread, 1 unread" })).toHaveCount(0);
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
  // Simulate a genuine user scroll-up: a bare scrollTop assignment is treated
  // as a programmatic write and leaves the viewport intent at live-edge, so
  // the component snaps back to the bottom and the anchor restoration never
  // engages (the assertion below then races that snap). A wheel event marks
  // the scroll as user-driven, switching the component to free-scroll exactly
  // as a real trackpad/scrollbar drag would.
  await container.evaluate((node) => {
    node.scrollTop = 50; // below the 80px threshold
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -50 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
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

test("timeline prefetches older messages within two viewport heights", async ({ page }) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 180);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-total-items", "180");

  await container.evaluate((node) => {
    node.scrollTop = Math.round(node.clientHeight * 1.5);
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

test("known-dimension media keeps row height stable across download completion", async ({
  page
}) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await page.addStyleTag({
    path: DESKTOP_STYLES_PATH
  });
  await waitAnimationFrames(page, 1);

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
      items: Array.from({ length: 100 }, (_, index) =>
        index === 70 ? makeImageItem("$image70") : makeItem(`$m${index}`, `message ${index}`)
      )
    }
  );

  const frame = page.locator('[data-frame-item-id="$image70"]');
  await expect(frame).toBeVisible();
  const beforeHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);

  await page.evaluate(
    ({ key }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          MediaDownloadCompleted: {
            request_id: "media-request",
            key,
            event_id: "$image70",
            source_url: "appmedia://synthetic-image",
            byte_count: 12345,
            mimetype: "image/png",
            width: 2048,
            height: 1188
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: timelineKey() }
  );

  await page.evaluate(() =>
    window.__harness.setMediaDownloadState("$image70", {
      kind: "ready",
      source_url: "appmedia://synthetic-image",
      width: 2048,
      height: 1188,
      mime_type: "image/png"
    })
  );

  await waitAnimationFrames(page, 3);
  const media = frame.locator(".message-media");
  await expect(media).toHaveAttribute("data-download-state", "ready");
  // #163: image-first layout — the image is the open link; the download is a
  // hover/focus overlay action.
  await expect(media.locator(".message-media-open")).toBeVisible();
  await expect(media.locator(".message-media-hover-action").first()).toBeVisible();
  await expect
    .poll(() => media.evaluate((node) => getComputedStyle(node).display))
    .toBe("grid");
  await expect
    .poll(() =>
      media.evaluate((node) => getComputedStyle(node).gridTemplateColumns.split(" ").length)
    )
    .toBe(1);
  const afterHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);
  expect(Math.abs(afterHeight - beforeHeight)).toBeLessThanOrEqual(1);
});

test("missing-dimension media keeps row height stable across download completion", async ({
  page
}) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await page.addStyleTag({
    path: DESKTOP_STYLES_PATH
  });
  await waitAnimationFrames(page, 1);

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
      items: Array.from({ length: 100 }, (_, index) =>
        index === 70
          ? makeImageItemWithoutDimensions("$image-missing70")
          : makeItem(`$m${index}`, `message ${index}`)
      )
    }
  );

  const frame = page.locator('[data-frame-item-id="$image-missing70"]');
  await expect(frame).toBeVisible();
  const beforeHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);

  await page.evaluate(
    ({ key }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          MediaDownloadCompleted: {
            request_id: "media-missing-request",
            key,
            event_id: "$image-missing70",
            source_url: "appmedia://synthetic-image-missing",
            byte_count: 12345,
            mimetype: "image/png",
            width: 2048,
            height: 1188
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    { key: timelineKey() }
  );
  await page.evaluate(() =>
    window.__harness.setMediaDownloadState("$image-missing70", {
      kind: "ready",
      source_url: "appmedia://synthetic-image-missing",
      width: 2048,
      height: 1188,
      mime_type: "image/png"
    })
  );

  await waitAnimationFrames(page, 3);
  await expect(frame.locator(".message-media")).toHaveAttribute("data-download-state", "ready");
  const afterHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);
  expect(Math.abs(afterHeight - beforeHeight)).toBeLessThanOrEqual(
    ANCHOR_PIXEL_TOLERANCE
  );
});

test("pending link previews reserve the ready card height", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await page.addStyleTag({
    path: DESKTOP_STYLES_PATH
  });
  await waitAnimationFrames(page, 1);

  const pendingItem = makePendingLinkPreviewItem("$preview70");
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
      items: Array.from({ length: 100 }, (_, index) =>
        index === 70 ? pendingItem : makeItem(`$m${index}`, `message ${index}`)
      )
    }
  );

  const frame = page.locator('[data-frame-item-id="$preview70"]');
  const card = frame.locator(".link-preview-card");
  await expect(card).toHaveAttribute("data-link-preview-state", "pending");
  const beforeHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);
  const cardBeforeHeight = await card.evaluate((node) => node.getBoundingClientRect().height);

  await page.evaluate(
    ({ key, readyItem }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: [{ Set: { index: 70, item: readyItem } }]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      readyItem: {
        ...pendingItem,
        link_previews: [
          {
            url: "https://example.invalid/preview",
            title: "Reserved preview title",
            description: "Two lines of synthetic preview text must stay inside the reserved card.",
            state: "ready"
          }
        ]
      }
    }
  );

  await expect(card).toHaveAttribute("data-link-preview-state", "ready");
  const afterHeight = await frame.evaluate((node) => node.getBoundingClientRect().height);
  const cardAfterHeight = await card.evaluate((node) => node.getBoundingClientRect().height);
  expect(Math.abs(afterHeight - beforeHeight)).toBeLessThanOrEqual(
    ANCHOR_PIXEL_TOLERANCE
  );
  expect(Math.abs(cardAfterHeight - cardBeforeHeight)).toBeLessThanOrEqual(1);
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
  // The 20000px setup jump triggers virtual-range recomposition frames. Wait
  // until those frames have fully settled (frame count stops changing across
  // two reads) before resetting diagnostics, so leftover setup frames cannot
  // leak into the active-scroll measurement window below. A fixed frame wait
  // is flaky under load: headless Chromium frame scheduling drifts and the
  // setup recomposition can spill past it.
  let __settleFrames = -1;
  await expect
    .poll(async () => {
      const current = await page.evaluate(
        () => window.__harness.scrollDiagnostics()?.scrollFrames ?? 0
      );
      const settled = current === __settleFrames;
      __settleFrames = current;
      return settled;
    })
    .toBe(true);

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

test("measurement commit compensates the local anchor before paint", async ({ page }) => {
  await page.goto("/harness.html?variableHeights=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 1_000);

  const container = page.locator("[data-testid=timeline-view]");
  // Enter free-scroll mid-history. A bare scrollTop write stays live-edge and
  // snaps back (see the prepend anchor test); a wheel marks it user-driven.
  await container.evaluate((node) => {
    node.scrollTop = 20_000;
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -50 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await waitAnimationFrames(page, 3);

  // Discover the first visible row (anchor) and the mounted row directly above
  // the viewport to grow, so the assertion is independent of measured heights.
  const targets = await container.evaluate((node) => {
    const containerTop = node.getBoundingClientRect().top;
    let anchorId: string | null = null;
    let growId: string | null = null;
    for (const frame of Array.from(
      node.querySelectorAll<HTMLElement>("[data-frame-item-id]")
    )) {
      const id = frame.dataset["frameItemId"] ?? null;
      if (!id) {
        continue;
      }
      if (frame.getBoundingClientRect().bottom > containerTop) {
        anchorId = id;
        break;
      }
      growId = id;
    }
    return { anchorId, growId };
  });
  expect(targets.growId).not.toBeNull();
  expect(targets.anchorId).not.toBeNull();

  const anchor = page.locator(`[data-item-id="${targets.anchorId}"]`);
  await expect(anchor).toBeVisible();
  const beforeTop = await anchor.evaluate((node) => node.getBoundingClientRect().top);

  await container.evaluate((node) => {
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -4 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.evaluate(() => window.__harness.resetScrollDiagnostics());

  // A cumulative flush can belong to initial virtualization, not this resize.
  // Read geometry on the actual grown row's native, before-paint notification.
  const probe = await page.evaluateHandle(({ growId, anchorId }) => {
    const grown = document.querySelector<HTMLElement>(`[data-frame-item-id="${growId}"]`);
    const target = document.querySelector<HTMLElement>(`[data-item-id="${anchorId}"]`);
    if (!grown || !target) throw new Error("resize probe target missing");
    const beforeHeight = grown.getBoundingClientRect().height;
    const result = { top: null as number | null, disconnect: () => observer.disconnect() };
    const observer = new ResizeObserver(() => {
      // The row's minimum height can absorb part of the pseudo-element.
      if (grown.getBoundingClientRect().height <= beforeHeight + 1) return;
      result.top = target.getBoundingClientRect().top;
      observer.disconnect();
    });
    observer.observe(grown);
    const style = document.createElement("style");
    style.textContent = `[data-frame-item-id="${growId}"] .message-body::after {
      content: ""; display: block; block-size: 96px;
    }`;
    document.head.append(style);
    return result;
  }, targets);
  try {
    await page.screenshot({ path: test.info().outputPath("measurement-frame.png") });
    // Wait only for notification, never for geometry to eventually converge.
    await expect.poll(() => probe.evaluate(value => value.top !== null)).toBe(true);
    const afterTop = await probe.evaluate(value => value.top!);
    expect(Math.abs(afterTop - beforeTop)).toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
  } finally {
    await probe.evaluate(value => value.disconnect());
    await probe.dispose();
  }
  // Secondary coverage: the actual commit must record its changed rows.
  // These counters never gate the before-paint geometry sample.
  const diagnostics = await page.evaluate(() => window.__harness.scrollDiagnostics());
  expect(diagnostics?.heightModelCommits ?? 0).toBeGreaterThan(0);
  expect(diagnostics?.changedMeasuredRows ?? 0).toBeGreaterThan(0);
});

test("active upward input keeps the anchor stable when prepend arrives", async ({ page }) => {
  await page.goto("/harness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await page.addStyleTag({
    path: DESKTOP_STYLES_PATH
  });
  await waitAnimationFrames(page, 1);
  await pushInitialTimelineItems(page, 100);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(100);
  await container.evaluate((node) => {
    node.scrollTop = 1_500;
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -40 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });

  const anchorBefore = await container.evaluate((node) => {
    const top = node.getBoundingClientRect().top;
    for (const row of node.querySelectorAll<HTMLElement>("[data-item-id]")) {
      if (row.getBoundingClientRect().bottom > top) {
        return {
          itemId: row.dataset["itemId"] ?? "",
          offsetTop: row.getBoundingClientRect().top - top
        };
      }
    }
    return null;
  });
  expect(anchorBefore).not.toBeNull();

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
      items: Array.from({ length: 4 }, (_, index) =>
        makeImageItem(`$deferred-old${index}`)
      )
    }
  );

  await waitAnimationFrames(page, 1);
  await expect
    .poll(() => container.locator('[data-item-id^="$deferred-old"]').count())
    .toBe(4);

  const anchorAfter = await container.evaluate((node, anchorId) => {
    const row = node.querySelector<HTMLElement>(`[data-item-id="${anchorId}"]`);
    return row
      ? row.getBoundingClientRect().top - node.getBoundingClientRect().top
      : null;
  }, anchorBefore!.itemId);
  expect(anchorAfter).not.toBeNull();
  expect(Math.abs(anchorAfter! - anchorBefore!.offsetTop)).toBeLessThanOrEqual(
    ANCHOR_PIXEL_TOLERANCE
  );
});

test("active mixed large prepend preserves the pre-apply virtual anchor", async ({ page }) => {
  await page.goto("/harness.html?variableHeights=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await page.addStyleTag({ path: DESKTOP_STYLES_PATH });
  await waitAnimationFrames(page, 1);
  await pushInitialTimelineItems(page, 1_000);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container).toHaveAttribute("data-virtualized", "true");
  await container.evaluate((node) => {
    node.scrollTop = 20_000;
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -80 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await waitAnimationFrames(page, 3);

  const anchorBefore = await container.evaluate((node) => {
    node.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -4 }));
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
    const top = node.getBoundingClientRect().top;
    for (const row of node.querySelectorAll<HTMLElement>("[data-item-id]")) {
      if (row.getBoundingClientRect().bottom > top) {
        return {
          itemId: row.dataset["itemId"] ?? "",
          offsetTop: row.getBoundingClientRect().top - top
        };
      }
    }
    return null;
  });
  expect(anchorBefore).not.toBeNull();

  // After 100 PushFront operations, original row $m800 is at index 900.
  // Hiding that far-below row makes this a mixed (not pure) projection without
  // changing viewport content, so the active branch must use its pre-apply anchor.
  await page.evaluate(
    ({ key, items, hiddenItem }) => {
      window.__harness.pushCoreEvent({
        kind: "Timeline",
        event: {
          ItemsUpdated: {
            key,
            generation: 1,
            batch_id: 1,
            diffs: [
              ...items.map((item) => ({ PushFront: { item } })),
              { Set: { index: 900, item: hiddenItem } }
            ]
          }
        }
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
      } as any);
    },
    {
      key: timelineKey(),
      items: Array.from({ length: 100 }, (_, index) =>
        index % 2 === 0
          ? makeImageItem(`$mixed-old${index}`)
          : makeItem(`$mixed-old${index}`, `mixed old ${index}`)
      ),
      hiddenItem: { ...makeItem("$m800", "message 800"), is_hidden: true }
    }
  );

  await expect(container).toHaveAttribute("data-total-items", "1099");
  await expect
    .poll(async () => {
      const offset = await container.evaluate((node, anchorId) => {
        const row = node.querySelector<HTMLElement>(`[data-item-id="${anchorId}"]`);
        return row
          ? row.getBoundingClientRect().top - node.getBoundingClientRect().top
          : null;
      }, anchorBefore!.itemId);
      return offset === null ? Number.POSITIVE_INFINITY : Math.abs(offset - anchorBefore!.offsetTop);
    })
    .toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
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

test("timeline navigation keeps the unread marker and bottom control independent", async ({
  page
}) => {
  await page.goto("/harness.html?autoLoadOlderMessages=true");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 30);

  const container = page.locator("[data-testid=timeline-view]");
  await expect(container.locator("[data-item-id]")).toHaveCount(30);

  await container.evaluate((node) => {
    node.scrollTop = 0;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
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
              local_viewed_event_id: "$m24",
              server_confirmed_read_event_id: "$m24",
              read_state_sync: "synced",
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

  await expect(page.getByRole("button", { name: "Jump to first unread, 3 unread" })).toHaveCount(0);

  await page.getByRole("button", { name: "Jump to bottom, 5 new messages" }).click();
  await expect
    .poll(() =>
      container.evaluate((node) =>
        Math.abs(node.scrollHeight - node.clientHeight - node.scrollTop)
      )
    )
    .toBeLessThanOrEqual(ANCHOR_PIXEL_TOLERANCE);
});

test("scrolling to bottom reports the latest readable event to Rust", async ({ page }) => {
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
      page.evaluate(
        () => window.__harness.invocationsOf("observe_timeline_viewport").at(-1)?.args
      )
    )
    .toMatchObject({
      roomId: ROOM_ID,
      lastVisibleEventId: "$m39",
      atBottom: true
    });
  expect(await page.evaluate(() => window.__harness.invocationsOf("send_read_receipt").length)).toBe(0);
  expect(await page.evaluate(() => window.__harness.invocationsOf("set_fully_read").length)).toBe(0);
});

test("timeline header omits date jump and keeps the latest control", async ({ page }) => {
  await page.goto("/appHarness.html");
  await page.waitForSelector("[data-testid=timeline-view]");
  await pushInitialTimelineItems(page, 8);

  await expect(page.getByRole("button", { name: "Jump to date" })).toHaveCount(0);
  await expect(
    page
      .getByRole("navigation", { name: "Timeline navigation" })
      .getByRole("button", { name: "Latest", exact: true })
  ).toBeVisible();
});
