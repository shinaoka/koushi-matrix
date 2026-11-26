/**
 * #2136: the "Some messages haven't been sent" bar is docked over the bottom of
 * the timeline, outside the scroll content. It must be in view in a long room
 * without scrolling, and showing or hiding it must not move the reader's row.
 */

import { expect, test, type Page } from "@playwright/test";
import { roomTimelineKey } from "../src/domain/coreEvents";

const ROOM_ID = "!harness-room:example.invalid";
const ACCOUNT = "@harness-user:example.invalid";
const SEED_EVENT_ID = "$seed-event:example.invalid";
const KEY = roomTimelineKey(ACCOUNT, ROOM_ID);
const COUNT = 240;

function eventItem(index: number) {
  return {
    id: { Event: { event_id: `$unsent-bar-${String(index).padStart(4, "0")}:example.invalid` } },
    sender: ACCOUNT,
    sender_label: "Harness User",
    body: `unsent bar message ${index}\nThis row has stable text height.`,
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

const failedItem = () => ({
  ...eventItem(COUNT),
  id: { Transaction: { transaction_id: "unsent-bar-txn" } },
  send_state: { kind: "notSent", reason: "recoverable" }
});

async function push(page: Page, event: unknown): Promise<void> {
  await page.evaluate(async (e) => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    await window.__harness.pushCoreEvent({ kind: "Timeline", event: e } as any);
  }, event);
}

async function seed(page: Page, withFailed: boolean): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.locator(`[data-event-id="${SEED_EVENT_ID}"]`)).toBeVisible();
  const items = Array.from({ length: COUNT }, (_, i) => eventItem(i));
  if (withFailed) items.push(failedItem() as never);
  await expect
    .poll(async () => {
      await push(page, { InitialItems: { request_id: null, key: KEY, generation: 2, items } });
      await page.waitForTimeout(50);
      return page.locator("[data-testid=timeline-view] [data-item-id]").count();
    })
    .toBeGreaterThan(0);
}

test("the unsent bar is in view in a long room, outside the scroller", async ({ page }) => {
  await seed(page, true);
  const bar = page.getByTestId("timeline-send-bar");
  await expect(bar).toBeInViewport({ ratio: 1 });
  const geometry = await page.evaluate(() => {
    const scroller = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const bar = document.querySelector<HTMLElement>("[data-testid=timeline-send-bar]")!;
    const s = scroller.getBoundingClientRect();
    const b = bar.getBoundingClientRect();
    return { inside: scroller.contains(bar), barBottom: b.bottom, scrollerBottom: s.bottom, barTop: b.top, scrollerTop: s.top };
  });
  expect(geometry.inside).toBe(false);
  expect(geometry.barBottom).toBeLessThanOrEqual(geometry.scrollerBottom);
  expect(geometry.barTop).toBeGreaterThan(geometry.scrollerTop);
  // Scrolling the room to the top leaves it where it is.
  await page.evaluate(() => {
    const c = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    c.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -120 }));
    c.scrollTop = 0;
    c.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await expect(bar).toBeInViewport({ ratio: 1 });
});

test("showing and hiding the unsent bar does not move the reader's row", async ({ page }) => {
  await seed(page, false);
  await expect(page.getByTestId("timeline-send-bar")).toHaveCount(0);
  // Park the reader in the middle of the room and pick the row at the top edge.
  await page.evaluate(() => {
    const c = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    c.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -120 }));
    c.scrollTop = Math.floor(c.scrollHeight / 2);
    c.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(300);
  const read = () =>
    page.evaluate(() => {
      const c = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
      const cr = c.getBoundingClientRect();
      const row = Array.from(c.querySelectorAll<HTMLElement>("[data-event-id]")).find(
        (r) => r.getBoundingClientRect().top >= cr.top + 40
      )!;
      return { id: row.getAttribute("data-event-id")!, top: row.getBoundingClientRect().top - cr.top, height: c.clientHeight };
    });
  const before = await read();
  const offsetOf = (id: string) =>
    page.evaluate((eid) => {
      const c = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
      const row = c.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(eid)}"]`);
      return row ? row.getBoundingClientRect().top - c.getBoundingClientRect().top : null;
    }, id);

  // The reader's own echo would pull the view to the live edge, so the failed
  // row is another sender's, far from the reader: only the bar can move them.
  await push(page, {
    ItemsUpdated: { key: KEY, generation: 2, batch_id: 1, diffs: [{ Set: { index: 5, item: { ...failedItem(), sender: "@other:example.invalid", body: eventItem(5).body } } }] }
  });
  await expect(page.getByTestId("timeline-send-bar")).toBeInViewport();
  await page.waitForTimeout(300);
  expect(await offsetOf(before.id)).toBeCloseTo(before.top, 0);
  expect((await read()).height).toBe(before.height);

  await push(page, {
    ItemsUpdated: { key: KEY, generation: 2, batch_id: 2, diffs: [{ Set: { index: 5, item: eventItem(5) } }] }
  });
  await expect(page.getByTestId("timeline-send-bar")).toHaveCount(0);
  await page.waitForTimeout(300);
  expect(await offsetOf(before.id)).toBeCloseTo(before.top, 0);
  expect((await read()).height).toBe(before.height);
});

test("the failed newest row's own Resend and Delete are not covered by the bar", async ({ page }) => {
  await seed(page, true);
  await expect(page.getByTestId("timeline-send-bar")).toBeInViewport({ ratio: 1 });
  // Live edge: scroll the room fully down. The bar must not sit over the row.
  await page.evaluate(() => {
    const c = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    c.scrollTop = c.scrollHeight;
  });
  await page.waitForTimeout(300);
  const row = page.locator('[data-testid=timeline-view] [data-item-id]').last();
  for (const name of ["Resend", "Delete"]) {
    const button = row.getByRole("button", { name, exact: true });
    await expect(button).toBeVisible();
    const hit = await button.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      return top === el || el.contains(top);
    });
    expect(hit, `${name} is hit-testable`).toBe(true);
  }
  // The spacer tracks the bar's measured footprint and leaves the pin intact.
  const m = await page.evaluate(() => {
    const c = document.querySelector<HTMLElement>("[data-testid=timeline-view]")!;
    const b = document.querySelector<HTMLElement>("[data-testid=timeline-send-bar]")!;
    const sp = document.querySelector<HTMLElement>("[data-testid=timeline-unsent-bar-clearance]")!;
    return {
      spacer: sp.getBoundingClientRect().height,
      footprint: c.getBoundingClientRect().bottom - b.getBoundingClientRect().top,
      gap: c.scrollHeight - c.clientHeight - c.scrollTop
    };
  });
  expect(m.spacer).toBeGreaterThanOrEqual(m.footprint);
  expect(m.gap).toBeLessThanOrEqual(1);
});
