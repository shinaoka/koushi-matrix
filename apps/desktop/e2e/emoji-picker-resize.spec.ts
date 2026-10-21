/**
 * Headless spec: emoji picker frame resizing (#1522, #1583).
 *
 * #1522 made one placement-following corner resize the picker. #1583 replaces
 * that single corner with the whole frame: every edge resizes its own
 * dimension, every corner resizes both, and the opposite edge stays put so the
 * drag is never fought by a re-anchored panel or a placement flip.
 *
 * The frame is verified the way a pointer sees it — `elementFromPoint` on each
 * edge and corner, the computed cursor per zone, and real mouse drags measured
 * on the rendered box — from the composer (opens above, inline-start), from
 * reaction menus on another user's and on the user's own message (opens below,
 * inline-end), and under RTL.
 */

import { expect, test, type Page } from "@playwright/test";
import { t } from "../src/i18n/messages";
import { seedTimelineItems } from "./support/basicOperations";

const VIEWPORT = { width: 1280, height: 1000 };
const TOLERANCE_PX = 1;
const RESIZE_KEY_STEP = 20;

type FrameZone =
  | "top"
  | "right"
  | "bottom"
  | "left"
  | "top-left"
  | "top-right"
  | "bottom-right"
  | "bottom-left";

interface PickerBox {
  bottom: number;
  height: number;
  left: number;
  right: number;
  top: number;
  width: number;
}

const CURSOR_BY_ZONE: Record<FrameZone, string> = {
  top: "ns-resize",
  bottom: "ns-resize",
  left: "ew-resize",
  right: "ew-resize",
  "top-left": "nwse-resize",
  "bottom-right": "nwse-resize",
  "top-right": "nesw-resize",
  "bottom-left": "nesw-resize"
};

const FRAME_ZONES = Object.keys(CURSOR_BY_ZONE) as FrameZone[];

function emojiPicker(page: Page) {
  return page.getByRole("dialog", { name: t("composer.emoji"), exact: true });
}

function frameZone(page: Page, zone: FrameZone) {
  return emojiPicker(page).locator(`[data-emoji-picker-resize="${zone}"]`);
}

function expectNear(actual: number, expected: number, what: string): void {
  expect(
    Math.abs(actual - expected),
    `${what}: expected ${expected} ±${TOLERANCE_PX}, got ${actual}`
  ).toBeLessThanOrEqual(TOLERANCE_PX);
}

async function settle(page: Page): Promise<void> {
  await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
}

