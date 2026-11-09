/**
 * Headless spec: the saved picker size wins over the anchor's room (#1839).
 *
 * The picker used to draw `min(saved size, space the anchor leaves)`, so a
 * reaction picker's height changed with the message's position on screen. The
 * window is now the only cap: the panel draws the saved size, the emoji grid
 * scrolls inside it, and the search field stays put. A message near the window
 * edge moves the panel by flipping or shifting it, never outside the viewport.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { seedTimelineItems } from "./support/basicOperations";

const VIEWPORT = { width: 1280, height: 1000 };
const WINDOW_MARGIN = 16;
const SAVED: [number, number] = [700, 900];
const TOLERANCE_PX = 1;

interface Box {
  bottom: number;
  height: number;
  left: number;
  right: number;
  top: number;
  width: number;
}

function emojiPicker(page: Page) {
  return page.getByRole("dialog", { name: t("composer.emoji"), exact: true });
}

async function savedSize(page: Page): Promise<[number, number] | null> {
  return page.evaluate(
    () => window.__harness.currentSnapshot().state.domain.settings.values.window.emoji_picker_size
  );
}

async function setSavedSize(page: Page, size: [number, number]): Promise<void> {
  await expect
    .poll(async () => {
      await page.evaluate((next) => {
        const snapshot = structuredClone(window.__harness.currentSnapshot());
        snapshot.state.domain.settings.values.window.emoji_picker_size = next;
        window.__harness.setSnapshot(snapshot);
        window.__harness.pushStateUpdate();
      }, size);
      return savedSize(page);
    })
    .toEqual(size);
}

/**
 * Spread messages down the timeline and answer the row whose box sits nearest
 * a vertical position, so a test can pin the picker's anchor mid-window or at
 * an edge.
 */
async function seedRows(page: Page, count: number): Promise<void> {
  await seedTimelineItems(
    page,
    Array.from({ length: count }, (_, index) => ({
      id: { Event: { event_id: `$saved-size-${index}:example.invalid` } },
      sender: "@remote-user:example.invalid",
      sender_label: "Remote User",
      body: `Saved-size target ${index}`,
      timestamp_ms: 1_800_000_000_000 + index * 60_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: [],
      actions: {
        can_copy: false,
        can_forward: false,
        can_reply: false,
        can_pin: true,
        can_permalink: false,
        can_view_source: false
      }
    }))
  );
}

/** Event id of the row whose centre is nearest `targetY`. */
interface RowBox {
  bottom: number;
  center: number;
  eventId: string;
  top: number;
}

/** The seeded message rows, top to bottom. */
async function rows(page: Page): Promise<RowBox[]> {
  const measured = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("[data-event-id]")]
      .filter((element) => element.dataset.eventId?.startsWith("$saved-size-"))
      .map((element) => {
        const rect = element.getBoundingClientRect();
        return {
          bottom: rect.bottom,
          center: rect.top + rect.height / 2,
          eventId: element.dataset.eventId as string,
          top: rect.top
        };
      })
      .sort((a, b) => a.center - b.center)
  );
  expect(measured.length, "seeded message rows").toBeGreaterThan(2);
  return measured;
}

async function openReactionPicker(page: Page, eventId: string): Promise<void> {
  const row = page.locator(`[data-event-id="${eventId}"]`);
  await row.hover();
  await row.getByRole("button", { name: t("timeline.addReaction") }).click();
  await expect(emojiPicker(page)).toBeVisible();
}

async function pickerBox(page: Page): Promise<Box> {
  const box = (await emojiPicker(page).boundingBox())!;
  return {
    bottom: box.y + box.height,
    height: box.height,
    left: box.x,
    right: box.x + box.width,
    top: box.y,
    width: box.width
  };
}

function expectNear(actual: number, expected: number, what: string): void {
  expect(
    Math.abs(actual - expected),
    `${what}: expected ${expected} ±${TOLERANCE_PX}, got ${actual}`
  ).toBeLessThanOrEqual(TOLERANCE_PX);
}

function expectInsideWindow(box: Box, what: string): void {
  expect(box.left, `${what} left`).toBeGreaterThanOrEqual(WINDOW_MARGIN - TOLERANCE_PX);
  expect(box.top, `${what} top`).toBeGreaterThanOrEqual(WINDOW_MARGIN - TOLERANCE_PX);
  expect(box.right, `${what} right`).toBeLessThanOrEqual(
    VIEWPORT.width - WINDOW_MARGIN + TOLERANCE_PX
  );
  expect(box.bottom, `${what} bottom`).toBeLessThanOrEqual(
    VIEWPORT.height - WINDOW_MARGIN + TOLERANCE_PX
  );
}