async function pickerBox(page: Page): Promise<PickerBox> {
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

async function gotoReadyShell(page: Page): Promise<void> {
  await page.goto("/appHarness.html");
  await expect(page.getByRole("main", { name: t("timeline.conversation") })).toBeVisible();
}

async function openComposerPicker(page: Page): Promise<void> {
  await page.getByRole("button", { name: t("composer.emoji"), exact: true }).click();
  await expect(emojiPicker(page)).toBeVisible();
}

/** Hit-test every edge and corner the way a pointer enters the frame. */
async function frameHitZones(page: Page): Promise<Record<string, string | null>> {
  return emojiPicker(page).evaluate((element) => {
    const rect = element.getBoundingClientRect();
    // 4px inside the frame for edges, the corner square's centre for corners.
    const probes: Record<string, [number, number]> = {
      top: [rect.left + rect.width / 2, rect.top + 4],
      right: [rect.right - 4, rect.top + rect.height / 2],
      bottom: [rect.left + rect.width / 2, rect.bottom - 4],
      left: [rect.left + 4, rect.top + rect.height / 2],
      "top-left": [rect.left + 7, rect.top + 7],
      "top-right": [rect.right - 7, rect.top + 7],
      "bottom-right": [rect.right - 7, rect.bottom - 7],
      "bottom-left": [rect.left + 7, rect.bottom - 7]
    };
    const zones: Record<string, string | null> = {};
    for (const [zone, [x, y]] of Object.entries(probes)) {
      const hit = document.elementFromPoint(x, y);
      zones[zone] =
        hit?.closest("[data-emoji-picker-resize]")?.getAttribute("data-emoji-picker-resize") ??
        null;
    }
    return zones;
  });
}

async function frameCursors(page: Page): Promise<Record<string, string>> {
  return emojiPicker(page).evaluate((element) =>
    Object.fromEntries(
      [...element.querySelectorAll<HTMLElement>("[data-emoji-picker-resize]")].map((handle) => [
        handle.dataset.emojiPickerResize ?? "",
        getComputedStyle(handle).cursor
      ])
    )
  );
}

/** The whole frame is a resize target, and the picker's own controls are not. */
async function expectFrameIsResizable(page: Page): Promise<void> {
  const zones = await frameHitZones(page);
  for (const zone of FRAME_ZONES) {
    expect(zones[zone], `${zone} frame zone receives the pointer`).toBe(zone);
  }
  await expect.poll(() => frameCursors(page)).toEqual(CURSOR_BY_ZONE);

  // The frame is only the panel border: search, close, tabs, the grid and the
  // scrolling body stay the topmost elements at their own centres.
  const probes = await emojiPicker(page).evaluate((element) => {
    const nodes = [
      element.querySelector<HTMLElement>(".emoji-picker-search input"),
      element.querySelector<HTMLElement>(".emoji-picker-header .icon-button"),
      element.querySelector<HTMLElement>(".emoji-picker-tab"),
      element.querySelector<HTMLElement>(".emoji-picker-item"),
      element.querySelector<HTMLElement>(".emoji-picker-body")
    ].filter((node): node is HTMLElement => node != null);
    return nodes.map((node) => {
      const rect = node.getBoundingClientRect();
      const hit = document.elementFromPoint(
        rect.left + rect.width / 2,
        rect.top + rect.height / 2
      );
      return {
        covers: hit != null && node.contains(hit),
        resizeHandle: hit?.closest("[data-emoji-picker-resize]") != null
      };
    });
  });
  expect(probes.length).toBe(5);
  for (const probe of probes) {
    expect(probe.covers).toBe(true);
    expect(probe.resizeHandle).toBe(false);
  }
}

async function dragBy(page: Page, x: number, y: number, dx: number, dy: number): Promise<void> {
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y + dy, { steps: 6 });
  await page.mouse.up();
  await settle(page);
}

async function dragZone(page: Page, zone: FrameZone, dx: number, dy: number): Promise<void> {
  const box = (await frameZone(page, zone).boundingBox())!;
  await dragBy(page, box.x + box.width / 2, box.y + box.height / 2, dx, dy);
}

async function dragAndMeasure(page: Page, zone: FrameZone, dx: number, dy: number) {
  const before = await pickerBox(page);
  await dragZone(page, zone, dx, dy);
  return { after: await pickerBox(page), before };
}

function savedPickerSize(page: Page) {
  return page.evaluate(
    () => window.__harness.currentSnapshot().state.domain.settings.values.window.emoji_picker_size
  );
}

test("every edge and corner of the frame resizes from the composer picker", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await gotoReadyShell(page);
  await openComposerPicker(page);
  await expectFrameIsResizable(page);

  // Horizontal edges resize the height and leave the opposite edge in place.
  const top = await dragAndMeasure(page, "top", 0, -40);
  expectNear(top.after.height, top.before.height + 40, "top edge height");
  expectNear(top.after.bottom, top.before.bottom, "top edge keeps the bottom edge");
  expectNear(top.after.width, top.before.width, "top edge keeps the width");

  const bottom = await dragAndMeasure(page, "bottom", 0, -40);
  expectNear(bottom.after.height, bottom.before.height - 40, "bottom edge height");
  expectNear(bottom.after.top, bottom.before.top, "bottom edge keeps the top edge");

  // Vertical edges resize the width and leave the opposite edge in place.
  const left = await dragAndMeasure(page, "left", -40, 0);
  expectNear(left.after.width, left.before.width + 40, "left edge width");
  expectNear(left.after.right, left.before.right, "left edge keeps the right edge");
  expectNear(left.after.height, left.before.height, "left edge keeps the height");

  const right = await dragAndMeasure(page, "right", -40, 0);
  expectNear(right.after.width, right.before.width - 40, "right edge width");
  expectNear(right.after.left, right.before.left, "right edge keeps the left edge");

  // Corners resize both dimensions and pin the opposite corner.
  const topLeft = await dragAndMeasure(page, "top-left", -30, -30);
  expectNear(topLeft.after.width, topLeft.before.width + 30, "top-left width");
  expectNear(topLeft.after.height, topLeft.before.height + 30, "top-left height");
  expectNear(topLeft.after.right, topLeft.before.right, "top-left keeps the right edge");
  expectNear(topLeft.after.bottom, topLeft.before.bottom, "top-left keeps the bottom edge");

  const bottomRight = await dragAndMeasure(page, "bottom-right", -30, -30);
  expectNear(bottomRight.after.width, bottomRight.before.width - 30, "bottom-right width");
  expectNear(bottomRight.after.height, bottomRight.before.height - 30, "bottom-right height");
  expectNear(bottomRight.after.left, bottomRight.before.left, "bottom-right keeps the left edge");
  expectNear(bottomRight.after.top, bottomRight.before.top, "bottom-right keeps the top edge");

  const topRight = await dragAndMeasure(page, "top-right", 30, -30);
  expectNear(topRight.after.width, topRight.before.width + 30, "top-right width");
  expectNear(topRight.after.height, topRight.before.height + 30, "top-right height");
  expectNear(topRight.after.left, topRight.before.left, "top-right keeps the left edge");
  expectNear(topRight.after.bottom, topRight.before.bottom, "top-right keeps the bottom edge");

  const bottomLeft = await dragAndMeasure(page, "bottom-left", 30, -30);
  expectNear(bottomLeft.after.width, bottomLeft.before.width - 30, "bottom-left width");
  expectNear(bottomLeft.after.height, bottomLeft.before.height - 30, "bottom-left height");
  expectNear(bottomLeft.after.right, bottomLeft.before.right, "bottom-left keeps the right edge");
  expectNear(bottomLeft.after.top, bottomLeft.before.top, "bottom-left keeps the top edge");

  // A drag that starts inside the picker body scrolls or selects; it must not
  // resize the panel.
  const beforeBody = await pickerBox(page);
  const body = (await emojiPicker(page).locator(".emoji-picker-body").boundingBox())!;
  await dragBy(page, body.x + body.width / 2, body.y + body.height / 2, -60, -60);
  const afterBody = await pickerBox(page);
  expectNear(afterBody.width, beforeBody.width, "body drag keeps the width");
  expectNear(afterBody.height, beforeBody.height, "body drag keeps the height");
});

test("the focused frame handle resizes with the keyboard and the size survives reopen", async ({
  page
}) => {
  await page.setViewportSize(VIEWPORT);
  await gotoReadyShell(page);
  await openComposerPicker(page);

  // Exactly one frame handle is in the tab order: the marked one.
  const handles = emojiPicker(page).locator("[data-emoji-picker-resize]");
  await expect(handles).toHaveCount(8);
  const focusable = emojiPicker(page).getByRole("button", { name: t("composer.emojiResize") });
  await expect(focusable).toHaveCount(1);

  const zone = (await focusable.getAttribute("data-emoji-picker-resize")) as FrameZone;
  const horizontal = zone.includes("left") ? -1 : 1;
  const vertical = zone.includes("top") ? -1 : 1;
  const before = await pickerBox(page);
  await focusable.focus();
  await expect(focusable).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowDown");
  await expect.poll(async () => (await pickerBox(page)).width).toBe(before.width + horizontal * RESIZE_KEY_STEP);
  await expect.poll(async () => (await pickerBox(page)).height).toBe(before.height + vertical * RESIZE_KEY_STEP);
  await expect.poll(() => savedPickerSize(page)).toEqual([
    before.width + horizontal * RESIZE_KEY_STEP,
    before.height + vertical * RESIZE_KEY_STEP
  ]);

  // A pointer drag commits the chosen size through the Rust-owned preference,
  // and reopening restores it.
  await dragZone(page, "top-left", -100, -60);
  const resized = await pickerBox(page);
  await expect.poll(() => savedPickerSize(page)).toEqual([resized.width, resized.height]);
  await page.keyboard.press("Escape");
  await expect(emojiPicker(page)).not.toBeVisible();
  await openComposerPicker(page);
  expectNear((await pickerBox(page)).width, resized.width, "reopened width");
  expectNear((await pickerBox(page)).height, resized.height, "reopened height");

  // A small window clamps the rendered panel without replacing that size.
  await page.setViewportSize({ width: 700, height: 650 });
  await expect
    .poll(async () => {
      const box = await pickerBox(page);
      return box.left >= 16 && box.top >= 16 && box.right <= 684 && box.bottom <= 634;
    })
    .toBe(true);
  await page.setViewportSize(VIEWPORT);
  await expect.poll(async () => (await pickerBox(page)).width).toBe(resized.width);
  await expect.poll(async () => (await pickerBox(page)).height).toBe(resized.height);
});