test("a reaction picker mid-timeline draws the saved size and scrolls inside it", async ({
  page
}) => {
  await page.setViewportSize(VIEWPORT);
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  await setSavedSize(page, SAVED);
  await seedRows(page, 12);

  // A message away from the window edges: the anchor's room is incidental, the
  // saved size is what must be drawn.
  const middle = (await rows(page))[6];
  expect(middle.center, "chosen row sits mid-timeline").toBeGreaterThan(300);
  await openReactionPicker(page, middle.eventId);

  const box = await pickerBox(page);
  expectNear(box.width, SAVED[0], "drawn width");
  expectNear(box.height, SAVED[1], "drawn height");
  expectInsideWindow(box, "mid-timeline picker");
  // The window holds the saved size here, so nothing is being drawn smaller and
  // the #1838 marker must stay away.
  await expect(emojiPicker(page).locator(".emoji-picker-clip")).toHaveCount(0);
  expect(await emojiPicker(page).getAttribute("aria-describedby")).toBeNull();

  // The grid scrolls inside the panel, so the whole emoji catalogue is
  // reachable at a size the window can hold.
  const body = emojiPicker(page).locator(".emoji-picker-body");
  const scroll = await body.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight
  }));
  expect(scroll.clientHeight, "body is drawn").toBeGreaterThan(0);
  expect(scroll.scrollHeight, "body has more emoji than fit").toBeGreaterThan(
    scroll.clientHeight
  );

  // The search field stays visible and in place while the grid scrolls.
  const search = emojiPicker(page).locator(".emoji-picker-search input");
  await expect(search).toBeVisible();
  const headerTop = (await emojiPicker(page).locator(".emoji-picker-header").boundingBox())!.y;
  await body.evaluate((element) => {
    element.scrollTop = 240;
  });
  expectNear(
    (await emojiPicker(page).locator(".emoji-picker-header").boundingBox())!.y,
    headerTop,
    "header stays put while the body scrolls"
  );
  const searchBox = (await search.boundingBox())!;
  expect(searchBox.y).toBeGreaterThanOrEqual(box.top);
  expect(searchBox.y + searchBox.height).toBeLessThanOrEqual(box.bottom);

  // Scrolling reached further down the catalogue rather than resizing anything.
  expect(await body.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  const afterScroll = await pickerBox(page);
  expectNear(afterScroll.height, box.height, "scrolling keeps the drawn height");

  // The region is a tab stop whose arrow keys scroll it; the marked frame
  // handle keeps its arrows for resizing (emoji-picker-resize.spec.ts).
  await body.evaluate((element) => {
    element.scrollTop = 0;
    element.focus();
  });
  await page.keyboard.press("ArrowDown");
  // Chromium animates keyboard scrolling, so the offset is polled.
  await expect
    .poll(() => body.evaluate((element) => element.scrollTop), { intervals: [50, 100, 250] })
    .toBeGreaterThan(0);
});

test("a picker larger than the window is capped by the window and never drawn outside it", async ({
  page
}) => {
  await page.setViewportSize(VIEWPORT);
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  await setSavedSize(page, [SAVED[0], 1400]);
  await seedRows(page, 12);

  const measured = await rows(page);
  const extremes = [measured[0], measured[measured.length - 1]];
  // The composer sits below the timeline, so the lowest reachable anchor is the
  // last message row rather than the window's bottom edge: sweep both reachable
  // extremes instead of pretending to hit a window corner.
  expect(extremes[0].center, "highest reachable anchor").toBeLessThan(120);
  expect(extremes[1].center, "lowest reachable anchor").toBeGreaterThan(700);

  for (const row of extremes) {
    await openReactionPicker(page, row.eventId);

    const box = await pickerBox(page);
    expectInsideWindow(box, `picker at ${row.eventId}`);
    // Only the window narrows the saved 1400px: it draws the usable height and
    // reports that, instead of shrinking to the room beside the anchor.
    expectNear(box.width, SAVED[0], `picker width at ${row.eventId}`);
    expectNear(box.height, VIEWPORT.height - 2 * WINDOW_MARGIN, `picker height at ${row.eventId}`);

    // The dialog says why it is smaller, and the header shows the marker, only
    // because the window cannot hold the saved size.
    expect(
      await emojiPicker(page).evaluate((element) => {
        const id = element.getAttribute("aria-describedby");
        return id ? element.ownerDocument.getElementById(id)?.textContent ?? null : null;
      }),
      `clip description at ${row.eventId}`
    ).toContain(`700×${VIEWPORT.height - 2 * WINDOW_MARGIN}`);
    await expect(emojiPicker(page).locator(".emoji-picker-clip")).toHaveCount(1);

    await page.keyboard.press("Escape");
    await expect(emojiPicker(page)).not.toBeVisible();
  }
});

test("a picker that fits on one side of its trigger opens on that side", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
  // Shorter than a comfortable popup but taller than the room under some rows:
  // the panel fits whole above its trigger and not below it.
  const saved: [number, number] = [SAVED[0], 480];
  await setSavedSize(page, saved);
  await seedRows(page, 12);

  // Pick the row the flip is about: the saved box fits in the room above it and
  // not in the room below it, so only a flip can draw the whole panel without
  // covering the message the picker was opened from.
  const measured = await rows(page);
  const row = measured.find(
    (candidate) =>
      candidate.top - 2 * WINDOW_MARGIN >= saved[1] &&
      VIEWPORT.height - WINDOW_MARGIN - candidate.bottom < saved[1]
  );
  expect(row, "a row the saved box only fits above").toBeTruthy();

  await openReactionPicker(page, (row as RowBox).eventId);

  const box = await pickerBox(page);
  expectNear(box.width, saved[0], "drawn width");
  expectNear(box.height, saved[1], "drawn height");
  expectInsideWindow(box, "flipped picker");
  // The panel is drawn above the trigger row instead of being clamped back down
  // over it (#1840 review: flip by the drawn box, not by the anchor heuristic).
  expect(
    box.bottom,
    "the picker does not cover the message it was opened from"
  ).toBeLessThanOrEqual((row as RowBox).top + TOLERANCE_PX);
  await expect(emojiPicker(page).locator(".emoji-picker-clip")).toHaveCount(0);
});