test("reaction pickers on another user's and the user's own message share the frame", async ({
  page
}) => {
  await page.setViewportSize(VIEWPORT);
  await gotoReadyShell(page);
  await seedTimelineItems(page, [
    {
      id: { Event: { event_id: "$remote-frame:example.invalid" } },
      sender: "@remote-user:example.invalid",
      sender_label: "Remote User",
      body: "Remote reaction target",
      timestamp_ms: 1_800_000_000_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: false,
      is_edited: false,
      can_edit: false,
      reactions: []
    },
    {
      id: { Event: { event_id: "$own-frame:example.invalid" } },
      sender: "@harness-user:example.invalid",
      sender_label: "Harness User",
      body: "Own reaction target",
      timestamp_ms: 1_800_000_060_000,
      in_reply_to_event_id: null,
      thread_root: null,
      thread_summary: null,
      can_react: true,
      is_redacted: false,
      is_hidden: false,
      can_redact: true,
      is_edited: false,
      can_edit: true,
      reactions: []
    }
  ]);

  for (const eventId of ["$remote-frame:example.invalid", "$own-frame:example.invalid"]) {
    const row = page.locator(`[data-event-id="${eventId}"]`);
    await row.hover();
    await row.getByRole("button", { name: t("timeline.addReaction") }).click();
    await expect(emojiPicker(page)).toBeVisible();
    // Placement flips for a trigger near the window bottom, so the frame must
    // not depend on which corner the anchor is on.
    await expectFrameIsResizable(page);

    const top = await dragAndMeasure(page, "top", 0, -40);
    expectNear(top.after.height, top.before.height + 40, `${eventId} top edge height`);
    expectNear(top.after.bottom, top.before.bottom, `${eventId} top edge keeps the bottom edge`);

    const right = await dragAndMeasure(page, "right", 40, 0);
    expectNear(right.after.width, right.before.width + 40, `${eventId} right edge width`);
    expectNear(right.after.left, right.before.left, `${eventId} right edge keeps the left edge`);

    const corner = await dragAndMeasure(page, "bottom-left", -30, -30);
    expectNear(corner.after.width, corner.before.width + 30, `${eventId} corner width`);
    expectNear(corner.after.height, corner.before.height - 30, `${eventId} corner height`);
    expectNear(corner.after.right, corner.before.right, `${eventId} corner keeps the right edge`);
    expectNear(corner.after.top, corner.before.top, `${eventId} corner keeps the top edge`);

    await page.keyboard.press("Escape");
    await expect(emojiPicker(page)).not.toBeVisible();
  }
});

test("RTL keeps the physical frame zones and drag directions", async ({ page }) => {
  await page.setViewportSize(VIEWPORT);
  await gotoReadyShell(page);
  // Root `dir` is Rust-owned locale profile output; the picker only reads it.
  await page.evaluate(() => {
    document.documentElement.dir = "rtl";
  });
  await openComposerPicker(page);
  await expectFrameIsResizable(page);

  // Physical edges, not logical ones: the left strip grows the panel leftward
  // and pins the right edge.
  const left = await dragAndMeasure(page, "left", -40, 0);
  expectNear(left.after.width, left.before.width + 40, "RTL left edge width");
  expectNear(left.after.right, left.before.right, "RTL left edge keeps the right edge");

  const bottomRight = await dragAndMeasure(page, "bottom-right", 30, -30);
  expectNear(bottomRight.after.width, bottomRight.before.width + 30, "RTL bottom-right width");
  expectNear(bottomRight.after.height, bottomRight.before.height - 30, "RTL bottom-right height");
  expectNear(bottomRight.after.left, bottomRight.before.left, "RTL bottom-right keeps the left edge");
  expectNear(bottomRight.after.top, bottomRight.before.top, "RTL bottom-right keeps the top edge");

  const topLeft = await dragAndMeasure(page, "top-left", -30, 30);
  expectNear(topLeft.after.width, topLeft.before.width + 30, "RTL top-left width");
  expectNear(topLeft.after.height, topLeft.before.height - 30, "RTL top-left height");
  expectNear(topLeft.after.right, topLeft.before.right, "RTL top-left keeps the right edge");
  expectNear(topLeft.after.bottom, topLeft.before.bottom, "RTL top-left keeps the bottom edge");
});
